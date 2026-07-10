//! `sakha-daemon` binary entry point: binds the axum router built by the
//! `sakha_daemon` library to `127.0.0.1` only (see `lib.rs::BIND_ADDR`) and
//! serves it. All route/handler logic lives in the library so it can be
//! exercised in-process by tests via `tower::ServiceExt::oneshot`.

use sakha_daemon::{build_router, DaemonState, BIND_ADDR};

#[tokio::main]
async fn main() {
    let _ = tracing_subscriber::fmt::try_init();
    let state = DaemonState::new();
    let app = build_router(state);

    let listener = match tokio::net::TcpListener::bind(BIND_ADDR).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("sakha-daemon: failed to bind local port: {err}");
            return;
        }
    };
    tracing::info!("sakha-daemon listening on {BIND_ADDR}");
    if let Err(err) = axum::serve(listener, app).await {
        tracing::error!("sakha-daemon: server error: {err}");
    }
}
