//! Talking to Bitcoin Core: the RPC client and chain sync.

use anyhow::Context;
use bdk_bitcoind_rpc::Emitter;
use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, RpcApi};
pub use bdk_bitcoind_rpc::bitcoincore_rpc::Client;
use bdk_wallet::PersistedWallet;
use bdk_wallet::rusqlite::Connection;

/// Connects to Bitcoin Core and checks that it answers.
pub fn connect(url: &str, user: &str, pass: &str) -> anyhow::Result<Client> {
    let client = Client::new(url, Auth::UserPass(user.to_owned(), pass.to_owned()))?;
    client.get_blockchain_info().with_context(|| {
        format!("cannot reach Bitcoin Core at {url}; is it running? (docker compose up -d)")
    })?;
    Ok(client)
}

pub struct SyncSummary {
    pub blocks_scanned: usize,
    pub tip_height: u32,
}

/// Brings the wallet up to date with Core's chain and mempool, then persists it.
///
/// Only blocks after the wallet's last checkpoint are fetched, so repeat syncs are cheap.
pub fn sync(
    wallet: &mut PersistedWallet<Connection>,
    conn: &mut Connection,
    rpc: &Client,
) -> anyhow::Result<SyncSummary> {
    // Hand the emitter our unconfirmed txs so it can tell us if any left the mempool
    // (e.g. replaced by a double-spend).
    let mut emitter = Emitter::new(
        rpc,
        wallet.latest_checkpoint(),
        0, // start height; regtest is tiny. A mainnet wallet would start at its birthday.
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
