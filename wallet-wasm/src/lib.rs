//! What the browser wallet can call. Thin wrappers: the logic lives in wallet-core, the same
//! code the CLI runs. Errors become JavaScript exceptions.

use std::str::FromStr;

use bdk_wallet::bitcoin::Network;
use bdk_wallet::keys::bip39::{Language, Mnemonic};
use bdk_wallet::{KeychainKind, Wallet};
use wallet_core::keys;
use wasm_bindgen::prelude::*;

/// The wallet's first receive address (index 0) for these 12 words on `network`, e.g.
/// "testnet4". Everything happens in the browser: the words never leave it.
#[wasm_bindgen(js_name = firstAddress)]
pub fn first_address(words: &str, network: &str) -> Result<String, JsError> {
    let network = parse_network(network)?;
    let mnemonic = Mnemonic::parse_in(Language::English, words.trim())
        .map_err(|e| JsError::new(&format!("invalid recovery phrase: {e}")))?;
    let account_key = keys::account_key(&mnemonic).map_err(js)?;
    let (external, internal) = keys::descriptors(&account_key).map_err(js)?;
    let wallet = Wallet::create(external, internal)
        .network(network)
        .create_wallet_no_persist()
        .map_err(js)?;
    Ok(wallet.peek_address(KeychainKind::External, 0).address.to_string())
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
