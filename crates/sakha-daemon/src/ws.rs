//! WebSocket upgrade handler: mirrors the SSE session event stream over a
//! WebSocket for clients (TUI/desktop) that prefer a persistent bidirectional
//! connection. See spec `modules/19-api-contracts.md` "API Styles" ->
//! WebSocket and `modules/13-ui-cli-tui-web-desktop.md` "Web socket
//! reconnect works".

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use sakha_core::{EventEnvelope, EventKind, SessionId, TurnId};
use sakha_memory::{SessionStore, TurnRecord};

use crate::state::DaemonState;

/// `GET /sessions/:id/ws`: upgrades to a WebSocket that mirrors the session's
/// live event stream (each `EventEnvelope` sent as a JSON text frame) and
/// accepts inbound text frames as raw session input (mirroring
/// `POST /sessions/:id/input` for clients that prefer a single connection).
async fn ws_handler(ws: WebSocketUpgrade, Path(id): Path<String>, State(state): State<DaemonState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, id, state))
}

async fn handle_socket(mut socket: WebSocket, id: String, state: DaemonState) {
    let Ok(session_id) = id.parse::<SessionId>() else {
        let _ = socket
            .send(Message::Text(serde_json::json!({"error": "invalid session id"}).to_string()))
            .await;
        let _ = socket.close().await;
        return;
    };

    let (mut rx, backlog) = state.events.subscribe(session_id);

    for envelope in backlog {
        let Ok(payload) = serde_json::to_string(&envelope) else { continue };
        if socket.send(Message::Text(payload)).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            biased;
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        // Mirror of POST /sessions/:id/input for WS-only clients:
                        // must both append the turn to the session store *and*
                        // emit the TurnStarted event, exactly like the REST
                        // handler, so `GET /sessions/:id/turns` reflects turns
                        // that came in over the WebSocket too.
                        let turn_id = TurnId::new();
                        let append_result = state
                            .sessions
                            .append_turn(TurnRecord {
                                id: turn_id,
                                session_id,
                                input_text: text.to_string(),
                                output_text: None,
                                created_at: sakha_core::time::now_utc(),
                            })
                            .await;

                        if let Err(err) = append_result {
                            let envelope = EventEnvelope::for_session(
                                session_id,
                                EventKind::SessionBlocked,
                                serde_json::json!({"reason": err.to_string(), "via": "websocket"}),
                            );
                            state.emit(envelope);
                            continue;
                        }

                        let envelope = EventEnvelope::for_session(
                            session_id,
                            EventKind::TurnStarted,
                            serde_json::json!({"turn_id": turn_id.to_string(), "input": text, "via": "websocket"}),
                        );
                        state.emit(envelope);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            broadcasted = rx.recv() => {
                match broadcasted {
                    Ok(envelope) => {
                        let Ok(payload) = serde_json::to_string(&envelope) else { continue };
                        if socket.send(Message::Text(payload)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

pub fn router() -> Router<DaemonState> {
    Router::new().route("/sessions/:id/ws", get(ws_handler))
}
