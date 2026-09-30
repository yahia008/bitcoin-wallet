//! Blocking HTTP client for the wallet API, used by the CLI.

use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{
    BroadcastRequest, BroadcastResponse, ErrorBody, PsbtRequest, PsbtResponse, RegisterRequest,
    RegisterResponse,
};

/// The server answered with an error status. Callers can downcast to this to react to
/// specific cases, e.g. 404 = wallet not registered.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "API error ({}): {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

pub struct ApiClient {
    base_url: String,
    agent: ureq::Agent,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            // Read error bodies ourselves instead of getting a bare status code.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        ApiClient { base_url: base_url.trim_end_matches('/').to_owned(), agent }
    }

    pub fn register(&self, request: &RegisterRequest) -> anyhow::Result<RegisterResponse> {
        self.post("/wallets", request)
    }

    pub fn build_psbt(&self, id: &str, request: &PsbtRequest) -> anyhow::Result<PsbtResponse> {
        self.post(&format!("/wallets/{id}/psbt"), request)
    }

    pub fn broadcast(
        &self,
        id: &str,
        request: &BroadcastRequest,
    ) -> anyhow::Result<BroadcastResponse> {
        self.post(&format!("/wallets/{id}/broadcast"), request)
    }

    fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> anyhow::Result<T> {
        let mut response = self
            .agent
            .post(format!("{}{path}", self.base_url))
            .send_json(body)
            .with_context(|| format!("cannot reach the wallet API at {}", self.base_url))?;

        let status = response.status();
        if status.is_success() {
            return response.body_mut().read_json().context("unexpected API response");
        }
        let message = response
            .body_mut()
            .read_json::<ErrorBody>()
            .map(|e| e.error)
            .unwrap_or_else(|_| status.to_string());
        Err(ApiError { status: status.as_u16(), message }.into())
    }
}
