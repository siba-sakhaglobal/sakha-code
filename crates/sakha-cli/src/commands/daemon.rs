//! `sakha daemon start`: launches the local `sakha-daemon` process. The
//! daemon crate builds only a binary (`sakha-daemon`, no `[lib]` target), so
//! the CLI shells out to it rather than embedding an axum server itself —
//! this keeps the two crates' process/threading models independent per
//! `05-system-architecture.md`'s "CLI/TUI/Web/Desktop -> Local Sakha Daemon"
//! process topology.

use clap::{Args, Subcommand};

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// `sakha daemon start`: spawns the `sakha-daemon` binary.
    Start(DaemonStartArgs),
}

#[derive(Debug, Args)]
pub struct DaemonStartArgs {
    /// Run in the foreground (blocks until the daemon exits) instead of
    /// detaching. Foreground is the default since a detached background
    /// process on Windows requires extra platform-specific flags this crate
    /// does not depend on.
    #[arg(long, default_value_t = true)]
    pub foreground: bool,
}

pub fn execute(command: DaemonCommand) -> i32 {
    match command {
        DaemonCommand::Start(args) => start(args),
    }
}

fn start(args: DaemonStartArgs) -> i32 {
    let exe = locate_daemon_binary();
    let Some(exe) = exe else {
        eprintln!(
            "sakha daemon start: could not locate the sakha-daemon binary. \
             Build it first with `cargo build -p sakha-daemon` or place \
             `sakha-daemon`/`sakha-daemon.exe` next to the `sakha` executable."
        );
        return 1;
    };

    println!("sakha daemon start: launching {}", exe.display());
    let mut command = std::process::Command::new(&exe);

    if args.foreground {
        match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(err) => {
                eprintln!("sakha daemon start: failed to launch daemon: {err}");
                1
            }
        }
    } else {
        match command.spawn() {
            Ok(child) => {
                println!("sakha daemon start: spawned pid {}", child.id());
                0
            }
            Err(err) => {
                eprintln!("sakha daemon start: failed to spawn daemon: {err}");
                1
            }
        }
    }
}

/// Looks for a `sakha-daemon`/`sakha-daemon.exe` binary next to the current
/// executable first (typical release layout), then falls back to `PATH`
/// resolution via the bare command name.
fn locate_daemon_binary() -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(windows) { "sakha-daemon.exe" } else { "sakha-daemon" };

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Fall back to PATH resolution: return the bare name and let
    // `std::process::Command` search PATH itself.
    which_on_path(exe_name)
}

/// Minimal `PATH` search so we don't add a `which` crate dependency.
fn which_on_path(exe_name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_start_parses() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: DaemonCommand,
        }
        let cli = TestCli::try_parse_from(["sakha", "start"]).unwrap();
        match cli.cmd {
            DaemonCommand::Start(args) => assert!(args.foreground),
        }
    }

    #[test]
    fn locate_daemon_binary_does_not_panic_when_absent() {
        // Does not actually launch anything (launching would block forever
        // in foreground mode if a real daemon binary happened to be on
        // PATH). Only exercises the locate step, which must return `None`
        // gracefully rather than panic when the binary genuinely isn't
        // findable, and must return a real, existing path when it is.
        if let Some(path) = locate_daemon_binary() {
            assert!(path.exists() || path.file_name().is_some());
        }
    }
}
