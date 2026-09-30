//! Talking to the chain: a Bitcoin Core node over RPC, or an Esplora HTTP server.
//!
//! Core syncs by downloading every block since the wallet's birthday and checking it for our
//! scripts. Esplora is an address index: we ask it about our scripts directly, so there's no
//! node to run and the birthday doesn't matter, but the server learns which addresses are ours.

use std::thread;
use std::time::Duration;

use anyhow::{Context, bail};
use bdk_bitcoind_rpc::Emitter;
use bdk_bitcoind_rpc::bitcoincore_rpc::{self, Auth, RpcApi, jsonrpc};
use bdk_esplora::EsploraExt;
use bdk_esplora::esplora_client::{self, BlockingClient};
use bdk_wallet::PersistedWallet;
use bdk_wallet::bitcoin::constants::genesis_block;
use bdk_wallet::bitcoin::{FeeRate, Network, Transaction, Txid};
use bdk_wallet::rusqlite::{Connection, OptionalExtension, params};

use crate::{default_rpc_url, parse_network};

/// Give up on an Esplora request after this long, so a stuck server can't hang us forever.
const ESPLORA_TIMEOUT_SECS: u64 = 30;
/// Tries per Esplora call when the connection itself fails (dropped, reset, timed out).
/// esplora-client only retries HTTP 429/500/503 answers, not failed connections.
const ESPLORA_ATTEMPTS: u32 = 3;
/// Esplora full scans stop after this many unused addresses in a row on each keychain.
/// 20 is the BIP44 gap limit, which other wallets restoring the same seed also use.
const STOP_GAP: usize = 20;
/// How many HTTP requests a sync may have in flight at once.
const PARALLEL_REQUESTS: usize = 5;

/// Chain options shared by the CLI and the server.
#[derive(clap::Args)]
pub struct ChainArgs {
    /// Which chain the wallet lives on: regtest, signet, testnet or testnet4
    #[arg(long, env = "NETWORK", default_value = "regtest", value_parser = parse_network)]
    pub network: Network,

    /// Where chain data comes from [default: core on regtest, esplora elsewhere]
    #[arg(long, env = "BACKEND", value_enum)]
    pub backend: Option<BackendKind>,

    /// Esplora API URL [default: mempool.space or blockstream.info for the network]
    #[arg(long, env = "ESPLORA_URL")]
    pub esplora_url: Option<String>,

    /// Bitcoin Core RPC URL [default: localhost on the network's default port]
    #[arg(long, env = "RPC_URL")]
    pub rpc_url: Option<String>,

    /// Bitcoin Core RPC username
    #[arg(long, env = "RPC_USER", default_value = "wallet")]
    pub rpc_user: String,

    /// Bitcoin Core RPC password
    #[arg(long, env = "RPC_PASS", default_value = "wallet", hide_default_value = true)]
    pub rpc_pass: String,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BackendKind {
    Core,
    Esplora,
}

impl ChainArgs {
    pub fn backend_kind(&self) -> BackendKind {
        self.backend.unwrap_or(match self.network {
            Network::Regtest => BackendKind::Core,
            _ => BackendKind::Esplora,
        })
    }

    /// Connects to the chosen backend and checks it's on `self.network`.
    pub fn connect(&self) -> anyhow::Result<Backend> {
        match self.backend_kind() {
            BackendKind::Core => {
                let url = self.rpc_url.clone().unwrap_or_else(|| default_rpc_url(self.network));
                connect_core(&url, &self.rpc_user, &self.rpc_pass, self.network)
            }
            BackendKind::Esplora => {
                let url = match &self.esplora_url {
                    Some(url) => url.clone(),
                    None => default_esplora_url(self.network)?.to_owned(),
                };
                connect_esplora(&url, self.network)
            }
        }
    }
}

/// Public Esplora instances. There's none for regtest: run your own and pass --esplora-url.
fn default_esplora_url(network: Network) -> anyhow::Result<&'static str> {
    Ok(match network {
        Network::Testnet4 => "https://mempool.space/testnet4/api",
        Network::Signet => "https://mempool.space/signet/api",
        Network::Testnet => "https://blockstream.info/testnet/api",
        _ => bail!("no public Esplora server for {network}; pass --esplora-url or --backend core"),
    })
}

pub enum Backend {
    Core(bitcoincore_rpc::Client),
    Esplora(BlockingClient),
}

/// Connects to Bitcoin Core and checks that it answers and is on `network`.
fn connect_core(url: &str, user: &str, pass: &str, network: Network) -> anyhow::Result<Backend> {
    let client =
        bitcoincore_rpc::Client::new(url, Auth::UserPass(user.to_owned(), pass.to_owned()))?;
    let info = client
        .get_blockchain_info()
        .with_context(|| format!("cannot reach Bitcoin Core at {url}; is it running?"))?;
    // Otherwise a testnet4 wallet synced against a regtest node would just look empty.
    if info.chain != network {
        bail!("Bitcoin Core at {url} is on {}, but the wallet uses {network}", info.chain);
    }
    Ok(Backend::Core(client))
}

/// Connects to Esplora and checks it serves `network`, by comparing genesis block hashes
/// (Esplora has no "which chain are you" call).
fn connect_esplora(url: &str, network: Network) -> anyhow::Result<Backend> {
    let client = esplora_client::Builder::new(url).timeout(ESPLORA_TIMEOUT_SECS).build_blocking();
    let genesis = with_retry(|| client.get_block_hash(0))
        .with_context(|| format!("cannot reach Esplora at {url}"))?;
    if genesis != genesis_block(network).block_hash() {
        bail!("Esplora at {url} is not a {network} server (different genesis block)");
    }
    Ok(Backend::Esplora(client))
}

/// How a backend call failed, for callers that map errors to responses (the API).
pub enum BackendError<'a> {
    /// Couldn't reach it at all.
    Unreachable,
    /// It answered with a refusal, e.g. a broadcast of a double-spend.
    Rejected(&'a str),
    Other,
}

impl BackendError<'_> {
    /// The backend failure somewhere in `e`'s cause chain, if any.
    pub fn find(e: &anyhow::Error) -> Option<BackendError<'_>> {
        e.chain().find_map(|cause| {
            if let Some(e) = cause.downcast_ref::<bitcoincore_rpc::Error>() {
                return Some(match e {
                    bitcoincore_rpc::Error::JsonRpc(jsonrpc::Error::Transport(_)) => {
                        BackendError::Unreachable
                    }
                    bitcoincore_rpc::Error::JsonRpc(jsonrpc::Error::Rpc(r)) => {
                        BackendError::Rejected(&r.message)
                    }
                    _ => BackendError::Other,
                });
            }
            cause.downcast_ref::<esplora_client::Error>().map(|e| match e {
                esplora_client::Error::Minreq(_) => BackendError::Unreachable,
                // Esplora answers 400 with Core's reason when it refuses a transaction.
                esplora_client::Error::HttpResponse { status: 400, message } => {
                    BackendError::Rejected(message)
                }
                _ => BackendError::Other,
            })
        })
    }
}

impl Backend {
    /// Current height of the best chain.
    pub fn tip_height(&self) -> anyhow::Result<u32> {
        match self {
            Backend::Core(rpc) => {
                let height =
                    rpc.get_block_count().context("asking Bitcoin Core for the chain tip")?;
                Ok(u32::try_from(height)?)
            }
            Backend::Esplora(client) => {
                with_retry(|| client.get_height()).context("asking Esplora for the chain tip")
            }
        }
    }

    /// A fee rate likely to confirm within `target` blocks, or None when there's no data
    /// (normal on a fresh regtest chain: Core estimates from how fast past txs confirmed).
    pub fn estimate_fee_rate(&self, target: u16) -> Option<FeeRate> {
        match self {
            Backend::Core(rpc) => {
                let estimate = rpc.estimate_smart_fee(target, None).ok()?;
                let btc_per_kvb = estimate.fee_rate?;
                // sat/kvB -> sat/kwu: 1 vbyte = 4 weight units.
                Some(FeeRate::from_sat_per_kwu(btc_per_kvb.to_sat() / 4))
            }
            Backend::Esplora(client) => {
                let estimates = match with_retry(|| client.get_fee_estimates()) {
                    Ok(estimates) => estimates,
                    // mempool.space answers 203 Non-Authoritative Information here, which
                    // esplora-client treats as an error even though the body is the data.
                    Err(esplora_client::Error::HttpResponse { status: 203, message }) => {
                        serde_json::from_str(&message).ok()?
                    }
                    Err(_) => return None,
                };
                // sat/vB (a float) -> sat/kwu: 1 vbyte = 4 weight units, so x1000/4.
                let sat_vb = esplora_client::convert_fee_rate(target.into(), estimates)?;
                Some(FeeRate::from_sat_per_kwu((sat_vb * 250.0).ceil() as u64))
            }
        }
    }

    pub fn broadcast(&self, tx: &Transaction) -> anyhow::Result<Txid> {
        // No "rejected" wording here: this also fails when the backend is simply unreachable.
        match self {
            Backend::Core(rpc) => {
                rpc.send_raw_transaction(tx).context("broadcasting transaction")
            }
            // Not retried: if the first try reached the server but the reply got lost, a retry
            // would be refused as already-known and we'd wrongly report a failure.
            Backend::Esplora(client) => {
                client.broadcast(tx).context("broadcasting transaction")?;
                Ok(tx.compute_txid())
            }
        }
    }
}

/// Runs an Esplora call, retrying with a short backoff when the connection fails. Answers
/// from the server (including errors like 400) are returned as they are.
fn with_retry<T>(
    mut call: impl FnMut() -> Result<T, esplora_client::Error>,
) -> Result<T, esplora_client::Error> {
    let mut attempt = 1;
    loop {
        match call() {
            Err(esplora_client::Error::Minreq(e)) if attempt < ESPLORA_ATTEMPTS => {
                eprintln!("Esplora connection failed ({e}), retrying...");
                thread::sleep(Duration::from_millis(500 * u64::from(attempt)));
                attempt += 1;
            }
            result => return result,
        }
    }
}

pub struct SyncSummary {
    /// What the sync looked at, e.g. "3 new block(s)".
    pub scanned: String,
    pub tip_height: u32,
}

/// Records the wallet's birthday: the height of the first block that could contain its
/// transactions. Syncing starts there instead of at genesis.
pub fn save_birthday(conn: &Connection, height: u32) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS birthday (
            id     INTEGER PRIMARY KEY CHECK (id = 0),
            height INTEGER NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO birthday (id, height) VALUES (0, ?1)",
        params![height],
    )
    .context("saving wallet birthday")?;
    Ok(())
}

/// The wallet's birthday, or 0 (scan from genesis) for wallets created without one.
pub fn birthday(conn: &Connection) -> anyhow::Result<u32> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'birthday')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(0);
    }
    let height = conn
        .query_row("SELECT height FROM birthday WHERE id = 0", [], |row| row.get(0))
        .optional()?;
    Ok(height.unwrap_or(0))
}

/// Rejects a birthday past the chain tip (usually a typo), which Core would otherwise answer
/// with a cryptic "Block height out of range" on the first sync.
pub fn check_birthday(backend: &Backend, height: u32) -> anyhow::Result<()> {
    let tip = backend.tip_height()?;
    if height > tip {
        bail!("birthday {height} is past the chain tip ({tip})");
    }
    Ok(())
}

/// Brings the wallet up to date with the chain and mempool, then persists it.
pub fn sync(
    wallet: &mut PersistedWallet<Connection>,
    conn: &mut Connection,
    backend: &Backend,
) -> anyhow::Result<SyncSummary> {
    let scanned = match backend {
        Backend::Core(rpc) => {
            if wallet.latest_checkpoint().height() == 0 {
                check_birthday(backend, birthday(conn)?)?;
            }
            sync_core(wallet, conn, rpc)?
        }
        Backend::Esplora(client) => sync_esplora(wallet, client)?,
    };
    wallet.persist(conn).context("saving wallet")?;
    Ok(SyncSummary { scanned, tip_height: wallet.latest_checkpoint().height() })
}

/// The first sync starts at the wallet's birthday; later ones fetch only blocks after the
/// wallet's last checkpoint, so repeat syncs are cheap.
fn sync_core(
    wallet: &mut PersistedWallet<Connection>,
    conn: &Connection,
    rpc: &bitcoincore_rpc::Client,
) -> anyhow::Result<String> {
    let birthday = birthday(conn)?;
    // Hand the emitter our unconfirmed txs so it can tell us if any left the mempool
    // (e.g. replaced by a double-spend).
    let mut emitter = Emitter::new(
        rpc,
        wallet.latest_checkpoint(),
        // Blocks before the birthday can't pay us, so the first sync skips them. Once the
        // wallet has checkpoints past it, this has no effect.
        birthday,
        wallet
            .transactions()
            .filter(|tx| tx.chain_position.is_unconfirmed()),
    );

    let mut blocks_scanned = 0;
    while let Some(event) = emitter.next_block()? {
        // `connected_to` links the block to its parent so BDK can detect reorgs.
        wallet.apply_block_connected_to(&event.block, event.block_height(), event.connected_to())?;
        blocks_scanned += 1;
    }

    let mempool = emitter.mempool()?;
    wallet.apply_evicted_txs(mempool.evicted);
    wallet.apply_unconfirmed_txs(mempool.update);
    Ok(format!("{blocks_scanned} new block(s)"))
}

/// A full scan every time: it walks each keychain's addresses until STOP_GAP unused ones in
/// a row, so it also finds payments to addresses this copy of the wallet never revealed (e.g.
/// ones the API server or another wallet app handed out). That's ~2x STOP_GAP requests more
/// than syncing only revealed addresses, which is fine at this wallet's size.
fn sync_esplora(
    wallet: &mut PersistedWallet<Connection>,
    client: &BlockingClient,
) -> anyhow::Result<String> {
    // A retry redoes the whole scan: bdk_esplora doesn't let us resume one mid-way.
    let update = with_retry(|| {
        client
            .full_scan(wallet.start_full_scan().build(), STOP_GAP, PARALLEL_REQUESTS)
            // bdk_esplora boxes the error; unbox it so with_retry and BackendError::find see it.
            .map_err(|e| *e)
    })
    .context("scanning addresses with Esplora")?;
    wallet.apply_update(update)?;
    Ok("addresses via Esplora".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birthday_defaults_to_genesis() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(birthday(&conn).unwrap(), 0);
    }

    #[test]
    fn birthday_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        save_birthday(&conn, 850_000).unwrap();
        assert_eq!(birthday(&conn).unwrap(), 850_000);
    }
}
