//! What the browser wallet can call. Thin wrappers: the logic lives in wallet-core, the same
//! code the CLI runs. Errors become JavaScript exceptions.

use std::str::FromStr;

use bdk_wallet::bitcoin::hex::{DisplayHex, FromHex};
use bdk_wallet::bitcoin::consensus::encode::deserialize_hex;
use bdk_wallet::bitcoin::{Address, Amount, Network, Psbt, Transaction, Txid};
use bdk_wallet::keys::bip39::{Language, Mnemonic, WordCount};
use bdk_wallet::keys::{GeneratableKey, GeneratedKey};
use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey, Segwitv0};
use bdk_wallet::{KeychainKind, Wallet};
use serde::{Deserialize, Serialize};
use wallet_core::crypto::{self, Encrypted, MIN_PASSWORD_LEN};
use wallet_core::{keys, ownership, review, sign, wallet_id};
use wasm_bindgen::prelude::*;

/// A fresh 12-word recovery phrase (128 bits from the browser's crypto.getRandomValues).
#[wasm_bindgen(js_name = generateMnemonic)]
pub fn generate_mnemonic() -> Result<String, JsError> {
    let mnemonic: GeneratedKey<Mnemonic, Segwitv0> =
        Mnemonic::generate((WordCount::Words12, Language::English))
            .map_err(|_| JsError::new("failed to generate a recovery phrase"))?;
    Ok(mnemonic.into_key().to_string())
}

/// Everything the browser keeps about a wallet. Only `encryptedKey` is secret, and it's
/// useless without the password; the recovery phrase itself is never returned or stored.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewWallet {
    /// The API's id for this wallet (same as the CLI computes).
    wallet_id: String,
    network: String,
    /// Public descriptors: what gets registered with the API.
    external: String,
    internal: String,
    first_address: String,
    encrypted_key: EncryptedKey,
}

/// The account key encrypted with the password (wallet_core::crypto), hex-encoded.
#[derive(Serialize, Deserialize)]
pub struct EncryptedKey {
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// Turns a recovery phrase into a wallet the browser can store: derives the account key,
/// encrypts it with `password`, and returns the public parts alongside. Slow on purpose
/// (Argon2id), about a second.
#[wasm_bindgen(js_name = createWallet)]
pub fn create_wallet(words: &str, password: &str, network: &str) -> Result<JsValue, JsError> {
    let network = parse_network(network)?;
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(JsError::new(&format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    let mnemonic = parse_mnemonic(words)?;
    let account_key = keys::account_key(&mnemonic).map_err(js)?;
    let wallet = public_wallet(&account_key, network)?;
    let external = wallet.public_descriptor(KeychainKind::External).to_string();
    let internal = wallet.public_descriptor(KeychainKind::Internal).to_string();
    let encrypted = crypto::encrypt(&account_key, password).map_err(js)?;

    let new_wallet = NewWallet {
        wallet_id: wallet_id(&external, &internal),
        network: network.to_string(),
        first_address: wallet.peek_address(KeychainKind::External, 0).address.to_string(),
        external,
        internal,
        encrypted_key: EncryptedKey {
            salt: encrypted.salt.to_lower_hex_string(),
            nonce: encrypted.nonce.to_lower_hex_string(),
            ciphertext: encrypted.ciphertext.to_lower_hex_string(),
        },
    };
    Ok(serde_wasm_bindgen::to_value(&new_wallet)?)
}

/// The address at `index` of a public descriptor, e.g. the wallet's receive descriptor. The
/// browser uses it to check addresses the server hands out: a compromised server could
/// otherwise show an attacker's address as "your receive address".
#[wasm_bindgen(js_name = addressAt)]
pub fn address_at(descriptor: &str, index: u32, network: &str) -> Result<String, JsError> {
    let network = parse_network(network)?;
    let descriptor = Descriptor::<DescriptorPublicKey>::from_str(descriptor).map_err(js)?;
    let address = descriptor.at_derivation_index(index).map_err(js)?.address(network).map_err(js)?;
    Ok(address.to_string())
}

/// What the user must see before signing, worked out from the PSBT itself.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReview {
    fee_sat: u64,
    change_sat: u64,
    inputs: usize,
    /// Fee rate once signed, sat/vB (a lower bound; see `signed_fee_rate`).
    fee_rate: f64,
    /// Set when the fee looks like a mistake (`check_fee`): the page must make the user
    /// confirm it explicitly, like the CLI's --allow-high-fee.
    fee_warning: Option<String>,
}

/// Checks a PSBT the server built for "pay `amount_sat` to `to`" before the user signs it: the
/// same `review` the CLI runs. It must pay exactly that, everything else must be our change,
/// and input values are verified against their previous transactions. Throws if not.
#[wasm_bindgen(js_name = reviewSend)]
pub fn review_send(
    wallet: JsValue,
    psbt: &str,
    to: &str,
    amount_sat: u64,
) -> Result<JsValue, JsError> {
    let stored: StoredWallet = serde_wasm_bindgen::from_value(wallet)?;
    let network = parse_network(&stored.network)?;
    let wallet = stored.watch_only(network)?;
    let psbt = Psbt::from_str(psbt).map_err(|e| JsError::new(&format!("invalid PSBT: {e}")))?;
    let to = Address::from_str(to)
        .map_err(|_| JsError::new("invalid address"))?
        .require_network(network)
        .map_err(|_| JsError::new("address is for a different network"))?;
    let amount = Amount::from_sat(amount_sat);

    let review = review::review(&wallet, &psbt, &to, amount).map_err(js)?;
    let fee_warning = review::check_fee(&psbt.unsigned_tx, amount, review.fee)
        .err()
        .map(|e| e.to_string());
    let summary = SendReview {
        fee_sat: review.fee.to_sat(),
        change_sat: review.change.to_sat(),
        inputs: review.inputs,
        fee_rate: review::signed_fee_rate(&psbt.unsigned_tx, review.fee),
        fee_warning,
    };
    Ok(serde_wasm_bindgen::to_value(&summary)?)
}

/// What the user must see before signing a fee bump, worked out from the PSBTs themselves.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BumpReview {
    /// What the payments add up to; unchanged from the original.
    paid_sat: u64,
    old_fee_sat: u64,
    /// The original is signed, so its rate is exact.
    old_fee_rate: f64,
    fee_sat: u64,
    fee_rate: f64,
    change_sat: u64,
    fee_warning: Option<String>,
}

/// Checks a fee bump (RBF replacement) the server built for our transaction `txid`, given the
/// original transaction (hex) the server sent along: the CLI's `review_bump`. The original's
/// txid must be `txid` (a txid is a hash of the transaction, so the server can't doctor it);
/// the replacement must spend all its coins, pay every payment exactly as before, send the
/// rest only to our change, and pay a higher fee. Throws if not.
#[wasm_bindgen(js_name = reviewBump)]
pub fn review_bump(
    wallet: JsValue,
    psbt: &str,
    original_tx: &str,
    txid: &str,
) -> Result<JsValue, JsError> {
    let stored: StoredWallet = serde_wasm_bindgen::from_value(wallet)?;
    let network = parse_network(&stored.network)?;
    let wallet = stored.watch_only(network)?;
    let psbt = Psbt::from_str(psbt).map_err(|e| JsError::new(&format!("invalid PSBT: {e}")))?;
    let txid = Txid::from_str(txid).map_err(|_| JsError::new("invalid txid"))?;
    let original: Transaction = deserialize_hex(original_tx)
        .map_err(|_| JsError::new("the server sent an invalid original transaction"))?;
    if original.compute_txid() != txid {
        return Err(JsError::new(&format!(
            "the server sent a different transaction than {txid}"
        )));
    }

    let (review, old_fee) = review::review_bump(&wallet, &psbt, &original).map_err(js)?;
    let output_total: Amount = psbt.unsigned_tx.output.iter().map(|o| o.value).sum();
    let paid = output_total - review.change;
    let fee_warning = review::check_fee(&psbt.unsigned_tx, paid, review.fee)
        .err()
        .map(|e| e.to_string());
    let summary = BumpReview {
        paid_sat: paid.to_sat(),
        old_fee_sat: old_fee.to_sat(),
        old_fee_rate: old_fee.to_sat() as f64 / original.vsize() as f64,
        fee_sat: review.fee.to_sat(),
        fee_rate: review::signed_fee_rate(&psbt.unsigned_tx, review.fee),
        change_sat: review.change.to_sat(),
        fee_warning,
    };
    Ok(serde_wasm_bindgen::to_value(&summary)?)
}

/// Decrypts the account key with `password` and signs `psbt` (base64), returning the signed
/// PSBT. The key exists only inside this call: it never reaches JavaScript. Throws on a wrong
/// password, or a key that doesn't belong to this wallet.
#[wasm_bindgen(js_name = signPsbt)]
pub fn sign_psbt(wallet: JsValue, password: &str, psbt: &str) -> Result<String, JsError> {
    let stored: StoredWallet = serde_wasm_bindgen::from_value(wallet)?;
    let network = parse_network(&stored.network)?;
    let wallet = stored.watch_only(network)?;
    let mut psbt =
        Psbt::from_str(psbt).map_err(|e| JsError::new(&format!("invalid PSBT: {e}")))?;
    let account_key = stored.decrypt_key(password)?;
    sign::sign(&wallet, &account_key, &mut psbt).map_err(js)?;
    Ok(psbt.to_string())
}

/// Signs the server's token-recovery `challenge` with the wallet's key (decrypted with
/// `password`, inside WASM), proving we own the wallet so the server issues a new API token.
#[wasm_bindgen(js_name = proveOwnership)]
pub fn prove_ownership(wallet: JsValue, password: &str, challenge: &str) -> Result<String, JsError> {
    let stored: StoredWallet = serde_wasm_bindgen::from_value(wallet)?;
    let network = parse_network(&stored.network)?;
    let account_key = stored.decrypt_key(password)?;
    // The server would refuse a signature by the wrong key anyway; this says why up front.
    sign::check_account_key(&stored.watch_only(network)?, &account_key).map_err(js)?;
    ownership::sign(&account_key, &stored.wallet_id, challenge).map_err(js)
}

/// The parts of the browser's stored wallet (web/src/lib/store.ts) these functions need.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWallet {
    wallet_id: String,
    network: String,
    external: String,
    internal: String,
    encrypted_key: EncryptedKey,
}

impl StoredWallet {
    /// A wallet from the public descriptors only, in memory: enough to review PSBTs and know
    /// which keys to sign with.
    fn watch_only(&self, network: Network) -> Result<Wallet, JsError> {
        Wallet::create(self.external.clone(), self.internal.clone())
            .network(network)
            .create_wallet_no_persist()
            .map_err(js)
    }

    /// The account key, decrypted with `password`. Never leaves WASM.
    fn decrypt_key(&self, password: &str) -> Result<String, JsError> {
        let key = &self.encrypted_key;
        let encrypted = Encrypted {
            salt: Vec::from_hex(&key.salt).map_err(js)?,
            nonce: Vec::from_hex(&key.nonce).map_err(js)?,
            ciphertext: Vec::from_hex(&key.ciphertext).map_err(js)?,
        };
        crypto::decrypt(&encrypted, password).map_err(js)
    }
}

/// An in-memory wallet for deriving descriptors and addresses. Nothing is persisted.
fn public_wallet(account_key: &str, network: Network) -> Result<Wallet, JsError> {
    let (external, internal) = keys::descriptors(account_key).map_err(js)?;
    Wallet::create(external, internal).network(network).create_wallet_no_persist().map_err(js)
}

fn parse_mnemonic(words: &str) -> Result<Mnemonic, JsError> {
    keys::parse_mnemonic(words).map_err(js)
}

/// Mainnet is refused, as in the CLI: keys use the testnet coin type.
fn parse_network(network: &str) -> Result<Network, JsError> {
    match Network::from_str(network) {
        Ok(Network::Bitcoin) | Err(_) => {
            Err(JsError::new(&format!("unsupported network {network:?}")))
        }
        Ok(network) => Ok(network),
    }
}

fn js(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}
