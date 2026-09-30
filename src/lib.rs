//! Wallet logic shared by the CLI (`src/main.rs`) and the HTTP server (`src/bin/server.rs`).

pub mod api;
pub mod chain;
pub mod client;
pub mod history;
pub mod keys;
pub mod secret;
pub mod send;

use std::path::Path;

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::bitcoin::hashes::{Hash, sha256};
use bdk_wallet::chain::{ChainPosition, ConfirmationBlockTime};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{PersistedWallet, Wallet};

/// Regtest only for now. Changing this also requires changing the coin type (1') in keys.rs.
pub const NETWORK: Network = Network::Regtest;

/// The API's id for a wallet: a hash of its canonical public descriptors (as printed by
/// `Descriptor`'s Display). The CLI and server both compute it, so ids never need copying.
pub fn wallet_id(external: &str, internal: &str) -> String {
    let hash = sha256::Hash::hash(format!("{external}\n{internal}").as_bytes());
    hash.to_string()[..16].to_owned()
}

/// Loads the wallet from disk. Needs no password: the database only holds public descriptors.
pub fn load(db: &Path) -> anyhow::Result<(Connection, PersistedWallet<Connection>)> {
    let mut conn = open_existing(db)?;
    let wallet = Wallet::load()
        .check_network(NETWORK)
        .load_wallet(&mut conn)
        .context("loading wallet")?
        .ok_or_else(|| anyhow!("{} contains no wallet", db.display()))?;
    Ok((conn, wallet))
}

fn open_existing(db: &Path) -> anyhow::Result<Connection> {
    if !db.exists() {
        bail!("{} not found; run `create` or `restore` first", db.display());
    }
    Connection::open(db).context("opening wallet database")
}

/// 0 while unconfirmed; 1 once in a block; +1 for every block mined on top.
pub fn confirmations(tip: u32, position: &ChainPosition<ConfirmationBlockTime>) -> u32 {
    match position {
        ChainPosition::Unconfirmed { .. } => 0,
        ChainPosition::Confirmed { anchor, .. } => tip - anchor.block_id.height + 1,
    }
}
