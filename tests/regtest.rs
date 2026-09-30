//! End-to-end test of the non-custodial API flow against a live regtest node, using the real
//! `server` binary. Needs the node from docker-compose.yml, so it's ignored by default:
//!
//!     docker compose up -d
//!     cargo test -- --ignored

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::str::FromStr;
use std::time::{Duration, Instant};

use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client, RpcApi};
use bdk_wallet::bitcoin::{Address, Amount, Psbt, Txid};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::Segwitv0;
use bdk_wallet::{KeychainKind, Wallet};
use bitcoin_wallet::api::{BroadcastRequest, PsbtRequest, RegisterRequest};
use bitcoin_wallet::client::{ApiClient, ApiError};
use bitcoin_wallet::{NETWORK, keys, send, wallet_id};
use serde_json::Value;

const RPC_URL: &str = "http://127.0.0.1:18443";
const FUNDER: &str = "itest-funder";

#[test]
#[ignore = "needs the regtest node: docker compose up -d"]
fn non_custodial_send_flow() {
    let funder = funder_wallet();

    // The client side: a fresh wallet whose keys never leave this test.
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English)).unwrap();
    let mnemonic = mnemonic.into_key();
    let (ext, int) = keys::descriptors(&mnemonic).unwrap();
    let local = Wallet::create(ext, int).network(NETWORK).create_wallet_no_persist().unwrap();
    let external = local.public_descriptor(KeychainKind::External).to_string();
    let internal = local.public_descriptor(KeychainKind::Internal).to_string();

    let server = Server::start();
    let api = ApiClient::new(&server.url);

    // Register: the server gets public descriptors only, and computes the same id we do.
    let id = api.register(&RegisterRequest { external: external.clone(), internal: internal.clone() })
        .unwrap()
        .id;
    assert_eq!(id, wallet_id(&external, &internal));
    let wallet_url = format!("{}/wallets/{id}", server.url);

    // Receive 1 BTC and confirm it.
    let address = http("POST", &format!("{wallet_url}/addresses"))["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let address = Address::from_str(&address).unwrap().require_network(NETWORK).unwrap();
    funder.send_to_address(&address, Amount::ONE_BTC, None, None, None, None, None, None).unwrap();
    mine(&funder, 1);
    assert_eq!(http("GET", &format!("{wallet_url}/balance"))["confirmed_sat"], 100_000_000);

    // Send 0.3 BTC: server builds, we review and sign, server broadcasts.
    let dest = funder.get_new_address(None, None).unwrap().require_network(NETWORK).unwrap();
    let amount = Amount::from_sat(30_000_000);
    let request = PsbtRequest {
        address: dest.to_string(),
        amount_sat: amount.to_sat(),
        fee_rate_sat_vb: Some(2),
    };
    let built = api.build_psbt(&id, &request).unwrap();
    let mut psbt = Psbt::from_str(&built.psbt).unwrap();

    let review = send::review(&local, &psbt, &dest, amount).unwrap();
    assert_eq!(review.fee.to_sat(), built.fee_sat, "server's fee claim matches the PSBT");

    // An unsigned PSBT must be refused.
    let unsigned = BroadcastRequest { psbt: psbt.to_string() };
    assert_eq!(api_status(api.broadcast(&id, &unsigned)), 400);

    send::sign(&local, &mnemonic, &mut psbt).unwrap();
    let signed = BroadcastRequest { psbt: psbt.to_string() };
    let txid = Txid::from_str(&api.broadcast(&id, &signed).unwrap().txid).unwrap();
    assert!(funder.get_raw_mempool().unwrap().contains(&txid), "tx reached Core's mempool");

    // Confirmation tracking.
    let tx_url = format!("{wallet_url}/transactions/{txid}");
    assert_eq!(http("GET", &tx_url)["confirmations"], 0);
    mine(&funder, 1);
    let tx = http("GET", &tx_url);
    assert_eq!(tx["confirmations"], 1);
    assert_eq!(tx["net_sat"], -((amount + review.fee).to_sat() as i64));

    // Broadcasting it again: Core refuses (outputs already exist), which is the client's
    // problem, so 400 rather than 5xx.
    assert_eq!(api_status(api.broadcast(&id, &signed)), 400);
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
fn http(method: &str, url: &str) -> Value {
    let response = match method {
        "GET" => ureq::get(url).call(),
        _ => ureq::post(url).send_empty(),
    };
    response.unwrap().body_mut().read_json().unwrap()
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
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let data_dir = std::env::temp_dir().join(format!("bitcoin-wallet-itest-{port}"));
        let child = Command::new(env!("CARGO_BIN_EXE_server"))
            .arg("--data-dir")
            .arg(&data_dir)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
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
