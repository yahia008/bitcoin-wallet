//! Talking to Bitcoin Core: the RPC client and chain sync.

use anyhow::{Context, bail};
use bdk_bitcoind_rpc::Emitter;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, RpcApi};
pub use bdk_bitcoind_rpc::bitcoincore_rpc::Client;
use bdk_wallet::PersistedWallet;
use bdk_wallet::bitcoin::Network;
use bdk_wallet::rusqlite::{Connection, OptionalExtension, params};

/// Connects to Bitcoin Core and checks that it answers and is on `network`.
pub fn connect(url: &str, user: &str, pass: &str, network: Network) -> anyhow::Result<Client> {
    let client = Client::new(url, Auth::UserPass(user.to_owned(), pass.to_owned()))?;
    let info = client
        .get_blockchain_info()
        .with_context(|| format!("cannot reach Bitcoin Core at {url}; is it running?"))?;
    // Otherwise a testnet4 wallet synced against a regtest node would just look empty.
    if info.chain != network {
        bail!("Bitcoin Core at {url} is on {}, but the wallet uses {network}", info.chain);
    }
    Ok(client)
}

pub struct SyncSummary {
    pub blocks_scanned: usize,
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

/// Current height of Core's best chain.
pub fn tip_height(rpc: &Client) -> anyhow::Result<u32> {
    let height = rpc.get_block_count().context("asking Bitcoin Core for the chain tip")?;
    Ok(u32::try_from(height)?)
}

/// Rejects a birthday past the chain tip (usually a typo), which Core would otherwise answer
/// with a cryptic "Block height out of range" on the first sync.
pub fn check_birthday(rpc: &Client, height: u32) -> anyhow::Result<()> {
    let tip = tip_height(rpc)?;
    if height > tip {
        bail!("birthday {height} is past the chain tip ({tip})");
    }
    Ok(())
}

/// Brings the wallet up to date with Core's chain and mempool, then persists it.
///
/// The first sync starts at the wallet's birthday; later ones fetch only blocks after the
/// wallet's last checkpoint, so repeat syncs are cheap.
pub fn sync(
    wallet: &mut PersistedWallet<Connection>,
    conn: &mut Connection,
    rpc: &Client,
) -> anyhow::Result<SyncSummary> {
    let birthday = birthday(conn)?;
    if wallet.latest_checkpoint().height() == 0 {
        check_birthday(rpc, birthday)?;
    }

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

    wallet.persist(conn).context("saving wallet")?;
    Ok(SyncSummary {
        blocks_scanned,
        tip_height: wallet.latest_checkpoint().height(),
    })
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
