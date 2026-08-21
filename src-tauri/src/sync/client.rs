use archive_sync_types::{
    CreateAccountRequest, CreateAccountResponse, JoinRequest, JoinResponse, PairCodeResponse,
    PullResponse, PushRequest, PushResponse,
};
use reqwest::{Client, StatusCode};
use std::sync::Arc;

/// Mirrors `usage_api::FetchOutcome`'s shape: a typed result that
/// distinguishes "the call worked," "the API key is no longer valid"
/// (stop retrying, surface a re-auth prompt), and "something transient
/// went wrong" (network blip, 5xx — safe to retry next cycle).
#[derive(Debug)]
pub enum SyncOutcome<T> {
    Ok(T),
    Unauthorized,
    Transient(String),
}

pub struct SyncClient {
    inner: Arc<Client>,
    base_url: String,
}

impl SyncClient {
    /// Trims a trailing slash from `base_url` before storing it — mirrors
    /// `providers::ollama::fetch_models`'s handling of its own
    /// user-configurable base URL. Without this, a URL saved with a
    /// trailing slash (e.g. `https://host.example.com/`) would double up
    /// the slash when concatenated with `/v1/...` below.
    pub fn new(inner: Arc<Client>, base_url: String) -> Self {
        Self { inner, base_url: base_url.trim_end_matches('/').to_string() }
    }

    pub async fn create_account(
        &self,
        device_id: String,
        device_name: String,
    ) -> SyncOutcome<CreateAccountResponse> {
        let url = format!("{}/v1/accounts", self.base_url);
        let req = CreateAccountRequest { device_id, device_name };
        match self.inner.post(&url).json(&req).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<CreateAccountResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn pair_code(&self, api_key: &str) -> SyncOutcome<PairCodeResponse> {
        let url = format!("{}/v1/devices/pair-code", self.base_url);
        match self.inner.post(&url).bearer_auth(api_key).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PairCodeResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn join(
        &self,
        pairing_code: String,
        device_id: String,
        device_name: String,
    ) -> SyncOutcome<JoinResponse> {
        let url = format!("{}/v1/devices/join", self.base_url);
        let req = JoinRequest { pairing_code, device_id, device_name };
        match self.inner.post(&url).json(&req).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<JoinResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn push(&self, api_key: &str, req: PushRequest) -> SyncOutcome<PushResponse> {
        let url = format!("{}/v1/archive/push", self.base_url);
        match self.inner.post(&url).bearer_auth(api_key).json(&req).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PushResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn pull(
        &self,
        api_key: &str,
        since_transcript_seq: i64,
        since_snapshot_seq: i64,
        limit: i64,
    ) -> SyncOutcome<PullResponse> {
        let url = format!(
            "{}/v1/archive/pull?since_transcript_seq={since_transcript_seq}&since_snapshot_seq={since_snapshot_seq}&limit={limit}",
            self.base_url
        );
        match self.inner.get(&url).bearer_auth(api_key).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PullResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_client(base_url: &str) -> SyncClient {
        let inner = reqwest::Client::builder().build().unwrap();
        SyncClient::new(Arc::new(inner), base_url.to_string())
    }

    #[tokio::test]
    async fn create_account_returns_ok_on_200() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"user_id":"u1","device_id":"d1","api_key":"k1"}"#)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client
            .create_account("d1".to_string(), "Test Device".to_string())
            .await;
        mock.assert_async().await;
        match result {
            SyncOutcome::Ok(resp) => {
                assert_eq!(resp.user_id, "u1");
                assert_eq!(resp.api_key, "k1");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_returns_unauthorized_on_401() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/v1/archive/push")
            .with_status(401)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client
            .push("bad-key", archive_sync_types::PushRequest { transcript_lines: vec![], file_snapshots: vec![] })
            .await;
        assert!(matches!(result, SyncOutcome::Unauthorized));
    }

    #[tokio::test]
    async fn pull_returns_transient_on_500() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", mockito::Matcher::Regex(r"^/v1/archive/pull".to_string()))
            .with_status(500)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client.pull("some-key", 0, 0, 500).await;
        assert!(matches!(result, SyncOutcome::Transient(_)));
    }

    /// Mirrors `tests/ollama_client.rs`'s
    /// `trailing_slash_on_base_url_does_not_double_up`: a backend URL saved
    /// with a trailing slash must not produce a double-slashed request path
    /// (mockito's mock only matches the exact single-slash path, so a
    /// double slash here would 501/miss and this test would fail).
    #[tokio::test]
    async fn trailing_slash_on_base_url_does_not_double_up() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"user_id":"u1","device_id":"d1","api_key":"k1"}"#)
            .create_async()
            .await;

        let base_url = format!("{}/", server.url());
        let client = test_client(&base_url);
        let result = client.create_account("d1".to_string(), "Test Device".to_string()).await;

        mock.assert_async().await;
        assert!(matches!(result, SyncOutcome::Ok(_)), "expected Ok, got {result:?}");
    }
}
