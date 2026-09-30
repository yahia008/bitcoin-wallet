//! JSON bodies shared by the API server and the CLI's HTTP client, so the two can't drift.
//! All amounts are integer satoshis: JSON numbers are floats in JavaScript, and floats can't
//! represent most BTC decimals exactly.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct RegisterRequest {
    /// Receive descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../0/*)`
    pub external: String,
    /// Change descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../1/*)`
    pub internal: String,
}

#[derive(Serialize, Deserialize)]
pub struct RegisterResponse {
    pub id: String,
}

#[derive(Serialize, Deserialize)]
pub struct PsbtRequest {
    pub address: String,
    pub amount_sat: u64,
    /// Omit to let the server ask Bitcoin Core for an estimate.
    #[serde(default)]
    pub fee_rate_sat_vb: Option<u64>,
}

/// An unsigned PSBT plus the server's summary of it. Signers must not trust the summary;
/// they should check the PSBT itself (see `send::review`).
#[derive(Serialize, Deserialize)]
pub struct PsbtResponse {
    /// Base64-encoded unsigned PSBT
    pub psbt: String,
    pub amount_sat: u64,
    pub fee_sat: u64,
    pub change_sat: u64,
    pub fee_rate_sat_vb: u64,
}

#[derive(Serialize, Deserialize)]
pub struct BroadcastRequest {
    /// Base64-encoded signed PSBT
    pub psbt: String,
}

#[derive(Serialize, Deserialize)]
pub struct BroadcastResponse {
    pub txid: String,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}
