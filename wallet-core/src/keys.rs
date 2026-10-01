//! BIP84 key derivation: mnemonic -> account key -> output descriptors.
//!
//! The mnemonic is the root of every key it can ever produce (all accounts, all coins). A
//! wallet only needs one branch of that tree, the account key at m/84'/1'/0', so that's the
//! only secret we keep: if it leaks, the rest of the tree stays safe.

use std::str::FromStr;

use bdk_wallet::KeychainKind;
use bdk_wallet::bitcoin::NetworkKind;
use anyhow::bail;
use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint, Xpriv};
use bdk_wallet::bitcoin::secp256k1::Secp256k1;
use bdk_wallet::descriptor::ExtendedDescriptor;
use anyhow::anyhow;
use bdk_wallet::keys::bip39::{self, Language, Mnemonic};
use bdk_wallet::miniscript::descriptor::{DescriptorSecretKey, DescriptorXKey, KeyMap, Wildcard};

/// BIP84 account path: purpose 84' (native SegWit) / coin type 1' (test networks) / account 0'.
const ACCOUNT_PATH: &str = "m/84h/1h/0h";

/// Parses a recovery phrase as typed: any spacing, any case. Errors are worded for people,
/// counting words from 1 (the bip39 crate counts from 0).
pub fn parse_mnemonic(phrase: &str) -> anyhow::Result<Mnemonic> {
    let words: Vec<String> = phrase.split_whitespace().map(str::to_lowercase).collect();
    Mnemonic::parse_in(Language::English, words.join(" ")).map_err(|e| match e {
        bip39::Error::UnknownWord(i) => anyhow!(
            "invalid recovery phrase: word {} (\"{}\") isn't in the recovery phrase word list",
            i + 1,
            words[i]
        ),
        bip39::Error::BadWordCount(n) => {
            anyhow!("invalid recovery phrase: it has {n} words; it should have 12 or 24")
        }
        bip39::Error::InvalidChecksum => anyhow!(
            "invalid recovery phrase: the words are all valid, but not in this combination; \
             check for a swapped or mistyped word"
        ),
        e => anyhow!("invalid recovery phrase: {e}"),
    })
}

/// Derives the account key from the mnemonic, as `[fingerprint/84'/1'/0']tprv…`. The origin
/// in brackets (master key fingerprint + path) is what lets a signer match a PSBT input's
/// BIP32 derivation info to this key.
pub fn account_key(mnemonic: &Mnemonic) -> anyhow::Result<String> {
    let secp = Secp256k1::new();
    // Empty BIP39 passphrase. NetworkKind::Test makes it a tprv, used by every test network.
    let master = Xpriv::new_master(NetworkKind::Test, &mnemonic.to_seed(""))?;
    let path = DerivationPath::from_str(ACCOUNT_PATH)?;
    let key = DescriptorSecretKey::XPrv(DescriptorXKey {
        origin: Some((master.fingerprint(&secp), path.clone())),
        xkey: master.derive_priv(&secp, &path)?,
        derivation_path: DerivationPath::master(),
        wildcard: Wildcard::None,
    });
    Ok(key.to_string())
}

/// Splits an account key into its origin (master fingerprint, account path) and the tprv.
pub fn parse_account_key(account_key: &str) -> anyhow::Result<(Fingerprint, DerivationPath, Xpriv)> {
    match DescriptorSecretKey::from_str(account_key)? {
        DescriptorSecretKey::XPrv(DescriptorXKey {
            origin: Some((fingerprint, path)),
            xkey,
            derivation_path,
            wildcard: Wildcard::None,
        }) if derivation_path.is_master() => Ok((fingerprint, path, xkey)),
        _ => bail!("not an account key ([fingerprint/path]tprv...)"),
    }
}

/// Tells a stored account key apart from a mnemonic (what older wallets stored).
pub fn is_account_key(secret: &str) -> bool {
    DescriptorSecretKey::from_str(secret).is_ok()
}

/// The descriptor for one keychain, `.../0/*` receive or `.../1/*` change, as the public
/// descriptor plus the map from its public keys to private keys.
pub fn derive(
    account_key: &str,
    keychain: KeychainKind,
) -> anyhow::Result<(ExtendedDescriptor, KeyMap)> {
    let branch = match keychain {
        KeychainKind::External => 0,
        KeychainKind::Internal => 1,
    };
    let descriptor = format!("wpkh({account_key}/{branch}/*)");
    Ok(ExtendedDescriptor::parse_descriptor(&Secp256k1::new(), &descriptor)?)
}

/// Receive and change descriptors with private keys embedded (tprv), for `Wallet::create`.
/// These strings must never be logged or stored; BDK itself only persists the public form.
pub fn descriptors(account_key: &str) -> anyhow::Result<(String, String)> {
    let (ext, ext_keys) = derive(account_key, KeychainKind::External)?;
    let (int, int_keys) = derive(account_key, KeychainKind::Internal)?;
    Ok((
        ext.to_string_with_secret(&ext_keys),
        int.to_string_with_secret(&int_keys),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mnemonic() -> Mnemonic {
        Mnemonic::parse(
            "abandon abandon abandon abandon abandon abandon \
             abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    #[test]
    fn parse_mnemonic_is_forgiving_and_explains_errors() {
        let messy = "  ABANDON abandon\tabandon abandon abandon abandon abandon abandon abandon \
                     abandon abandon   about ";
        assert_eq!(parse_mnemonic(messy).unwrap(), mnemonic());
        let unknown = parse_mnemonic(&mnemonic().to_string().replacen("about", "aboot", 1));
        assert!(unknown.unwrap_err().to_string().contains("word 12 (\"aboot\")"));
        let short = parse_mnemonic("abandon abandon abandon");
        assert!(short.unwrap_err().to_string().contains("3 words"));
        let swapped = parse_mnemonic(&mnemonic().to_string().replacen("about", "abandon", 1));
        assert!(swapped.unwrap_err().to_string().contains("not in this combination"));
    }

    #[test]
    fn account_key_has_origin() {
        let key = account_key(&mnemonic()).unwrap();
        assert!(key.starts_with("[73c5da0a/84'/1'/0']tprv"), "{key}");
        assert!(is_account_key(&key));
        assert!(!is_account_key(&mnemonic().to_string()));
    }

    /// Wallets created before account-key storage derived their descriptors straight from
    /// the mnemonic with BDK's `descriptor!` macro. The account key must give the same public
    /// descriptors, or upgrading those wallets would fail the match check in `signing_keys`.
    #[test]
    fn matches_old_mnemonic_derivation() {
        use bdk_wallet::descriptor;
        use bdk_wallet::descriptor::IntoWalletDescriptor;
        // The macro expands to paths starting with `miniscript::`.
        use bdk_wallet::miniscript;

        let account = account_key(&mnemonic()).unwrap();
        for (keychain, branch) in [(KeychainKind::External, 0), (KeychainKind::Internal, 1)] {
            let path = DerivationPath::from_str(&format!("{ACCOUNT_PATH}/{branch}")).unwrap();
            let (old, _) = descriptor!(wpkh(((mnemonic(), None::<String>), path)))
                .unwrap()
                .into_wallet_descriptor(&Secp256k1::new(), NetworkKind::Test)
                .unwrap();
            let (new, _) = derive(&account, keychain).unwrap();
            assert_eq!(new.to_string(), old.to_string());
        }
    }

    /// BIP84's test vector for this mnemonic: first testnet receive address.
    #[test]
    fn matches_bip84_test_vector() {
        let (descriptor, _) = derive(&account_key(&mnemonic()).unwrap(), KeychainKind::External)
            .unwrap();
        let address = descriptor
            .at_derivation_index(0)
            .unwrap()
            .address(bdk_wallet::bitcoin::Network::Testnet)
            .unwrap();
        assert_eq!(address.to_string(), "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl");
    }
}
