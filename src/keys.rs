//! BIP84 key derivation: mnemonic -> output descriptors and their private keys.

use std::str::FromStr;

use bdk_wallet::bitcoin::NetworkKind;
use bdk_wallet::bitcoin::bip32::DerivationPath;
use bdk_wallet::bitcoin::secp256k1::Secp256k1;
use bdk_wallet::descriptor::{ExtendedDescriptor, IntoWalletDescriptor};
use bdk_wallet::keys::bip39::Mnemonic;
use bdk_wallet::miniscript::{self, descriptor::KeyMap};
use bdk_wallet::{KeychainKind, descriptor};

/// BIP84 account path: purpose 84' (native SegWit) / coin type 1' (test networks) / account 0'.
const ACCOUNT_PATH: &str = "m/84h/1h/0h";

/// Derives the descriptor for one keychain: `.../0/*` receive, `.../1/*` change.
/// Returns the public descriptor plus the map from its public keys to private keys.
pub fn derive(
    mnemonic: &Mnemonic,
    keychain: KeychainKind,
) -> anyhow::Result<(ExtendedDescriptor, KeyMap)> {
    let branch = match keychain {
        KeychainKind::External => 0,
        KeychainKind::Internal => 1,
    };
    let path = DerivationPath::from_str(&format!("{ACCOUNT_PATH}/{branch}"))?;
    // (mnemonic, None) = no BIP39 passphrase.
    let key = (mnemonic.clone(), None::<String>);

    // NetworkKind::Test makes the keys tprv/tpub, used by testnet, signet and regtest.
    let (descriptor, keymap) = descriptor!(wpkh((key, path)))?
        .into_wallet_descriptor(&Secp256k1::new(), NetworkKind::Test)?;
    Ok((descriptor, keymap))
}

/// Receive and change descriptors with private keys embedded (tprv), for `Wallet::create`.
/// These strings must never be logged or stored; BDK itself only persists the public form.
pub fn descriptors(mnemonic: &Mnemonic) -> anyhow::Result<(String, String)> {
    let (ext, ext_keys) = derive(mnemonic, KeychainKind::External)?;
    let (int, int_keys) = derive(mnemonic, KeychainKind::Internal)?;
    Ok((
        ext.to_string_with_secret(&ext_keys),
        int.to_string_with_secret(&int_keys),
    ))
}
