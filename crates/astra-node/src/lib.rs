//! The embeddable Astra node used by the desktop shell (Tauri), mobile builds and `astrad`.
//!
//! A node owns a SQLite store, reduces ops into [`State`], fans changes out to local subscribers, syncs with
//! a relay over WebSocket when configured, and (feature `runtime`) executes `command` ops addressed to its
//! node id with the agent runtime.
//!
//! Cursor semantics for local clients: `snapshot().cursor` and `NodeEvent::Ops.cursor` are the node's local
//! op-log sequence (not the relay's), so a LAN client can resume with [`Node::ops_since`].

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use astra_protocol::{HlcClock, Op, OpBody, Presence, RelayStatus, Seed, Snapshot, State};
use astra_store::{Origin, Store};
use astra_sync::{SyncClient, SyncConfig, SyncHandle, SyncHost};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

mod ws;

pub use ws::WsConnector;

#[cfg(feature = "runtime")]
pub use astra_runtime;

/// Node errors.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("store: {0}")]
    Store(#[from] astra_store::StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("seed: {0}")]
    Seed(String),
    #[error("{0}")]
    Other(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, NodeError>;

/// Relay (SaaS server or another node) to sync with.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// WebSocket URL of the sync endpoint, e.g. `wss://astra.example.com/v1/sync`.
    pub url: String,
    /// Bearer token sent in `hello`.
    pub token: String,
}

/// Node configuration.
#[derive(Clone)]
pub struct NodeConfig {
    pub data_dir: PathBuf,
    pub node_id: String,
    pub workspace_id: String,
    /// The local person id.
    pub me: String,
    /// Run the agent runtime (requires feature `runtime`).
    pub runtime: bool,
    pub relay: Option<RelayConfig>,
    /// Load this seed document when the store is empty.
    pub seed_if_empty: Option<PathBuf>,
    /// Secrets for agents; default: OS keychain (feature `keyring`) or memory, with env fallback.
    #[cfg(feature = "runtime")]
    pub secrets: Option<Arc<dyn astra_runtime::SecretStore>>,
}

impl NodeConfig {
    /// A config with runtime off, no relay, no seed.
    pub fn new(
        data_dir: impl Into<PathBuf>,
        node_id: impl Into<String>,
        workspace_id: impl Into<String>,
        me: impl Into<String>,
    ) -> Self {
        NodeConfig {
            data_dir: data_dir.into(),
            node_id: node_id.into(),
            workspace_id: workspace_id.into(),
            me: me.into(),
            runtime: false,
            relay: None,
            seed_if_empty: None,
            #[cfg(feature = "runtime")]
            secrets: None,
        }
    }
}

/// Events for local clients (Tauri: `astra://ops`, `astra://presence`, `astra://connectivity`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeEvent {
    /// New ops (local or remote), in log order; `cursor` is the node's local sequence after them.
    Ops { ops: Vec<Op>, cursor: u64 },
    /// Everyone's presence.
    Presence { presence: Vec<Presence> },
    /// Relay connectivity.
    Connectivity { online: bool, relay: RelayStatus },
}

struct Inner {
    config: NodeConfig,
    store: Mutex<Store>,
    clock: Mutex<HlcClock>,
    events: broadcast::Sender<NodeEvent>,
    sync: Mutex<Option<SyncHandle>>,
    /// Presence by source (`local`, `lan:<id>`, ...).
    presence: Mutex<BTreeMap<String, Vec<Presence>>>,
    remote_presence: Mutex<Option<Vec<Presence>>>,
    connectivity: Mutex<(bool, RelayStatus)>,
    #[cfg(feature = "runtime")]
    orchestrator: Mutex<Option<astra_runtime::Orchestrator>>,
}

/// An Astra node.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked mid-update; the data is still usable.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Node {
    /// Open (or create) the node in `config.data_dir`. Must be called inside a tokio runtime.
    pub async fn open(config: NodeConfig) -> Result<Node> {
        std::fs::create_dir_all(&config.data_dir)?;
        let mut store = Store::open(config.data_dir.join("astra.db"))?;
        if let Some(seed_path) = &config.seed_if_empty {
            if store.op_count()? == 0 && store.state().is_empty() {
                let text = std::fs::read_to_string(seed_path)?;
                let seed: Seed = serde_json::from_str(&text).map_err(|e| NodeError::Seed(e.to_string()))?;
                store.seed(&seed)?;
            }
        }
        store.set_identity(&config.workspace_id, &config.me)?;
        let mut clock = HlcClock::new(config.node_id.clone());
        if let Some((_, op)) = store.ops_since(store.head_seq()?.saturating_sub(1), 1)?.into_iter().next() {
            clock.observe(&op.hlc);
        }
        let (events, _) = broadcast::channel(1024);
        let relay_status = if config.relay.is_some() { RelayStatus::Disconnected } else { RelayStatus::Disabled };
        let inner = Arc::new(Inner {
            config: config.clone(),
            store: Mutex::new(store),
            clock: Mutex::new(clock),
            events,
            sync: Mutex::new(None),
            presence: Mutex::new(BTreeMap::new()),
            remote_presence: Mutex::new(None),
            connectivity: Mutex::new((false, relay_status)),
            #[cfg(feature = "runtime")]
            orchestrator: Mutex::new(None),
        });
        let node = Node { inner };

        if config.runtime {
            #[cfg(feature = "runtime")]
            node.start_runtime()?;
            #[cfg(not(feature = "runtime"))]
            return Err(NodeError::Other("this build has no agent runtime (feature `runtime`)".into()));
        }

        if let Some(relay) = &config.relay {
            let host: Arc<dyn SyncHost> = Arc::new(NodeSyncHost { node: node.clone() });
            let (client, handle) = SyncClient::new(
                SyncConfig::new(config.workspace_id.clone(), config.node_id.clone(), Some(relay.token.clone())),
                host,
            );
            *lock(&node.inner.sync) = Some(handle);
            tokio::spawn(client.run(Arc::new(WsConnector::new(relay.url.clone()))));
        }
        Ok(node)
    }

    #[cfg(feature = "runtime")]
    fn start_runtime(&self) -> Result<()> {
        use astra_runtime::secrets::{MemorySecrets, WithEnvFallback};
        let secrets: Arc<dyn astra_runtime::SecretStore> = match &self.inner.config.secrets {
            Some(s) => s.clone(),
            None => {
                #[cfg(feature = "keyring")]
                {
                    Arc::new(WithEnvFallback(astra_runtime::secrets::KeyringSecrets::new("ai.mindify.astra")))
                }
                #[cfg(not(feature = "keyring"))]
                {
                    Arc::new(WithEnvFallback(MemorySecrets::new()))
                }
            }
        };
        #[cfg(feature = "keyring")]
        let _ = MemorySecrets::new;
        let host: Arc<dyn astra_runtime::NodeHost> = Arc::new(RuntimeHost { node: Arc::downgrade(&self.inner) });
        let orch = astra_runtime::Orchestrator::start(
            host,
            astra_runtime::OrchestratorConfig::new(self.inner.config.data_dir.clone(), secrets),
        );
        // Recover commands that arrived but were never executed (e.g. the node stopped mid-way).
        for op in lock(&self.inner.store).unexecuted_commands(&self.inner.config.node_id)? {
            orch.submit(&op);
        }
        *lock(&self.inner.orchestrator) = Some(orch);
        Ok(())
    }

    /// This node's id.
    pub fn node_id(&self) -> &str {
        &self.inner.config.node_id
    }

    /// The configuration.
    pub fn config(&self) -> &NodeConfig {
        &self.inner.config
    }

    /// State (with merged presence) and the local cursor.
    pub fn snapshot(&self) -> Result<Snapshot> {
        let (mut state, cursor) = {
            let store = lock(&self.inner.store);
            (store.state().clone(), store.head_seq()?)
        };
        state.presence = self.presence();
        Ok(Snapshot { state, cursor })
    }

    /// Read the state without cloning it.
    pub fn with_state<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(lock(&self.inner.store).state())
    }

    /// Append ops made by local clients (or LAN peers) and sync them. Returns the ops that were new.
    pub fn apply(&self, ops: Vec<Op>) -> Result<Vec<Op>> {
        {
            let mut clock = lock(&self.inner.clock);
            for op in &ops {
                clock.observe(&op.hlc);
            }
        }
        let fresh = self.append(&ops, Origin::Local)?;
        if !fresh.is_empty() {
            if let Some(h) = lock(&self.inner.sync).as_ref() {
                h.notify_local();
            }
        }
        Ok(fresh)
    }

    /// Stamp an op with this node's clock, as `actor`.
    pub fn make_op(&self, actor: impl Into<String>, body: OpBody) -> Op {
        let hlc = lock(&self.inner.clock).tick();
        Op::new(hlc, actor, body)
    }

    fn append(&self, ops: &[Op], origin: Origin) -> Result<Vec<Op>> {
        let (fresh, cursor) = {
            let mut store = lock(&self.inner.store);
            let fresh = store.append(ops, origin)?;
            (fresh, store.head_seq()?)
        };
        if !fresh.is_empty() {
            let _ = self.inner.events.send(NodeEvent::Ops { ops: fresh.clone(), cursor });
            #[cfg(feature = "runtime")]
            if let Some(orch) = lock(&self.inner.orchestrator).as_ref() {
                for op in &fresh {
                    orch.submit(op);
                }
            }
        }
        Ok(fresh)
    }

    /// Ops after local sequence `cursor` (for LAN clients), oldest first.
    pub fn ops_since(&self, cursor: u64, limit: usize) -> Result<Vec<(u64, Op)>> {
        Ok(lock(&self.inner.store).ops_since(cursor, limit)?)
    }

    /// The local head sequence.
    pub fn cursor(&self) -> Result<u64> {
        Ok(lock(&self.inner.store).head_seq()?)
    }

    /// Subscribe to node events.
    pub fn subscribe(&self) -> broadcast::Receiver<NodeEvent> {
        self.inner.events.subscribe()
    }

    /// Set this device's presence.
    pub fn set_presence(&self, presence: Vec<Presence>) {
        self.set_presence_for("local", presence);
    }

    /// Set presence on behalf of a source (e.g. a LAN peer). Empty clears it.
    pub fn set_presence_for(&self, source: &str, presence: Vec<Presence>) {
        {
            let mut p = lock(&self.inner.presence);
            if presence.is_empty() {
                p.remove(source);
            } else {
                p.insert(source.to_string(), presence);
            }
        }
        let ours: Vec<Presence> = lock(&self.inner.presence).values().flatten().cloned().collect();
        if let Some(h) = lock(&self.inner.sync).as_ref() {
            h.set_presence(ours);
        }
        let _ = self.inner.events.send(NodeEvent::Presence { presence: self.presence() });
    }

    /// Everyone's presence: the relay's view when connected, merged with this node's own sources.
    pub fn presence(&self) -> Vec<Presence> {
        let ours: Vec<Presence> = lock(&self.inner.presence).values().flatten().cloned().collect();
        match lock(&self.inner.remote_presence).as_ref() {
            Some(remote) => {
                let mut all = remote.clone();
                for p in ours {
                    if !all.contains(&p) {
                        all.push(p);
                    }
                }
                all
            }
            None => ours,
        }
    }

    /// Current connectivity.
    pub fn connectivity(&self) -> (bool, RelayStatus) {
        *lock(&self.inner.connectivity)
    }

    /// Recent process logs of a mission agent (runtime nodes).
    pub fn agent_logs(&self, mission_agent_id: &str) -> Vec<String> {
        #[cfg(feature = "runtime")]
        if let Some(o) = lock(&self.inner.orchestrator).as_ref() {
            return o.logs(mission_agent_id);
        }
        let _ = mission_agent_id;
        vec![]
    }

    /// Stop syncing and cancel running agents.
    pub fn shutdown(&self) {
        if let Some(h) = lock(&self.inner.sync).take() {
            h.shutdown();
        }
        #[cfg(feature = "runtime")]
        if let Some(o) = lock(&self.inner.orchestrator).take() {
            o.shutdown();
        }
    }
}

struct NodeSyncHost {
    node: Node,
}

impl SyncHost for NodeSyncHost {
    fn cursor(&self) -> astra_sync::Result<u64> {
        Ok(lock(&self.node.inner.store).cursor()?)
    }
    fn pending(&self) -> astra_sync::Result<Vec<Op>> {
        Ok(lock(&self.node.inner.store).pending_outbound()?)
    }
    fn apply_remote(&self, ops: Vec<Op>, cursor: u64) -> astra_sync::Result<()> {
        {
            let mut clock = lock(&self.node.inner.clock);
            for op in &ops {
                clock.observe(&op.hlc);
            }
        }
        self.node.append(&ops, Origin::Remote).map_err(|e| astra_sync::SyncError::Storage(e.to_string()))?;
        lock(&self.node.inner.store).set_cursor(cursor)?;
        Ok(())
    }
    fn acked(&self, ids: &[String]) -> astra_sync::Result<()> {
        Ok(lock(&self.node.inner.store).mark_acked(ids, None)?)
    }
    fn remote_presence(&self, presence: Vec<Presence>) {
        *lock(&self.node.inner.remote_presence) = Some(presence);
        let _ = self.node.inner.events.send(NodeEvent::Presence { presence: self.node.presence() });
    }
    fn connectivity(&self, online: bool) {
        let status = if online { RelayStatus::Connected } else { RelayStatus::Disconnected };
        let changed = {
            let mut c = lock(&self.node.inner.connectivity);
            let changed = *c != (online, status);
            *c = (online, status);
            changed
        };
        if !online {
            *lock(&self.node.inner.remote_presence) = None;
        }
        if changed {
            let _ = self.node.inner.events.send(NodeEvent::Connectivity { online, relay: status });
        }
    }
}

#[cfg(feature = "runtime")]
struct RuntimeHost {
    node: std::sync::Weak<Inner>,
}

#[cfg(feature = "runtime")]
impl astra_runtime::NodeHost for RuntimeHost {
    fn node_id(&self) -> String {
        self.node.upgrade().map(|n| n.config.node_id.clone()).unwrap_or_default()
    }
    fn with_state(&self, f: &mut dyn FnMut(&State)) {
        match self.node.upgrade() {
            Some(n) => f(lock(&n.store).state()),
            None => f(&State::default()),
        }
    }
    fn emit(&self, bodies: Vec<OpBody>) -> std::result::Result<Vec<Op>, String> {
        let inner = self.node.upgrade().ok_or("node closed")?;
        let node = Node { inner };
        let actor = node.inner.config.node_id.clone();
        let ops: Vec<Op> = bodies.into_iter().map(|b| node.make_op(actor.clone(), b)).collect();
        node.apply(ops.clone()).map_err(|e| e.to_string())?;
        Ok(ops)
    }
    fn has_command_result(&self, command_op: &str) -> bool {
        self.node.upgrade().is_some_and(|n| lock(&n.store).has_command_result(command_op).unwrap_or(false))
    }
}
