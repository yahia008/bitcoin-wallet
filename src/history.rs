//! Read-only views of a wallet, shared by the CLI and the API: transaction summaries and
//! receive addresses. Call `chain::sync` first for up-to-date results.

use bdk_wallet::bitcoin::{Address, Amount, ScriptBuf, SignedAmount, Txid};
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
    /// Unix seconds: the block's time once confirmed, when it was first seen in the mempool
    /// before that (None if the backend didn't say).
    pub time: Option<u64>,
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
    let (block_height, time) = match &tx.chain_position {
        ChainPosition::Unconfirmed { first_seen, .. } => (None, *first_seen),
        ChainPosition::Confirmed { anchor, .. } => {
            (Some(anchor.block_id.height), Some(anchor.confirmation_time))
        }
    };
    TxSummary {
        txid: tx.tx_node.txid,
        net: SignedAmount::from_sat(received.to_sat() as i64 - sent.to_sat() as i64),
        fee,
        confirmations: confirmations(tip, &tx.chain_position),
        block_height,
        time,
    }
}

/// Whose an input or output is, as far as this wallet can tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    /// One of our receive addresses.
    Receive,
    /// One of our change addresses.
    Change,
    /// Someone else's, or unknown.
    External,
}

/// One input or output of a transaction.
pub struct TxIo {
    /// None for scripts that have no address (e.g. OP_RETURN), or an input whose previous
    /// transaction the wallet hasn't seen.
    pub address: Option<Address>,
    /// None for an input whose previous transaction the wallet hasn't seen.
    pub value: Option<Amount>,
    pub owner: Owner,
}

/// Everything about one transaction, for a detail view.
pub struct TxDetail {
    pub summary: TxSummary,
    pub vsize: u64,
    /// sat/vB; only set when the fee is (we paid it).
    pub fee_rate: Option<f64>,
    /// Signals replace-by-fee (BIP125), so it can still be sped up while unconfirmed.
    pub rbf: bool,
    pub inputs: Vec<TxIo>,
    pub outputs: Vec<TxIo>,
}

/// One transaction with its inputs and outputs. None in the same cases as `transaction`.
pub fn transaction_detail(wallet: &Wallet, txid: Txid) -> Option<TxDetail> {
    let wtx = wallet.get_tx(txid)?;
    let summary = summarize(wallet, &wtx);
    let tx = &wtx.tx_node.tx;
    let vsize = tx.vsize() as u64;
    let fee_rate = summary.fee.map(|fee| fee.to_sat() as f64 / vsize as f64);
    let io = |script: ScriptBuf, value: Option<Amount>| TxIo {
        address: Address::from_script(&script, wallet.network()).ok(),
        value,
        owner: match wallet.derivation_of_spk(script) {
            Some((KeychainKind::External, _)) => Owner::Receive,
            Some((KeychainKind::Internal, _)) => Owner::Change,
            None => Owner::External,
        },
    };
    let inputs = tx
        .input
        .iter()
        .map(|input| match wallet.tx_graph().get_txout(input.previous_output) {
            Some(prev) => io(prev.script_pubkey.clone(), Some(prev.value)),
            None => TxIo { address: None, value: None, owner: Owner::External },
        })
        .collect();
    let outputs = tx
        .output
        .iter()
        .map(|output| io(output.script_pubkey.clone(), Some(output.value)))
        .collect();
    Some(TxDetail {
        summary,
        vsize,
        fee_rate,
        rbf: tx.is_explicitly_rbf(),
        inputs,
        outputs,
    })
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
