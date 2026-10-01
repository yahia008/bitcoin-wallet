//! Watch-only HTTP API. It can see wallets (addresses, balance, history) but never holds
//! private keys: clients register public descriptors only, and signing stays with the client.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Path, Request, State};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderValue, Method};
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bdk_wallet::bitcoin::{Address, Amount, Network, OutPoint, Psbt, Transaction, Txid};
use bdk_wallet::bitcoin::consensus::encode::serialize_hex;
use bdk_wallet::bitcoin::hashes::{Hash, sha256};
use bdk_wallet::bitcoin::hex::DisplayHex;
use bdk_wallet::bitcoin::secp256k1::Secp256k1;
use bdk_wallet::miniscript::Descriptor;
use bdk_wallet::rusqlite::{Connection, OptionalExtension, params};
use bdk_wallet::{KeychainKind, PersistedWallet, Wallet};
use bitcoin_wallet::api::{
    BroadcastRequest, BroadcastResponse, BumpRequest, BumpResponse, ChallengeResponse, ErrorBody,
    PsbtRequest, PsbtResponse, RegisterRequest, RegisterResponse, TokenRequest, TokenResponse,
};
use bitcoin_wallet::send::{self, BuildError, FeePriority};
use bitcoin_wallet::chain::{self, Backend, BackendError, ChainArgs};
use bitcoin_wallet::{history, load, wallet_id};
use chacha20poly1305::aead::Generate;
use clap::Parser;
use serde::Serialize;
use tower_http::cors::CorsLayer;
use wallet_core::ownership;

#[derive(Parser)]
#[command(about = "Watch-only wallet HTTP API")]
struct Args {
    /// Directory holding one watch-only database per registered wallet
    #[arg(long, env = "DATA_DIR", default_value = "server-data")]
    data_dir: PathBuf,

    /// Address to listen on. Keep it on localhost unless it's behind TLS and auth.
    #[arg(long, env = "LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,

    /// Use a separate data dir per network: wallets are tied to the one they registered on.
    #[command(flatten)]
    chain: ChainArgs,

    /// Web page origins allowed to call this API from a browser (CORS), comma-separated.
    /// The web wallet in web/ runs on port 3001 in development.
    #[arg(
        long,
        env = "CORS_ORIGINS",
        value_delimiter = ',',
        default_value = "http://localhost:3001,http://127.0.0.1:3001"
    )]
    cors_origins: Vec<HeaderValue>,

    /// A wallet synced this many seconds ago or less isn't synced again. Each sync costs
    /// dozens of requests to an Esplora backend (public ones rate-limit, e.g. blockstream.info
    /// at 700/hour), and a web page load asks for balance, history and an address at once.
    /// Blocks are ~10 minutes apart and our own broadcasts are recorded immediately, so a
    /// little staleness is harmless. 0 = sync on every request.
    #[arg(long, env = "SYNC_INTERVAL_SECS", default_value_t = 30)]
    sync_interval_secs: u64,

    /// Requests per minute allowed from one IP, across all endpoints
    #[arg(long, env = "RATE_LIMIT_PER_MINUTE", default_value_t = 60)]
    rate_limit_per_minute: u32,

    /// Registrations per hour allowed from one IP. Each one creates a database file.
    #[arg(long, env = "REGISTER_LIMIT_PER_HOUR", default_value_t = 5)]
    register_limit_per_hour: u32,
}

/// How long a token-recovery challenge can be answered.
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);

/// How long coins picked for a PSBT stay reserved if the client never broadcasts it.
const RESERVATION_TTL: Duration = Duration::from_secs(10 * 60);


struct WalletEntry {
    conn: Connection,
    wallet: PersistedWallet<Connection>,
    reserved: Reservations,
    /// SHA-256 of the wallet's API token. The token itself is never stored.
    token_hash: sha256::Hash,
    /// When the wallet was last synced with the chain; None until the first sync.
    last_sync: Option<Instant>,
}

/// Coins picked for PSBTs that were handed out but not broadcast yet. BDK only learns a coin
/// is spent once its transaction is broadcast, so without this two /psbt calls in a row would
/// pick the same coins and the second broadcast would be rejected as a double-spend.
///
/// Kept in memory only: after a restart the worst case is that old behavior, not lost funds.
#[derive(Default)]
struct Reservations(HashMap<OutPoint, Instant>);

impl Reservations {
    /// Forgets reservations that expired by `now` and returns the coins still reserved.
    fn active(&mut self, now: Instant) -> Vec<OutPoint> {
        self.0.retain(|_, expires| *expires > now);
        self.0.keys().copied().collect()
    }

    fn reserve(&mut self, coins: impl IntoIterator<Item = OutPoint>, until: Instant) {
        for coin in coins {
            self.0.insert(coin, until);
        }
    }

    fn release(&mut self, coins: impl IntoIterator<Item = OutPoint>) {
        for coin in coins {
            self.0.remove(&coin);
        }
    }
}

/// Counts requests per client IP in fixed time windows: at most `limit` requests per `window`,
/// then 429 until the window ends. Kept in memory; a restart just resets the counts.
///
/// Behind a reverse proxy every request would come from the proxy's IP, so this would need
/// to read `X-Forwarded-For` instead. The server only listens on localhost for now.
struct RateLimiter {
    limit: u32,
    window: Duration,
    /// Per IP: when its current window started, and requests made in it.
    clients: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl RateLimiter {
    /// Above this many tracked IPs, forget the ones whose window already ended.
    const PRUNE_AT: usize = 10_000;

    fn new(limit: u32, window: Duration) -> Self {
        RateLimiter { limit, window, clients: Mutex::new(HashMap::new()) }
    }

    /// Counts one request from `ip`. `Err` holds how long until it may try again.
    fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        // A panic elsewhere can't leave this map inconsistent, so a poisoned lock is fine.
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if clients.len() >= Self::PRUNE_AT {
            clients.retain(|_, (start, _)| now < *start + self.window);
        }

        let (start, count) = clients.entry(ip).or_insert((now, 0));
        if now >= *start + self.window {
            (*start, *count) = (now, 0);
        }
        if *count >= self.limit {
            return Err(*start + self.window - now);
        }
        *count += 1;
        Ok(())
    }
}

/// Middleware: rejects the request with 429 and `Retry-After` once its IP is over the limit.
async fn rate_limit(
    State(limiter): State<Arc<RateLimiter>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    match limiter.check(client.ip(), Instant::now()) {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            let seconds = wait.as_secs_f64().ceil().max(1.0) as u64;
            let mut response = ApiError {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: format!("too many requests; try again in {seconds} s"),
            }
            .into_response();
            response.headers_mut().insert(RETRY_AFTER, seconds.into());
            response
        }
    }
}

/// The coins a transaction spends.
fn spent_coins(tx: &Transaction) -> impl Iterator<Item = OutPoint> + '_ {
    tx.input.iter().map(|input| input.previous_output)
}

struct AppState {
    data_dir: PathBuf,
    network: Network,
    backend: Backend,
    sync_interval: Duration,
    /// Wallets opened so far. Each has its own lock, so requests for different wallets
    /// don't wait on each other; requests for the same wallet run one at a time.
    wallets: Mutex<HashMap<String, Arc<Mutex<WalletEntry>>>>,
    /// Outstanding token-recovery challenges: wallet id -> (challenge, valid until). One per
    /// wallet; asking again replaces it.
    challenges: Mutex<HashMap<String, (String, Instant)>>,
}

type SharedState = Arc<AppState>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    std::fs::create_dir_all(&args.data_dir)?;
    let backend = args.chain.connect()?;
    let state = Arc::new(AppState {
        data_dir: args.data_dir,
        network: args.chain.network,
        backend,
        sync_interval: Duration::from_secs(args.sync_interval_secs),
        wallets: Mutex::new(HashMap::new()),
        challenges: Mutex::new(HashMap::new()),
    });

    let per_ip = Arc::new(RateLimiter::new(args.rate_limit_per_minute, Duration::from_secs(60)));
    let registrations =
        Arc::new(RateLimiter::new(args.register_limit_per_hour, Duration::from_secs(60 * 60)));

    let app = Router::new()
        .route("/health", get(health))
        .route(
            "/wallets",
            post(register_wallet)
                .layer(middleware::from_fn_with_state(registrations.clone(), rate_limit)),
        )
        .route("/wallets/{id}/challenge", post(token_challenge))
        // Issuing tokens is as sensitive as registering, so it shares that stricter limit.
        .route(
            "/wallets/{id}/token",
            post(recover_token).layer(middleware::from_fn_with_state(registrations, rate_limit)),
        )
        .route("/wallets/{id}/balance", get(balance))
        .route("/wallets/{id}/addresses", get(list_addresses).post(new_address))
        .route("/wallets/{id}/transactions", get(list_transactions))
        .route("/wallets/{id}/transactions/{txid}", get(get_transaction))
        .route("/wallets/{id}/psbt", post(build_psbt))
        .route("/wallets/{id}/bump", post(bump_fee))
        .route("/wallets/{id}/broadcast", post(broadcast))
        .fallback(|| async { ApiError::not_found("no such endpoint") })
        .layer(middleware::from_fn_with_state(per_ip, rate_limit))
        // Outermost, so even rate-limit and error responses carry CORS headers and the browser
        // can read them. Only listed origins: any other site's scripts can't use the API.
        .layer(
            CorsLayer::new()
                .allow_origin(args.cors_origins)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE])
                .expose_headers([RETRY_AFTER]),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    println!("Listening on http://{} ({})", args.listen, args.chain.network);
    // connect_info lets handlers and middleware see the caller's address (for rate limiting).
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    /// Clients must use the same network, e.g. to parse addresses.
    network: String,
}

async fn health(State(state): State<SharedState>) -> Json<Health> {
    Json(Health { status: "ok", network: state.network.to_string() })
}

/// Registers a watch-only wallet and returns its API token. The token is shown only here, once:
/// descriptors aren't secret, so registering an existing wallet again gets 409, never a token.
async fn register_wallet(
    State(state): State<SharedState>,
    ApiJson(req): ApiJson<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    let external = public_descriptor(&req.external)?;
    let internal = public_descriptor(&req.internal)?;
    if external == internal {
        return Err(ApiError::bad_request("external and internal descriptors must differ"));
    }

    // Hash the parsed (canonical) form, so formatting differences map to the same wallet.
    let id = wallet_id(&external, &internal);
    let birthday = req.birthday;

    run_blocking(move || {
        chain::check_birthday(&state.backend, birthday).map_err(|e| match BackendError::find(&e) {
            Some(_) => ApiError::from(e),
            None => ApiError::bad_request(e.to_string()),
        })?;
        let db = state.db_path(&id);
        // create_new claims the file atomically, so two concurrent registrations can't both
        // win (and the loser's cleanup below only ever deletes a file it created).
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&db) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ApiError::conflict("wallet already registered"));
            }
            Err(e) => return Err(anyhow::Error::from(e).into()),
        }

        let token = new_token();
        let result = create_wallet(&db, state.network, external, internal, birthday, &token);
        if result.is_err() {
            let _ = std::fs::remove_file(&db);
        }
        result?;
        Ok((StatusCode::CREATED, Json(RegisterResponse { id, token })))
    })
    .await
}

/// Writes the wallet, its birthday and its token hash to `db` in one SQLite transaction.
fn create_wallet(
    db: &std::path::Path,
    network: Network,
    external: String,
    internal: String,
    birthday: u32,
    token: &str,
) -> Result<(), ApiError> {
    let mut conn = Connection::open(db)?;
    let mut tx = conn.transaction()?;
    Wallet::create(external, internal)
        .network(network)
        .create_wallet(&mut tx)
        // e.g. mainnet xpub on a testnet4 server.
        .map_err(|e| ApiError::bad_request(format!("invalid wallet: {e}")))?;
    chain::save_birthday(&tx, birthday)?;
    tx.execute(
        "CREATE TABLE api_token (id INTEGER PRIMARY KEY CHECK (id = 0), hash BLOB NOT NULL)",
        [],
    )?;
    tx.execute(
        "INSERT INTO api_token (id, hash) VALUES (0, ?1)",
        params![token_hash(token).as_byte_array()],
    )?;
    tx.commit()?;
    Ok(())
}

/// 256 random bits, hex-encoded.
fn new_token() -> String {
    <[u8; 32]>::generate().to_lower_hex_string()
}

/// Starts API token recovery, for a client that lost its token: returns a random one-time
/// challenge to sign with the wallet's key (`wallet_core::ownership`). Needs no token: the
/// signature, not the request, is what proves ownership.
async fn token_challenge(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    run_blocking(move || {
        state.wallet(&id)?; // 404 for wallets we don't know
        let challenge = new_token();
        let until = Instant::now() + CHALLENGE_TTL;
        state
            .challenges
            .lock()
            .map_err(|_| anyhow!("challenge map lock poisoned"))?
            .insert(id, (challenge.clone(), until));
        Ok(Json(ChallengeResponse { challenge, expires_in_secs: CHALLENGE_TTL.as_secs() }))
    })
    .await
}

/// Issues a new API token to a client that signed our challenge with the wallet's key. The
/// old token stops working, so a leaked one can be revoked this way too.
async fn recover_token(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<TokenRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    run_blocking(move || {
        // A challenge gets one answer, right or wrong, so signatures can't be retried
        // against it (or replayed later).
        let issued = state
            .challenges
            .lock()
            .map_err(|_| anyhow!("challenge map lock poisoned"))?
            .remove(&id);
        match issued {
            Some((challenge, until)) if challenge == req.challenge && Instant::now() < until => {}
            _ => {
                return Err(ApiError::unauthorized(
                    "unknown or expired challenge; request a new one",
                ));
            }
        }

        let entry = state.wallet(&id)?;
        let mut entry = entry.lock().map_err(|_| anyhow!("wallet lock poisoned"))?;
        let external = entry.wallet.public_descriptor(KeychainKind::External).clone();
        ownership::verify(&external, &id, &req.challenge, &req.signature)
            .map_err(|e| ApiError::unauthorized(format!("{e:#}")))?;

        let token = new_token();
        let hash = token_hash(&token);
        entry.conn.execute(
            "UPDATE api_token SET hash = ?1 WHERE id = 0",
            params![hash.as_byte_array()],
        )?;
        entry.token_hash = hash;
        Ok(Json(TokenResponse { token }))
    })
    .await
}

/// Tokens are 256 random bits, so unlike passwords there's nothing to brute-force and a fast
/// hash is enough. Storing only the hash means a leaked database doesn't leak working tokens.
fn token_hash(token: &str) -> sha256::Hash {
    sha256::Hash::hash(token.as_bytes())
}

/// The token from an `Authorization: Bearer <token>` header. Every per-wallet route takes one.
struct Bearer(String);

impl<S: Send + Sync> FromRequestParts<S> for Bearer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(|token| Bearer(token.trim().to_owned()))
            .ok_or_else(|| {
                ApiError::unauthorized("missing API token: send `Authorization: Bearer <token>`")
            })
    }
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
    token: Bearer,
) -> Result<Json<BalanceResponse>, ApiError> {
    let balance = with_wallet(state, id, token, |w, _| Ok(w.wallet.balance())).await?;
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
    token: Bearer,
) -> Result<(StatusCode, Json<AddressResponse>), ApiError> {
    // Sync first so indexes already paid to on chain count as used; otherwise a fresh or
    // stale wallet would hand out an address that was already used (address reuse).
    let info = with_wallet(state, id, token, |w, _| {
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
    token: Bearer,
) -> Result<Json<Vec<AddressResponse>>, ApiError> {
    let addresses =
        with_wallet(state, id, token, |w, _| Ok(history::receive_addresses(&w.wallet))).await?;
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
    token: Bearer,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let txs = with_wallet(state, id, token, |w, _| Ok(history::transactions(&w.wallet))).await?;
    Ok(Json(txs.into_iter().map(Into::into).collect()))
}

/// Poll this to track confirmations after broadcasting.
async fn get_transaction(
    State(state): State<SharedState>,
    Path((id, txid)): Path<(String, String)>,
    token: Bearer,
) -> Result<Json<TransactionResponse>, ApiError> {
    let txid = Txid::from_str(&txid).map_err(|_| ApiError::bad_request("invalid txid"))?;
    let tx = with_wallet(state, id, token, move |w, _| {
        history::transaction(&w.wallet, txid).ok_or_else(|| {
            ApiError::not_found("transaction not found: not a wallet transaction, or dropped from the mempool")
        })
    })
    .await?;
    Ok(Json(tx.into()))
}

/// Builds an UNSIGNED transaction. The server picks coins, fee and change, but can't sign:
/// the client must review the PSBT, sign it locally, and send it back to /broadcast.
async fn build_psbt(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    token: Bearer,
    ApiJson(req): ApiJson<PsbtRequest>,
) -> Result<Json<PsbtResponse>, ApiError> {
    let to = Address::from_str(&req.address)
        .map_err(|_| ApiError::bad_request("invalid address"))?
        .require_network(state.network)
        .map_err(|_| ApiError::bad_request("address is for a different network"))?;
    let amount = Amount::from_sat(req.amount_sat);

    let (draft, fee_rate) = with_wallet(state, id, token, move |w, backend| {
        let fee_rate = send::choose_fee_rate(backend, req.fee_rate_sat_vb, req.fee_priority)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        let now = Instant::now();
        let reserved = w.reserved.active(now);
        let draft = send::build(&mut w.wallet, &to, amount, fee_rate, &reserved)
            .map_err(|e| build_error(e, &reserved))?;
        w.reserved.reserve(spent_coins(&draft.psbt.unsigned_tx), now + RESERVATION_TTL);
        Ok((draft, fee_rate))
    })
    .await?;

    Ok(Json(PsbtResponse {
        psbt: draft.psbt.to_string(),
        amount_sat: amount.to_sat(),
        fee_sat: draft.fee.to_sat(),
        change_sat: draft.change.to_sat(),
        fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
    }))
}

/// Builds an UNSIGNED replacement for one of the wallet's unconfirmed transactions, paying a
/// higher fee (RBF). Like /psbt, the client reviews it (against the original, which we send
/// along), signs it locally and hands it back to /broadcast.
async fn bump_fee(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    token: Bearer,
    ApiJson(req): ApiJson<BumpRequest>,
) -> Result<Json<BumpResponse>, ApiError> {
    let txid = Txid::from_str(&req.txid).map_err(|_| ApiError::bad_request("invalid txid"))?;
    let priority = req.fee_priority.unwrap_or(FeePriority::Fast);

    let (draft, fee_rate, original) = with_wallet(state, id, token, move |w, backend| {
        let fee_rate =
            send::choose_bump_fee_rate(backend, &w.wallet, txid, req.fee_rate_sat_vb, priority)
                .map_err(|e| build_error(e, &[]))?;
        let original = w
            .wallet
            .get_tx(txid)
            .ok_or_else(|| ApiError::bad_request("transaction not found in this wallet"))?
            .tx_node
            .tx;
        let now = Instant::now();
        let reserved = w.reserved.active(now);
        let draft = send::bump(&mut w.wallet, txid, fee_rate, &reserved)
            .map_err(|e| build_error(e, &reserved))?;
        // Only coins the bump added need reserving; the original's are already spent.
        w.reserved.reserve(spent_coins(&draft.psbt.unsigned_tx), now + RESERVATION_TTL);
        Ok((draft, fee_rate, original))
    })
    .await?;

    Ok(Json(BumpResponse {
        psbt: draft.psbt.to_string(),
        original_tx: serialize_hex(original.as_ref()),
        fee_sat: draft.fee.to_sat(),
        fee_rate_sat_vb: fee_rate.to_sat_per_vb_ceil(),
    }))
}

/// Maps a failure to build a transaction to an HTTP error. Everything but `Other` is the
/// client's problem (400).
fn build_error(e: BuildError, reserved: &[OutPoint]) -> ApiError {
    match e {
        BuildError::Other(e) => ApiError::from(e),
        // The balance can look big enough while coins sit reserved; say why.
        e @ BuildError::InsufficientFunds(_) if !reserved.is_empty() => {
            ApiError::bad_request(format!(
                "{e} ({} coin(s) are reserved by PSBTs not broadcast yet; they free up once \
                 broadcast or after {} minutes)",
                reserved.len(),
                RESERVATION_TTL.as_secs() / 60,
            ))
        }
        e => ApiError::bad_request(e.to_string()),
    }
}

/// Finalizes a PSBT the client signed and broadcasts it. Finalizing needs only the public
/// descriptors; it fails unless every input carries a valid-looking signature.
async fn broadcast(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    token: Bearer,
    ApiJson(req): ApiJson<BroadcastRequest>,
) -> Result<Json<BroadcastResponse>, ApiError> {
    let psbt = Psbt::from_str(&req.psbt)
        .map_err(|e| ApiError::bad_request(format!("invalid PSBT: {e}")))?;

    let txid = with_wallet(state, id, token, move |w, backend| {
        let tx = send::finalize(&w.wallet, psbt)
            .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
        let spent: Vec<OutPoint> = spent_coins(&tx).collect();
        let WalletEntry { conn, wallet, reserved, .. } = w;
        let txid = send::broadcast(wallet, conn, backend, tx).map_err(|e| {
            match BackendError::find(&e) {
                // The backend answered, but refused the tx (e.g. double-spend, fee too low):
                // the client's issue.
                Some(BackendError::Rejected(reason)) => ApiError::bad_request(format!(
                    "the transaction was rejected: {reason}"
                )),
                _ => e.into(),
            }
        })?;
        // BDK now sees these coins as spent, so the reservation has done its job.
        reserved.release(spent);
        Ok(txid)
    })
    .await?;
    Ok(Json(BroadcastResponse { txid: txid.to_string() }))
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
        let (conn, wallet) = load(&db, self.network)?;
        let token_hash = conn
            .query_row("SELECT hash FROM api_token WHERE id = 0", [], |row| row.get::<_, Vec<u8>>(0))
            .optional()
            .map_err(anyhow::Error::from)?
            .and_then(|hash| sha256::Hash::from_slice(&hash).ok())
            .ok_or_else(|| {
                // Registered before tokens existed, so nobody holds a token for it.
                ApiError::unauthorized(
                    "wallet has no API token; delete it from the server's data dir and register again",
                )
            })?;
        let reserved = Reservations::default();
        let entry = Arc::new(Mutex::new(WalletEntry { conn, wallet, reserved, token_hash, last_sync: None }));
        wallets.insert(id.to_owned(), entry.clone());
        Ok(entry)
    }
}

/// Checks `token` against wallet `id`, syncs the wallet with the chain (unless it was synced
/// within the sync interval), then runs `f` with exclusive access to it on a blocking thread.
async fn with_wallet<T: Send + 'static>(
    state: SharedState,
    id: String,
    token: Bearer,
    f: impl FnOnce(&mut WalletEntry, &Backend) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    run_blocking(move || {
        let entry = state.wallet(&id)?;
        let mut entry = entry.lock().map_err(|_| anyhow!("wallet lock poisoned"))?;
        // Comparing hashes, not tokens: an early-exit compare can only leak how much of the
        // hash matched, which doesn't help anyone find a token that produces it.
        if token_hash(&token.0) != entry.token_hash {
            return Err(ApiError::unauthorized("invalid API token"));
        }
        let WalletEntry { conn, wallet, last_sync, .. } = &mut *entry;
        if last_sync.is_none_or(|t| t.elapsed() >= state.sync_interval) {
            chain::sync(wallet, conn, &state.backend)?;
            *last_sync = Some(Instant::now());
        }
        f(&mut entry, &state.backend)
    })
    .await
}

/// Runs blocking work (BDK, SQLite, chain backend calls) off the async runtime's worker threads.
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

    fn unauthorized(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::UNAUTHORIZED, message: message.into() }
    }

    fn not_found(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::NOT_FOUND, message: message.into() }
    }

    fn conflict(message: impl Into<String>) -> Self {
        ApiError { status: StatusCode::CONFLICT, message: message.into() }
    }
}

/// Unexpected failures: 502 if the chain backend is down or erroring (not our fault, and
/// worth retrying), else 500. Details go to the server log, never to the client, since they can
/// include internal paths and state.
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        eprintln!("error: {e:#}");
        match BackendError::find(&e) {
            Some(BackendError::Unreachable) => ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "the chain backend is unreachable".to_owned(),
            },
            Some(_) => ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "the chain backend returned an error".to_owned(),
            },
            None => ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "internal error".to_owned(),
            },
        }
    }
}

/// Malformed or mistyped JSON bodies get our usual `{"error": ...}` shape.
impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        ApiError { status: rejection.status(), message: rejection.body_text() }
    }
}

/// Like `axum::Json`, but rejections become `ApiError`s (JSON) instead of plain text.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
struct ApiJson<T>(T);

impl From<bdk_wallet::rusqlite::Error> for ApiError {
    fn from(e: bdk_wallet::rusqlite::Error) -> Self {
        anyhow::Error::from(e).into()
    }
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

    fn account_key() -> String {
        let mnemonic = Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        keys::account_key(&mnemonic).unwrap()
    }

    #[test]
    fn accepts_public_descriptor() {
        let (public, _) = keys::derive(&account_key(), KeychainKind::External).unwrap();
        assert!(public_descriptor(&public.to_string()).is_ok());
    }

    #[test]
    fn rejects_private_descriptor() {
        let (with_secret, _) = keys::descriptors(&account_key()).unwrap();
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

    #[test]
    fn tokens_are_random_and_stored_hashed() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_ne!(token_hash(&a), token_hash(&b));
    }

    #[test]
    fn rate_limiter_blocks_until_window_ends() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let (me, other) = (IpAddr::from([127, 0, 0, 1]), IpAddr::from([10, 0, 0, 1]));
        let t0 = Instant::now();

        assert!(limiter.check(me, t0).is_ok());
        assert!(limiter.check(me, t0).is_ok());
        let wait = limiter.check(me, t0 + Duration::from_secs(20)).unwrap_err();
        assert_eq!(wait, Duration::from_secs(40), "blocked until the window ends");
        assert!(limiter.check(other, t0).is_ok(), "limits are per IP");
        assert!(limiter.check(me, t0 + Duration::from_secs(60)).is_ok(), "new window");
    }

    fn coin(vout: u32) -> OutPoint {
        OutPoint::new(Txid::from_str(&"11".repeat(32)).unwrap(), vout)
    }

    #[test]
    fn reservations_expire() {
        let now = Instant::now();
        let mut r = Reservations::default();
        r.reserve([coin(0)], now + RESERVATION_TTL);
        assert_eq!(r.active(now), vec![coin(0)]);
        assert!(r.active(now + RESERVATION_TTL).is_empty());
    }

    #[test]
    fn release_frees_only_the_given_coins() {
        let now = Instant::now();
        let mut r = Reservations::default();
        r.reserve([coin(0), coin(1)], now + RESERVATION_TTL);
        r.release([coin(0)]);
        assert_eq!(r.active(now), vec![coin(1)]);
    }
}
