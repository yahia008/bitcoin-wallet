//! Wallet logic with no I/O: no database, no network. It compiles to WebAssembly, so the
//! browser wallet runs exactly the same key handling and PSBT checks as the CLI.

pub mod crypto;
pub mod keys;
pub mod review;
pub mod sign;

use bdk_wallet::bitcoin::hashes::{Hash, sha256};

/// The API's id for a wallet: a hash of its canonical public descriptors (as printed by
/// `Descriptor`'s Display). The CLI, server and browser all compute it, so ids never need
/// copying.
pub fn wallet_id(external: &str, internal: &str) -> String {
    let hash = sha256::Hash::hash(format!("{external}\n{internal}").as_bytes());
    hash.to_string()[..16].to_owned()
}
