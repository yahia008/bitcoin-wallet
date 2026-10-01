//! Proving to the API server that you control a wallet's keys, without revealing them, so it
//! can issue a new API token (e.g. after the browser lost it). The server sends a random
//! one-time challenge; the wallet signs it with the key of its first receive address
//! (external/0); the server checks the signature against the public key it derives from the
//! wallet's registered descriptor. Only the key's holder can produce that signature.

use anyhow::{Context, bail};
use bdk_wallet::bitcoin::bip32::{ChildNumber, DerivationPath};
use bdk_wallet::bitcoin::hashes::{Hash, sha256};
use bdk_wallet::bitcoin::hex::{DisplayHex, FromHex};
use bdk_wallet::bitcoin::secp256k1::{Message, Secp256k1, ecdsa::Signature};
use bdk_wallet::descriptor::ExtendedDescriptor;
use bdk_wallet::miniscript::Descriptor;

use crate::keys;

/// What gets signed. The fixed prefix means the signature can't be passed off as anything
/// else (a transaction, or another app's message); the wallet id and challenge bind it to
/// this request only.
fn message(wallet_id: &str, challenge: &str) -> Message {
    let text =
        format!("bitcoin-wallet API token recovery\nwallet: {wallet_id}\nchallenge: {challenge}");
    Message::from_digest(sha256::Hash::hash(text.as_bytes()).to_byte_array())
}

/// Signs `challenge` with the account's external/0 key. Returns the DER signature, hex.
pub fn sign(account_key: &str, wallet_id: &str, challenge: &str) -> anyhow::Result<String> {
    let secp = Secp256k1::new();
    let (_, _, account) = keys::parse_account_key(account_key)?;
    let path = DerivationPath::from(vec![ChildNumber::Normal { index: 0 }; 2]); // external/0
    let key = account.derive_priv(&secp, &path)?.private_key;
    let signature = secp.sign_ecdsa(&message(wallet_id, challenge), &key);
    Ok(signature.serialize_der().to_lower_hex_string())
}

/// Checks `signature` (DER, hex) over `challenge` against the key at index 0 of the wallet's
/// public receive descriptor.
pub fn verify(
    external: &ExtendedDescriptor,
    wallet_id: &str,
    challenge: &str,
    signature: &str,
) -> anyhow::Result<()> {
    let secp = Secp256k1::verification_only();
    let Descriptor::Wpkh(wpkh) =
        external.at_derivation_index(0)?.derived_descriptor(&secp)?
    else {
        bail!("only wpkh wallets can prove ownership");
    };
    let signature = Vec::from_hex(signature).context("signature isn't hex")?;
    let signature = Signature::from_der(&signature).context("signature isn't valid DER")?;
    secp.verify_ecdsa(&message(wallet_id, challenge), &signature, &wpkh.as_inner().inner)
        .context("the signature doesn't match this wallet's key")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bdk_wallet::keys::bip39::Mnemonic;

    use super::*;

    fn account(words: &str) -> String {
        keys::account_key(&Mnemonic::parse(words).unwrap()).unwrap()
    }

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon \
                           abandon abandon abandon abandon abandon about";
    const LEGAL: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    fn external(account_key: &str) -> ExtendedDescriptor {
        let (descriptor, _) =
            keys::derive(account_key, bdk_wallet::KeychainKind::External).unwrap();
        ExtendedDescriptor::from_str(&descriptor.to_string()).unwrap()
    }

    #[test]
    fn owner_can_prove() {
        let key = account(ABANDON);
        let signature = sign(&key, "abcd", "c0ffee").unwrap();
        verify(&external(&key), "abcd", "c0ffee", &signature).unwrap();
    }

    #[test]
    fn signature_is_bound_to_challenge_and_wallet() {
        let key = account(ABANDON);
        let signature = sign(&key, "abcd", "c0ffee").unwrap();
        assert!(verify(&external(&key), "abcd", "other", &signature).is_err());
        assert!(verify(&external(&key), "efgh", "c0ffee", &signature).is_err());
    }

    #[test]
    fn another_wallets_key_is_refused() {
        let signature = sign(&account(LEGAL), "abcd", "c0ffee").unwrap();
        assert!(verify(&external(&account(ABANDON)), "abcd", "c0ffee", &signature).is_err());
    }

    #[test]
    fn garbage_is_refused() {
        let key = account(ABANDON);
        assert!(verify(&external(&key), "abcd", "c0ffee", "zz").is_err());
        assert!(verify(&external(&key), "abcd", "c0ffee", "3044").is_err());
    }
}
