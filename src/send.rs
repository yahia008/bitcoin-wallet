//! Spending, split into the steps a transaction goes through:
//!
//!   build (watch-only) -> review (signer checks it) -> sign (needs keys)
//!   -> finalize (watch-only) -> broadcast (watch-only)
//!
//! Only `sign` touches private keys, so the API server can do everything else while the
//! signing stays on the user's device.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use bdk_wallet::bitcoin::bip32::{ChildNumber, DerivationPath, Fingerprint, Xpriv};
use bdk_wallet::bitcoin::psbt::{GetKey, GetKeyError, KeyRequest};
use bdk_wallet::bitcoin::secp256k1::{Secp256k1, Signing};
use bdk_wallet::bitcoin::{
    Address, Amount, EcdsaSighashType, FeeRate, OutPoint, PrivateKey, Psbt, Transaction, Txid,
};
use bdk_wallet::error::CreateTxError;
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, PersistedWallet, SignOptions, Wallet};

use crate::chain::Backend;
use crate::keys;

/// Aim for confirmation within this many blocks when asking for a fee estimate.
const CONF_TARGET: u16 = 6;

/// Used when the backend has no fee estimate (always the case on a fresh regtest chain).
pub const FALLBACK_FEE_RATE_SAT_VB: u64 = 2;

/// The caller's fee rate if given, else the backend's estimate, else the fallback.
pub fn choose_fee_rate(
    backend: &Backend,
    requested_sat_vb: Option<u64>,
) -> anyhow::Result<FeeRate> {
    match requested_sat_vb {
        Some(rate) => FeeRate::from_sat_per_vb(rate).context("fee rate too large"),
        None => Ok(backend
            .estimate_fee_rate(CONF_TARGET)
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

/// Why a transaction couldn't be built. The first two are the caller's problem (HTTP 400).
#[derive(Debug)]
pub enum BuildError {
    InsufficientFunds(String),
    BelowDust,
    Other(anyhow::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::InsufficientFunds(e) => write!(f, "insufficient funds: {e}"),
            BuildError::BelowDust => write!(f, "amount is below the dust limit; send more"),
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
    let psbt = builder.finish().map_err(|e| match e {
        CreateTxError::CoinSelection(e) => BuildError::InsufficientFunds(e.to_string()),
        CreateTxError::OutputBelowDustLimit(_) => BuildError::BelowDust,
        e => BuildError::Other(e.into()),
    })?;

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
    let tx = &psbt.unsigned_tx;
    if tx.input.len() != psbt.inputs.len() || tx.output.len() != psbt.outputs.len() {
        bail!("malformed PSBT");
    }

    let mut input_total = Amount::ZERO;
    for (i, (txin, input)) in tx.input.iter().zip(&psbt.inputs).enumerate() {
        // SIGHASH_ALL commits to all inputs and outputs. Anything weaker (NONE, SINGLE,
        // ANYONECANPAY) would let someone change the transaction after we sign it.
        if let Some(sighash) = input.sighash_type {
            if sighash.ecdsa_hash_ty() != Ok(EcdsaSighashType::All) {
                bail!("input {i} requests sighash {sighash}; only ALL is allowed");
            }
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
        input_total += spent.value;
    }

    let recipient = to.script_pubkey();
    let change_descriptor = wallet.public_descriptor(KeychainKind::Internal);
    let (mut paid, mut change) = (Amount::ZERO, Amount::ZERO);
    for (i, (txout, output)) in tx.output.iter().zip(&psbt.outputs).enumerate() {
        if txout.script_pubkey == recipient {
            paid += txout.value;
            continue;
        }
        // Not the recipient, so it must be our change. The PSBT says which change index it is;
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
    if paid != amount {
        bail!("PSBT pays {paid} to the recipient, but you asked for {amount}");
    }

    let output_total: Amount = tx.output.iter().map(|o| o.value).sum();
    let fee = input_total
        .checked_sub(output_total)
        .ok_or_else(|| anyhow!("PSBT outputs exceed its inputs"))?;
    Ok(Review { fee, change, inputs: tx.input.len() })
}

/// Checks that `account_key` really belongs to `wallet`: its public descriptors must match
/// the wallet's exactly.
pub fn check_account_key(wallet: &Wallet, account_key: &str) -> anyhow::Result<()> {
    for keychain in [KeychainKind::External, KeychainKind::Internal] {
        let (descriptor, _) = keys::derive(account_key, keychain)?;
        if descriptor.to_string() != wallet.public_descriptor(keychain).to_string() {
            bail!("the stored key does not match this wallet's descriptors");
        }
    }
    Ok(())
}

/// Signs every input we hold keys for, using keys derived from `account_key`. This is the
/// only step that touches private keys; they live only inside this function.
pub fn sign(wallet: &Wallet, account_key: &str, psbt: &mut Psbt) -> anyhow::Result<()> {
    check_account_key(wallet, account_key)?;
    let (master_fingerprint, account_path, xprv) = keys::parse_account_key(account_key)?;
    let signer = AccountSigner { master_fingerprint, account_path, xprv };

    // Each PSBT input records which key (master fingerprint + BIP32 path) can spend it; the
    // signer derives that key and adds a signature per input.
    psbt.sign(&signer, wallet.secp_ctx())
        .map_err(|(_, errors)| anyhow!("signing failed: {errors:?}"))?;
    Ok(())
}

/// Hands the PSBT signer the key for each input, derived from the account key.
///
/// We don't use miniscript's `KeyMapWrapper` here: for a key with an origin like ours
/// (`[fp/84'/1'/0']tprv`), miniscript 12.3 derives the input's whole path (84'/1'/0'/0/i)
/// from the account key instead of just the part after the origin (0/i), so it signs with
/// a key that isn't ours.
struct AccountSigner {
    master_fingerprint: Fingerprint,
    account_path: DerivationPath,
    xprv: Xpriv,
}

impl GetKey for AccountSigner {
    type Error = GetKeyError;

    fn get_key<C: Signing>(
        &self,
        key_request: KeyRequest,
        secp: &Secp256k1<C>,
    ) -> Result<Option<PrivateKey>, Self::Error> {
        let KeyRequest::Bip32((fingerprint, path)) = key_request else {
            return Ok(None);
        };
        // Only keys below our account, e.g. m/84'/1'/0' + 0/5.
        let Some(rest) = path.as_ref().strip_prefix(self.account_path.as_ref()) else {
            return Ok(None);
        };
        if fingerprint != self.master_fingerprint {
            return Ok(None);
        }
        let rest = DerivationPath::from(rest.to_vec());
        Ok(Some(self.xprv.derive_priv(secp, &rest)?.to_priv()))
    }
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

    fn recipient_output(psbt: &Psbt) -> usize {
        let spk = addr(RECIPIENT).script_pubkey();
        psbt.unsigned_tx.output.iter().position(|o| o.script_pubkey == spk).unwrap()
    }
}
