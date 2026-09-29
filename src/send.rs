//! Spending, split into the three steps a wallet goes through:
//! build an unsigned PSBT (watch-only) -> sign it (needs keys) -> broadcast it.
//! Keeping them separate means the API can later build on the server and sign on the client.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use bdk_bitcoind_rpc::bitcoincore_rpc::RpcApi;
use bdk_wallet::bitcoin::{Address, Amount, FeeRate, Psbt, Transaction, Txid};
use bdk_wallet::error::CreateTxError;
use bdk_wallet::keys::bip39::Mnemonic;
use bdk_wallet::miniscript::descriptor::{KeyMap, KeyMapWrapper};
use bdk_wallet::rusqlite::Connection;
use bdk_wallet::{KeychainKind, PersistedWallet, SignOptions};

use crate::chain::Client;
use crate::keys;

/// Aim for confirmation within this many blocks when asking Core for a fee estimate.
const CONF_TARGET: u16 = 6;

/// An unsigned transaction plus the numbers the user should check before signing.
pub struct Draft {
    pub psbt: Psbt,
    pub fee: Amount,
    pub change: Amount,
    pub inputs: usize,
}

/// Asks Core for a fee rate. Returns None when Core has no data, which is normal on a fresh
/// regtest chain: it estimates from how quickly past transactions confirmed.
pub fn estimate_fee_rate(rpc: &Client) -> Option<FeeRate> {
    let estimate = rpc.estimate_smart_fee(CONF_TARGET, None).ok()?;
    let btc_per_kvb = estimate.fee_rate?;
    // sat/kvB -> sat/kwu: 1 vbyte = 4 weight units.
    Some(FeeRate::from_sat_per_kwu(btc_per_kvb.to_sat() / 4))
}

/// Selects coins and builds an unsigned PSBT paying `amount` to `to`, with change back to us.
pub fn build(
    wallet: &mut PersistedWallet<Connection>,
    to: &Address,
    amount: Amount,
    fee_rate: FeeRate,
) -> anyhow::Result<Draft> {
    let mut builder = wallet.build_tx();
    builder.add_recipient(to.script_pubkey(), amount).fee_rate(fee_rate);
    let psbt = builder.finish().map_err(|e| match e {
        CreateTxError::CoinSelection(e) => anyhow!("insufficient funds: {e}"),
        CreateTxError::OutputBelowDustLimit(_) => {
            anyhow!("amount is below the dust limit; send a larger amount")
        }
        e => anyhow!(e).context("building transaction"),
    })?;

    let fee = psbt.fee().context("computing fee")?;
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

/// Signs and finalizes the PSBT with keys derived from `mnemonic`, returning the final tx.
/// The keys live only in this function; the wallet itself stays watch-only.
pub fn sign(
    wallet: &PersistedWallet<Connection>,
    mnemonic: &Mnemonic,
    mut psbt: Psbt,
) -> anyhow::Result<Transaction> {
    let mut keymap = KeyMap::new();
    for keychain in [KeychainKind::External, KeychainKind::Internal] {
        let (descriptor, keys) = keys::derive(mnemonic, keychain)?;
        // Guard against signing with keys that don't belong to this wallet.
        if descriptor.to_string() != wallet.public_descriptor(keychain).to_string() {
            bail!("the stored mnemonic does not match this wallet's descriptors");
        }
        keymap.extend(keys);
    }

    // Each PSBT input records which key (fingerprint + BIP32 path) can spend it; the signer
    // looks those keys up in our keymap and adds a signature per input.
    psbt.sign(&KeyMapWrapper::from(keymap), wallet.secp_ctx())
        .map_err(|(_, errors)| anyhow!("signing failed: {errors:?}"))?;

    // Turn signatures into final witnesses, i.e. make the tx valid to broadcast.
    let finalized = wallet.finalize_psbt(&mut psbt, SignOptions::default())?;
    if !finalized {
        bail!("could not finalize the transaction: some inputs are not signed");
    }
    Ok(psbt.extract_tx()?)
}

/// Sends the transaction to Core and records it in the wallet as unconfirmed.
pub fn broadcast(
    wallet: &mut PersistedWallet<Connection>,
    conn: &mut Connection,
    rpc: &Client,
    tx: Transaction,
) -> anyhow::Result<Txid> {
    let txid = rpc
        .send_raw_transaction(&tx)
        .context("Bitcoin Core rejected the transaction")?;

    // Record it now so balance/history reflect it immediately, without waiting for a sync.
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    wallet.apply_unconfirmed_txs([(tx, now)]);
    wallet.persist(conn).context("saving wallet")?;
    Ok(txid)
}
