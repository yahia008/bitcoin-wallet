//! Signing: the only step that touches private keys.

use anyhow::{anyhow, bail};
use bdk_wallet::bitcoin::bip32::{DerivationPath, Fingerprint, Xpriv};
use bdk_wallet::bitcoin::psbt::{GetKey, GetKeyError, KeyRequest};
use bdk_wallet::bitcoin::secp256k1::{Secp256k1, Signing};
use bdk_wallet::bitcoin::{PrivateKey, Psbt};
use bdk_wallet::{KeychainKind, Wallet};

use crate::keys;

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
