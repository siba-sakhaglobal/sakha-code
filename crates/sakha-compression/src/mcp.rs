//! Headroom-over-MCP client adapter: implements `HeadroomClient` by calling
//! `headroom_compress` / `headroom_retrieve` / `headroom_stats` MCP tools.

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};

use crate::headroom::{HeadroomClient, HeadroomCompressRequest, HeadroomCompressResponse, HeadroomMode};

/// Talks to Headroom via an MCP server exposing the `headroom_*` tools.
/// Stub: does not yet hold a real MCP connection (that lives in
/// `sakha-mcp`); constructed with a server name for now.
pub struct HeadroomMcpClient {
    pub server_name: String,
}

impl HeadroomMcpClient {
    pub fn new(server_name: impl Into<String>) -> Self {
        Self { server_name: server_name.into() }
    }
}

#[async_trait]
impl HeadroomClient for HeadroomMcpClient {
    async fn compress(&self, _request: HeadroomCompressRequest) -> SakhaResult<HeadroomCompressResponse> {
        Err(SakhaError::not_implemented("sakha-compression", "HeadroomMcpClient::compress"))
    }

    async fn retrieve(&self, _marker_id: &str) -> SakhaResult<String> {
        Err(SakhaError::not_implemented("sakha-compression", "HeadroomMcpClient::retrieve"))
    }

    async fn stats(&self) -> SakhaResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    fn mode(&self) -> HeadroomMode {
        HeadroomMode::Mcp
    }
}
