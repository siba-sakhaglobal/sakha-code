//! Sandbox profiles and the adapter trait executors use to constrain tool
//! execution. See spec "Sandbox Profiles".
//!
//! Real OS-level sandboxing (Linux namespaces/seccomp, macOS sandbox-exec,
//! Windows job objects/AppContainer) is intentionally **not** implemented
//! here — see "Future Work" below. What this module provides today is:
//!
//! - `SandboxProfile`: the data describing what a profile *should* allow.
//! - `NoopSandbox`: an adapter that enforces the workspace-write and
//!   network-access constraints implied by a profile in pure Rust (path
//!   containment checks, a network-request gate) without any OS-level
//!   isolation for arbitrary child-process behavior. This is enough to
//!   satisfy tool-level policy checks (e.g. "would this write escape the
//!   workspace root", "is network access allowed for this profile") and to
//!   make the crate's own tests meaningful, but it does **not** stop a
//!   malicious subprocess from doing its own syscalls outside these checks.
//!
//! ## Future Work: OS Sandbox Adapters
//!
//! - **Linux**: bind-mount workspace root read/write, mount everything else
//!   read-only or not at all via `unshare`/`bwrap`/`landlock`; seccomp-bpf
//!   filter to deny `connect`/`socket` when the profile disables network.
//! - **macOS**: `sandbox-exec` with a generated `.sb` profile scoping file
//!   reads/writes to the workspace root and denying `network-outbound` when
//!   disabled.
//! - **Windows**: run the tool subprocess in a restricted job object /
//!   AppContainer with an ACL'd temp+workspace directory and WFP (Windows
//!   Filtering Platform) rules to block outbound connections when disabled.
//!
//! Each of these should implement `SandboxAdapter` and can be selected at
//! runtime based on `std::env::consts::OS`, falling back to `NoopSandbox`
//! when the platform-specific backend is unavailable.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

use sakha_core::SakhaError;

/// Named sandbox profiles. See spec "Sandbox Profiles".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    ReadOnly,
    WorkspaceWrite,
    NetworkDisabled,
    NetworkAllowed,
    BuildTest,
    FullTrust,
    Custom,
}

impl SandboxProfile {
    /// Whether this profile permits writing anywhere under the workspace
    /// root. `ReadOnly` and `NetworkDisabled`-only-for-reads style profiles
    /// return false.
    pub fn allows_write(&self) -> bool {
        match self {
            SandboxProfile::ReadOnly => false,
            SandboxProfile::WorkspaceWrite
            | SandboxProfile::NetworkDisabled
            | SandboxProfile::NetworkAllowed
            | SandboxProfile::BuildTest
            | SandboxProfile::FullTrust
            | SandboxProfile::Custom => true,
        }
    }

    /// Whether this profile permits outbound network access.
    pub fn allows_network(&self) -> bool {
        matches!(
            self,
            SandboxProfile::NetworkAllowed | SandboxProfile::FullTrust | SandboxProfile::Custom
        )
    }

    /// Whether writes are constrained to stay inside the workspace root, as
    /// opposed to `FullTrust`/`Custom` which may permit escapes.
    pub fn confines_writes_to_workspace(&self) -> bool {
        !matches!(self, SandboxProfile::FullTrust)
    }
}

/// A concrete sandbox execution request (e.g. a shell command to run under a
/// profile, or a file write to validate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequest {
    pub profile: SandboxProfile,
    pub workspace_root: String,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Paths this request will write to, if any (for pre-flight containment
    /// checks before actually spawning a process).
    #[serde(default)]
    pub write_paths: Vec<String>,
    /// Whether this request requires outbound network access.
    #[serde(default)]
    pub needs_network: bool,
}

impl SandboxRequest {
    pub fn new(profile: SandboxProfile, workspace_root: impl Into<String>) -> Self {
        Self {
            profile,
            workspace_root: workspace_root.into(),
            command: Vec::new(),
            env: Vec::new(),
            write_paths: Vec::new(),
            needs_network: false,
        }
    }

    pub fn with_command(mut self, command: Vec<String>) -> Self {
        self.command = command;
        self
    }

    pub fn with_write_paths(mut self, paths: Vec<String>) -> Self {
        self.write_paths = paths;
        self
    }

    pub fn with_network(mut self, needs_network: bool) -> Self {
        self.needs_network = needs_network;
        self
    }
}

/// Outcome of running something under a sandbox adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    /// Set when the sandbox itself refused to run the request (as opposed to
    /// the command running and failing on its own).
    pub blocked_reason: Option<String>,
}

impl SandboxOutcome {
    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            blocked_reason: Some(reason.into()),
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked_reason.is_some()
    }
}

/// Abstraction over an OS-level sandbox backend (Linux namespaces/seccomp,
/// macOS sandbox-exec, Windows job objects, or a no-op passthrough for tests).
#[async_trait]
pub trait SandboxAdapter: Send + Sync {
    async fn run(&self, request: SandboxRequest) -> Result<SandboxOutcome, SakhaError>;
    fn name(&self) -> &str;
}

/// Normalizes a path to a `PathBuf` with `.`/`..` components resolved
/// lexically (no filesystem access, so this works for paths that don't exist
/// yet — important for pre-flight write checks before a file is created).
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Returns true if `candidate` (after lexical normalization) is contained
/// within `root` (after lexical normalization).
pub fn path_is_within_root(root: &str, candidate: &str) -> bool {
    let root_norm = lexically_normalize(Path::new(root));
    let candidate_path = if Path::new(candidate).is_absolute() {
        PathBuf::from(candidate)
    } else {
        Path::new(root).join(candidate)
    };
    let candidate_norm = lexically_normalize(&candidate_path);
    candidate_norm.starts_with(&root_norm)
}

/// A sandbox adapter that enforces workspace-containment and
/// network-access-gate checks in pure Rust, without any OS-level process
/// isolation. See module docs for what this does and does not guarantee.
///
/// This is the default adapter: it never requires a real OS sandbox to be
/// present, so tests and early development always have a working (if
/// non-isolating) backend.
#[derive(Debug, Default)]
pub struct NoopSandbox;

impl NoopSandbox {
    pub fn new() -> Self {
        Self
    }

    /// Validates a request against its declared profile's constraints without
    /// running anything. Returns `Err` describing the violation if blocked.
    pub fn validate(&self, request: &SandboxRequest) -> Result<(), String> {
        if request.needs_network && !request.profile.allows_network() {
            return Err(format!(
                "network access denied: profile {:?} disables outbound network",
                request.profile
            ));
        }
        if !request.write_paths.is_empty() && !request.profile.allows_write() {
            return Err(format!("write denied: profile {:?} is read-only", request.profile));
        }
        if request.profile.confines_writes_to_workspace() {
            for path in &request.write_paths {
                if !path_is_within_root(&request.workspace_root, path) {
                    return Err(format!(
                        "write to '{}' escapes workspace root '{}'",
                        path, request.workspace_root
                    ));
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SandboxAdapter for NoopSandbox {
    async fn run(&self, request: SandboxRequest) -> Result<SandboxOutcome, SakhaError> {
        if let Err(reason) = self.validate(&request) {
            return Ok(SandboxOutcome::blocked(reason));
        }
        // No real process isolation: this adapter only performs the
        // pre-flight checks above. Actual command execution is the caller's
        // (e.g. sakha-tools shell builtin's) responsibility; this trait
        // implementation exists so callers have a uniform interface even
        // before a real OS backend is wired in.
        Err(SakhaError::not_implemented(
            "sakha-security",
            "NoopSandbox::run does not execute processes; wire a platform SandboxAdapter for real execution",
        ))
    }

    fn name(&self) -> &str {
        "noop"
    }
}

/// Backward-compatible alias for `NoopSandbox`.
pub type PassthroughSandboxAdapter = NoopSandbox;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_within_workspace_is_allowed() {
        let req = SandboxRequest::new(SandboxProfile::WorkspaceWrite, "/workspace")
            .with_write_paths(vec!["/workspace/src/main.rs".into()]);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_ok());
    }

    #[test]
    fn write_outside_workspace_is_blocked() {
        let req = SandboxRequest::new(SandboxProfile::WorkspaceWrite, "/workspace")
            .with_write_paths(vec!["/etc/passwd".into()]);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_err());
    }

    #[test]
    fn write_outside_workspace_via_dotdot_is_blocked() {
        let req = SandboxRequest::new(SandboxProfile::WorkspaceWrite, "/workspace")
            .with_write_paths(vec!["/workspace/../secrets/leak.txt".into()]);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_err());
    }

    #[test]
    fn read_only_profile_blocks_any_write() {
        let req = SandboxRequest::new(SandboxProfile::ReadOnly, "/workspace")
            .with_write_paths(vec!["/workspace/file.txt".into()]);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_err());
    }

    #[test]
    fn network_disabled_profile_blocks_network_request() {
        let req = SandboxRequest::new(SandboxProfile::NetworkDisabled, "/workspace").with_network(true);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_err());
    }

    #[test]
    fn network_allowed_profile_permits_network_request() {
        let req = SandboxRequest::new(SandboxProfile::NetworkAllowed, "/workspace").with_network(true);
        let sandbox = NoopSandbox::new();
        assert!(sandbox.validate(&req).is_ok());
    }

    #[tokio::test]
    async fn run_reports_blocked_outcome_for_disallowed_write() {
        let req = SandboxRequest::new(SandboxProfile::ReadOnly, "/workspace")
            .with_write_paths(vec!["/workspace/file.txt".into()]);
        let sandbox = NoopSandbox::new();
        let outcome = sandbox.run(req).await.unwrap();
        assert!(outcome.is_blocked());
    }
}
