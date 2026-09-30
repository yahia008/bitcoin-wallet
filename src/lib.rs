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

/// Parses `--network`. Mainnet is refused until keys.rs uses coin type 0' for it and fees
/// are handled properly.
pub fn parse_network(s: &str) -> anyhow::Result<Network> {
    let network = match s {
        "regtest" => Network::Regtest,
        "signet" => Network::Signet,
        "testnet" | "testnet3" => Network::Testnet,
        "testnet4" => Network::Testnet4,
        "bitcoin" | "mainnet" => {
            bail!("mainnet is not supported yet (keys use testnet coin type 1', no fee handling)")
        }
        other => bail!("unknown network {other:?}; use regtest, signet, testnet or testnet4"),
    };
    Ok(network)
}

/// Bitcoin Core's default RPC URL for each network.
pub fn default_rpc_url(network: Network) -> String {
    let port = match network {
        Network::Regtest => 18443,
        Network::Signet => 38332,
        Network::Testnet => 18332,
        Network::Testnet4 => 48332,
        _ => 8332, // mainnet
    };
    format!("http://127.0.0.1:{port}")
}

/// The API's id for a wallet: a hash of its canonical public descriptors (as printed by
/// `Descriptor`'s Display). The CLI and server both compute it, so ids never need copying.
pub fn wallet_id(external: &str, internal: &str) -> String {
    let hash = sha256::Hash::hash(format!("{external}\n{internal}").as_bytes());
    hash.to_string()[..16].to_owned()
}

/// Loads the wallet from disk. Needs no password: the database only holds public descriptors.
/// Fails if the wallet was created for a different network than `network`.
pub fn load(db: &Path, network: Network) -> anyhow::Result<(Connection, PersistedWallet<Connection>)> {
    let mut conn = open_existing(db)?;
    let wallet = Wallet::load()
        .check_network(network)
        .load_wallet(&mut conn)
        .with_context(|| format!("loading wallet (is it for a network other than {network}?)"))?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_test_networks() {
        assert_eq!(parse_network("testnet4").unwrap(), Network::Testnet4);
        assert_eq!(parse_network("testnet").unwrap(), Network::Testnet);
        assert_eq!(parse_network("regtest").unwrap(), Network::Regtest);
    }

    #[test]
    fn refuses_mainnet() {
        assert!(parse_network("bitcoin").is_err());
        assert!(parse_network("mainnet").is_err());
        assert!(parse_network("testnet5").is_err());
    }
}
