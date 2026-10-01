//! Blocking HTTP client for the wallet API, used by the CLI.

use std::time::Duration;

use anyhow::Context;
use bdk_wallet::rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{
    BroadcastRequest, BroadcastResponse, BumpRequest, BumpResponse, ChallengeResponse, ErrorBody,
    PsbtRequest, PsbtResponse, RegisterRequest, RegisterResponse, TokenRequest, TokenResponse,
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
    token: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            // Read error bodies ourselves instead of getting a bare status code.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        ApiClient { base_url: base_url.trim_end_matches('/').to_owned(), agent, token: None }
    }

    /// Sends `token` (from registration) with every request, as `Authorization: Bearer`.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn register(&self, request: &RegisterRequest) -> anyhow::Result<RegisterResponse> {
        self.post("/wallets", request)
    }

    pub fn build_psbt(&self, id: &str, request: &PsbtRequest) -> anyhow::Result<PsbtResponse> {
        self.post(&format!("/wallets/{id}/psbt"), request)
    }

    pub fn bump(&self, id: &str, request: &BumpRequest) -> anyhow::Result<BumpResponse> {
        self.post(&format!("/wallets/{id}/bump"), request)
    }

    pub fn broadcast(
        &self,
        id: &str,
        request: &BroadcastRequest,
    ) -> anyhow::Result<BroadcastResponse> {
        self.post(&format!("/wallets/{id}/broadcast"), request)
    }

    /// Whether the server accepts our token for wallet `id` (any wallet read would do).
    pub fn token_works(&self, id: &str) -> anyhow::Result<bool> {
        match self.get::<serde_json::Value>(&format!("/wallets/{id}/balance")) {
            Ok(_) => Ok(true),
            Err(e) if e.downcast_ref::<ApiError>().is_some_and(|e| e.status == 401) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Starts token recovery: a one-time challenge to sign with the wallet's key.
    pub fn challenge(&self, id: &str) -> anyhow::Result<ChallengeResponse> {
        self.post(&format!("/wallets/{id}/challenge"), &serde_json::json!({}))
    }

    /// Trades a signed challenge for a new token; the previous one stops working.
    pub fn recover_token(&self, id: &str, request: &TokenRequest) -> anyhow::Result<TokenResponse> {
        self.post(&format!("/wallets/{id}/token"), request)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let mut request = self.agent.get(format!("{}{path}", self.base_url));
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = request
            .call()
            .with_context(|| format!("cannot reach the wallet API at {}", self.base_url))?;
        read(response)
    }

    fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> anyhow::Result<T> {
        let mut request = self.agent.post(format!("{}{path}", self.base_url));
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let response = request
            .send_json(body)
            .with_context(|| format!("cannot reach the wallet API at {}", self.base_url))?;
        read(response)
    }
}

/// The parsed body of a successful response, or an `ApiError` with the server's message.
fn read<T: DeserializeOwned>(mut response: ureq::http::Response<ureq::Body>) -> anyhow::Result<T> {
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

/// Saves the API token a server issued for this wallet, in the wallet's own database. It's
/// kept unencrypted: it lets a holder see the wallet and build PSBTs, but never spend.
pub fn save_token(conn: &Connection, server: &str, token: &str) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS api_token (server TEXT PRIMARY KEY, token TEXT NOT NULL)",
        [],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO api_token (server, token) VALUES (?1, ?2)",
        params![server_key(server), token],
    )
    .context("saving API token")?;
    Ok(())
}

/// The token saved for `server`, if this wallet was registered with it.
pub fn load_token(conn: &Connection, server: &str) -> anyhow::Result<Option<String>> {
    let table_exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'api_token')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(None);
    }
    let token = conn
        .query_row(
            "SELECT token FROM api_token WHERE server = ?1",
            params![server_key(server)],
            |row| row.get(0),
        )
        .optional()?;
    Ok(token)
}

/// `http://host:3000/` and `http://host:3000` are the same server.
fn server_key(server: &str) -> &str {
    server.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(load_token(&conn, "http://a").unwrap(), None);
        save_token(&conn, "http://a/", "t1").unwrap();
        assert_eq!(load_token(&conn, "http://a").unwrap().as_deref(), Some("t1"));
        assert_eq!(load_token(&conn, "http://b").unwrap(), None);
    }
}
