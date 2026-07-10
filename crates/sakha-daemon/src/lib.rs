//! sakha-daemon: local HTTP/WebSocket/SSE API, session multiplexing, UI protocol, background loop runner.
//!
//! Public API skeleton — see spec `modules/19-api-contracts.md`,
//! `modules/13-ui-cli-tui-web-desktop.md`, and `crates/crate-work-breakdown.md`.
//!
//! Exposed as a library (in addition to the `sakha-daemon` binary in
//! `main.rs`) so route handlers can be exercised in-process via
//! `tower::ServiceExt::oneshot` without spawning a real TCP listener, per
//! the crate-work-breakdown "Tests" list ("Health endpoint", "Session
//! create", "SSE reconnect", "Permission decision").

mod api;
mod events;
mod loop_routes;
mod permission_queue;
mod permission_routes;
mod session_routes;
mod sse;
mod state;
mod ws;

pub use api::{build_router, ApiError};
pub use events::EventBus;
pub use permission_queue::{PendingPermission, PermissionQueue, PermissionRequestId};
pub use state::DaemonState;

/// Local-only bind address. The daemon must never listen on a non-loopback
/// interface (see task spec "Bind 127.0.0.1 only").
pub const BIND_ADDR: &str = "127.0.0.1:4173";
