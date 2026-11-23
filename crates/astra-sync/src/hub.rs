//! The server half of sync: an in-process hub over a pluggable op log.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use astra_protocol::{ClientFrame, Op, OpBody, Presence, ServerFrame};
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;

use crate::client::{Connector, Transport};
use crate::{Result, SyncError};

/// Ops per replay page.
pub const REPLAY_PAGE: usize = 500;

/// Durable per-workspace op log with server sequence numbers.
#[async_trait::async_trait]
pub trait OpLog: Send + Sync {
    /// Store ops not seen before (dedup by op id), assigning consecutive sequence numbers.
    /// Returns only the newly stored ops, in sequence order.
    async fn append(&self, workspace: &str, ops: &[Op]) -> Result<Vec<(u64, Op)>>;
    /// Ops with sequence greater than `cursor`, oldest first, at most `limit`.
    async fn since(&self, workspace: &str, cursor: u64, limit: usize) -> Result<Vec<(u64, Op)>>;
    /// The newest sequence number (0 when empty).
    async fn head(&self, workspace: &str) -> Result<u64>;
    /// Node registry: `node` connected to `workspace` now.
    async fn node_seen(&self, _workspace: &str, _node: &str) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct MemWorkspace {
    ops: Vec<(u64, Op)>,
    ids: std::collections::HashSet<String>,
}

/// In-memory [`OpLog`].
#[derive(Default)]
pub struct MemoryOpLog {
    inner: Mutex<HashMap<String, MemWorkspace>>,
    nodes: Mutex<BTreeMap<(String, String), u64>>,
}

impl MemoryOpLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Nodes that have connected, with how many times.
    pub fn nodes(&self) -> Vec<(String, String)> {
        self.nodes
            .lock()
            .map(|n| n.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, MemWorkspace>>> {
        self.inner
            .lock()
            .map_err(|_| SyncError::Storage("lock poisoned".into()))
    }
}

#[async_trait::async_trait]
impl OpLog for MemoryOpLog {
    async fn append(&self, workspace: &str, ops: &[Op]) -> Result<Vec<(u64, Op)>> {
        let mut all = self.lock()?;
        let ws = all.entry(workspace.to_string()).or_default();
        let mut out = Vec::new();
        for op in ops {
            if ws.ids.insert(op.id.clone()) {
                let seq = ws.ops.len() as u64 + 1;
                ws.ops.push((seq, op.clone()));
                out.push((seq, op.clone()));
            }
        }
        Ok(out)
    }

    async fn since(&self, workspace: &str, cursor: u64, limit: usize) -> Result<Vec<(u64, Op)>> {
        let all = self.lock()?;
        Ok(all
            .get(workspace)
            .map(|ws| {
                ws.ops
                    .iter()
                    .filter(|(s, _)| *s > cursor)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn head(&self, workspace: &str) -> Result<u64> {
        Ok(self
            .lock()?
            .get(workspace)
            .map(|ws| ws.ops.len() as u64)
            .unwrap_or(0))
    }

    async fn node_seen(&self, workspace: &str, node: &str) -> Result<()> {
        if let Ok(mut n) = self.nodes.lock() {
            *n.entry((workspace.to_string(), node.to_string()))
                .or_default() += 1;
        }
        Ok(())
    }
}

/// Who a connection authenticated as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Person id or node id.
    pub subject: String,
    /// The signed-in person, when the hello was authenticated as one. Pushed ops are then held to it:
    /// commands must be authored by this person (runner nodes authorize approve/merge/steer by `actor`),
    /// and other ops by this person or the connection's own node; presence is rewritten to this person.
    pub person: Option<String>,
}

/// Decides whether a `hello` may join its workspace.
#[async_trait::async_trait]
pub trait Authorizer: Send + Sync {
    async fn authorize(
        &self,
        workspace_id: &str,
        node_id: &str,
        token: Option<&str>,
    ) -> Result<Principal>;
}

/// Accepts every hello (tests, trusted in-process use).
pub struct AllowAll;

#[async_trait::async_trait]
impl Authorizer for AllowAll {
    async fn authorize(
        &self,
        _workspace_id: &str,
        node_id: &str,
        _token: Option<&str>,
    ) -> Result<Principal> {
        Ok(Principal {
            subject: node_id.to_string(),
            person: None,
        })
    }
}

/// Connection id within a hub.
pub type ConnId = u64;

struct Conn {
    tx: mpsc::UnboundedSender<ServerFrame>,
    workspace: Option<String>,
    node: Option<String>,
    principal: Option<Principal>,
    live: bool,
    presence: Vec<Presence>,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<ConnId, Conn>,
    workspace_locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

/// The sync hub: many connections, one op log.
pub struct Hub {
    log: Arc<dyn OpLog>,
    auth: Arc<dyn Authorizer>,
    inner: Mutex<Inner>,
    next: AtomicU64,
}

impl Hub {
    /// A hub over `log` authorizing with `auth`.
    pub fn new(log: Arc<dyn OpLog>, auth: Arc<dyn Authorizer>) -> Arc<Hub> {
        Arc::new(Hub {
            log,
            auth,
            inner: Mutex::new(Inner::default()),
            next: AtomicU64::new(1),
        })
    }

    /// Append ops made by the server itself (e.g. a person entity on first sign-in) and fan them out.
    pub async fn append_server_ops(&self, workspace: &str, ops: &[Op]) -> Result<Vec<(u64, Op)>> {
        let lock = self.workspace_lock(workspace)?;
        let _guard = lock.lock().await;
        let stored = self.log.append(workspace, ops).await?;
        self.fan_out(workspace, &stored)?;
        Ok(stored)
    }

    fn fan_out(&self, ws: &str, stored: &[(u64, Op)]) -> Result<()> {
        if let Some((cursor, _)) = stored.last() {
            let frame = ServerFrame::Ops {
                ops: stored.iter().map(|(_, op)| op.clone()).collect(),
                cursor: *cursor,
            };
            let inner = self.lock()?;
            for c in inner.conns.values() {
                if c.live && c.workspace.as_deref() == Some(ws) {
                    let _ = c.tx.send(frame.clone());
                }
            }
        }
        Ok(())
    }

    /// The op log.
    pub fn log(&self) -> &Arc<dyn OpLog> {
        &self.log
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| SyncError::Storage("hub lock poisoned".into()))
    }

    /// Register a connection. Frames for it arrive on the returned receiver.
    pub fn connect(&self) -> (ConnId, mpsc::UnboundedReceiver<ServerFrame>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut inner) = self.lock() {
            inner.conns.insert(
                id,
                Conn {
                    tx,
                    workspace: None,
                    node: None,
                    principal: None,
                    live: false,
                    presence: vec![],
                },
            );
        }
        (id, rx)
    }

    /// Unregister a connection (relays the presence change).
    pub fn disconnect(&self, conn: ConnId) {
        let removed = self
            .lock()
            .ok()
            .and_then(|mut inner| inner.conns.remove(&conn));
        if let Some(Conn {
            workspace: Some(ws),
            presence,
            ..
        }) = removed
            && !presence.is_empty()
        {
            self.broadcast_presence(&ws);
        }
    }

    /// Node ids currently connected (live) to a workspace.
    pub fn online_nodes(&self, workspace: &str) -> Vec<String> {
        self.lock()
            .map(|inner| {
                inner
                    .conns
                    .values()
                    .filter(|c| c.live && c.workspace.as_deref() == Some(workspace))
                    .filter_map(|c| c.node.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The authenticated principal of a connection.
    pub fn principal(&self, conn: ConnId) -> Option<Principal> {
        self.lock()
            .ok()
            .and_then(|inner| inner.conns.get(&conn).and_then(|c| c.principal.clone()))
    }

    fn workspace_lock(&self, ws: &str) -> Result<Arc<tokio::sync::Mutex<()>>> {
        Ok(self
            .lock()?
            .workspace_locks
            .entry(ws.to_string())
            .or_default()
            .clone())
    }

    fn send(&self, conn: ConnId, frame: ServerFrame) {
        if let Ok(inner) = self.lock()
            && let Some(c) = inner.conns.get(&conn)
        {
            let _ = c.tx.send(frame);
        }
    }

    fn conn_workspace(&self, conn: ConnId) -> Result<(String, bool)> {
        let inner = self.lock()?;
        let c = inner.conns.get(&conn).ok_or(SyncError::Closed)?;
        match &c.workspace {
            Some(ws) => Ok((ws.clone(), c.live)),
            None => Err(SyncError::Protocol("hello first".into())),
        }
    }

    /// Rejects ops whose `actor` the connection may not claim (see [`Principal::person`]).
    fn check_actors(&self, conn: ConnId, ops: &[Op]) -> Result<()> {
        let (person, node) = {
            let inner = self.lock()?;
            let c = inner.conns.get(&conn).ok_or(SyncError::Closed)?;
            (
                c.principal.as_ref().and_then(|p| p.person.clone()),
                c.node.clone(),
            )
        };
        let Some(person) = person else {
            return Ok(());
        };
        for op in ops {
            let allowed = match op.body {
                OpBody::Command { .. } => op.actor == person,
                _ => op.actor == person || node.as_deref() == Some(op.actor.as_str()),
            };
            if !allowed {
                return Err(SyncError::Protocol(format!(
                    "op {} claims actor {} but the connection is {}",
                    op.id, op.actor, person
                )));
            }
        }
        Ok(())
    }

    /// Process one client frame. On error an `error` frame has been queued and the caller should close.
    pub async fn handle(&self, conn: ConnId, frame: ClientFrame) -> Result<()> {
        let r = self.handle_inner(conn, frame).await;
        if let Err(e) = &r {
            self.send(
                conn,
                ServerFrame::Error {
                    message: e.to_string(),
                },
            );
        }
        r
    }

    async fn handle_inner(&self, conn: ConnId, frame: ClientFrame) -> Result<()> {
        match frame {
            ClientFrame::Hello {
                workspace_id,
                node_id,
                cursor,
                token,
            } => {
                if self
                    .lock()?
                    .conns
                    .get(&conn)
                    .is_some_and(|c| c.workspace.is_some())
                {
                    return Err(SyncError::Protocol("duplicate hello".into()));
                }
                let principal = self
                    .auth
                    .authorize(&workspace_id, &node_id, token.as_deref())
                    .await?;
                let lock = self.workspace_lock(&workspace_id)?;
                let _guard = lock.lock().await;
                {
                    let mut inner = self.lock()?;
                    let c = inner.conns.get_mut(&conn).ok_or(SyncError::Closed)?;
                    c.workspace = Some(workspace_id.clone());
                    c.node = Some(node_id.clone());
                    c.principal = Some(principal);
                }
                let head = self.log.head(&workspace_id).await?;
                self.send(conn, ServerFrame::Welcome { cursor: head });
                let mut from = cursor;
                loop {
                    let page = self.log.since(&workspace_id, from, REPLAY_PAGE).await?;
                    let Some((last, _)) = page.last() else { break };
                    from = *last;
                    let n = page.len();
                    self.send(
                        conn,
                        ServerFrame::Ops {
                            ops: page.into_iter().map(|(_, op)| op).collect(),
                            cursor: from,
                        },
                    );
                    if n < REPLAY_PAGE {
                        break;
                    }
                }
                {
                    let mut inner = self.lock()?;
                    let c = inner.conns.get_mut(&conn).ok_or(SyncError::Closed)?;
                    c.live = true;
                }
                // Current presence of the workspace.
                let presence = self.presence_of(&workspace_id);
                if !presence.is_empty() {
                    self.send(conn, ServerFrame::Presence { presence });
                }
                self.log.node_seen(&workspace_id, &node_id).await?;
                Ok(())
            }
            ClientFrame::Push { ops } => {
                let (ws, live) = self.conn_workspace(conn)?;
                if !live {
                    return Err(SyncError::Protocol("push before welcome".into()));
                }
                self.check_actors(conn, &ops)?;
                let lock = self.workspace_lock(&ws)?;
                let _guard = lock.lock().await;
                let ids: Vec<String> = ops.iter().map(|o| o.id.clone()).collect();
                let stored = self.log.append(&ws, &ops).await?;
                self.fan_out(&ws, &stored)?;
                self.send(conn, ServerFrame::Ack { ids });
                Ok(())
            }
            ClientFrame::Presence { mut presence } => {
                let (ws, _) = self.conn_workspace(conn)?;
                if let Some(person) = self.principal(conn).and_then(|p| p.person) {
                    for p in &mut presence {
                        p.person_id.clone_from(&person);
                    }
                }
                {
                    let mut inner = self.lock()?;
                    if let Some(c) = inner.conns.get_mut(&conn) {
                        c.presence = presence;
                    }
                }
                self.broadcast_presence(&ws);
                Ok(())
            }
        }
    }

    fn presence_of(&self, ws: &str) -> Vec<Presence> {
        self.lock()
            .map(|inner| {
                let mut ids: Vec<&ConnId> = inner.conns.keys().collect();
                ids.sort();
                ids.into_iter()
                    .filter_map(|id| inner.conns.get(id))
                    .filter(|c| c.workspace.as_deref() == Some(ws))
                    .flat_map(|c| c.presence.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn broadcast_presence(&self, ws: &str) {
        let presence = self.presence_of(ws);
        if let Ok(inner) = self.lock() {
            for c in inner.conns.values() {
                if c.live && c.workspace.as_deref() == Some(ws) {
                    let _ = c.tx.send(ServerFrame::Presence {
                        presence: presence.clone(),
                    });
                }
            }
        }
    }

    /// Serve one connection over a frame stream/sink until either side closes.
    pub async fn serve<St, Si>(self: Arc<Self>, mut incoming: St, mut outgoing: Si)
    where
        St: Stream<Item = Result<ClientFrame>> + Unpin + Send,
        Si: Sink<ServerFrame, Error = SyncError> + Unpin + Send,
    {
        let (conn, mut rx) = self.connect();
        loop {
            tokio::select! {
                frame = incoming.next() => match frame {
                    Some(Ok(frame)) => {
                        if self.handle(conn, frame).await.is_err() {
                            // Flush the error frame, then close.
                            while let Ok(f) = rx.try_recv() {
                                let _ = outgoing.send(f).await;
                            }
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "hub: bad frame");
                        let _ = outgoing.send(ServerFrame::Error { message: e.to_string() }).await;
                        break;
                    }
                    None => break,
                },
                out = rx.recv() => match out {
                    Some(f) => {
                        if outgoing.send(f).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        self.disconnect(conn);
        let _ = outgoing.close().await;
    }
}

/// A [`Connector`] that connects straight into an in-process [`Hub`].
#[derive(Clone)]
pub struct HubConnector {
    pub hub: Arc<Hub>,
}

#[async_trait::async_trait]
impl Connector for HubConnector {
    async fn connect(&self) -> Result<Transport> {
        let (c_tx, c_rx) = futures::channel::mpsc::unbounded::<ClientFrame>();
        let (s_tx, s_rx) = futures::channel::mpsc::unbounded::<ServerFrame>();
        let hub = self.hub.clone();
        tokio::spawn(hub.serve(c_rx.map(Ok), s_tx.sink_map_err(|_| SyncError::Closed)));
        Ok(Transport {
            sink: Box::pin(c_tx.sink_map_err(|_| SyncError::Closed)),
            stream: Box::pin(s_rx.map(Ok)),
        })
    }
}
