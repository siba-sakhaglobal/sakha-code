//! Sidecar process launcher for a locally-managed Headroom service.
//! Separated from `headroom.rs` (the client contract) so process management
//! can evolve independently of the wire protocol. A sidecar is never
//! required: `LocalFallbackCompressor` works without one, so a failed
//! launch is a normal, handled outcome rather than a hard error for callers
//! that already tolerate "Headroom unavailable".

use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::time::sleep;

use sakha_core::{SakhaError, SakhaResult};

use crate::headroom::{HeadroomClient, HeadroomHttpConfig, HeadroomMode, HttpHeadroomClient};

/// Configuration for launching the Headroom sidecar process.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    pub binary_path: Option<std::path::PathBuf>,
    pub port: u16,
    pub startup_timeout_secs: u64,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            binary_path: None,
            port: 8787,
            startup_timeout_secs: 10,
        }
    }
}

impl SidecarConfig {
    /// The `HeadroomHttpConfig` a client would use to reach a sidecar
    /// launched with this config, assuming it binds to `127.0.0.1:port`.
    pub fn client_config(&self) -> HeadroomHttpConfig {
        HeadroomHttpConfig {
            base_url: format!("http://127.0.0.1:{}", self.port),
            mode: HeadroomMode::Sidecar,
            request_timeout: Duration::from_secs(10),
            api_key: None,
        }
    }
}

/// Launches and supervises a sidecar process. If `binary_path` is unset or
/// the binary cannot be spawned, `launch` returns a `Fatal`/non-retryable
/// error (per module 05 "Failure Modes": Headroom unavailable) so callers
/// fall back to the local compressor rather than blocking startup on an
/// optional dependency.
pub struct SidecarLauncher {
    pub config: SidecarConfig,
    child: Option<Child>,
}

impl SidecarLauncher {
    pub fn new(config: SidecarConfig) -> Self {
        Self { config, child: None }
    }

    /// Spawns the sidecar binary and polls its HTTP `headroom_stats`
    /// endpoint until it responds or `startup_timeout_secs` elapses.
    pub async fn launch(&mut self) -> SakhaResult<HttpHeadroomClient> {
        let binary = self
            .config
            .binary_path
            .clone()
            .ok_or_else(|| SakhaError::not_implemented("sakha-compression", "SidecarLauncher::launch: no binary_path configured"))?;

        let child = Command::new(&binary)
            .arg("--port")
            .arg(self.config.port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| {
                SakhaError::fatal("sakha-compression", format!("failed to spawn headroom sidecar at {}", binary.display()))
                    .with_cause(err)
            })?;
        self.child = Some(child);

        let client = HttpHeadroomClient::try_new(self.config.client_config())?;
        let deadline = Duration::from_secs(self.config.startup_timeout_secs);
        let poll_interval = Duration::from_millis(200);
        let mut waited = Duration::ZERO;
        loop {
            if client.stats().await.is_ok() {
                return Ok(client);
            }
            if waited >= deadline {
                return Err(SakhaError::transient(
                    "sakha-compression",
                    format!("headroom sidecar did not become healthy within {deadline:?}"),
                ));
            }
            sleep(poll_interval).await;
            waited += poll_interval;
        }
    }

    /// Terminates the sidecar process if one was launched. Safe to call even
    /// if `launch` was never called or already failed.
    pub async fn shutdown(&mut self) -> SakhaResult<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn launch_without_binary_path_returns_not_implemented_not_panic() {
        let mut launcher = SidecarLauncher::new(SidecarConfig::default());
        let result = launcher.launch().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_without_launch_is_a_safe_noop() {
        let mut launcher = SidecarLauncher::new(SidecarConfig::default());
        assert!(launcher.shutdown().await.is_ok());
    }

    #[test]
    fn client_config_targets_localhost_configured_port() {
        let config = SidecarConfig { port: 9999, ..SidecarConfig::default() };
        let client_config = config.client_config();
        assert_eq!(client_config.base_url, "http://127.0.0.1:9999");
        assert_eq!(client_config.mode, HeadroomMode::Sidecar);
    }
}
