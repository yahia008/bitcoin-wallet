//! Read-only views of a wallet, shared by the CLI and the API: transaction summaries and
//! receive addresses. Call `chain::sync` first for up-to-date results.

use bdk_wallet::bitcoin::{Address, Amount, SignedAmount, Txid};
use bdk_wallet::chain::ChainPosition;
use bdk_wallet::{KeychainKind, Wallet, WalletTx};

use crate::confirmations;

pub struct TxSummary {
    pub txid: Txid,
    /// Effect on our balance: positive = received, negative = sent (including the fee).
    pub net: SignedAmount,
    /// Only set when we paid it, i.e. when the transaction spent our coins.
    pub fee: Option<Amount>,
    pub confirmations: u32,
    /// None while unconfirmed.
    pub block_height: Option<u32>,
}

impl TxSummary {
    pub fn status(&self) -> String {
        match self.block_height {
            None => "unconfirmed".to_owned(),
            Some(height) => format!("{} conf (block {height})", self.confirmations),
        }
    }
}

/// All wallet transactions: unconfirmed first, then confirmed from newest block to oldest.
pub fn transactions(wallet: &Wallet) -> Vec<TxSummary> {
    let mut txs: Vec<_> = wallet.transactions().map(|tx| summarize(wallet, &tx)).collect();
    txs.sort_by_key(|tx| match tx.block_height {
        None => (0, 0),
        Some(height) => (1, u32::MAX - height),
    });
    txs
}

/// One transaction. None if it isn't ours, or if it was replaced or evicted from the mempool
/// (`get_tx` only returns txs in the wallet's current view of the chain).
pub fn transaction(wallet: &Wallet, txid: Txid) -> Option<TxSummary> {
    wallet.get_tx(txid).map(|tx| summarize(wallet, &tx))
}

fn summarize(wallet: &Wallet, tx: &WalletTx) -> TxSummary {
    let tip = wallet.latest_checkpoint().height();
    let (sent, received) = wallet.sent_and_received(&tx.tx_node.tx);
    // Only report fees we paid. BDK can sometimes compute the fee of an incoming tx too (when
    // the sender spent change from an earlier payment to us), but that fee isn't ours.
    let fee = match wallet.calculate_fee(&tx.tx_node.tx) {
        Ok(fee) if sent.to_sat() > 0 => Some(fee),
        _ => None,
    };
    let block_height = match &tx.chain_position {
        ChainPosition::Unconfirmed { .. } => None,
        ChainPosition::Confirmed { anchor, .. } => Some(anchor.block_id.height),
    };
    TxSummary {
        txid: tx.tx_node.txid,
        net: SignedAmount::from_sat(received.to_sat() as i64 - sent.to_sat() as i64),
        fee,
        confirmations: confirmations(tip, &tx.chain_position),
        block_height,
    }
}

pub struct ReceiveAddress {
    pub index: u32,
    pub address: Address,
    /// A transaction paying to it has been seen on chain or in the mempool.
    pub used: bool,
}

/// Every receive address handed out so far, in derivation order.
pub fn receive_addresses(wallet: &Wallet) -> Vec<ReceiveAddress> {
    let Some(last) = wallet.derivation_index(KeychainKind::External) else {
        return Vec::new();
    };
    (0..=last)
        .map(|index| ReceiveAddress {
            index,
            address: wallet.peek_address(KeychainKind::External, index).address,
            used: wallet.spk_index().is_used(KeychainKind::External, index),
        })
        .collect()
}
