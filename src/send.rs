//! Spending, split into the steps a transaction goes through:
//!
//!   build (watch-only) -> review (signer checks it) -> sign (needs keys)
//!   -> finalize (watch-only) -> broadcast (watch-only)
//!
//! Only `sign` touches private keys, so the API server can do everything else while the
//! signing stays on the user's device.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::{Address, Amount, FeeRate, OutPoint, Psbt, Transaction, Txid};
use bdk_wallet::error::{BuildFeeBumpError, CreateTxError};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, PersistedWallet, SignOptions, Wallet};

use serde::{Deserialize, Serialize};

use crate::chain::Backend;

// The checks and signing live in wallet-core so the browser runs the same code; re-exported
// here so callers keep using `send::`.
pub use wallet_core::review::{Review, check_fee, review, review_bump, signed_fee_rate};
pub use wallet_core::sign::{check_account_key, sign};

/// How soon the user wants a transaction confirmed, when they don't pick an exact fee rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum FeePriority {
    /// Next block or two
    Fast,
    /// Within about an hour
    #[default]
    Normal,
    /// Within about a day; cheapest
    Slow,
}

impl FeePriority {
    /// Confirmation target in blocks, as fee estimators take it. Core treats 1 as 2 anyway.
    pub fn target_blocks(self) -> u16 {
        match self {
            FeePriority::Fast => 2,
            FeePriority::Normal => 6,
            FeePriority::Slow => 144,
        }
    }
}

/// Used when the backend has no fee estimate (always the case on a fresh regtest chain).
pub const FALLBACK_FEE_RATE_SAT_VB: u64 = 2;

/// The caller's fee rate if given, else the backend's estimate for `priority`, else the
/// fallback.
pub fn choose_fee_rate(
    backend: &Backend,
    requested_sat_vb: Option<u64>,
    priority: FeePriority,
) -> anyhow::Result<FeeRate> {
    match requested_sat_vb {
        Some(rate) => FeeRate::from_sat_per_vb(rate).context("fee rate too large"),
        None => Ok(backend
            .estimate_fee_rate(priority.target_blocks())
            // Estimates can dip below 1 sat/vB on quiet test networks, and many nodes
            // won't relay transactions that cheap.
            .map(|rate| rate.max(FeeRate::BROADCAST_MIN))
            .unwrap_or(FeeRate::from_sat_per_kwu(FALLBACK_FEE_RATE_SAT_VB * 250))),
    }
}

/// An unsigned transaction plus the numbers the user should check before signing.
pub struct Draft {
    pub psbt: Psbt,
    pub fee: Amount,
    pub change: Amount,
    pub inputs: usize,
}

/// Why a transaction couldn't be built. All but `Other` are the caller's problem (HTTP 400).
#[derive(Debug)]
pub enum BuildError {
    InsufficientFunds(String),
    BelowDust,
    /// The transaction can't be fee-bumped: unknown, already confirmed, or not replaceable.
    CannotBump(String),
    /// A replacement must pay more than the original, by at least the node's relay fee.
    FeeTooLow(String),
    Other(anyhow::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::InsufficientFunds(e) => write!(f, "insufficient funds: {e}"),
            BuildError::BelowDust => write!(f, "amount is below the dust limit; send more"),
            BuildError::CannotBump(e) => write!(f, "can't bump this transaction: {e}"),
            BuildError::FeeTooLow(e) => write!(f, "fee rate too low for a replacement: {e}"),
            BuildError::Other(e) => write!(f, "building transaction: {e:#}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Selects coins and builds an unsigned PSBT paying `amount` to `to`, with change back to us.
/// Coins in `unspendable` are never selected (the server passes coins reserved by PSBTs it
/// has handed out but not yet seen broadcast).
pub fn build(
    wallet: &mut Wallet,
    to: &Address,
    amount: Amount,
    fee_rate: FeeRate,
    unspendable: &[OutPoint],
) -> Result<Draft, BuildError> {
    let mut builder = wallet.build_tx();
    builder
        .add_recipient(to.script_pubkey(), amount)
        .fee_rate(fee_rate)
        .unspendable(unspendable.to_vec());
    let psbt = builder.finish().map_err(build_error)?;
    draft(wallet, psbt)
}

/// Builds an unsigned replacement for our unconfirmed transaction `txid` (RBF): the same
/// coins and payments, a higher `fee_rate`, the extra fee taken from our change (or from
/// more coins if the change can't cover it). Coins in `unspendable` are never added.
pub fn bump(
    wallet: &mut Wallet,
    txid: Txid,
    fee_rate: FeeRate,
    unspendable: &[OutPoint],
) -> Result<Draft, BuildError> {
    let mut builder = wallet.build_fee_bump(txid).map_err(|e| match e {
        BuildFeeBumpError::TransactionNotFound(_)
        | BuildFeeBumpError::TransactionConfirmed(_)
        | BuildFeeBumpError::IrreplaceableTransaction(_) => BuildError::CannotBump(e.to_string()),
        e => BuildError::Other(e.into()),
    })?;
    builder.fee_rate(fee_rate).unspendable(unspendable.to_vec());
    let psbt = builder.finish().map_err(build_error)?;
    draft(wallet, psbt)
}

/// The fee rate for bumping `txid`: exactly `requested` if given (the builder rejects it if
/// it's too low), else the estimate for `priority`, but never less than the original's rate
/// plus 1 sat/vB, the least a replacement can add and still be relayed.
pub fn choose_bump_fee_rate(
    backend: &Backend,
    wallet: &Wallet,
    txid: Txid,
    requested_sat_vb: Option<u64>,
    priority: FeePriority,
) -> Result<FeeRate, BuildError> {
    if let Some(rate) = requested_sat_vb {
        return FeeRate::from_sat_per_vb(rate)
            .ok_or_else(|| BuildError::FeeTooLow("fee rate too large".to_owned()));
    }
    let original = wallet
        .get_tx(txid)
        .ok_or_else(|| BuildError::CannotBump(format!("transaction {txid} is not in this wallet")))?
        .tx_node
        .tx;
    let fee = wallet
        .calculate_fee(&original)
        .map_err(|e| BuildError::Other(anyhow!("computing the original's fee: {e}")))?;
    // Round the original rate up, so "+1 sat/vB" really is at least 1 more.
    let original_rate =
        FeeRate::from_sat_per_kwu((fee.to_sat() * 1000).div_ceil(original.weight().to_wu()));
    let floor = FeeRate::from_sat_per_kwu(
        original_rate.to_sat_per_kwu() + FeeRate::BROADCAST_MIN.to_sat_per_kwu(),
    );
    Ok(backend
        .estimate_fee_rate(priority.target_blocks())
        .map_or(floor, |estimate| estimate.max(floor)))
}

fn build_error(e: CreateTxError) -> BuildError {
    match e {
        CreateTxError::CoinSelection(e) => BuildError::InsufficientFunds(e.to_string()),
        CreateTxError::OutputBelowDustLimit(_) => BuildError::BelowDust,
        e @ (CreateTxError::FeeTooLow { .. } | CreateTxError::FeeRateTooLow { .. }) => {
            BuildError::FeeTooLow(e.to_string())
        }
        e => BuildError::Other(e.into()),
    }
}

/// Works out the numbers a user checks before signing: fee, and change (outputs paying our
/// internal keychain).
fn draft(wallet: &Wallet, psbt: Psbt) -> Result<Draft, BuildError> {
    let fee = psbt.fee().map_err(|e| BuildError::Other(e.into()))?;
    // Change = outputs paying our internal (change) keychain.
    let change = psbt
        .unsigned_tx
        .output
        .iter()
        .filter(|out| {
            matches!(
                wallet.derivation_of_spk(out.script_pubkey.clone()),
                Some((KeychainKind::Internal, _))
            )
        })
        .map(|out| out.value)
        .sum();
    let inputs = psbt.inputs.len();

    Ok(Draft { psbt, fee, change, inputs })
}

/// Turns signatures into final witnesses and extracts the broadcastable transaction.
/// Needs only public descriptors, so the watch-only server can do it.
pub fn finalize(wallet: &Wallet, mut psbt: Psbt) -> anyhow::Result<Transaction> {
    let finalized = wallet.finalize_psbt(&mut psbt, SignOptions::default())?;
    if !finalized {
        bail!("transaction is not fully signed");
    }
    Ok(psbt.extract_tx()?)
}

/// Sends the transaction to the network and records it in the wallet as unconfirmed.
pub fn broadcast(
    wallet: &mut PersistedWallet<Connection>,
    conn: &mut Connection,
    backend: &Backend,
    tx: Transaction,
) -> anyhow::Result<Txid> {
    let txid = backend.broadcast(&tx)?;

    // Record it now so balance/history reflect it immediately, without waiting for a sync.
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    wallet.apply_unconfirmed_txs([(tx, now)]);
    wallet.persist(conn).context("saving wallet")?;
    Ok(txid)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bdk_wallet::bitcoin::absolute::LockTime;
    use bdk_wallet::bitcoin::hashes::Hash;
    use bdk_wallet::bitcoin::transaction::Version;
    use bdk_wallet::bitcoin::psbt::PsbtSighashType;
    use bdk_wallet::bitcoin::{TxIn, TxOut};

    use super::*;
    use crate::keys;
    use bdk_wallet::bitcoin::Network;

    const RECIPIENT: &str = "bcrt1q62sez8m4jmuk0uljr0c27jcl0gjq5rvx9j5nrk";
    const ATTACKER: &str = "bcrt1q995yuvawdx39hdzy6ra64dqxkkw76q7ethjm20";

    fn addr(s: &str) -> Address {
        Address::from_str(s).unwrap().require_network(Network::Regtest).unwrap()
    }

    fn account_key() -> String {
        let mnemonic = bdk_wallet::keys::bip39::Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        keys::account_key(&mnemonic).unwrap()
    }

    /// A wallet holding one unconfirmed 1 BTC coin, and an honest PSBT paying 0.3 BTC.
    fn setup() -> (Wallet, Psbt) {
        let (ext, int) = keys::descriptors(&account_key()).unwrap();
        let mut wallet = Wallet::create(ext, int).network(Network::Regtest).create_wallet_no_persist().unwrap();

        let ours = wallet.reveal_next_address(KeychainKind::External).address;
        let funding = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::all_zeros(), 0),
                ..Default::default()
            }],
            output: vec![TxOut { value: Amount::ONE_BTC, script_pubkey: ours.script_pubkey() }],
        };
        wallet.apply_unconfirmed_txs([(funding, 0)]);

        let fee_rate = FeeRate::from_sat_per_vb(2).unwrap();
        let draft =
            build(&mut wallet, &addr(RECIPIENT), Amount::from_sat(30_000_000), fee_rate, &[])
                .unwrap();
        (wallet, draft.psbt)
    }

    fn check(wallet: &Wallet, psbt: &Psbt) -> anyhow::Result<Review> {
        review(wallet, psbt, &addr(RECIPIENT), Amount::from_sat(30_000_000))
    }

    #[test]
    fn honest_psbt_passes_and_fee_matches() {
        let (wallet, psbt) = setup();
        let review = check(&wallet, &psbt).unwrap();
        assert_eq!(review.fee, psbt.fee().unwrap());
        assert!(review.change > Amount::ZERO);
    }

    #[test]
    fn rejects_changed_recipient_amount() {
        let (wallet, mut psbt) = setup();
        let i = recipient_output(&psbt);
        psbt.unsigned_tx.output[i].value = Amount::from_sat(20_000_000);
        assert!(check(&wallet, &psbt).is_err());
    }

    #[test]
    fn rejects_change_redirected_to_attacker() {
        let (wallet, mut psbt) = setup();
        let i = 1 - recipient_output(&psbt);
        psbt.unsigned_tx.output[i].script_pubkey = addr(ATTACKER).script_pubkey();
        let err = check(&wallet, &psbt).err().unwrap().to_string();
        assert!(err.contains("change"), "{err}");
    }

    #[test]
    fn rejects_extra_output_to_attacker() {
        let (wallet, mut psbt) = setup();
        psbt.unsigned_tx
            .output
            .push(TxOut { value: Amount::from_sat(1000), script_pubkey: addr(ATTACKER).script_pubkey() });
        psbt.outputs.push(Default::default());
        assert!(check(&wallet, &psbt).is_err());
    }

    /// Regression: signing must produce a PSBT the watch-only side can finalize.
    #[test]
    fn signed_psbt_finalizes() {
        let (wallet, mut psbt) = setup();
        sign(&wallet, &account_key(), &mut psbt).unwrap();
        let tx = finalize(&wallet, psbt).unwrap();
        assert!(tx.input.iter().all(|input| !input.witness.is_empty()));
    }

    /// The pre-signing estimate may only err low, and not by much.
    #[test]
    fn signed_fee_rate_matches_real_rate() {
        let (wallet, mut psbt) = setup();
        let fee = psbt.fee().unwrap();
        let estimate = signed_fee_rate(&psbt.unsigned_tx, fee);
        sign(&wallet, &account_key(), &mut psbt).unwrap();
        let tx = finalize(&wallet, psbt).unwrap();
        let real = fee.to_sat() as f64 / tx.vsize() as f64;
        assert!(estimate <= real && estimate > real * 0.98, "estimate {estimate}, real {real}");
    }

    #[test]
    fn fee_guard() {
        let (_, psbt) = setup(); // ~141 vB
        let tx = &psbt.unsigned_tx;
        let sats = Amount::from_sat;
        // Normal: 2 sat/vB on 0.3 BTC.
        assert!(check_fee(tx, sats(30_000_000), sats(282)).is_ok());
        // Small fee, big share of a small payment: fine.
        assert!(check_fee(tx, sats(1_000), sats(5_000)).is_ok());
        // Over 10% and over the floor.
        assert!(check_fee(tx, sats(100_000), sats(20_000)).is_err());
        // Rate cap: ~700 sat/vB, however large the amount.
        assert!(check_fee(tx, sats(30_000_000), sats(100_000)).is_err());
    }

    #[test]
    fn fee_priority_json_names() {
        let json = serde_json::to_string(&[FeePriority::Fast, FeePriority::Slow]).unwrap();
        assert_eq!(json, r#"["fast","slow"]"#);
        let parsed: FeePriority = serde_json::from_str(r#""normal""#).unwrap();
        assert_eq!(parsed, FeePriority::Normal);
        assert!(FeePriority::Fast.target_blocks() < FeePriority::Slow.target_blocks());
    }

    #[test]
    fn refuses_another_wallets_key() {
        let (wallet, mut psbt) = setup();
        let other = bdk_wallet::keys::bip39::Mnemonic::parse(
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
        )
        .unwrap();
        let other = keys::account_key(&other).unwrap();
        assert!(sign(&wallet, &other, &mut psbt).is_err());
    }

    #[test]
    fn rejects_weak_sighash() {
        let (wallet, mut psbt) = setup();
        psbt.inputs[0].sighash_type = Some(PsbtSighashType::from_str("SIGHASH_NONE").unwrap());
        assert!(check(&wallet, &psbt).is_err());
    }

    #[test]
    fn rejects_faked_input_value() {
        let (wallet, mut psbt) = setup();
        psbt.inputs[0].witness_utxo.as_mut().unwrap().value = Amount::from_sat(50_000_000);
        assert!(check(&wallet, &psbt).is_err());
    }

    #[test]
    fn rejects_missing_previous_transaction() {
        let (wallet, mut psbt) = setup();
        psbt.inputs[0].non_witness_utxo = None;
        assert!(check(&wallet, &psbt).is_err());
    }

    /// `setup`'s payment signed and in the wallet as unconfirmed, plus an honest bump of it
    /// to 5 sat/vB.
    fn setup_bump() -> (Wallet, Transaction, Psbt) {
        let (mut wallet, mut psbt) = setup();
        sign(&wallet, &account_key(), &mut psbt).unwrap();
        let original = finalize(&wallet, psbt).unwrap();
        wallet.apply_unconfirmed_txs([(original.clone(), 1)]);
        let rate = FeeRate::from_sat_per_vb(5).unwrap();
        let draft = bump(&mut wallet, original.compute_txid(), rate, &[]).unwrap();
        (wallet, original, draft.psbt)
    }

    #[test]
    fn honest_bump_passes_and_signs() {
        let (wallet, original, mut psbt) = setup_bump();
        let (review, old_fee) = review_bump(&wallet, &psbt, &original).unwrap();
        assert_eq!(old_fee, wallet.calculate_fee(&original).unwrap());
        assert!(review.fee > old_fee);
        // Same payment; the extra fee came out of the change.
        let old_change = original.output.iter().map(|o| o.value).sum::<Amount>()
            - Amount::from_sat(30_000_000);
        assert_eq!(review.change + (review.fee - old_fee), old_change);
        sign(&wallet, &account_key(), &mut psbt).unwrap();
        finalize(&wallet, psbt).unwrap();
    }

    #[test]
    fn bump_rejects_changed_payment() {
        let (wallet, original, mut psbt) = setup_bump();
        let i = recipient_output(&psbt);
        psbt.unsigned_tx.output[i].value = Amount::from_sat(20_000_000);
        assert!(review_bump(&wallet, &psbt, &original).is_err());
    }

    #[test]
    fn bump_rejects_redirected_payment() {
        let (wallet, original, mut psbt) = setup_bump();
        let i = recipient_output(&psbt);
        psbt.unsigned_tx.output[i].script_pubkey = addr(ATTACKER).script_pubkey();
        assert!(review_bump(&wallet, &psbt, &original).is_err());
    }

    #[test]
    fn bump_rejects_unrelated_transaction() {
        // A fresh payment instead of a replacement: it doesn't spend the original's coins.
        let (wallet, original, _) = setup_bump();
        let (_, other) = setup();
        let other_tx = Transaction { input: vec![], ..other.unsigned_tx.clone() };
        let mut fake = other.clone();
        fake.unsigned_tx = other_tx;
        fake.inputs.clear();
        assert!(review_bump(&wallet, &fake, &original).is_err());
    }

    #[test]
    fn bump_rejects_same_fee() {
        // The original itself, offered as its own "replacement".
        let (wallet, psbt) = setup();
        let mut signed = psbt.clone();
        sign(&wallet, &account_key(), &mut signed).unwrap();
        let original = finalize(&wallet, signed).unwrap();
        let err = review_bump(&wallet, &psbt, &original).err().unwrap().to_string();
        assert!(err.contains("isn't higher"), "{err}");
    }

    #[test]
    fn cannot_bump_unknown_transaction() {
        let (mut wallet, _) = setup();
        let rate = FeeRate::from_sat_per_vb(5).unwrap();
        let err = bump(&mut wallet, Txid::all_zeros(), rate, &[]).err().unwrap();
        assert!(matches!(err, BuildError::CannotBump(_)), "{err}");
    }

    fn recipient_output(psbt: &Psbt) -> usize {
        let spk = addr(RECIPIENT).script_pubkey();
        psbt.unsigned_tx.output.iter().position(|o| o.script_pubkey == spk).unwrap()
    }
}
