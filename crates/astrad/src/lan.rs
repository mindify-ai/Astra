//! LAN host: the sync protocol over a Noise-encrypted WebSocket, backed by the local node.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use astra_node::noise::{self, Cipher, StaticKey};
use astra_node::{Node, NodeEvent};
use astra_protocol::{ClientFrame, ServerFrame};
use astra_sync::codec::{self, Format};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use tokio::sync::broadcast::error::RecvError;

use crate::pairing::LanFiles;

/// Shared LAN server state.
#[derive(Clone)]
pub struct LanState {
    pub node: Node,
    pub files: LanFiles,
    pub key: StaticKey,
    next: Arc<AtomicU64>,
}

impl LanState {
    /// State for `node`.
    pub fn new(node: Node, files: LanFiles, key: StaticKey) -> Self {
        LanState {
            node,
            files,
            key,
            next: Arc::new(AtomicU64::new(1)),
        }
    }
}

/// `GET /v1/lan` (Noise WebSocket) and `GET /healthz`.
pub fn router(state: LanState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/lan", get(upgrade))
        .with_state(state)
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<LanState>) -> Response {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = session(socket, state).await {
            tracing::debug!(error = %e, "lan session ended");
        }
    })
}

async fn recv_binary(socket: &mut WebSocket) -> Result<Vec<u8>, String> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Binary(b))) => return Ok(b.to_vec()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(other)) => return Err(format!("unexpected message {other:?}")),
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("closed".into()),
        }
    }
}

async fn send_frame(
    socket: &mut WebSocket,
    cipher: &mut Cipher,
    frame: &ServerFrame,
) -> Result<(), String> {
    let bytes = codec::encode(Format::MsgPack, frame).map_err(|e| e.to_string())?;
    for m in cipher.seal(&bytes).map_err(|e| e.to_string())? {
        socket
            .send(Message::Binary(m.into()))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn catch_up(
    socket: &mut WebSocket,
    cipher: &mut Cipher,
    node: &Node,
    sent: &mut u64,
) -> Result<(), String> {
    loop {
        let page = node.ops_since(*sent, 500).map_err(|e| e.to_string())?;
        let Some((last, _)) = page.last() else {
            return Ok(());
        };
        *sent = *last;
        let n = page.len();
        send_frame(
            socket,
            cipher,
            &ServerFrame::Ops {
                ops: page.into_iter().map(|(_, o)| o).collect(),
                cursor: *sent,
            },
        )
        .await?;
        if n < 500 {
            return Ok(());
        }
    }
}

async fn session(mut socket: WebSocket, state: LanState) -> Result<(), String> {
    // Noise XX handshake (responder).
    let mut hs = noise::responder(&state.key).map_err(|e| e.to_string())?;
    noise::hs_read(&mut hs, &recv_binary(&mut socket).await?).map_err(|e| e.to_string())?;
    let m2 = noise::hs_write(&mut hs).map_err(|e| e.to_string())?;
    socket
        .send(Message::Binary(m2.into()))
        .await
        .map_err(|e| e.to_string())?;
    noise::hs_read(&mut hs, &recv_binary(&mut socket).await?).map_err(|e| e.to_string())?;
    let peer = hs
        .get_remote_static()
        .map(<[u8]>::to_vec)
        .ok_or("no peer key")?;
    let mut cipher = Cipher::new(hs).map_err(|e| e.to_string())?;
    let conn = state.next.fetch_add(1, Ordering::Relaxed);
    let source = format!("lan:{conn}");

    let mut events = state.node.subscribe();
    let mut sent: u64 = 0;
    let mut hello_done = false;
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    let result: Result<(), String> = async {
        loop {
            tokio::select! {
                msg = socket.recv() => {
                    let bytes = match msg {
                        Some(Ok(Message::Binary(b))) => b,
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Ok(()),
                        Some(Ok(_)) => continue,
                    };
                    let Some(plain) = cipher.open(&bytes).map_err(|e| e.to_string())? else { continue };
                    let frame: ClientFrame = codec::decode(Format::MsgPack, &plain).map_err(|e| e.to_string())?;
                    match frame {
                        ClientFrame::Hello { workspace_id, node_id, cursor, token } => {
                            let paired = state.files.is_paired(&peer);
                            let by_code = !paired
                                && token.as_deref().and_then(|t| t.strip_prefix("pair:")).is_some_and(|c| state.files.consume_code(c));
                            let workspace_ok = workspace_id == state.node.config().workspace_id;
                            if hello_done || !workspace_ok || !(paired || by_code) {
                                send_frame(&mut socket, &mut cipher, &ServerFrame::Error { message: "unauthorized".into() }).await?;
                                let _ = socket.send(Message::Close(Some(CloseFrame { code: 4401, reason: "unauthorized".into() }))).await;
                                return Ok(());
                            }
                            if by_code {
                                state.files.add_device(&peer, &node_id).map_err(|e| e.to_string())?;
                                tracing::info!(%node_id, "paired a new LAN device");
                            }
                            hello_done = true;
                            let head = state.node.cursor().map_err(|e| e.to_string())?;
                            send_frame(&mut socket, &mut cipher, &ServerFrame::Welcome { cursor: head }).await?;
                            sent = cursor;
                            catch_up(&mut socket, &mut cipher, &state.node, &mut sent).await?;
                            let presence = state.node.presence();
                            if !presence.is_empty() {
                                send_frame(&mut socket, &mut cipher, &ServerFrame::Presence { presence }).await?;
                            }
                            // Stream live changes from here on.
                            loop_live(&mut socket, &mut cipher, &state, &mut events, &mut sent, &source, &mut ping).await?;
                            return Ok(());
                        }
                        _ => {
                            send_frame(&mut socket, &mut cipher, &ServerFrame::Error { message: "hello first".into() }).await?;
                            return Ok(());
                        }
                    }
                }
                _ = ping.tick() => {
                    socket.send(Message::Ping(Default::default())).await.map_err(|e| e.to_string())?;
                }
            }
        }
    }
    .await;
    state.node.set_presence_for(&source, vec![]);
    result
}

async fn loop_live(
    socket: &mut WebSocket,
    cipher: &mut Cipher,
    state: &LanState,
    events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
    sent: &mut u64,
    source: &str,
    ping: &mut tokio::time::Interval,
) -> Result<(), String> {
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let bytes = match msg {
                    Some(Ok(Message::Binary(b))) => b,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Ok(()),
                    Some(Ok(_)) => continue,
                };
                let Some(plain) = cipher.open(&bytes).map_err(|e| e.to_string())? else { continue };
                match codec::decode::<ClientFrame>(Format::MsgPack, &plain).map_err(|e| e.to_string())? {
                    ClientFrame::Push { ops } => {
                        let ids = ops.iter().map(|o| o.id.clone()).collect();
                        state.node.apply_ops(ops).map_err(|e| e.to_string())?;
                        catch_up(socket, cipher, &state.node, sent).await?;
                        send_frame(socket, cipher, &ServerFrame::Ack { ids }).await?;
                    }
                    ClientFrame::Presence { presence } => state.node.set_presence_for(source, presence),
                    ClientFrame::Hello { .. } => {
                        send_frame(socket, cipher, &ServerFrame::Error { message: "duplicate hello".into() }).await?;
                        return Ok(());
                    }
                }
            }
            ev = events.recv() => match ev {
                Ok(NodeEvent::Ops { .. }) | Err(RecvError::Lagged(_)) => catch_up(socket, cipher, &state.node, sent).await?,
                Ok(NodeEvent::Presence(presence)) => send_frame(socket, cipher, &ServerFrame::Presence { presence }).await?,
                Ok(NodeEvent::Connectivity(_)) => {}
                Err(RecvError::Closed) => return Ok(()),
            },
            _ = ping.tick() => {
                socket.send(Message::Ping(Default::default())).await.map_err(|e| e.to_string())?;
            }
        }
    }
}

/// Advertise `_astra._tcp` over mDNS. Keep the returned daemon alive while serving.
pub fn advertise(
    node_id: &str,
    workspace: &str,
    ip: std::net::IpAddr,
    port: u16,
    fp: &str,
) -> Result<mdns_sd::ServiceDaemon, String> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host = format!(
        "{}.local.",
        node_id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-")
    );
    let props = [("node", node_id), ("ws", workspace), ("fp", fp)];
    let info =
        mdns_sd::ServiceInfo::new("_astra._tcp.local.", node_id, &host, ip, port, &props[..])
            .map_err(|e| e.to_string())?;
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(daemon)
}
