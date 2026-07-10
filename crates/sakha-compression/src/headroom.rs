//! Headroom integration: library/proxy/MCP/sidecar modes. See spec
//! "Headroom Integration Modes". `HttpHeadroomClient` is the concrete
//! adapter used by proxy/sidecar modes (both expose an HTTP endpoint);
//! library mode would bind to a native library instead but is out of scope
//! here (no such crate is in the pre-declared dependency set).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult};

/// Which Headroom integration mode is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadroomMode {
    Library,
    Proxy,
    Mcp,
    Sidecar,
    Unavailable,
}

/// Request/response types for the Headroom compress/retrieve/stats calls,
/// mirrored across whichever transport (library/proxy/MCP) is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadroomCompressRequest {
    pub content: String,
    pub content_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadroomCompressResponse {
    pub compressed: String,
    pub marker_id: String,
}

/// A client for talking to Headroom, regardless of transport. See spec
/// `headroom_compress` / `headroom_retrieve` / `headroom_stats` MCP tools.
#[async_trait]
pub trait HeadroomClient: Send + Sync {
    async fn compress(&self, request: HeadroomCompressRequest) -> SakhaResult<HeadroomCompressResponse>;
    async fn retrieve(&self, marker_id: &str) -> SakhaResult<String>;
    async fn stats(&self) -> SakhaResult<serde_json::Value>;
    fn mode(&self) -> HeadroomMode;
}

/// A `HeadroomClient` that reports itself unavailable. Used as the default
/// so compression always has a safe fallback path (module 05 "Failure
/// Modes": Headroom unavailable).
#[derive(Debug, Default)]
pub struct UnavailableHeadroomClient;

#[async_trait]
impl HeadroomClient for UnavailableHeadroomClient {
    async fn compress(&self, _request: HeadroomCompressRequest) -> SakhaResult<HeadroomCompressResponse> {
        Err(SakhaError::not_implemented("sakha-compression", "Headroom unavailable"))
    }

    async fn retrieve(&self, _marker_id: &str) -> SakhaResult<String> {
        Err(SakhaError::not_implemented("sakha-compression", "Headroom unavailable"))
    }

    async fn stats(&self) -> SakhaResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    fn mode(&self) -> HeadroomMode {
        HeadroomMode::Unavailable
    }
}

/// Configuration for `HttpHeadroomClient`. Both `Proxy` and `Sidecar` modes
/// speak the same HTTP contract; `mode` only affects how the endpoint is
/// interpreted/labeled (proxy is remote, sidecar is a locally-spawned
/// process this crate manages via `SidecarLauncher`).
#[derive(Debug, Clone)]
pub struct HeadroomHttpConfig {
    pub base_url: String,
    pub mode: HeadroomMode,
    pub request_timeout: Duration,
    pub api_key: Option<String>,
}

impl Default for HeadroomHttpConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8787".to_string(),
            mode: HeadroomMode::Sidecar,
            request_timeout: Duration::from_secs(10),
            api_key: None,
        }
    }
}

/// An HTTP-based `HeadroomClient` adapter, used for `Proxy` and `Sidecar`
/// modes. Never required for correctness: any network/protocol failure is
/// surfaced as a `SakhaError::transient` so callers (e.g.
/// `HeadroomBackedCompressor`) can fall back to `LocalFallbackCompressor`.
pub struct HttpHeadroomClient {
    pub config: HeadroomHttpConfig,
    client: reqwest::Client,
}

impl HttpHeadroomClient {
    pub fn new(config: HeadroomHttpConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.config.base_url.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.config.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }
}

#[async_trait]
impl HeadroomClient for HttpHeadroomClient {
    async fn compress(&self, request: HeadroomCompressRequest) -> SakhaResult<HeadroomCompressResponse> {
        let builder = self.client.post(self.url("headroom_compress")).json(&request);
        let response = self
            .apply_auth(builder)
            .send()
            .await
            .map_err(|err| SakhaError::transient("sakha-compression", "headroom_compress request failed").with_cause(err))?;

        if !response.status().is_success() {
            return Err(SakhaError::transient(
                "sakha-compression",
                format!("headroom_compress returned status {}", response.status()),
            ));
        }

        response
            .json::<HeadroomCompressResponse>()
            .await
            .map_err(|err| SakhaError::integrity("sakha-compression", "headroom_compress response was not valid JSON").with_cause(err))
    }

    async fn retrieve(&self, marker_id: &str) -> SakhaResult<String> {
        let builder = self
            .client
            .post(self.url("headroom_retrieve"))
            .json(&serde_json::json!({ "marker_id": marker_id }));
        let response = self
            .apply_auth(builder)
            .send()
            .await
            .map_err(|err| SakhaError::transient("sakha-compression", "headroom_retrieve request failed").with_cause(err))?;

        if !response.status().is_success() {
            return Err(SakhaError::transient(
                "sakha-compression",
                format!("headroom_retrieve returned status {}", response.status()),
            ));
        }

        #[derive(Deserialize)]
        struct RetrieveResponse {
            content: String,
        }

        let parsed = response
            .json::<RetrieveResponse>()
            .await
            .map_err(|err| SakhaError::integrity("sakha-compression", "headroom_retrieve response was not valid JSON").with_cause(err))?;
        Ok(parsed.content)
    }

    async fn stats(&self) -> SakhaResult<serde_json::Value> {
        let builder = self.client.get(self.url("headroom_stats"));
        let response = self
            .apply_auth(builder)
            .send()
            .await
            .map_err(|err| SakhaError::transient("sakha-compression", "headroom_stats request failed").with_cause(err))?;

        if !response.status().is_success() {
            return Err(SakhaError::transient(
                "sakha-compression",
                format!("headroom_stats returned status {}", response.status()),
            ));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|err| SakhaError::integrity("sakha-compression", "headroom_stats response was not valid JSON").with_cause(err))
    }

    fn mode(&self) -> HeadroomMode {
        self.config.mode
    }
}

/// Manages the lifecycle of a locally-run Headroom sidecar process (spawn,
/// health-check, shutdown). Stub: no process is actually spawned yet; use
/// `crate::sidecar::SidecarLauncher` once that grows a real implementation.
#[derive(Debug, Default)]
pub struct HeadroomSidecar {
    pub running: bool,
}

impl HeadroomSidecar {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(&mut self) -> SakhaResult<()> {
        Err(SakhaError::not_implemented("sakha-compression", "HeadroomSidecar::start"))
    }

    pub async fn stop(&mut self) -> SakhaResult<()> {
        self.running = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn headroom_unavailable_returns_error_not_panic() {
        let client = UnavailableHeadroomClient;
        let result = client
            .compress(HeadroomCompressRequest {
                content: "x".into(),
                content_kind: "text".into(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn http_client_compress_against_unreachable_endpoint_is_transient_error_not_panic() {
        // No server is listening on this port; the request must fail
        // gracefully (transient error) rather than panic, so callers can
        // fall back per the "Headroom unavailable" failure mode.
        let config = HeadroomHttpConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            request_timeout: Duration::from_millis(200),
            ..HeadroomHttpConfig::default()
        };
        let client = HttpHeadroomClient::new(config);
        let result = client
            .compress(HeadroomCompressRequest {
                content: "x".into(),
                content_kind: "text".into(),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().retryable);
    }

    #[test]
    fn url_joins_base_and_path_without_double_slash() {
        let config = HeadroomHttpConfig {
            base_url: "http://localhost:8787/".to_string(),
            ..HeadroomHttpConfig::default()
        };
        let client = HttpHeadroomClient::new(config);
        assert_eq!(client.url("/headroom_compress"), "http://localhost:8787/headroom_compress");
    }
}
