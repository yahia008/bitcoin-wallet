//! End-to-end test of the non-custodial API flow against a live regtest node, using the real
//! `server` binary. Needs the node from docker-compose.yml, so it's ignored by default:
//!
//!     docker compose up -d bitcoind
//!     cargo test -- --ignored

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::str::FromStr;
use std::time::{Duration, Instant};

use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client, RpcApi};
use bdk_wallet::bitcoin::{Address, Amount, Network, Psbt, Txid};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::Segwitv0;
use bdk_wallet::{KeychainKind, Wallet};
use bitcoin_wallet::api::{BroadcastRequest, PsbtRequest, RegisterRequest};
use bitcoin_wallet::client::{ApiClient, ApiError};
use bitcoin_wallet::{keys, send, wallet_id};
use serde_json::{Value, json};
use wallet_core::ownership;

const RPC_URL: &str = "http://127.0.0.1:18443";
const FUNDER: &str = "itest-funder";

#[test]
#[ignore = "needs the regtest node: docker compose up -d bitcoind"]
fn non_custodial_send_flow() {
    let funder = funder_wallet();

    // The client side: a fresh wallet whose keys never leave this test.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English)).unwrap();
    let mnemonic = mnemonic.into_key();
    let account_key = keys::account_key(&mnemonic).unwrap();
    let (ext, int) = keys::descriptors(&account_key).unwrap();
    let local = Wallet::create(ext, int).network(Network::Regtest).create_wallet_no_persist().unwrap();
    let external = local.public_descriptor(KeychainKind::External).to_string();
    let internal = local.public_descriptor(KeychainKind::Internal).to_string();

    let server = Server::start();

    // Register: the server gets public descriptors only, and computes the same id we do.
    let register =
        RegisterRequest { external: external.clone(), internal: internal.clone(), birthday: 0 };
    let registration = ApiClient::new(&server.url).register(&register).unwrap();
    let (id, token) = (registration.id, registration.token);
    assert_eq!(id, wallet_id(&external, &internal));
    let api = ApiClient::new(&server.url).with_token(&token);
    let wallet_url = format!("{}/wallets/{id}", server.url);

    // Auth. The token is issued once: descriptors aren't secret, so registering again is 409.
    assert_eq!(api_status(ApiClient::new(&server.url).register(&register)), 409);
    let balance_url = format!("{wallet_url}/balance");
    assert_eq!(http_status("GET", &balance_url, None), 401, "no token");
    assert_eq!(http_status("GET", &balance_url, Some(&"00".repeat(32))), 401, "wrong token");
    let other = other_wallet_token(&server.url);
    assert_eq!(http_status("GET", &balance_url, Some(&other)), 401, "another wallet's token");

    // Receive 1 BTC and confirm it.
    let address = http("POST", &format!("{wallet_url}/addresses"), &token)["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let address = Address::from_str(&address).unwrap().require_network(Network::Regtest).unwrap();
    funder.send_to_address(&address, Amount::ONE_BTC, None, None, None, None, None, None).unwrap();
    mine(&funder, 1);
    assert_eq!(http("GET", &format!("{wallet_url}/balance"), &token)["confirmed_sat"], 100_000_000);

    // Send 0.3 BTC: server builds, we review and sign, server broadcasts.
    let dest = funder.get_new_address(None, None).unwrap().require_network(Network::Regtest).unwrap();
    let amount = Amount::from_sat(30_000_000);
    let request = PsbtRequest {
        address: dest.to_string(),
        amount_sat: amount.to_sat(),
        fee_rate_sat_vb: Some(2),
        fee_priority: Default::default(),
        send_all: false,
    };
    let built = api.build_psbt(&id, &request).unwrap();
    let mut psbt = Psbt::from_str(&built.psbt).unwrap();

    let review = send::review(&local, &psbt, &dest, amount).unwrap();
    assert_eq!(review.fee.to_sat(), built.fee_sat, "server's fee claim matches the PSBT");

    // An unsigned PSBT must be refused.
    let unsigned = BroadcastRequest { psbt: psbt.to_string() };
    assert_eq!(api_status(api.broadcast(&id, &unsigned)), 400);

    send::sign(&local, &account_key, &mut psbt).unwrap();
    let signed = BroadcastRequest { psbt: psbt.to_string() };
    let txid = Txid::from_str(&api.broadcast(&id, &signed).unwrap().txid).unwrap();
    assert!(funder.get_raw_mempool().unwrap().contains(&txid), "tx reached Core's mempool");

    // Confirmation tracking.
    let tx_url = format!("{wallet_url}/transactions/{txid}");
    assert_eq!(http("GET", &tx_url, &token)["confirmations"], 0);
    mine(&funder, 1);
    let tx = http("GET", &tx_url, &token);
    assert_eq!(tx["confirmations"], 1);
    assert_eq!(tx["net_sat"], -((amount + review.fee).to_sat() as i64));

    // Details: our coin goes in, the payment goes out to someone else, the rest is change.
    assert!(tx["time"].as_u64().unwrap() > 0, "block time");
    assert!(tx["vsize"].as_u64().unwrap() > 0);
    assert!(tx["fee_rate_sat_vb"].as_f64().unwrap() >= 1.0);
    assert!(tx["inputs"].as_array().unwrap().iter().all(|i| i["owner"] != "external"));
    let outputs = tx["outputs"].as_array().unwrap();
    let payment = outputs.iter().find(|o| o["address"] == dest.to_string()).unwrap();
    assert_eq!(payment["owner"], "external");
    assert_eq!(payment["value_sat"], amount.to_sat());
    assert!(outputs.iter().any(|o| o["owner"] == "change"));

    // Broadcasting it again: Core refuses (outputs already exist), which is the client's
    // problem, so 400 rather than 5xx.
    assert_eq!(api_status(api.broadcast(&id, &signed)), 400);

    // Coin reservations. Fund a second coin so the wallet holds two (the 0.7 BTC change
    // plus this 1 BTC), then build two PSBTs before broadcasting either.
    funder.send_to_address(&address, Amount::ONE_BTC, None, None, None, None, None, None).unwrap();
    mine(&funder, 1);
    let first = Psbt::from_str(&api.build_psbt(&id, &request).unwrap().psbt).unwrap();
    let second = Psbt::from_str(&api.build_psbt(&id, &request).unwrap().psbt).unwrap();
    let coins = |p: &Psbt| p.unsigned_tx.input.iter().map(|i| i.previous_output).collect::<Vec<_>>();
    assert!(
        coins(&first).iter().all(|c| !coins(&second).contains(c)),
        "second PSBT must not reuse coins reserved by the first"
    );

    // Every coin is reserved now, so a third PSBT can't be built, and the error says why.
    let err = api.build_psbt(&id, &request).err().expect("all coins are reserved");
    assert!(format!("{err:#}").contains("reserved"), "unexpected error: {err:#}");

    // Both still broadcast: before reservations, the second would be a double-spend.
    for mut psbt in [first, second] {
        send::review(&local, &psbt, &dest, amount).unwrap();
        send::sign(&local, &account_key, &mut psbt).unwrap();
        api.broadcast(&id, &BroadcastRequest { psbt: psbt.to_string() }).unwrap();
    }
}

#[test]
#[ignore = "needs the regtest node: docker compose up -d bitcoind"]
fn registration_is_rate_limited() {
    let server = Server::start_with(&["--register-limit-per-hour", "2"]);
    let api = ApiClient::new(&server.url);
    let wallet = random_wallet();
    let request = RegisterRequest {
        external: wallet.public_descriptor(KeychainKind::External).to_string(),
        internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
        birthday: 0,
    };

    api.register(&request).unwrap();
    assert_eq!(api_status(api.register(&request)), 409, "2nd attempt still counts");
    assert_eq!(api_status(api.register(&request)), 429, "3rd attempt is over the limit");

    // Retry-After tells the client how long to wait (at most the 1 hour window).
    let response = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
        .post(&format!("{}/wallets", server.url))
        .send_json(&request)
        .unwrap();
    let retry_after: u64 = response.headers()["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((1..=3600).contains(&retry_after), "Retry-After = {retry_after}");

    // Other endpoints have their own, separate limit.
    assert!(ureq::get(&format!("{}/health", server.url)).call().is_ok());
}

#[test]
#[ignore = "needs the regtest node: docker compose up -d bitcoind"]
fn birthday_skips_earlier_blocks() {
    let funder = funder_wallet();
    let wallet = random_wallet();
    let address = wallet.peek_address(KeychainKind::External, 0).address;
    let txid =
        funder.send_to_address(&address, Amount::ONE_BTC, None, None, None, None, None, None).unwrap();
    mine(&funder, 2);
    let funded_at = funder.get_transaction(&txid, None).unwrap().info.blockheight.unwrap();

    let confirmed = |birthday: u32| {
        let server = Server::start();
        let request = RegisterRequest {
            external: wallet.public_descriptor(KeychainKind::External).to_string(),
            internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
            birthday,
        };
        let token = ApiClient::new(&server.url).register(&request).unwrap().token;
        let id = wallet_id(&request.external, &request.internal);
        http("GET", &format!("{}/wallets/{id}/balance", server.url), &token)["confirmed_sat"]
            .as_u64()
            .unwrap()
    };
    // A birthday after the funding block means that block is never scanned: the coin is
    // invisible. That's why a birthday that's too late is the dangerous mistake.
    assert_eq!(confirmed(funded_at + 1), 0);
    assert_eq!(confirmed(funded_at), 100_000_000);

    // A birthday past the chain tip is rejected up front.
    let server = Server::start();
    let tip = funder.get_block_count().unwrap() as u32;
    let wallet = random_wallet();
    let request = RegisterRequest {
        external: wallet.public_descriptor(KeychainKind::External).to_string(),
        internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
        birthday: tip + 1000,
    };
    assert_eq!(api_status(ApiClient::new(&server.url).register(&request)), 400);
}

#[test]
#[ignore = "needs the regtest node: docker compose up -d bitcoind"]
fn lost_token_is_recovered_by_proving_key_ownership() {
    let server = Server::start();
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English)).unwrap();
    let account_key = keys::account_key(&mnemonic.into_key()).unwrap();
    let (ext, int) = keys::descriptors(&account_key).unwrap();
    let wallet =
        Wallet::create(ext, int).network(Network::Regtest).create_wallet_no_persist().unwrap();
    let request = RegisterRequest {
        external: wallet.public_descriptor(KeychainKind::External).to_string(),
        internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
        birthday: 0,
    };
    let registered = ApiClient::new(&server.url).register(&request).unwrap();
    let old_token = registered.token;
    let wallet_url = format!("{}/wallets/{}", server.url, registered.id);
    let challenge = || {
        let response = post_json(&format!("{wallet_url}/challenge"), json!({})).unwrap();
        response["challenge"].as_str().unwrap().to_owned()
    };
    let recover = |challenge: &str, signature: &str| {
        post_json(
            &format!("{wallet_url}/token"),
            json!({ "challenge": challenge, "signature": signature }),
        )
    };

    // Someone without the key can't get a token: a signature by another wallet fails.
    let c = challenge();
    let other = Mnemonic::parse(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .unwrap();
    let other = keys::account_key(&other).unwrap();
    let forged = ownership::sign(&other, &registered.id, &c).unwrap();
    assert_eq!(recover(&c, &forged).unwrap_err(), 401);

    // The owner signs a fresh challenge and gets a new token.
    let c = challenge();
    let signature = ownership::sign(&account_key, &registered.id, &c).unwrap();
    let new_token = recover(&c, &signature).unwrap()["token"].as_str().unwrap().to_owned();
    // A challenge is good for one answer only.
    assert_eq!(recover(&c, &signature).unwrap_err(), 401);

    let balance_url = format!("{wallet_url}/balance");
    assert_eq!(http_status("GET", &balance_url, Some(&old_token)), 401, "old token revoked");
    assert_eq!(http("GET", &balance_url, &new_token)["total_sat"], 0);
}

/// A Core wallet with spendable coins to fund the test wallet from.
fn funder_wallet() -> Client {
    let auth = || Auth::UserPass("wallet".into(), "wallet".into());
    let node = Client::new(RPC_URL, auth()).expect("regtest node reachable");
    if !node.list_wallets().unwrap().contains(&FUNDER.to_owned())
        && node.load_wallet(FUNDER).is_err()
    {
        node.create_wallet(FUNDER, None, None, None, None).unwrap();
    }
    let funder = Client::new(&format!("{RPC_URL}/wallet/{FUNDER}"), auth()).unwrap();
    if funder.get_balance(None, None).unwrap() < Amount::from_btc(5.0).unwrap() {
        // Coinbase outputs need 100 confirmations before they can be spent.
        mine(&funder, 101);
    }
    funder
}

fn mine(funder: &Client, blocks: u64) {
    let to = funder.get_new_address(None, None).unwrap().assume_checked();
    funder.generate_to_address(blocks, &to).unwrap();
}

/// Minimal JSON request helper for endpoints `ApiClient` doesn't wrap.
fn http(method: &str, url: &str, token: &str) -> Value {
    request(method, url, Some(token)).unwrap().body_mut().read_json().unwrap()
}

/// The error status of a request that's expected to fail.
fn http_status(method: &str, url: &str, token: Option<&str>) -> u16 {
    match request(method, url, token) {
        Err(ureq::Error::StatusCode(status)) => status,
        other => panic!("expected an error status, got {other:?}"),
    }
}

fn request(
    method: &str,
    url: &str,
    token: Option<&str>,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let auth = token.map(|token| format!("Bearer {token}"));
    match method {
        "GET" => {
            let mut request = ureq::get(url);
            if let Some(auth) = &auth {
                request = request.header("Authorization", auth);
            }
            request.call()
        }
        _ => {
            let mut request = ureq::post(url);
            if let Some(auth) = &auth {
                request = request.header("Authorization", auth);
            }
            request.send_empty()
        }
    }
}

/// POSTs a JSON body; the parsed response, or the error status.
fn post_json(url: &str, body: Value) -> Result<Value, u16> {
    match ureq::post(url).send_json(&body) {
        Ok(mut response) => Ok(response.body_mut().read_json().unwrap()),
        Err(ureq::Error::StatusCode(status)) => Err(status),
        Err(e) => panic!("request to {url} failed: {e}"),
    }
}

/// A fresh in-memory wallet with random keys.
fn random_wallet() -> Wallet {
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English)).unwrap();
    let (ext, int) = keys::descriptors(&keys::account_key(&mnemonic.into_key()).unwrap()).unwrap();
    Wallet::create(ext, int).network(Network::Regtest).create_wallet_no_persist().unwrap()
}

/// Registers a second, unrelated wallet and returns its token.
fn other_wallet_token(server_url: &str) -> String {
    let wallet = random_wallet();
    let request = RegisterRequest {
        external: wallet.public_descriptor(KeychainKind::External).to_string(),
        internal: wallet.public_descriptor(KeychainKind::Internal).to_string(),
        birthday: 0,
    };
    ApiClient::new(server_url).register(&request).unwrap().token
}

fn api_status<T>(result: anyhow::Result<T>) -> u16 {
    let err = result.err().expect("expected an API error");
    err.downcast_ref::<ApiError>().expect("expected an HTTP error status").status
}

/// The real server binary on a free port with a throwaway data directory.
struct Server {
    url: String,
    child: Child,
    data_dir: PathBuf,
}

impl Server {
    fn start() -> Self {
        Self::start_with(&[])
    }

    fn start_with(extra_args: &[&str]) -> Self {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let data_dir = std::env::temp_dir().join(format!("bitcoin-wallet-itest-{port}"));
        let child = Command::new(env!("CARGO_BIN_EXE_server"))
            .arg("--data-dir")
            .arg(&data_dir)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
            // The tests mine blocks and expect the very next request to see them.
            .args(["--sync-interval-secs", "0"])
            .args(extra_args)
            .spawn()
            .unwrap();
        let server = Server { url: format!("http://127.0.0.1:{port}"), child, data_dir };

        let deadline = Instant::now() + Duration::from_secs(10);
        while ureq::get(&format!("{}/health", server.url)).call().is_err() {
            assert!(Instant::now() < deadline, "server didn't start");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}
