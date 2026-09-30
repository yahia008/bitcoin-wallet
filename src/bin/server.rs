//! Watch-only HTTP API. It can see the wallet (addresses, balance, history) but never touches
//! private keys: signing stays with the client.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use bdk_wallet::PersistedWallet;
use bdk_wallet::rusqlite::Connection;
use bitcoin_wallet::{chain, load};
use clap::Parser;
use serde::Serialize;

#[derive(Parser)]
#[command(about = "Watch-only wallet HTTP API")]
struct Args {
    /// Wallet database to serve
    #[arg(long, default_value = "wallet.sqlite")]
    db: PathBuf,

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

struct WalletState {
    conn: Connection,
    wallet: PersistedWallet<Connection>,
    rpc: chain::Client,
}

/// One wallet, one lock: requests that touch the wallet run one at a time.
type AppState = Arc<Mutex<WalletState>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let (conn, wallet) = load(&args.db)?;
    let rpc = chain::connect(&args.rpc_url, &args.rpc_user, &args.rpc_pass)?;
    let state: AppState = Arc::new(Mutex::new(WalletState { conn, wallet, rpc }));

    let app = Router::new()
        .route("/health", get(health))
        .route("/balance", get(balance))
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

/// All amounts are integer satoshis: JSON numbers are floats in JavaScript, and floats
/// can't represent most BTC decimals exactly.
#[derive(Serialize)]
struct BalanceResponse {
    confirmed_sat: u64,
    unconfirmed_sat: u64,
    immature_sat: u64,
    total_sat: u64,
}

async fn balance(State(state): State<AppState>) -> Result<Json<BalanceResponse>, ApiError> {
    let balance = with_wallet(state, |s| {
        chain::sync(&mut s.wallet, &mut s.conn, &s.rpc)?;
        Ok(s.wallet.balance())
    })
    .await?;

    Ok(Json(BalanceResponse {
        confirmed_sat: balance.confirmed.to_sat(),
        // trusted_pending = our own unconfirmed change; untrusted_pending = incoming from others.
        unconfirmed_sat: (balance.trusted_pending + balance.untrusted_pending).to_sat(),
        immature_sat: balance.immature.to_sat(),
        total_sat: balance.total().to_sat(),
    }))
}

/// Runs `f` with exclusive access to the wallet on a blocking thread. BDK and the RPC client do
/// blocking disk and network I/O, which must not run on the async runtime's worker threads.
async fn with_wallet<T: Send + 'static>(
    state: AppState,
    f: impl FnOnce(&mut WalletState) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    let result = tokio::task::spawn_blocking(move || {
        let mut guard = state.lock().map_err(|_| anyhow!("wallet state lock poisoned"))?;
        f(&mut guard)
    })
    .await
    .map_err(|e| anyhow!("wallet task failed: {e}"))?;
    Ok(result?)
}

/// Turns any error into `500 {"error": "..."}`.
struct ApiError(anyhow::Error);

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(e)
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody { error: format!("{:#}", self.0) };
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}
