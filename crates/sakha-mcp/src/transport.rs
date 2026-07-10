//! MCP transport abstraction (stdio, HTTP/SSE). See spec
//! `modules/10-mcp-plugin-connector-system.md`.

use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use sakha_core::{SakhaError, SakhaResult};

/// Which wire transport an MCP connection uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Stdio,
    HttpSse,
}

/// A raw JSON-RPC message exchanged over an MCP transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMessage(pub serde_json::Value);

/// Abstraction over an MCP transport's send/receive/reconnect lifecycle.
///
/// `send` writes one framed message; `receive` reads the next framed message
/// sent back from the peer. Implementations are responsible for correlating
/// request/response pairs by `id` at a higher layer (`McpConnection`), since
/// a transport only guarantees ordered delivery, not routing.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, message: RpcMessage) -> SakhaResult<()>;
    async fn receive(&self) -> SakhaResult<RpcMessage>;
    async fn reconnect(&self) -> SakhaResult<()>;
    fn kind(&self) -> TransportKind;
}

/// A transport that is never actually connected; used as a safe default so
/// the workspace never requires a live MCP server to compile or test.
#[derive(Debug, Default)]
pub struct DisconnectedTransport;

#[async_trait]
impl McpTransport for DisconnectedTransport {
    async fn send(&self, _message: RpcMessage) -> SakhaResult<()> {
        Err(SakhaError::not_implemented("sakha-mcp", "DisconnectedTransport::send"))
    }

    async fn receive(&self) -> SakhaResult<RpcMessage> {
        Err(SakhaError::not_implemented("sakha-mcp", "DisconnectedTransport::receive"))
    }

    async fn reconnect(&self) -> SakhaResult<()> {
        Err(SakhaError::not_implemented("sakha-mcp", "DisconnectedTransport::reconnect"))
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Stdio
    }
}

/// Spawns an MCP server as a child process and frames JSON-RPC messages as
/// newline-delimited JSON over its stdin/stdout, per the MCP stdio
/// transport convention.
pub struct StdioTransport {
    command: String,
    args: Vec<String>,
    inner: Mutex<Option<StdioChild>>,
}

struct StdioChild {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

impl StdioTransport {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self { command: command.into(), args, inner: Mutex::new(None) }
    }

    async fn spawn(&self) -> SakhaResult<StdioChild> {
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd
            .spawn()
            .map_err(|e| SakhaError::transient("sakha-mcp", format!("failed to spawn MCP server '{}': {e}", self.command)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SakhaError::integrity("sakha-mcp", "MCP child process has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SakhaError::integrity("sakha-mcp", "MCP child process has no stdout"))?;
        Ok(StdioChild { child, stdin, stdout: BufReader::new(stdout) })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, message: RpcMessage) -> SakhaResult<()> {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn().await?);
        }
        let child = guard.as_mut().expect("just populated");
        let mut line = serde_json::to_string(&message.0)
            .map_err(|e| SakhaError::integrity("sakha-mcp", format!("failed to serialize MCP message: {e}")))?;
        line.push('\n');
        child
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| SakhaError::transient("sakha-mcp", format!("failed to write to MCP server stdin: {e}")))?;
        child
            .stdin
            .flush()
            .await
            .map_err(|e| SakhaError::transient("sakha-mcp", format!("failed to flush MCP server stdin: {e}")))?;
        Ok(())
    }

    async fn receive(&self) -> SakhaResult<RpcMessage> {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn().await?);
        }
        let child = guard.as_mut().expect("just populated");
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = child
                .stdout
                .read_line(&mut buf)
                .await
                .map_err(|e| SakhaError::transient("sakha-mcp", format!("failed to read from MCP server stdout: {e}")))?;
            if n == 0 {
                return Err(SakhaError::transient("sakha-mcp", "MCP server closed stdout"));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|e| SakhaError::integrity("sakha-mcp", format!("invalid JSON from MCP server: {e}")))?;
            return Ok(RpcMessage(value));
        }
    }

    async fn reconnect(&self) -> SakhaResult<()> {
        let mut guard = self.inner.lock().await;
        if let Some(mut old) = guard.take() {
            let _ = old.child.start_kill();
        }
        *guard = Some(self.spawn().await?);
        Ok(())
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Stdio
    }
}

/// An in-process fake transport for tests: paired queues stand in for a
/// server on the other end without spawning any process. Construct with
/// `FakeTransport::pair()` to get a client-side and server-side handle that
/// talk to each other.
pub struct FakeTransport {
    outbox: Arc<Mutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    inbox: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>>>,
}

impl FakeTransport {
    /// Returns `(client, server)` transports wired to each other: messages
    /// sent on `client` are received on `server` and vice versa.
    pub fn pair() -> (FakeTransport, FakeTransport) {
        let (c2s_tx, c2s_rx) = tokio::sync::mpsc::unbounded_channel();
        let (s2c_tx, s2c_rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FakeTransport { outbox: Arc::new(Mutex::new(c2s_tx)), inbox: Arc::new(Mutex::new(s2c_rx)) };
        let server = FakeTransport { outbox: Arc::new(Mutex::new(s2c_tx)), inbox: Arc::new(Mutex::new(c2s_rx)) };
        (client, server)
    }
}

#[async_trait]
impl McpTransport for FakeTransport {
    async fn send(&self, message: RpcMessage) -> SakhaResult<()> {
        self.outbox
            .lock()
            .await
            .send(message.0)
            .map_err(|_| SakhaError::transient("sakha-mcp", "fake transport peer dropped"))
    }

    async fn receive(&self) -> SakhaResult<RpcMessage> {
        let mut guard = self.inbox.lock().await;
        guard
            .recv()
            .await
            .map(RpcMessage)
            .ok_or_else(|| SakhaError::transient("sakha-mcp", "fake transport peer closed"))
    }

    async fn reconnect(&self) -> SakhaResult<()> {
        Ok(())
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Stdio
    }
}
