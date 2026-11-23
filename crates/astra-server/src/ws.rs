//! `GET /v1/sync`: the sync WebSocket (MessagePack by default, `?format=json` for JSON).

use std::collections::HashMap;

use astra_protocol::{ClientFrame, ServerFrame};
use astra_sync::SyncError;
use astra_sync::codec::{self, Format};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;

use crate::AppState;

/// Close code for failed authentication.
pub const CLOSE_UNAUTHORIZED: u16 = 4401;

/// Upgrade handler.
pub async fn sync_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let format = Format::from_query(q.get("format").map(String::as_str));
    ws.on_upgrade(move |socket| serve_socket(socket, state, format))
}

fn encode(format: Format, frame: &ServerFrame) -> Option<Message> {
    let bytes = codec::encode(format, frame).ok()?;
    Some(match format {
        Format::MsgPack => Message::Binary(bytes.into()),
        Format::Json => Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
    })
}

async fn close(socket: &mut WebSocket, code: u16, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.to_string().into(),
        })))
        .await;
}

async fn serve_socket(mut socket: WebSocket, state: AppState, format: Format) {
    let hub = state.hub.clone();
    let (conn, mut rx) = hub.connect();
    let mut shutdown = state.shutdown.clone();
    let mut ping = tokio::time::interval(state.config.ping_interval);
    ping.tick().await;
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let frame: Result<ClientFrame, SyncError> = match msg {
                    Some(Ok(Message::Binary(b))) => codec::decode(format, &b),
                    Some(Ok(Message::Text(t))) => codec::decode(Format::Json, t.as_bytes()),
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                let result = match frame {
                    Ok(f) => hub.handle(conn, f).await,
                    Err(e) => {
                        let _ = socket.send(encode(format, &ServerFrame::Error { message: e.to_string() }).unwrap_or(Message::Ping(Default::default()))).await;
                        Err(e)
                    }
                };
                if let Err(e) = result {
                    // Flush queued frames (including the error frame), then close.
                    while let Ok(f) = rx.try_recv() {
                        if let Some(m) = encode(format, &f) {
                            let _ = socket.send(m).await;
                        }
                    }
                    match e {
                        SyncError::Unauthorized(reason) => {
                            tracing::info!(%reason, "sync: unauthorized");
                            close(&mut socket, CLOSE_UNAUTHORIZED, "unauthorized").await;
                        }
                        other => {
                            tracing::debug!(error = %other, "sync: closing connection");
                            close(&mut socket, 1008, "protocol error").await;
                        }
                    }
                    break;
                }
            }
            out = rx.recv() => match out {
                Some(f) => {
                    let Some(m) = encode(format, &f) else { continue };
                    if socket.send(m).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
            _ = shutdown.changed() => {
                close(&mut socket, 1000, "server shutting down").await;
                break;
            }
        }
    }
    hub.disconnect(conn);
}
