//! JSON bodies shared by the API server and the CLI's HTTP client, so the two can't drift.
//! All amounts are integer satoshis: JSON numbers are floats in JavaScript, and floats can't
//! represent most BTC decimals exactly.

use serde::{Deserialize, Serialize};

use crate::send::FeePriority;

#[derive(Serialize, Deserialize)]
pub struct RegisterRequest {
    /// Receive descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../0/*)`
    pub external: String,
    /// Change descriptor, e.g. `wpkh([fingerprint/84'/1'/0']tpub.../1/*)`
    pub internal: String,
    /// Height of the first block that could hold the wallet's transactions; the server's
    /// first sync starts there. Omit (0) to scan from genesis.
    #[serde(default)]
    pub birthday: u32,
}

#[derive(Serialize, Deserialize)]
pub struct RegisterResponse {
    pub id: String,
    /// Send as `Authorization: Bearer <token>` on every `/wallets/{id}/...` request. Returned
    /// only once, at registration; the server keeps just a hash of it. Lost it? Prove you
    /// hold the wallet's key via `/challenge` + `/token` to get a new one.
    pub token: String,
}

/// A one-time challenge for API token recovery: sign it with `wallet_core::ownership::sign`.
#[derive(Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub challenge: String,
    pub expires_in_secs: u64,
}

#[derive(Serialize, Deserialize)]
pub struct TokenRequest {
    pub challenge: String,
    /// `wallet_core::ownership::sign` over the challenge: DER ECDSA signature, hex.
    pub signature: String,
}

/// A fresh API token. The wallet's previous token stops working.
#[derive(Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
}

#[derive(Serialize, Deserialize)]
pub struct PsbtRequest {
    pub address: String,
    pub amount_sat: u64,
    /// An exact fee rate. Omit to let the server estimate one for `fee_priority`.
    #[serde(default)]
    pub fee_rate_sat_vb: Option<u64>,
    /// `fast`, `normal` (default) or `slow`. Ignored when `fee_rate_sat_vb` is set.
    #[serde(default)]
    pub fee_priority: FeePriority,
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

/// Asks for a fee bump (RBF replacement) of one of the wallet's unconfirmed transactions.
#[derive(Serialize, Deserialize)]
pub struct BumpRequest {
    pub txid: String,
    /// An exact fee rate. Omit to let the server estimate one for `fee_priority`; either way
    /// it's at least the original's rate + 1 sat/vB.
    #[serde(default)]
    pub fee_rate_sat_vb: Option<u64>,
    /// Defaults to `fast`: you bump because you want it confirmed sooner.
    #[serde(default)]
    pub fee_priority: Option<FeePriority>,
}

/// An unsigned replacement PSBT, plus the original transaction so the signer can check the
/// replacement against it (`send::review_bump`) without trusting the server: its txid must
/// match the one requested.
#[derive(Serialize, Deserialize)]
pub struct BumpResponse {
    /// Base64-encoded unsigned PSBT
    pub psbt: String,
    /// The transaction being replaced, consensus-encoded hex
    pub original_tx: String,
    pub fee_sat: u64,
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
