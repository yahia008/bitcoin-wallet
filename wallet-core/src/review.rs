//! Checks the signer runs before signing a PSBT someone else built (the API server), plus
//! the fee checks shown to the user. Everything here works from the PSBT alone and the
//! wallet's public descriptors.

use std::collections::HashMap;

use anyhow::{anyhow, bail};
use bdk_wallet::bitcoin::bip32::ChildNumber;
use bdk_wallet::bitcoin::{
    Address, Amount, EcdsaSighashType, OutPoint, Psbt, Transaction, TxOut,
};
use bdk_wallet::{KeychainKind, Wallet};

/// Weight a P2WPKH input's witness adds once signed: item count (1) + signature with its
/// length byte (1 + up to 73) + compressed public key with its length byte (1 + 33).
const P2WPKH_WITNESS_WEIGHT: u64 = 1 + 1 + 73 + 1 + 33;

/// The SegWit marker and flag bytes a signed transaction gains (1 weight unit each).
const SEGWIT_HEADER_WEIGHT: u64 = 2;

/// The fee rate `tx` will pay once signed, in sat/vB. The unsigned transaction has no
/// witnesses yet, so add the largest a P2WPKH signature can make them; the real rate comes
/// out the same or slightly higher.
pub fn signed_fee_rate(tx: &Transaction, fee: Amount) -> f64 {
    let weight = tx.weight().to_wu()
        + SEGWIT_HEADER_WEIGHT
        + P2WPKH_WITNESS_WEIGHT * tx.input.len() as u64;
    // 1 vbyte = 4 weight units, rounded up like the network does.
    fee.to_sat() as f64 / weight.div_ceil(4) as f64
}

/// Above this rate a fee is almost certainly a typo: even busy mainnet rarely goes past a few
/// hundred sat/vB.
const MAX_FEE_RATE_SAT_VB: f64 = 500.0;
/// A fee over this share of the amount sent looks like a mistake...
const MAX_FEE_PERCENT: u64 = 10;
/// ...unless it's small anyway: small payments often pay a big share in fees, harmlessly.
const FEE_PERCENT_FLOOR: Amount = Amount::from_sat(10_000);

/// Refuses fees that look like a mistake (a mistyped --fee-rate, or a server trying to drain
/// the wallet through fees): over MAX_FEE_RATE_SAT_VB, or over MAX_FEE_PERCENT of `amount`
/// once the fee is above FEE_PERCENT_FLOOR.
pub fn check_fee(tx: &Transaction, amount: Amount, fee: Amount) -> anyhow::Result<()> {
    let rate = signed_fee_rate(tx, fee);
    if rate > MAX_FEE_RATE_SAT_VB {
        bail!("fee rate ~{rate:.0} sat/vB is above {MAX_FEE_RATE_SAT_VB} sat/vB");
    }
    // Compare in integers: fee / amount > MAX_FEE_PERCENT / 100.
    if fee > FEE_PERCENT_FLOOR && fee.to_sat() * 100 > amount.to_sat() * MAX_FEE_PERCENT {
        let percent = fee.to_sat() * 100 / amount.to_sat().max(1);
        bail!("fee {fee} is {percent}% of the {amount} being sent");
    }
    Ok(())
}

/// What a PSBT really does, computed by the signer from the PSBT itself.
pub struct Review {
    pub fee: Amount,
    pub change: Amount,
    pub inputs: usize,
}

/// Checks a PSBT that someone else (the API server) built, before we sign it. A compromised
/// server could otherwise get us to sign a transaction that pays the attacker.
///
/// It must pay exactly `amount` to `to`, every other output must be our own change (we derive
/// the change script ourselves and compare), and the fee is computed from input values we
/// verify against the previous transactions' hashes.
pub fn review(wallet: &Wallet, psbt: &Psbt, to: &Address, amount: Amount) -> anyhow::Result<Review> {
    let payment = TxOut { value: amount, script_pubkey: to.script_pubkey() };
    let inputs = verified_inputs(psbt)?;
    let change = check_outputs(wallet, psbt, &[payment])?;
    Ok(Review { fee: fee_of(&psbt.unsigned_tx, &inputs)?, change, inputs: inputs.len() })
}

/// Checks a fee bump (RBF replacement) of `original` that someone else built. Besides the
/// usual checks, the replacement must:
/// - spend every coin the original spent, so it really replaces it (and we know what the
///   original paid in fees, from input values verified the same way);
/// - pay every payment of the original exactly as before: only our change may shrink;
/// - pay a higher fee than the original.
///
/// Returns the review and the original's fee.
pub fn review_bump(
    wallet: &Wallet,
    psbt: &Psbt,
    original: &Transaction,
) -> anyhow::Result<(Review, Amount)> {
    let inputs = verified_inputs(psbt)?;
    for txin in &original.input {
        if !inputs.contains_key(&txin.previous_output) {
            bail!(
                "the replacement doesn't spend {}, so it doesn't replace the original",
                txin.previous_output
            );
        }
    }
    // The original's payments are its outputs that aren't our change.
    let payments: Vec<TxOut> = original
        .output
        .iter()
        .filter(|out| {
            !matches!(
                wallet.derivation_of_spk(out.script_pubkey.clone()),
                Some((KeychainKind::Internal, _))
            )
        })
        .cloned()
        .collect();
    let change = check_outputs(wallet, psbt, &payments)?;

    let fee = fee_of(&psbt.unsigned_tx, &inputs)?;
    let old_fee = fee_of(original, &inputs)?;
    if fee <= old_fee {
        bail!("the replacement's fee {fee} isn't higher than the original's {old_fee}");
    }
    Ok((Review { fee, change, inputs: inputs.len() }, old_fee))
}

/// The value of every coin the PSBT spends, checked against the previous transactions.
fn verified_inputs(psbt: &Psbt) -> anyhow::Result<HashMap<OutPoint, Amount>> {
    let tx = &psbt.unsigned_tx;
    if tx.input.len() != psbt.inputs.len() || tx.output.len() != psbt.outputs.len() {
        bail!("malformed PSBT");
    }

    let mut values = HashMap::new();
    for (i, (txin, input)) in tx.input.iter().zip(&psbt.inputs).enumerate() {
        // SIGHASH_ALL commits to all inputs and outputs. Anything weaker (NONE, SINGLE,
        // ANYONECANPAY) would let someone change the transaction after we sign it.
        if let Some(sighash) = input.sighash_type
            && sighash.ecdsa_hash_ty() != Ok(EcdsaSighashType::All)
        {
            bail!("input {i} requests sighash {sighash}; only ALL is allowed");
        }
        // The full previous tx is hash-committed by its txid, so its output value can't be
        // faked. (A bare witness_utxo value could be, which would hide a huge fee.)
        let prev = input
            .non_witness_utxo
            .as_ref()
            .ok_or_else(|| anyhow!("input {i} lacks its previous transaction; can't verify it"))?;
        if prev.compute_txid() != txin.previous_output.txid {
            bail!("input {i}: previous transaction doesn't match the outpoint");
        }
        let spent = prev
            .output
            .get(txin.previous_output.vout as usize)
            .ok_or_else(|| anyhow!("input {i}: outpoint index out of range"))?;
        if input.witness_utxo.as_ref().is_some_and(|w| w != spent) {
            bail!("input {i}: witness_utxo disagrees with the previous transaction");
        }
        values.insert(txin.previous_output, spent.value);
    }
    Ok(values)
}

/// Requires the PSBT to pay each of `payments` exactly, with every other output going to our
/// own change. Returns the change total.
fn check_outputs(wallet: &Wallet, psbt: &Psbt, payments: &[TxOut]) -> anyhow::Result<Amount> {
    let mut unpaid: Vec<&TxOut> = payments.iter().collect();
    let change_descriptor = wallet.public_descriptor(KeychainKind::Internal);
    let mut change = Amount::ZERO;
    for (i, (txout, output)) in psbt.unsigned_tx.output.iter().zip(&psbt.outputs).enumerate() {
        if let Some(pos) = unpaid.iter().position(|p| p.script_pubkey == txout.script_pubkey) {
            let payment = unpaid.remove(pos);
            if txout.value != payment.value {
                bail!(
                    "PSBT pays {} to the recipient, but you asked for {}",
                    txout.value,
                    payment.value
                );
            }
            continue;
        }
        // Not a payment, so it must be our change. The PSBT says which change index it is;
        // derive that script from our own descriptor and require an exact match.
        let index = output
            .bip32_derivation
            .values()
            .find_map(|(_, path)| match path.into_iter().last() {
                Some(ChildNumber::Normal { index }) => Some(*index),
                _ => None,
            })
            .ok_or_else(|| anyhow!("output {i} pays an address that is neither yours nor the recipient's"))?;
        let expected = change_descriptor.at_derivation_index(index)?.script_pubkey();
        if expected != txout.script_pubkey {
            bail!("output {i} claims to be change but doesn't pay your change address");
        }
        change += txout.value;
    }
    if let Some(missing) = unpaid.first() {
        bail!("PSBT is missing the payment of {} to {}", missing.value, missing.script_pubkey);
    }
    Ok(change)
}

/// Inputs minus outputs, with input values from `values` (already verified).
fn fee_of(tx: &Transaction, values: &HashMap<OutPoint, Amount>) -> anyhow::Result<Amount> {
    let mut input_total = Amount::ZERO;
    for txin in &tx.input {
        input_total += *values
            .get(&txin.previous_output)
            .ok_or_else(|| anyhow!("no verified value for input {}", txin.previous_output))?;
    }
    let output_total: Amount = tx.output.iter().map(|o| o.value).sum();
    input_total
        .checked_sub(output_total)
        .ok_or_else(|| anyhow!("outputs exceed inputs"))
}
