//! The client half of sync.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_protocol::{ClientFrame, Op, Presence, ServerFrame};
use astra_store::{Origin, Store};
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;

use crate::{Result, SyncError};

/// Maximum ops per `push` frame.
pub const PUSH_BATCH: usize = 256;

/// What the sync client needs from the local node.
pub trait SyncHost: Send + Sync + 'static {
    /// The last server cursor received.
    fn cursor(&self) -> Result<u64>;
    /// Local ops not yet acknowledged by the server, oldest first.
    fn pending(&self) -> Result<Vec<Op>>;
    /// Apply ops received from the server and store the new cursor.
    fn apply_remote(&self, ops: Vec<Op>, cursor: u64) -> Result<()>;
    /// The server durably stored these pushed ops.
    fn acked(&self, ids: &[String]) -> Result<()>;
    /// Presence of everyone in the workspace.
    fn remote_presence(&self, _presence: Vec<Presence>) {}
    /// Connection came up (after `welcome`) or went down.
    fn connectivity(&self, _online: bool) {}
}

/// A [`SyncHost`] over a shared [`Store`] (no event fan-out).
#[derive(Clone)]
pub struct StoreHost {
    pub store: Arc<Mutex<Store>>,
}

impl StoreHost {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>> {
        self.store.lock().map_err(|_| SyncError::Storage("store lock poisoned".into()))
    }
}

impl SyncHost for StoreHost {
    fn cursor(&self) -> Result<u64> {
        Ok(self.lock()?.cursor()?)
    }
    fn pending(&self) -> Result<Vec<Op>> {
        Ok(self.lock()?.pending_outbound()?)
    }
    fn apply_remote(&self, ops: Vec<Op>, cursor: u64) -> Result<()> {
        let mut s = self.lock()?;
        s.append(&ops, Origin::Remote)?;
        s.set_cursor(cursor)?;
        Ok(())
    }
    fn acked(&self, ids: &[String]) -> Result<()> {
        Ok(self.lock()?.mark_acked(ids, None)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Disconnected,
    AwaitWelcome,
    Live,
}

/// Sans-IO client state machine. Feed it connection events and server frames; send what it returns.
#[derive(Debug)]
pub struct ClientCore {
    workspace_id: String,
    node_id: String,
    token: Option<String>,
    phase: Phase,
    in_flight: HashSet<String>,
    presence: Option<Vec<Presence>>,
}

impl ClientCore {
    /// A client for `workspace_id` identifying as `node_id`.
    pub fn new(workspace_id: impl Into<String>, node_id: impl Into<String>, token: Option<String>) -> Self {
        ClientCore {
            workspace_id: workspace_id.into(),
            node_id: node_id.into(),
            token,
            phase: Phase::Disconnected,
            in_flight: HashSet::new(),
            presence: None,
        }
    }

    /// True after `welcome`.
    pub fn is_live(&self) -> bool {
        self.phase == Phase::Live
    }

    /// A transport connected: returns the `hello` frame.
    pub fn on_connected(&mut self, host: &dyn SyncHost) -> Result<Vec<ClientFrame>> {
        self.phase = Phase::AwaitWelcome;
        self.in_flight.clear();
        Ok(vec![ClientFrame::Hello {
            workspace_id: self.workspace_id.clone(),
            node_id: self.node_id.clone(),
            cursor: host.cursor()?,
            token: self.token.clone(),
        }])
    }

    /// The transport closed.
    pub fn on_disconnected(&mut self) {
        self.phase = Phase::Disconnected;
        self.in_flight.clear();
    }

    /// Handle a server frame; returns frames to send.
    pub fn on_frame(&mut self, host: &dyn SyncHost, frame: ServerFrame) -> Result<Vec<ClientFrame>> {
        match frame {
            ServerFrame::Welcome { .. } => {
                if self.phase != Phase::AwaitWelcome {
                    return Err(SyncError::Protocol("unexpected welcome".into()));
                }
                self.phase = Phase::Live;
                let mut out = self.push_pending(host)?;
                if let Some(p) = &self.presence {
                    out.push(ClientFrame::Presence { presence: p.clone() });
                }
                Ok(out)
            }
            ServerFrame::Ops { ops, cursor } => {
                if self.phase == Phase::Disconnected {
                    return Err(SyncError::Protocol("ops before hello".into()));
                }
                host.apply_remote(ops, cursor)?;
                Ok(vec![])
            }
            ServerFrame::Ack { ids } => {
                host.acked(&ids)?;
                for id in &ids {
                    self.in_flight.remove(id);
                }
                Ok(vec![])
            }
            ServerFrame::Presence { presence } => {
                host.remote_presence(presence);
                Ok(vec![])
            }
            ServerFrame::Error { message } => Err(SyncError::Server(message)),
        }
    }

    /// Local ops were appended; returns `push` frames when live.
    pub fn on_local_change(&mut self, host: &dyn SyncHost) -> Result<Vec<ClientFrame>> {
        if self.phase != Phase::Live {
            return Ok(vec![]);
        }
        self.push_pending(host)
    }

    /// Local presence changed; returns a `presence` frame when live (otherwise sent after `welcome`).
    pub fn on_presence(&mut self, presence: Vec<Presence>) -> Vec<ClientFrame> {
        self.presence = Some(presence.clone());
        if self.phase == Phase::Live { vec![ClientFrame::Presence { presence }] } else { vec![] }
    }

    fn push_pending(&mut self, host: &dyn SyncHost) -> Result<Vec<ClientFrame>> {
        let pending: Vec<Op> =
            host.pending()?.into_iter().filter(|op| !self.in_flight.contains(&op.id)).collect();
        let mut out = Vec::new();
        for chunk in pending.chunks(PUSH_BATCH) {
            for op in chunk {
                self.in_flight.insert(op.id.clone());
            }
            out.push(ClientFrame::Push { ops: chunk.to_vec() });
        }
        Ok(out)
    }
}

/// A connected frame transport.
pub struct Transport {
    pub sink: Pin<Box<dyn Sink<ClientFrame, Error = SyncError> + Send>>,
    pub stream: Pin<Box<dyn Stream<Item = Result<ServerFrame>> + Send>>,
}

/// Opens transports (WebSocket, in-memory, ...).
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self) -> Result<Transport>;
}

/// Client configuration.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub workspace_id: String,
    pub node_id: String,
    pub token: Option<String>,
    pub min_backoff: Duration,
    pub max_backoff: Duration,
}

impl SyncConfig {
    /// Defaults: 250 ms → 30 s exponential backoff.
    pub fn new(workspace_id: impl Into<String>, node_id: impl Into<String>, token: Option<String>) -> Self {
        SyncConfig {
            workspace_id: workspace_id.into(),
            node_id: node_id.into(),
            token,
            min_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(30),
        }
    }
}

enum Control {
    LocalChange,
    Presence(Vec<Presence>),
    Shutdown,
}

/// Handle to a running [`SyncClient`].
#[derive(Clone)]
pub struct SyncHandle {
    tx: mpsc::UnboundedSender<Control>,
}

impl SyncHandle {
    /// Tell the client that local ops were appended.
    pub fn notify_local(&self) {
        let _ = self.tx.send(Control::LocalChange);
    }
    /// Replace this node's presence.
    pub fn set_presence(&self, presence: Vec<Presence>) {
        let _ = self.tx.send(Control::Presence(presence));
    }
    /// Stop the client.
    pub fn shutdown(&self) {
        let _ = self.tx.send(Control::Shutdown);
    }
}

/// The async sync client: connects, syncs, and reconnects with exponential backoff.
pub struct SyncClient {
    config: SyncConfig,
    host: Arc<dyn SyncHost>,
    core: ClientCore,
    rx: mpsc::UnboundedReceiver<Control>,
}

enum SessionEnd {
    Shutdown,
    Lost { was_live: bool },
}

impl SyncClient {
    /// A client and its control handle.
    pub fn new(config: SyncConfig, host: Arc<dyn SyncHost>) -> (SyncClient, SyncHandle) {
        let (tx, rx) = mpsc::unbounded_channel();
        let core = ClientCore::new(config.workspace_id.clone(), config.node_id.clone(), config.token.clone());
        (SyncClient { config, host, core, rx }, SyncHandle { tx })
    }

    /// Run until [`SyncHandle::shutdown`] (or every handle is dropped).
    pub async fn run(mut self, connector: Arc<dyn Connector>) {
        let mut backoff = self.config.min_backoff;
        loop {
            match connector.connect().await {
                Ok(transport) => match self.session(transport).await {
                    SessionEnd::Shutdown => return,
                    SessionEnd::Lost { was_live } => {
                        if was_live {
                            backoff = self.config.min_backoff;
                        }
                    }
                },
                Err(e) => tracing::debug!(error = %e, "sync connect failed"),
            }
            self.core.on_disconnected();
            self.host.connectivity(false);
            let sleep = tokio::time::sleep(backoff);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    ctl = self.rx.recv() => match ctl {
                        None | Some(Control::Shutdown) => return,
                        Some(Control::Presence(p)) => { self.core.on_presence(p); }
                        Some(Control::LocalChange) => {}
                    }
                }
            }
            backoff = (backoff * 2).min(self.config.max_backoff);
        }
    }

    async fn session(&mut self, mut t: Transport) -> SessionEnd {
        let mut was_live = false;
        let hello = match self.core.on_connected(self.host.as_ref()) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "sync hello failed");
                return SessionEnd::Lost { was_live };
            }
        };
        if send_all(&mut t, hello).await.is_err() {
            return SessionEnd::Lost { was_live };
        }
        loop {
            let out = tokio::select! {
                frame = t.stream.next() => match frame {
                    Some(Ok(frame)) => {
                        let r = self.core.on_frame(self.host.as_ref(), frame);
                        if self.core.is_live() && !was_live {
                            was_live = true;
                            self.host.connectivity(true);
                        }
                        r
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "sync transport error");
                        return SessionEnd::Lost { was_live };
                    }
                    None => return SessionEnd::Lost { was_live },
                },
                ctl = self.rx.recv() => match ctl {
                    None | Some(Control::Shutdown) => {
                        let _ = t.sink.close().await;
                        return SessionEnd::Shutdown;
                    }
                    Some(Control::LocalChange) => self.core.on_local_change(self.host.as_ref()),
                    Some(Control::Presence(p)) => Ok(self.core.on_presence(p)),
                },
            };
            match out {
                Ok(frames) => {
                    if send_all(&mut t, frames).await.is_err() {
                        return SessionEnd::Lost { was_live };
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "sync session error");
                    return SessionEnd::Lost { was_live };
                }
            }
        }
    }
}

async fn send_all(t: &mut Transport, frames: Vec<ClientFrame>) -> Result<()> {
    for f in frames {
        t.sink.feed(f).await?;
    }
    t.sink.flush().await
}
