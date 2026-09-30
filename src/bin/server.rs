//! Watch-only HTTP API. It can see wallets (addresses, balance, history) but never holds
//! private keys: clients register public descriptors only, and signing stays with the client.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bdk_wallet::bitcoin::Txid;
use bdk_wallet::bitcoin::hashes::{Hash, sha256};
use bdk_wallet::bitcoin::secp256k1::Secp256k1;
use bdk_wallet::miniscript::Descriptor;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, PersistedWallet, Wallet};
use bitcoin_wallet::{NETWORK, chain, history, load};
use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(about = "Watch-only wallet HTTP API")]
struct Args {
    /// Directory holding one watch-only database per registered wallet
    #[arg(long, env = "DATA_DIR", default_value = "server-data")]
    data_dir: PathBuf,

    /// Address to listen on. Keep it on localhost unless it's behind TLS and auth.
    #[arg(long, env = "LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,

    /// Bitcoin Core RPC URL
    #[arg(long, env = "RPC_URL", default_value = "http://127.0.0.1:18443")]
    rpc_url: String,

    /// Bitcoin Core RPC username
    #[arg(long, env = "RPC_USER", default_value = "wallet")]
    rpc_user: String,

    /// Bitcoin Core RPC password
    #[arg(long, env = "RPC_PASS", default_value = "wallet", hide_default_value = true)]
    rpc_pass: String,
}

struct WalletEntry {
    conn: Connection,
    wallet: PersistedWallet<Connection>,
}

struct AppState {
    data_dir: PathBuf,
    rpc: chain::Client,
    /// Wallets opened so far. Each has its own lock, so requests for different wallets
    /// don't wait on each other; requests for the same wallet run one at a time.
    wallets: Mutex<HashMap<String, Arc<Mutex<WalletEntry>>>>,
}

type SharedState = Arc<AppState>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    std::fs::create_dir_all(&args.data_dir)?;
    let rpc = chain::connect(&args.rpc_url, &args.rpc_user, &args.rpc_pass)?;
    let state = Arc::new(AppState {
        data_dir: args.data_dir,
        rpc,
        wallets: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/wallets", post(register_wallet))
        .route("/wallets/{id}/balance", get(balance))
        .route("/wallets/{id}/addresses", get(list_addresses).post(new_address))
        .route("/wallets/{id}/transactions", get(list_transactions))
        .route("/wallets/{id}/transactions/{txid}", get(get_transaction))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    println!("Listening on http://{}", args.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

#[derive(Deserialize)]
struct RegisterRequest {
    /// Receive descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../0/*)`
    external: String,
    /// Change descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../1/*)`
    internal: String,
}

#[derive(Serialize)]
struct RegisterResponse {
    id: String,
}

/// Registers a watch-only wallet. Idempotent: the id is derived from the descriptors, so
/// registering the same wallet again returns the same id with 200 instead of 201.
async fn register_wallet(
    State(state): State<SharedState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    let external = public_descriptor(&req.external)?;
    let internal = public_descriptor(&req.internal)?;
    if external == internal {
        return Err(ApiError::bad_request("external and internal descriptors must differ"));
    }

    // Hash the parsed (canonical) form, so formatting differences map to the same wallet.
    let hash = sha256::Hash::hash(format!("{external}\n{internal}").as_bytes());
    let id = hash.to_string()[..16].to_owned();

    run_blocking(move || {
        let db = state.db_path(&id);
        if db.exists() {
            return Ok((StatusCode::OK, Json(RegisterResponse { id })));
        }
        let mut conn = Connection::open(&db)?;
        let created = Wallet::create(external, internal)
            .network(NETWORK)
            .create_wallet(&mut conn);
        if let Err(e) = created {
            drop(conn);
            let _ = std::fs::remove_file(&db);
            // e.g. mainnet xpub on a regtest server.
            return Err(ApiError::bad_request(format!("invalid wallet: {e}")));
        }
        Ok((StatusCode::CREATED, Json(RegisterResponse { id })))
    })
    .await
}

/// Parses a descriptor and rejects it if it contains any private key.
fn public_descriptor(s: &str) -> Result<String, ApiError> {
    let (descriptor, secret_keys) = Descriptor::parse_descriptor(&Secp256k1::new(), s.trim())
        .map_err(|e| ApiError::bad_request(format!("invalid descriptor: {e}")))?;
    if !secret_keys.is_empty() {
        // Deliberately don't echo the input back: it contains a private key.
        return Err(ApiError::bad_request(
            "descriptor contains a private key; send only public (tpub/xpub) descriptors",
        ));
    }
    Ok(descriptor.to_string())
}

/// All amounts are integer satoshis: JSON numbers are floats in JavaScript, and floats
/// can't represent most BTC decimals exactly.
#[derive(Serialize)]
struct BalanceResponse {
    confirmed_sat: u64,
    unconfirmed_sat: u64,
    immature_sat: u64,
    total_sat: u64,
}

async fn balance(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<BalanceResponse>, ApiError> {
    let balance = with_wallet(state, id, |w| Ok(w.wallet.balance())).await?;
    Ok(Json(BalanceResponse {
        confirmed_sat: balance.confirmed.to_sat(),
        // trusted_pending = our own unconfirmed change; untrusted_pending = incoming from others.
        unconfirmed_sat: (balance.trusted_pending + balance.untrusted_pending).to_sat(),
        immature_sat: balance.immature.to_sat(),
        total_sat: balance.total().to_sat(),
    }))
}

#[derive(Serialize)]
struct AddressResponse {
    index: u32,
    address: String,
    used: bool,
}

/// Hands out a fresh receive address and records that its index is now in use.
async fn new_address(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<AddressResponse>), ApiError> {
    // Sync first so indexes already paid to on chain count as used; otherwise a fresh or
    // stale wallet would hand out an address that was already used (address reuse).
    let info = with_wallet(state, id, |w| {
        let info = w.wallet.reveal_next_address(KeychainKind::External);
        w.wallet.persist(&mut w.conn).map_err(anyhow::Error::from)?;
        Ok(info)
    })
    .await?;
    let response =
        AddressResponse { index: info.index, address: info.address.to_string(), used: false };
    Ok((StatusCode::CREATED, Json(response)))
}

async fn list_addresses(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AddressResponse>>, ApiError> {
    let addresses =
        with_wallet(state, id, |w| Ok(history::receive_addresses(&w.wallet))).await?;
    Ok(Json(
        addresses
            .into_iter()
            .map(|a| AddressResponse { index: a.index, address: a.address.to_string(), used: a.used })
            .collect(),
    ))
}

#[derive(Serialize)]
struct TransactionResponse {
    txid: String,
    /// Effect on the balance: positive = received, negative = sent (fee included).
    net_sat: i64,
    /// null unless this wallet paid the fee.
    fee_sat: Option<u64>,
    confirmed: bool,
    confirmations: u32,
    block_height: Option<u32>,
}

impl From<history::TxSummary> for TransactionResponse {
    fn from(tx: history::TxSummary) -> Self {
        TransactionResponse {
            txid: tx.txid.to_string(),
            net_sat: tx.net.to_sat(),
            fee_sat: tx.fee.map(|fee| fee.to_sat()),
            confirmed: tx.block_height.is_some(),
            confirmations: tx.confirmations,
            block_height: tx.block_height,
        }
    }
}

async fn list_transactions(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let txs = with_wallet(state, id, |w| Ok(history::transactions(&w.wallet))).await?;
    Ok(Json(txs.into_iter().map(Into::into).collect()))
}

/// Poll this to track confirmations after broadcasting.
async fn get_transaction(
    State(state): State<SharedState>,
    Path((id, txid)): Path<(String, String)>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let txid = Txid::from_str(&txid).map_err(|_| ApiError::bad_request("invalid txid"))?;
    let tx = with_wallet(state, id, move |w| {
        history::transaction(&w.wallet, txid).ok_or_else(|| {
            ApiError::not_found("transaction not found: not a wallet transaction, or dropped from the mempool")
        })
    })
    .await?;
    Ok(Json(tx.into()))
}

impl AppState {
    fn db_path(&self, id: &str) -> PathBuf {
        self.data_dir.join(format!("{id}.sqlite"))
    }

    /// Returns the wallet with this id, opening it from disk on first use.
    fn wallet(&self, id: &str) -> Result<Arc<Mutex<WalletEntry>>, ApiError> {
        // The id ends up in a file path, so only accept exactly what we generate (16 hex
        // chars). This blocks path traversal like `../../somewhere`.
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::not_found("wallet not found"));
        }

        let mut wallets = self.wallets.lock().map_err(|_| anyhow!("wallet map lock poisoned"))?;
        if let Some(entry) = wallets.get(id) {
            return Ok(entry.clone());
        }
        let db = self.db_path(id);
        if !db.exists() {
            return Err(ApiError::not_found("wallet not found"));
        }
        let (conn, wallet) = load(&db)?;
        let entry = Arc::new(Mutex::new(WalletEntry { conn, wallet }));
        wallets.insert(id.to_owned(), entry.clone());
        Ok(entry)
    }
}

/// Syncs wallet `id` with Bitcoin Core, then runs `f` with exclusive access to it on a
/// blocking thread.
async fn with_wallet<T: Send + 'static>(
    state: SharedState,
    id: String,
    f: impl FnOnce(&mut WalletEntry) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    run_blocking(move || {
        let entry = state.wallet(&id)?;
        let mut entry = entry.lock().map_err(|_| anyhow!("wallet lock poisoned"))?;
        let WalletEntry { conn, wallet } = &mut *entry;
        chain::sync(wallet, conn, &state.rpc)?;
        f(&mut entry)
    })
    .await
}

/// Runs blocking work (BDK, SQLite, Bitcoin Core RPC) off the async runtime's worker threads.
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow!("wallet task failed: {e}"))?
}

/// An error returned to the client as `{"error": "..."}` with an HTTP status.
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    fn not_found(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::NOT_FOUND, message: message.into() }
    }
}

/// Unexpected failures become 500s.
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("{e:#}") }
    }
}

impl From<bdk_wallet::rusqlite::Error> for ApiError {
    fn from(e: bdk_wallet::rusqlite::Error) -> Self {
        anyhow::Error::from(e).into()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.message })).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::KeychainKind;
    use bdk_wallet::keys::bip39::Mnemonic;
    use bitcoin_wallet::keys;

    fn mnemonic() -> Mnemonic {
        Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    #[test]
    fn accepts_public_descriptor() {
        let (public, _) = keys::derive(&mnemonic(), KeychainKind::External).unwrap();
        assert!(public_descriptor(&public.to_string()).is_ok());
    }

    #[test]
    fn rejects_private_descriptor() {
        let (with_secret, _) = keys::descriptors(&mnemonic()).unwrap();
        assert!(with_secret.contains("tprv"));
        let err = public_descriptor(&with_secret).err().unwrap();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(!err.message.contains("tprv"), "must not echo the private key back");
    }

    #[test]
    fn rejects_garbage() {
        let err = public_descriptor("wpkh(not-a-key)").err().unwrap();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}
