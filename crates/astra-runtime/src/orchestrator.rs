//! Executes protocol commands addressed to this runner node. Every effect is expressed as ops
//! (`put`/`patch` + `commandResult`) emitted through the [`NodeHost`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_protocol::*;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};

use crate::adapters::{self, AgentAdapter, AgentContext, AgentControl, AgentEvent, Approver, PermissionRequest};
use crate::git;
use crate::policy::{may_decide, may_steer};
use crate::process::LogRing;
use crate::secrets::SecretStore;

/// What the orchestrator needs from its node.
pub trait NodeHost: Send + Sync + 'static {
    /// This node's id (commands addressed here are executed).
    fn node_id(&self) -> String;
    /// Read the current state.
    fn with_state(&self, f: &mut dyn FnMut(&State));
    /// Stamp, append and sync ops made by this node. Returns the stamped ops.
    fn emit(&self, bodies: Vec<OpBody>) -> Result<Vec<Op>, String>;
    /// Whether a `commandResult` exists for this command op id.
    fn has_command_result(&self, command_op: &str) -> bool;
}

fn read<R>(host: &dyn NodeHost, f: impl FnOnce(&State) -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    host.with_state(&mut |s| {
        if let Some(f) = f.take() {
            out = Some(f(s));
        }
    });
    // `with_state` always calls back exactly once.
    out.unwrap_or_else(|| unreachable_state())
}

#[cold]
fn unreachable_state<R>() -> R {
    panic!("NodeHost::with_state did not call back")
}

/// Builds adapters from protocol adapter configs (override in tests).
pub type AdapterFactory = Arc<dyn Fn(&Adapter) -> Box<dyn AgentAdapter> + Send + Sync>;

/// Orchestrator configuration.
#[derive(Clone)]
pub struct OrchestratorConfig {
    /// Worktrees live under `<data_dir>/worktrees`.
    pub data_dir: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
    pub adapters: AdapterFactory,
}

impl OrchestratorConfig {
    /// Defaults with the standard adapters.
    pub fn new(data_dir: impl Into<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        OrchestratorConfig { data_dir: data_dir.into(), secrets, adapters: Arc::new(adapters::for_config) }
    }
}

#[derive(Debug, Clone, Default)]
struct TurnState {
    /// Completed turns (done or failed).
    seq: u64,
    error: Option<String>,
    exited: bool,
}

struct LiveAgent {
    mission_id: String,
    control: mpsc::UnboundedSender<AgentControl>,
    turns: watch::Receiver<TurnState>,
}

#[derive(Default)]
struct AgentStream {
    message_id: Option<String>,
    body: String,
    plan: Vec<PlanStep>,
    log: Vec<LogLine>,
    dirty: bool,
    turn_text: String,
    last_plan: Vec<PlanStep>,
    latest: VecDeque<LogLine>,
}

struct Inner {
    host: Arc<dyn NodeHost>,
    config: OrchestratorConfig,
    agents: Mutex<HashMap<String, LiveAgent>>,
    streams: Mutex<HashMap<String, AgentStream>>,
    logs: Mutex<HashMap<String, LogRing>>,
    task_of: Mutex<HashMap<String, String>>,
    approvals: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    executing: Mutex<HashSet<String>>,
    cmd_tx: mpsc::UnboundedSender<Op>,
}

/// The command executor of a runner node.
#[derive(Clone)]
pub struct Orchestrator {
    inner: Arc<Inner>,
}

type CmdResult = Result<(), String>;

fn ulid_suffix() -> String {
    new_op_id().to_lowercase()
}

impl Orchestrator {
    /// Start the orchestrator (spawns its command loop on the current tokio runtime).
    pub fn start(host: Arc<dyn NodeHost>, config: OrchestratorConfig) -> Orchestrator {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Op>();
        let inner = Arc::new(Inner {
            host,
            config,
            agents: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            logs: Mutex::new(HashMap::new()),
            task_of: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            executing: Mutex::new(HashSet::new()),
            cmd_tx,
        });
        let orch = Orchestrator { inner };
        let o = orch.clone();
        tokio::spawn(async move {
            while let Some(op) = cmd_rx.recv().await {
                o.execute(op).await;
            }
        });
        let o = orch.clone();
        tokio::spawn(async move {
            // Coalesce streamed agent text into message patches.
            let mut tick = tokio::time::interval(Duration::from_millis(150));
            loop {
                tick.tick().await;
                if Arc::strong_count(&o.inner) <= 2 {
                    break;
                }
                let ids: Vec<String> =
                    o.inner.streams.lock().map(|s| s.iter().filter(|(_, v)| v.dirty).map(|(k, _)| k.clone()).collect()).unwrap_or_default();
                for id in ids {
                    o.flush(&id, false);
                }
            }
        });
        orch
    }

    /// Queue an op; command ops addressed to this node are executed exactly once, in order.
    pub fn submit(&self, op: &Op) {
        if let Some((node, _)) = op.as_command() {
            if node == self.inner.host.node_id() {
                let _ = self.inner.cmd_tx.send(op.clone());
            }
        }
    }

    /// Recent log lines of a mission agent's process / session.
    pub fn logs(&self, mission_agent_id: &str) -> Vec<String> {
        self.inner.logs.lock().ok().and_then(|l| l.get(mission_agent_id).map(LogRing::lines)).unwrap_or_default()
    }

    /// Whether a mission agent has a live session.
    pub fn is_running(&self, mission_agent_id: &str) -> bool {
        self.inner.agents.lock().map(|a| a.contains_key(mission_agent_id)).unwrap_or(false)
    }

    /// Cancel every running agent.
    pub fn shutdown(&self) {
        if let Ok(agents) = self.inner.agents.lock() {
            for a in agents.values() {
                let _ = a.control.send(AgentControl::Cancel);
            }
        }
    }

    fn host(&self) -> &dyn NodeHost {
        self.inner.host.as_ref()
    }

    fn emit(&self, bodies: Vec<OpBody>) {
        if bodies.is_empty() {
            return;
        }
        if let Err(e) = self.inner.host.emit(bodies) {
            tracing::error!(error = %e, "orchestrator failed to emit ops");
        }
    }

    async fn execute(&self, op: Op) {
        let Some((_, command)) = op.as_command() else { return };
        if self.host().has_command_result(&op.id) {
            return;
        }
        {
            let Ok(mut ex) = self.inner.executing.lock() else { return };
            if !ex.insert(op.id.clone()) {
                return;
            }
        }
        let command = command.clone();
        let actor = op.actor.clone();
        tracing::info!(op = %op.id, ?command, %actor, "executing command");
        let result = match command {
            Command::StartMission { mission_id } => self.start_mission(&mission_id),
            Command::Approve { approval_id } => self.decide(&approval_id, &actor, true),
            Command::Deny { approval_id } => self.decide(&approval_id, &actor, false),
            Command::SendMessage { mission_id, to, body } => self.send_message(&mission_id, to.as_deref(), &body, &actor),
            Command::TakeSteering { mission_agent_id } => self.take_steering(&mission_agent_id, &actor),
            Command::ReleaseSteering { mission_agent_id } => self.release_steering(&mission_agent_id, &actor),
            Command::Pause { mission_agent_id } => self.pause(&mission_agent_id, true),
            Command::Resume { mission_agent_id } => self.pause(&mission_agent_id, false),
            Command::Merge { result_id } => self.merge(&result_id, &actor).await,
            Command::ConnectAgent { agent_id } => self.check_agent(&agent_id, true).await,
            Command::TestAgent { agent_id } => self.check_agent(&agent_id, false).await,
        };
        if let Err(e) = &result {
            tracing::warn!(op = %op.id, error = %e, "command failed");
        }
        self.emit(vec![OpBody::CommandResult { command_op: op.id.clone(), ok: result.is_ok(), error: result.err() }]);
        if let Ok(mut ex) = self.inner.executing.lock() {
            ex.remove(&op.id);
        }
    }

    // ------------------------------------------------------------------------------------- approvals

    fn decide(&self, approval_id: &str, actor: &str, approve: bool) -> CmdResult {
        let (approval, allowed, tasks, other_pending) = read(self.host(), |s| {
            let a = s.approvals.get(approval_id).cloned();
            let allowed = a.as_ref().is_some_and(|a| {
                let policy = s
                    .mission_agents
                    .get(&a.mission_agent_id)
                    .and_then(|ma| s.agents.get(&ma.agent_id))
                    .map(|ag| ag.command_policy);
                may_decide(s, &a.mission_id, actor, policy)
            });
            let tasks: Vec<String> =
                s.tasks.values().filter(|t| t.approval_id.as_deref() == Some(approval_id)).map(|t| t.id.clone()).collect();
            let other = a.as_ref().is_some_and(|a| {
                s.approvals.values().any(|o| o.mission_id == a.mission_id && o.id != a.id && o.state == ApprovalState::Pending)
            });
            (a, allowed, tasks, other)
        });
        let approval = approval.ok_or_else(|| format!("unknown approval {approval_id}"))?;
        if approval.state != ApprovalState::Pending {
            return Err(format!("approval {approval_id} was already decided"));
        }
        if !allowed {
            return Err(format!("{actor} may not decide approvals in mission {}", approval.mission_id));
        }
        let mut ops = vec![
            OpBody::patch(
                EntityKind::Approval,
                approval_id,
                [
                    ("state", json!(if approve { "approved" } else { "denied" })),
                    ("decidedBy", json!(actor)),
                    ("decidedAt", json!(now_rfc3339())),
                ],
            ),
            OpBody::patch(
                EntityKind::MissionAgent,
                &approval.mission_agent_id,
                [("status", json!(if approve { "working" } else { "idle" }))],
            ),
        ];
        for t in tasks {
            let fields: Vec<(&str, Value)> = if approve {
                vec![("column", json!("working"))]
            } else {
                vec![("column", json!("done")), ("note", json!(format!("Denied by {actor}")))]
            };
            ops.push(OpBody::patch(EntityKind::Task, t, fields));
        }
        if !other_pending {
            ops.push(OpBody::patch(EntityKind::Mission, &approval.mission_id, [("status", json!("working"))]));
        }
        self.emit(ops);
        if let Some(tx) = self.inner.approvals.lock().ok().and_then(|mut a| a.remove(approval_id)) {
            let _ = tx.send(approve);
        }
        Ok(())
    }

    async fn request_approval(&self, mission_id: &str, ma_id: &str, req: PermissionRequest) -> bool {
        self.flush(ma_id, true);
        let approval_id = format!("ap-{}", ulid_suffix());
        let (tx, rx) = oneshot::channel();
        if let Ok(mut a) = self.inner.approvals.lock() {
            a.insert(approval_id.clone(), tx);
        }
        let task_id = self.inner.task_of.lock().ok().and_then(|t| t.get(ma_id).cloned());
        let role = read(self.host(), |s| s.mission_agents.get(ma_id).map(|m| m.role)).unwrap_or(Role::Builder);
        let mut ops = vec![
            OpBody::put(Approval {
                id: approval_id.clone(),
                mission_id: mission_id.into(),
                mission_agent_id: ma_id.into(),
                prompt: req.prompt.clone(),
                command: req.command.clone(),
                state: ApprovalState::Pending,
                decided_by: None,
                decided_at: None,
            }),
            OpBody::patch(EntityKind::MissionAgent, ma_id, [("status", json!("needsYou"))]),
            OpBody::put(Message {
                id: format!("msg-{}", ulid_suffix()),
                mission_id: mission_id.into(),
                author: Author::Agent { mission_agent_id: ma_id.into() },
                steering: None,
                to: None,
                body: req.prompt.clone(),
                plan: vec![],
                log: vec![],
                approval_id: Some(approval_id.clone()),
                at: now_rfc3339(),
            }),
            OpBody::patch(EntityKind::Mission, mission_id, [("status", json!("needsYou"))]),
        ];
        match task_id {
            Some(t) => ops.push(OpBody::patch(
                EntityKind::Task,
                t,
                [("column", json!("needsYou")), ("approvalId", json!(approval_id))],
            )),
            None => ops.push(OpBody::put(Task {
                id: format!("t-{}", ulid_suffix()),
                mission_id: mission_id.into(),
                title: format!("Approve: {}", req.prompt),
                role,
                mission_agent_id: Some(ma_id.into()),
                column: TaskColumn::NeedsYou,
                note: None,
                merged_by: None,
                approval_id: Some(approval_id.clone()),
                order: 0,
            })),
        }
        self.emit(ops);
        rx.await.unwrap_or(false)
    }

    // -------------------------------------------------------------------------------------- steering

    fn send_message(&self, mission_id: &str, to: Option<&str>, body: &str, actor: &str) -> CmdResult {
        let (exists, steerable, target) = read(self.host(), |s| {
            (
                s.missions.contains_key(mission_id),
                may_steer(s, mission_id, actor),
                to.map(|t| s.mission_agents.get(t).cloned()),
            )
        });
        if !exists {
            return Err(format!("unknown mission {mission_id}"));
        }
        if !steerable {
            return Err(format!("{actor} may not steer mission {mission_id}"));
        }
        match to {
            Some(ma_id) => {
                let ma = target.flatten().filter(|m| m.mission_id == mission_id).ok_or_else(|| format!("unknown agent {ma_id}"))?;
                match ma.steered_by.as_deref() {
                    Some(holder) if holder != actor => return Err(format!("steering held by {holder}")),
                    Some(_) => {}
                    None => self.emit(vec![OpBody::patch(EntityKind::MissionAgent, ma_id, [("steeredBy", json!(actor))])]),
                }
                if !self.deliver(ma_id, body) {
                    return Err(format!("agent {ma_id} is not running"));
                }
            }
            None => {
                let ids: Vec<String> = self
                    .inner
                    .agents
                    .lock()
                    .map(|a| a.iter().filter(|(_, l)| l.mission_id == mission_id).map(|(k, _)| k.clone()).collect())
                    .unwrap_or_default();
                for id in ids {
                    self.deliver(&id, body);
                }
            }
        }
        Ok(())
    }

    fn deliver(&self, ma_id: &str, body: &str) -> bool {
        let sent = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|a| a.get(ma_id).map(|l| l.control.send(AgentControl::Message(body.to_string())).is_ok()))
            .unwrap_or(false);
        if sent {
            self.flush(ma_id, true);
            self.emit(vec![OpBody::patch(EntityKind::MissionAgent, ma_id, [("status", json!("working"))])]);
        }
        sent
    }

    fn take_steering(&self, ma_id: &str, actor: &str) -> CmdResult {
        let (ma, ok) = read(self.host(), |s| {
            let ma = s.mission_agents.get(ma_id).cloned();
            let ok = ma.as_ref().is_some_and(|m| may_steer(s, &m.mission_id, actor));
            (ma, ok)
        });
        ma.ok_or_else(|| format!("unknown agent {ma_id}"))?;
        if !ok {
            return Err(format!("{actor} may not steer this mission"));
        }
        self.emit(vec![OpBody::patch(EntityKind::MissionAgent, ma_id, [("steeredBy", json!(actor))])]);
        Ok(())
    }

    fn release_steering(&self, ma_id: &str, actor: &str) -> CmdResult {
        let ma = read(self.host(), |s| s.mission_agents.get(ma_id).cloned()).ok_or_else(|| format!("unknown agent {ma_id}"))?;
        match ma.steered_by.as_deref() {
            Some(h) if h == actor => {
                self.emit(vec![OpBody::patch(EntityKind::MissionAgent, ma_id, [("steeredBy", Value::Null)])]);
                Ok(())
            }
            Some(h) => Err(format!("steering held by {h}")),
            None => Ok(()),
        }
    }

    fn pause(&self, ma_id: &str, pause: bool) -> CmdResult {
        let sent = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|a| {
                a.get(ma_id).map(|l| l.control.send(if pause { AgentControl::Pause } else { AgentControl::Resume }).is_ok())
            })
            .unwrap_or(false);
        if !sent {
            return Err(format!("agent {ma_id} is not running"));
        }
        let fields = if pause {
            vec![("status", json!("idle")), ("statusDetail", json!("paused"))]
        } else {
            vec![("status", json!("working")), ("statusDetail", Value::Null)]
        };
        self.emit(vec![OpBody::patch(EntityKind::MissionAgent, ma_id, fields)]);
        Ok(())
    }

    // ---------------------------------------------------------------------------------------- agents

    async fn check_agent(&self, agent_id: &str, connect: bool) -> CmdResult {
        let agent = read(self.host(), |s| s.agents.get(agent_id).cloned()).ok_or_else(|| format!("unknown agent {agent_id}"))?;
        let check: Result<(), String> = match &agent.adapter {
            None => Err("no adapter configured".into()),
            Some(Adapter::Acp { command, .. }) | Some(Adapter::Command { command, .. }) => {
                which::which(command).map(|_| ()).map_err(|_| format!("`{command}` was not found on PATH"))
            }
            Some(Adapter::Langgraph { endpoint, .. }) => {
                let url = format!("{}/ok", endpoint.trim_end_matches('/'));
                match reqwest::Client::builder().timeout(Duration::from_secs(3)).build() {
                    Ok(http) => http.get(url).send().await.map(|_| ()).map_err(|_| format!("{endpoint} is not reachable")),
                    Err(e) => Err(e.to_string()),
                }
            }
            Some(Adapter::Builtin { provider, model }) => {
                let b = adapters::builtin::BuiltinAdapter { policy: *provider, model: model.clone(), router_override: None };
                astra_llm::Router::new(b.router_config(self.inner.config.secrets.as_ref()))
                    .select()
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };
        let status = if check.is_ok() { "connected" } else { "notConnected" };
        let mut fields = vec![("status", json!(status))];
        if connect && check.is_ok() {
            fields.push(("nodeId", json!(self.host().node_id())));
        }
        self.emit(vec![OpBody::patch(EntityKind::Agent, agent_id, fields)]);
        check
    }

    fn logs_for(&self, ma_id: &str) -> LogRing {
        self.inner
            .logs
            .lock()
            .map(|mut l| l.entry(ma_id.to_string()).or_insert_with(|| LogRing::new(500)).clone())
            .unwrap_or_else(|_| LogRing::new(500))
    }

    /// Start a session for a mission agent (or reuse the live one).
    fn ensure_session(&self, ma: &MissionAgent, prompt: &str, worktree: &std::path::Path) -> Result<bool, String> {
        if self.inner.agents.lock().map(|a| a.contains_key(&ma.id)).unwrap_or(false) {
            return Ok(false);
        }
        let (adapter, policy) = read(self.host(), |s| {
            s.agents.get(&ma.agent_id).map(|a| (a.adapter.clone(), a.command_policy))
        })
        .ok_or_else(|| format!("unknown agent {}", ma.agent_id))?;
        let adapter = adapter.ok_or_else(|| format!("agent {} has no adapter configured", ma.agent_id))?;
        let approver = Arc::new(MissionApprover { orch: self.clone(), mission_id: ma.mission_id.clone(), ma_id: ma.id.clone() });
        let ctx = AgentContext {
            mission_agent_id: ma.id.clone(),
            role: ma.role,
            prompt: prompt.to_string(),
            worktree: worktree.to_path_buf(),
            command_policy: policy,
            approver,
            secrets: self.inner.config.secrets.clone(),
            logs: self.logs_for(&ma.id),
        };
        let session = (self.inner.config.adapters)(&adapter).start(ctx).map_err(|e| e.to_string())?;
        let (turn_tx, turn_rx) = watch::channel(TurnState::default());
        if let Ok(mut a) = self.inner.agents.lock() {
            a.insert(
                ma.id.clone(),
                LiveAgent { mission_id: ma.mission_id.clone(), control: session.control, turns: turn_rx },
            );
        }
        let o = self.clone();
        let ma_id = ma.id.clone();
        let mission_id = ma.mission_id.clone();
        let mut events = session.events;
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                o.on_agent_event(&mission_id, &ma_id, ev, &turn_tx);
            }
            turn_tx.send_modify(|t| t.exited = true);
            if let Ok(mut a) = o.inner.agents.lock() {
                a.remove(&ma_id);
            }
        });
        Ok(true)
    }

    fn on_agent_event(&self, mission_id: &str, ma_id: &str, ev: AgentEvent, turns: &watch::Sender<TurnState>) {
        let mut flush_now = false;
        let mut extra = Vec::new();
        {
            let Ok(mut streams) = self.inner.streams.lock() else { return };
            let st = streams.entry(ma_id.to_string()).or_default();
            match ev {
                AgentEvent::Text(t) => {
                    st.body.push_str(&t);
                    st.turn_text.push_str(&t);
                    st.dirty = true;
                }
                AgentEvent::Plan(steps) => {
                    st.plan = steps.clone();
                    st.last_plan = steps;
                    st.dirty = true;
                    flush_now = true;
                }
                AgentEvent::Log(line) => {
                    st.log.push(line.clone());
                    st.latest.push_back(line);
                    while st.latest.len() > 3 {
                        st.latest.pop_front();
                    }
                    st.dirty = true;
                    flush_now = true;
                    extra.push(OpBody::patch(
                        EntityKind::MissionAgent,
                        ma_id,
                        [("latest", json!(st.latest.iter().cloned().collect::<Vec<_>>()))],
                    ));
                }
                AgentEvent::TurnDone => {
                    drop(streams);
                    self.flush(ma_id, true);
                    turns.send_modify(|t| {
                        t.seq += 1;
                        t.error = None;
                    });
                    return;
                }
                AgentEvent::Error(e) => {
                    st.body.push_str(&format!("{}Error: {e}", if st.body.is_empty() { "" } else { "\n\n" }));
                    st.dirty = true;
                    drop(streams);
                    self.flush(ma_id, true);
                    self.emit(vec![OpBody::patch(
                        EntityKind::MissionAgent,
                        ma_id,
                        [("status", json!("idle")), ("statusDetail", json!(format!("error: {e}")))],
                    )]);
                    turns.send_modify(|t| {
                        t.seq += 1;
                        t.error = Some(e);
                    });
                    return;
                }
                AgentEvent::Exited => {}
            }
        }
        let _ = mission_id;
        self.emit(extra);
        if flush_now {
            self.flush(ma_id, false);
        }
    }

    /// Write the agent's in-progress message (put on first write, then patches). `close` ends it.
    fn flush(&self, ma_id: &str, close: bool) {
        let ops = {
            let Ok(mut streams) = self.inner.streams.lock() else { return };
            let Some(st) = streams.get_mut(ma_id) else { return };
            let mut ops = Vec::new();
            if st.dirty {
                st.dirty = false;
                match &st.message_id {
                    Some(id) => ops.push(OpBody::patch(
                        EntityKind::Message,
                        id,
                        [("body", json!(st.body)), ("plan", json!(st.plan)), ("log", json!(st.log))],
                    )),
                    None => {
                        let mission_id = read(self.host(), |s| s.mission_agents.get(ma_id).map(|m| m.mission_id.clone()))
                            .unwrap_or_default();
                        let id = format!("msg-{}", ulid_suffix());
                        ops.push(OpBody::put(Message {
                            id: id.clone(),
                            mission_id,
                            author: Author::Agent { mission_agent_id: ma_id.into() },
                            steering: None,
                            to: None,
                            body: st.body.clone(),
                            plan: st.plan.clone(),
                            log: st.log.clone(),
                            approval_id: None,
                            at: now_rfc3339(),
                        }));
                        st.message_id = Some(id);
                    }
                }
            }
            if close {
                st.message_id = None;
                st.body.clear();
                st.plan.clear();
                st.log.clear();
            }
            ops
        };
        self.emit(ops);
    }

    fn take_turn_text(&self, ma_id: &str) -> (String, Vec<PlanStep>) {
        self.inner
            .streams
            .lock()
            .ok()
            .and_then(|mut s| s.get_mut(ma_id).map(|st| (std::mem::take(&mut st.turn_text), st.last_plan.clone())))
            .unwrap_or_default()
    }

    /// Run one prompt turn of a mission agent in `worktree` and wait for it to finish.
    async fn run_turn(&self, ma: &MissionAgent, prompt: &str, worktree: &std::path::Path, task_id: Option<&str>) -> Result<String, String> {
        if let Some(t) = task_id {
            if let Ok(mut m) = self.inner.task_of.lock() {
                m.insert(ma.id.clone(), t.to_string());
            }
            self.emit(vec![OpBody::patch(EntityKind::Task, t, [("column", json!("working"))])]);
        }
        let mut fields = vec![("status", json!("working")), ("statusDetail", Value::Null)];
        if ma.started_at.is_none() {
            fields.push(("startedAt", json!(now_rfc3339())));
        }
        self.emit(vec![OpBody::patch(EntityKind::MissionAgent, &ma.id, fields)]);
        let _ = self.take_turn_text(&ma.id);
        let fresh = self.ensure_session(ma, prompt, worktree)?;
        let mut rx = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|a| a.get(&ma.id).map(|l| l.turns.clone()))
            .ok_or_else(|| "agent session ended".to_string())?;
        let before = rx.borrow().seq;
        if !fresh && !self.deliver(&ma.id, prompt) {
            return Err("agent session ended".into());
        }
        let outcome = loop {
            {
                let t = rx.borrow();
                if t.seq > before {
                    break match &t.error {
                        Some(e) => Err(e.clone()),
                        None => Ok(()),
                    };
                }
                if t.exited {
                    break Err("agent exited before finishing".to_string());
                }
            }
            if rx.changed().await.is_err() {
                break Err("agent exited before finishing".to_string());
            }
        };
        let (text, _) = self.take_turn_text(&ma.id);
        match &outcome {
            Ok(()) => {
                let mut ops = vec![OpBody::patch(EntityKind::MissionAgent, &ma.id, [("status", json!("done"))])];
                if let Some(t) = task_id {
                    ops.push(OpBody::patch(EntityKind::Task, t, [("column", json!("done"))]));
                }
                self.emit(ops);
            }
            Err(e) => {
                if let Some(t) = task_id {
                    self.emit(vec![OpBody::patch(EntityKind::Task, t, [("note", json!(format!("Failed: {e}")))])]);
                }
            }
        }
        outcome.map(|_| text)
    }

    // -------------------------------------------------------------------------------------- missions

    fn mission_context(&self, mission_id: &str) -> Result<(Mission, Repository, Vec<MissionAgent>, PathBuf), String> {
        let (mission, repo, agents, adapters_ok) = read(self.host(), |s| {
            let m = s.missions.get(mission_id).cloned();
            let r = m.as_ref().and_then(|m| s.repositories.get(&m.repo_id).cloned());
            let mut a: Vec<MissionAgent> =
                s.mission_agents.values().filter(|ma| ma.mission_id == mission_id && ma.enabled).cloned().collect();
            a.sort_by_key(|ma| (role_rank(ma.role), ma.id.clone()));
            let missing: Vec<String> = a
                .iter()
                .filter(|ma| s.agents.get(&ma.agent_id).and_then(|ag| ag.adapter.as_ref()).is_none())
                .map(|ma| ma.agent_id.clone())
                .collect();
            (m, r, a, missing)
        });
        let mission = mission.ok_or_else(|| format!("unknown mission {mission_id}"))?;
        let repo = repo.ok_or_else(|| format!("unknown repository {}", mission.repo_id))?;
        let path = repo.path.as_deref().map(git::expand_home).ok_or_else(|| format!("repository {} has no local path", repo.name))?;
        if !adapters_ok.is_empty() {
            return Err(format!("agents without an adapter: {}", adapters_ok.join(", ")));
        }
        if agents.is_empty() {
            return Err("the mission has no enabled agents".into());
        }
        git2_check(&path, &mission.base_branch)?;
        Ok((mission, repo, agents, path))
    }

    fn worktree_dir(&self, mission_id: &str, name: &str) -> PathBuf {
        self.inner.config.data_dir.join("worktrees").join(sanitize(mission_id)).join(sanitize(name))
    }

    fn start_mission(&self, mission_id: &str) -> CmdResult {
        let (mission, repo, agents, path) = self.mission_context(mission_id)?;
        if !matches!(mission.status, MissionStatus::Draft | MissionStatus::NeedsYou | MissionStatus::Working) {
            return Err(format!("mission {mission_id} is {:?}", mission.status));
        }
        if mission.mode == MissionMode::Race && !agents.iter().any(|a| a.role == Role::Builder) {
            return Err("a race needs at least one builder".into());
        }
        self.emit(vec![OpBody::patch(
            EntityKind::Mission,
            mission_id,
            [("status", json!("working")), ("nodeId", json!(self.host().node_id()))],
        )]);
        let o = self.clone();
        tokio::spawn(async move {
            let r = match mission.mode {
                MissionMode::Split => o.run_split(&mission, &repo, &agents, &path).await,
                MissionMode::Race => o.run_race(&mission, &repo, &agents, &path).await,
            };
            if let Err(e) = r {
                tracing::warn!(mission = %mission.id, error = %e, "mission failed");
                let author = agents.first().map(|a| a.id.clone()).unwrap_or_default();
                o.emit(vec![
                    OpBody::put(Message {
                        id: format!("msg-{}", ulid_suffix()),
                        mission_id: mission.id.clone(),
                        author: Author::Agent { mission_agent_id: author },
                        steering: None,
                        to: None,
                        body: format!("The mission stopped: {e}"),
                        plan: vec![],
                        log: vec![],
                        approval_id: None,
                        at: now_rfc3339(),
                    }),
                    OpBody::patch(EntityKind::Mission, &mission.id, [("status", json!("needsYou"))]),
                ]);
            }
        });
        Ok(())
    }

    fn new_task(&self, mission: &Mission, ma: Option<&MissionAgent>, role: Role, title: String, column: TaskColumn, order: i64) -> String {
        let id = format!("t-{}", ulid_suffix());
        self.emit(vec![OpBody::put(Task {
            id: id.clone(),
            mission_id: mission.id.clone(),
            title,
            role,
            mission_agent_id: ma.map(|m| m.id.clone()),
            column,
            note: None,
            merged_by: None,
            approval_id: None,
            order,
        })]);
        id
    }

    async fn run_split(&self, mission: &Mission, _repo: &Repository, agents: &[MissionAgent], path: &std::path::Path) -> CmdResult {
        let wt_dir = self.worktree_dir(&mission.id, "main");
        let (p, base, branch, d) = (path.to_path_buf(), mission.base_branch.clone(), mission.branch.clone(), wt_dir.clone());
        let worktree = blocking(move || git::create_worktree(&p, &base, &branch, &d).map_err(|e| e.to_string())).await?;
        self.emit(
            agents
                .iter()
                .map(|a| OpBody::patch(EntityKind::MissionAgent, &a.id, [("branch", json!(mission.branch))]))
                .collect(),
        );

        let mut plan_text = String::new();
        let mut order = 1;
        if let Some(planner) = agents.iter().find(|a| a.role == Role::Planner) {
            let title = planner.task.clone().unwrap_or_else(|| format!("Plan: {}", mission.title));
            let task = self.new_task(mission, Some(planner), Role::Planner, title, TaskColumn::Queued, order);
            order += 1;
            let text = self.run_turn(planner, &prompt_for(Role::Planner, mission, ""), &worktree, Some(&task)).await?;
            let (_, plan) = self.take_turn_text(&planner.id);
            plan_text = render_plan(&plan, &text);
        }
        let workers: Vec<&MissionAgent> = agents.iter().filter(|a| a.role != Role::Planner).collect();
        let mut queue = Vec::new();
        for ma in &workers {
            let title = ma.task.clone().unwrap_or_else(|| default_task_title(ma.role, &mission.title));
            let task = self.new_task(mission, Some(ma), ma.role, title, TaskColumn::Queued, order);
            order += 1;
            queue.push(((*ma).clone(), task));
        }
        let mut result_agent = agents.first().map(|a| a.id.clone()).unwrap_or_default();
        for (ma, task) in queue {
            if ma.role == Role::Reviewer {
                // Commit first so the reviewer sees the full diff.
                let (wt, msg) = (worktree.clone(), format!("astra: {}", mission.title));
                blocking(move || git::commit_all(&wt, &msg).map_err(|e| e.to_string())).await?;
            } else if ma.role == Role::Builder {
                result_agent = ma.id.clone();
            }
            self.run_turn(&ma, &prompt_for(ma.role, mission, &plan_text), &worktree, Some(&task)).await?;
        }

        let (wt, msg) = (worktree.clone(), format!("astra: {}", mission.title));
        blocking(move || git::commit_all(&wt, &msg).map_err(|e| e.to_string())).await?;
        let (p, base, branch) = (path.to_path_buf(), mission.base_branch.clone(), mission.branch.clone());
        let (stats, diff) = blocking(move || git::diff_branch(&p, &base, &branch).map_err(|e| e.to_string())).await?;
        self.emit(vec![
            OpBody::put(MissionResult {
                id: format!("res-{}", mission.id),
                mission_id: mission.id.clone(),
                label: "A".into(),
                mission_agent_id: result_agent,
                branch: mission.branch.clone(),
                stats,
                checks: Checks::None,
                reviewer_pick: false,
                diff,
            }),
            OpBody::patch(EntityKind::Mission, &mission.id, [("stats", json!(stats)), ("status", json!("needsYou"))]),
        ]);
        self.new_task(mission, None, Role::Reviewer, format!("Review and merge {}", mission.short_name), TaskColumn::NeedsYou, order);
        Ok(())
    }

    async fn run_race(&self, mission: &Mission, _repo: &Repository, agents: &[MissionAgent], path: &std::path::Path) -> CmdResult {
        let builders: Vec<&MissionAgent> = agents.iter().filter(|a| a.role == Role::Builder).collect();
        let mut runs = Vec::new();
        let mut entries = Vec::new();
        for (i, ma) in builders.iter().enumerate() {
            let label = label_for(i);
            let branch = format!("{}-{}", mission.branch, label.to_lowercase());
            let dir = self.worktree_dir(&mission.id, &label);
            let (p, base, b, d) = (path.to_path_buf(), mission.base_branch.clone(), branch.clone(), dir);
            let wt = blocking(move || git::create_worktree(&p, &base, &b, &d).map_err(|e| e.to_string())).await?;
            self.emit(vec![OpBody::patch(EntityKind::MissionAgent, &ma.id, [("branch", json!(branch))])]);
            let title = ma.task.clone().unwrap_or_else(|| mission.title.clone());
            let task = self.new_task(mission, Some(ma), Role::Builder, title, TaskColumn::Queued, i as i64 + 1);
            entries.push(((*ma).clone(), label, branch, wt.clone()));
            let o = self.clone();
            let ma = (*ma).clone();
            let prompt = prompt_for(Role::Builder, mission, "");
            runs.push(async move { o.run_turn(&ma, &prompt, &wt, Some(&task)).await });
        }
        let outcomes = futures::future::join_all(runs).await;

        let mut results = Vec::new();
        for ((ma, label, branch, wt), outcome) in entries.into_iter().zip(outcomes) {
            if let Err(e) = outcome {
                tracing::warn!(agent = %ma.id, error = %e, "race builder failed");
                continue;
            }
            let msg = format!("astra: {} ({label})", mission.title);
            let w = wt.clone();
            blocking(move || git::commit_all(&w, &msg).map_err(|e| e.to_string())).await?;
            let (p, base, b) = (path.to_path_buf(), mission.base_branch.clone(), branch.clone());
            let (stats, diff) = blocking(move || git::diff_branch(&p, &base, &b).map_err(|e| e.to_string())).await?;
            let r = MissionResult {
                id: format!("res-{}-{}", mission.id, label.to_lowercase()),
                mission_id: mission.id.clone(),
                label,
                mission_agent_id: ma.id.clone(),
                branch,
                stats,
                checks: Checks::None,
                reviewer_pick: false,
                diff,
            };
            self.emit(vec![OpBody::put(r.clone())]);
            results.push(r);
        }
        if results.is_empty() {
            return Err("every builder failed".into());
        }

        if let Some(reviewer) = agents.iter().find(|a| a.role == Role::Reviewer) {
            let dir = self.worktree_dir(&mission.id, "review");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let title = reviewer.task.clone().unwrap_or_else(|| format!("Compare the {} results", results.len()));
            let task = self.new_task(mission, Some(reviewer), Role::Reviewer, title, TaskColumn::Queued, results.len() as i64 + 1);
            let prompt = review_prompt(mission, &results);
            match self.run_turn(reviewer, &prompt, &dir, Some(&task)).await {
                Ok(text) => {
                    let labels: Vec<String> = results.iter().map(|r| r.label.clone()).collect();
                    if let Some(pick) = parse_pick(&text, &labels) {
                        if let Some(r) = results.iter().find(|r| r.label == pick) {
                            self.emit(vec![OpBody::patch(EntityKind::Result, &r.id, [("reviewerPick", json!(true))])]);
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "reviewer failed"),
            }
        }
        self.new_task(
            mission,
            None,
            Role::Reviewer,
            format!("Pick one of {} results", results.len()),
            TaskColumn::NeedsYou,
            results.len() as i64 + 2,
        );
        self.emit(vec![OpBody::patch(EntityKind::Mission, &mission.id, [("status", json!("needsYou"))])]);
        Ok(())
    }

    async fn merge(&self, result_id: &str, actor: &str) -> CmdResult {
        let (result, mission, repo, others, tasks, allowed) = read(self.host(), |s| {
            let r = s.results.get(result_id).cloned();
            let m = r.as_ref().and_then(|r| s.missions.get(&r.mission_id).cloned());
            let repo = m.as_ref().and_then(|m| s.repositories.get(&m.repo_id).cloned());
            let others: Vec<String> = r
                .as_ref()
                .map(|r| {
                    s.results.values().filter(|o| o.mission_id == r.mission_id && o.id != r.id).map(|o| o.branch.clone()).collect()
                })
                .unwrap_or_default();
            let tasks: Vec<String> = r
                .as_ref()
                .map(|r| {
                    s.tasks
                        .values()
                        .filter(|t| t.mission_id == r.mission_id && t.column == TaskColumn::NeedsYou && t.approval_id.is_none())
                        .map(|t| t.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            let allowed = r.as_ref().is_some_and(|r| may_decide(s, &r.mission_id, actor, None));
            (r, m, repo, others, tasks, allowed)
        });
        let result = result.ok_or_else(|| format!("unknown result {result_id}"))?;
        let mission = mission.ok_or_else(|| format!("unknown mission {}", result.mission_id))?;
        let repo = repo.ok_or_else(|| format!("unknown repository {}", mission.repo_id))?;
        if !allowed {
            return Err(format!("{actor} may not merge in mission {}", mission.id));
        }
        if mission.status == MissionStatus::Merged {
            return Err(format!("mission {} is already merged", mission.id));
        }
        let path = repo.path.as_deref().map(git::expand_home).ok_or_else(|| format!("repository {} has no local path", repo.name))?;

        // Stop the mission's agents and remove its worktrees (branches are kept).
        let live: Vec<String> = self
            .inner
            .agents
            .lock()
            .map(|a| a.iter().filter(|(_, l)| l.mission_id == mission.id).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default();
        for id in live {
            if let Some(l) = self.inner.agents.lock().ok().and_then(|mut a| a.remove(&id)) {
                let _ = l.control.send(AgentControl::Cancel);
            }
        }
        let message = format!("Merge {} (mission {}: {})", result.branch, mission.number, mission.title);
        let (p, b, into) = (path.clone(), result.branch.clone(), mission.base_branch.clone());
        blocking(move || git::merge_branch(&p, &b, &into, &message).map_err(|e| e.to_string())).await?;
        let wt_root = self.inner.config.data_dir.join("worktrees").join(sanitize(&mission.id));
        let p = path.clone();
        let _ = blocking(move || -> Result<(), String> {
            if let Ok(rd) = std::fs::read_dir(&wt_root) {
                for e in rd.flatten() {
                    let _ = git::remove_worktree(&p, &e.path());
                }
            }
            let _ = std::fs::remove_dir_all(&wt_root);
            Ok(())
        })
        .await;

        let mut ops = vec![
            OpBody::put(Merge {
                id: mission.id.clone(),
                mission_id: mission.id.clone(),
                result_id: Some(result.id.clone()),
                into: format!("{} · {}", repo.name, mission.base_branch),
                stats: result.stats,
                checks: result.checks,
                approved_by: actor.into(),
                note: None,
                kept_branches: others,
                at: now_rfc3339(),
            }),
            OpBody::patch(EntityKind::Mission, &mission.id, [("status", json!("merged")), ("stats", json!(result.stats))]),
        ];
        for t in tasks {
            ops.push(OpBody::patch(EntityKind::Task, t, [("column", json!("done")), ("mergedBy", json!(actor))]));
        }
        self.emit(ops);
        Ok(())
    }
}

/// The approver given to each agent session.
struct MissionApprover {
    orch: Orchestrator,
    mission_id: String,
    ma_id: String,
}

#[async_trait::async_trait]
impl Approver for MissionApprover {
    async fn request(&self, req: PermissionRequest) -> bool {
        self.orch.request_approval(&self.mission_id, &self.ma_id, req).await
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn git2_check(path: &std::path::Path, base: &str) -> Result<(), String> {
    if !git::branch_exists(path, base) {
        return Err(format!("{} is not a git repository with branch {base}", path.display()));
    }
    Ok(())
}

fn role_rank(r: Role) -> u8 {
    match r {
        Role::Planner => 0,
        Role::Builder => 1,
        Role::Tester => 2,
        Role::Writer => 3,
        Role::Reviewer => 4,
    }
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// `A`, `B`, ... `Z`, `AA`, ...
pub fn label_for(i: usize) -> String {
    let mut n = i;
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}

fn default_task_title(role: Role, title: &str) -> String {
    match role {
        Role::Planner => format!("Plan: {title}"),
        Role::Builder => title.to_string(),
        Role::Tester => format!("Test: {title}"),
        Role::Reviewer => format!("Review: {title}"),
        Role::Writer => format!("Document: {title}"),
    }
}

fn render_plan(plan: &[PlanStep], text: &str) -> String {
    if plan.is_empty() {
        return text.trim().to_string();
    }
    plan.iter().enumerate().map(|(i, p)| format!("{}. {}", i + 1, p.text)).collect::<Vec<_>>().join("\n")
}

/// The prompt for a role.
pub fn prompt_for(role: Role, mission: &Mission, plan: &str) -> String {
    let plan = if plan.is_empty() { String::new() } else { format!("\n\nThe plan:\n{plan}") };
    match role {
        Role::Planner => format!(
            "Goal: {}\n\nYou are the planner. Read the code and write a short, numbered plan that the builder, tester \
             and reviewer will follow. Do not change files.",
            mission.goal
        ),
        Role::Builder => format!("Goal: {}{plan}\n\nYou are the builder. Make the change in this worktree.", mission.goal),
        Role::Tester => format!("Goal: {}{plan}\n\nYou are the tester. Add or run tests for the change and fix what fails.", mission.goal),
        Role::Writer => format!("Goal: {}{plan}\n\nYou are the writer. Update the documentation for the change.", mission.goal),
        Role::Reviewer => format!(
            "Goal: {}{plan}\n\nYou are the reviewer. Review the change on this branch and point out problems.",
            mission.goal
        ),
    }
}

fn review_prompt(mission: &Mission, results: &[MissionResult]) -> String {
    let mut s = format!(
        "Goal: {}\n\nSeveral agents raced on this goal. Compare their results and say which one to merge. \
         Answer with a line `Pick: <label>` and a short reason.\n",
        mission.goal
    );
    for r in results {
        s.push_str(&format!(
            "\n## Result {} (branch {}, {} files, +{} -{})\n",
            r.label, r.branch, r.stats.files, r.stats.additions, r.stats.deletions
        ));
        for f in &r.diff {
            s.push_str(&format!("--- {}\n", f.path));
            for l in f.lines.iter().take(400) {
                let p = match l.kind {
                    DiffLineKind::Add => '+',
                    DiffLineKind::Del => '-',
                    DiffLineKind::Ctx => ' ',
                };
                s.push(p);
                s.push_str(&l.text);
                s.push('\n');
            }
        }
    }
    s
}

/// Find the reviewer's pick (`Pick: B`, `merge A`, ...) among `labels`.
pub fn parse_pick(text: &str, labels: &[String]) -> Option<String> {
    let alts = labels.iter().map(|l| regex::escape(l)).collect::<Vec<_>>().join("|");
    let strong = regex::Regex::new(&format!(
        r"(?i)\b(?:pick|merge|choose|prefer|recommend|go with)\b[^A-Za-z0-9]*(?:result\s+)?\b({alts})\b"
    ))
    .ok()?;
    if let Some(c) = strong.captures(text) {
        let found = c.get(1)?.as_str();
        return labels.iter().find(|l| l.eq_ignore_ascii_case(found)).cloned();
    }
    let weak = regex::Regex::new(&format!(r"\b(?:result\s+)?({alts})\b")).ok()?;
    weak.captures(text).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_picks() {
        assert_eq!(label_for(0), "A");
        assert_eq!(label_for(1), "B");
        assert_eq!(label_for(26), "AA");
        let labels = vec!["A".to_string(), "B".to_string()];
        assert_eq!(parse_pick("Pick: B\nIt is smaller.", &labels).as_deref(), Some("B"));
        assert_eq!(parse_pick("I would merge A now and keep B as a follow-up.", &labels).as_deref(), Some("A"));
        assert_eq!(parse_pick("Result B is cleaner.", &labels).as_deref(), Some("B"));
        assert_eq!(parse_pick("no idea", &labels), None);
    }
}
