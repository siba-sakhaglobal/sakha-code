//! Shared test-only utilities. `SAKHA_HOME`/`HOME`/`USERPROFILE` are
//! process-global environment variables, but several command test modules
//! (`chat`, `config`, `doctor`, `run`, `session`) temporarily point them at a
//! per-test temp directory so `load_config`/`default_config_path` never
//! touch the real `~/.sakha/config.toml`. Cargo runs tests in the same
//! process by default, so two such tests running concurrently can otherwise
//! stomp on each other's home dir mid-test. `TempHome` gives every test that
//! needs an isolated config directory a single process-wide mutex to
//! serialize on, plus a temp dir that is guaranteed cleaned up and
//! unwound-from on drop.

use std::sync::{Mutex, MutexGuard, OnceLock};

static HOME_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Acquires the process-wide home-directory test lock. Prefer `TempHome`
/// over calling this directly.
fn home_env_lock() -> MutexGuard<'static, ()> {
    HOME_ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// RAII guard that points `default_config_path()` at a fresh, isolated temp
/// directory for the duration of the guard, and restores the prior
/// environment on drop. Holds the process-wide home-env lock the whole time,
/// so concurrent tests never race on these process-global env vars.
///
/// Sets `SAKHA_HOME` (which `default_config_path()` checks first) rather
/// than relying on `HOME`/`USERPROFILE` alone: on Windows, `dirs::home_dir()`
/// resolves via a system API that ignores `HOME`/`USERPROFILE` env var
/// overrides entirely, so a test that only set those could silently read/
/// write the real `~/.sakha/config.toml` instead of its temp dir. `HOME`/
/// `USERPROFILE` are still set too, for platforms/tools that do honor them.
pub struct TempHome {
    _lock: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    prev_sakha_home: Option<String>,
    prev_home: Option<String>,
    prev_profile: Option<String>,
}

impl TempHome {
    pub fn new() -> Self {
        let lock = home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let prev_sakha_home = std::env::var("SAKHA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();
        let prev_profile = std::env::var("USERPROFILE").ok();
        std::env::set_var("SAKHA_HOME", dir.path());
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());
        Self { _lock: lock, dir, prev_sakha_home, prev_home, prev_profile }
    }

    pub fn path(&self) -> std::path::PathBuf {
        self.dir.path().to_path_buf()
    }
}

impl Default for TempHome {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev_sakha_home {
            Some(v) => std::env::set_var("SAKHA_HOME", v),
            None => std::env::remove_var("SAKHA_HOME"),
        }
        match &self.prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}
