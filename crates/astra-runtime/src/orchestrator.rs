//! Executes protocol commands addressed to this runner node. Every effect is expressed as ops
//! (`put`/`patch` + `commandResult`) emitted through the [`NodeHost`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_llm::{ChatMessage, Provider, Router, StopReason, TurnRequest};
use astra_protocol::*;
use futures::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};

use crate::adapters::{
    self, AgentAdapter, AgentContext, AgentControl, AgentEvent, Approver, PermissionRequest,
};
use crate::git;
use crate::policy::{may_decide, may_manage, may_steer};
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

fn read<R>(host: &dyn NodeHost, f: impl Fn(&State) -> R) -> R {
    let mut out = None;
    host.with_state(&mut |s| out = Some(f(s)));
    // A host that does not call back is treated as having an empty state.
    out.unwrap_or_else(|| f(&State::default()))
}

/// Builds adapters from protocol adapter configs (override in tests).
pub type AdapterFactory = Arc<dyn Fn(&Adapter) -> Box<dyn AgentAdapter> + Send + Sync>;

/// Picks the model provider that answers side questions (override in tests).
pub type SideProviderFactory = Arc<
    dyn Fn(Arc<dyn SecretStore>) -> BoxFuture<'static, Result<Arc<dyn Provider>, String>>
        + Send
        + Sync,
>;

/// The default side-question provider: the built-in agent's routing (cloud when keyed and reachable, else
/// the first local model server).
pub fn default_side_provider() -> SideProviderFactory {
    Arc::new(|secrets: Arc<dyn SecretStore>| {
        Box::pin(async move {
            let config = adapters::builtin::BuiltinAdapter {
                policy: ProviderPolicy::Auto,
                model: None,
                router_override: None,
            }
            .router_config(secrets.as_ref());
            Router::new(config)
                .select()
                .await
                .map_err(|e| e.to_string())
        })
    })
}

/// Orchestrator configuration.
#[derive(Clone)]
pub struct OrchestratorConfig {
    /// Worktrees live under `<data_dir>/worktrees`.
    pub data_dir: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
    pub adapters: AdapterFactory,
    /// Answers side questions (`sendMessage` with `side`).
    pub side_provider: SideProviderFactory,
}

impl OrchestratorConfig {
    /// Defaults with the standard adapters.
    pub fn new(data_dir: impl Into<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        OrchestratorConfig {
            data_dir: data_dir.into(),
            secrets,
            adapters: Arc::new(adapters::for_config),
            side_provider: default_side_provider(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TurnState {
    /// Completed turns (done or failed).
    seq: u64,
    error: Option<String>,
    exited: bool,
}

/// Why a session is being stopped on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handoff {
    /// Its role moves to another agent, which takes over the turn in the same worktree.
    Switch,
    /// It leaves the mission; the turn ends without an error so the mission carries on.
    Remove,
    /// Its mission was archived or deleted: the turn ends and the mission's run stops quietly.
    Halt,
}

const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// `run_turn`'s error for an agent taken off its mission mid-turn (turned into a quiet success).
const REMOVED: &str = "removed from the mission";
/// The error a mission's run ends with when it was archived or deleted (reported nowhere).
const HALTED: &str = "the mission was archived or deleted";

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
    /// Mission agents whose session is being stopped on purpose (switched or removed): their failure isn't
    /// an error.
    stopping: Mutex<HashMap<String, Handoff>>,
    /// Mission agents inside `run_turn` (a switch hands their turn to the new agent).
    in_turn: Mutex<HashSet<String>>,
    /// Missions being deleted: nothing new may start in them.
    halted: Mutex<HashSet<String>>,
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
            stopping: Mutex::new(HashMap::new()),
            in_turn: Mutex::new(HashSet::new()),
            halted: Mutex::new(HashSet::new()),
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
                let ids: Vec<String> = o
                    .inner
                    .streams
                    .lock()
                    .map(|s| {
                        s.iter()
                            .filter(|(_, v)| v.dirty)
                            .map(|(k, _)| k.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for id in ids {
                    o.flush(&id, false);
                }
            }
        });
        orch
    }

    /// Queue an op; command ops addressed to this node are executed exactly once, in order.
    pub fn submit(&self, op: &Op) {
        if let Some((node, _)) = op.as_command()
            && node == self.inner.host.node_id()
        {
            let _ = self.inner.cmd_tx.send(op.clone());
        }
    }

    /// Recent log lines of a mission agent's process / session.
    pub fn logs(&self, mission_agent_id: &str) -> Vec<String> {
        self.inner
            .logs
            .lock()
            .ok()
            .and_then(|l| l.get(mission_agent_id).map(LogRing::lines))
            .unwrap_or_default()
    }

    /// Whether a mission agent has a live session.
    pub fn is_running(&self, mission_agent_id: &str) -> bool {
        self.inner
            .agents
            .lock()
            .map(|a| a.contains_key(mission_agent_id))
            .unwrap_or(false)
    }

    /// Mission agents this node was running when it stopped have lost their sessions: stop showing them as
    /// working. Sending one a message starts it again (see `send_message`).
    pub fn recover_interrupted(&self) {
        let me = self.host().node_id();
        let ids: Vec<String> = read(self.host(), |s| {
            s.mission_agents
                .values()
                .filter(|ma| ma.status == MissionAgentStatus::Working)
                .filter(|ma| {
                    s.missions
                        .get(&ma.mission_id)
                        .and_then(|m| m.node_id.as_deref())
                        == Some(me.as_str())
                })
                .map(|ma| ma.id.clone())
                .collect()
        });
        let ops = ids
            .iter()
            .filter(|id| !self.is_running(id))
            .map(|id| {
                OpBody::patch(
                    EntityKind::MissionAgent,
                    id,
                    [
                        ("status", json!("idle")),
                        ("statusDetail", json!("interrupted")),
                    ],
                )
            })
            .collect();
        self.emit(ops);
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
        let Some((_, command)) = op.as_command() else {
            return;
        };
        if self.host().has_command_result(&op.id) {
            return;
        }
        {
            let Ok(mut ex) = self.inner.executing.lock() else {
                return;
            };
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
            Command::SendMessage {
                mission_id,
                to,
                body,
                side,
            } => self.send_message(&mission_id, to.as_deref(), &body, side.as_deref(), &actor),
            Command::TakeSteering { mission_agent_id } => {
                self.take_steering(&mission_agent_id, &actor)
            }
            Command::ReleaseSteering { mission_agent_id } => {
                self.release_steering(&mission_agent_id, &actor)
            }
            Command::Pause { mission_agent_id } => self.pause(&mission_agent_id, true),
            Command::Resume { mission_agent_id } => self.pause(&mission_agent_id, false),
            Command::Merge { result_id } => self.merge(&result_id, &actor).await,
            Command::ConnectAgent { agent_id } => self.check_agent(&agent_id, true).await,
            Command::TestAgent { agent_id } => self.check_agent(&agent_id, false).await,
            Command::SwitchAgent {
                mission_agent_id,
                agent_id,
            } => {
                self.switch_agent(&mission_agent_id, &agent_id, &actor)
                    .await
            }
            Command::AddAgent { mission_agent_id } => self.add_agent(&mission_agent_id, &actor),
            Command::RemoveAgent { mission_agent_id } => {
                self.remove_agent(&mission_agent_id, &actor).await
            }
            Command::ArchiveMission {
                mission_id,
                archived,
            } => self.archive_mission(&mission_id, archived, &actor).await,
            Command::DeleteMission { mission_id } => self.delete_mission(&mission_id, &actor).await,
        };
        if let Err(e) = &result {
            tracing::warn!(op = %op.id, error = %e, "command failed");
        }
        self.emit(vec![OpBody::CommandResult {
            command_op: op.id.clone(),
            ok: result.is_ok(),
            error: result.err(),
        }]);
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
            let tasks: Vec<String> = s
                .tasks
                .values()
                .filter(|t| t.approval_id.as_deref() == Some(approval_id))
                .map(|t| t.id.clone())
                .collect();
            let other = a.as_ref().is_some_and(|a| {
                s.approvals.values().any(|o| {
                    o.mission_id == a.mission_id
                        && o.id != a.id
                        && o.state == ApprovalState::Pending
                })
            });
            (a, allowed, tasks, other)
        });
        let approval = approval.ok_or_else(|| format!("unknown approval {approval_id}"))?;
        if approval.state != ApprovalState::Pending {
            return Err(format!("approval {approval_id} was already decided"));
        }
        if !allowed {
            return Err(format!(
                "{actor} may not decide approvals in mission {}",
                approval.mission_id
            ));
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
                vec![
                    ("column", json!("done")),
                    ("note", json!(format!("Denied by {actor}"))),
                ]
            };
            ops.push(OpBody::patch(EntityKind::Task, t, fields));
        }
        if !other_pending {
            ops.push(OpBody::patch(
                EntityKind::Mission,
                &approval.mission_id,
                [("status", json!("working"))],
            ));
        }
        self.emit(ops);
        if let Some(tx) = self
            .inner
            .approvals
            .lock()
            .ok()
            .and_then(|mut a| a.remove(approval_id))
        {
            let _ = tx.send(approve);
        }
        Ok(())
    }

    async fn request_approval(
        &self,
        mission_id: &str,
        ma_id: &str,
        req: PermissionRequest,
    ) -> bool {
        self.flush(ma_id, true);
        let approval_id = format!("ap-{}", ulid_suffix());
        let (tx, rx) = oneshot::channel();
        if let Ok(mut a) = self.inner.approvals.lock() {
            a.insert(approval_id.clone(), tx);
        }
        let task_id = self
            .inner
            .task_of
            .lock()
            .ok()
            .and_then(|t| t.get(ma_id).cloned());
        let role = read(self.host(), |s| s.mission_agents.get(ma_id).map(|m| m.role))
            .unwrap_or(Role::Builder);
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
            OpBody::patch(
                EntityKind::MissionAgent,
                ma_id,
                [("status", json!("needsYou"))],
            ),
            OpBody::put(Message {
                id: format!("msg-{}", ulid_suffix()),
                mission_id: mission_id.into(),
                author: Author::Agent {
                    mission_agent_id: ma_id.into(),
                },
                steering: None,
                to: None,
                body: req.prompt.clone(),
                plan: vec![],
                log: vec![],
                approval_id: Some(approval_id.clone()),
                at: now_rfc3339(),
                side: None,
            }),
            OpBody::patch(
                EntityKind::Mission,
                mission_id,
                [("status", json!("needsYou"))],
            ),
        ];
        match task_id {
            Some(t) => ops.push(OpBody::patch(
                EntityKind::Task,
                t,
                [
                    ("column", json!("needsYou")),
                    ("approvalId", json!(approval_id)),
                ],
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

    fn send_message(
        &self,
        mission_id: &str,
        to: Option<&str>,
        body: &str,
        side: Option<&str>,
        actor: &str,
    ) -> CmdResult {
        let (exists, steerable, target) = read(self.host(), |s| {
            (
                s.missions
                    .get(mission_id)
                    .is_some_and(|m| m.archived_at.is_none()),
                may_steer(s, mission_id, actor),
                to.map(|t| s.mission_agents.get(t).cloned()),
            )
        });
        if !exists {
            return Err(format!("unknown or archived mission {mission_id}"));
        }
        if !steerable {
            return Err(format!("{actor} may not steer mission {mission_id}"));
        }
        if let Some(side) = side {
            if let Some(ma_id) = to
                && !target.flatten().is_some_and(|m| m.mission_id == mission_id)
            {
                return Err(format!("unknown agent {ma_id}"));
            }
            return self.side_question(mission_id, to, body, side, actor);
        }
        match to {
            Some(ma_id) => {
                let ma = target
                    .flatten()
                    .filter(|m| m.mission_id == mission_id)
                    .ok_or_else(|| format!("unknown agent {ma_id}"))?;
                match ma.steered_by.as_deref() {
                    Some(holder) if holder != actor => {
                        return Err(format!("steering held by {holder}"));
                    }
                    Some(_) => {}
                    None => self.emit(vec![OpBody::patch(
                        EntityKind::MissionAgent,
                        ma_id,
                        [("steeredBy", json!(actor))],
                    )]),
                }
                if !self.deliver(ma_id, body) {
                    self.restart(ma, body)?;
                }
            }
            None => {
                let ids: Vec<String> = self
                    .inner
                    .agents
                    .lock()
                    .map(|a| {
                        a.iter()
                            .filter(|(_, l)| l.mission_id == mission_id)
                            .map(|(k, _)| k.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if !ids.is_empty() {
                    for id in ids {
                        self.deliver(&id, body);
                    }
                    return Ok(());
                }
                // Nobody is running: resume the agents that were interrupted or failed.
                let stopped: Vec<MissionAgent> = read(self.host(), |s| {
                    s.mission_agents
                        .values()
                        .filter(|m| m.mission_id == mission_id && m.enabled)
                        .filter(|m| m.status == MissionAgentStatus::Idle)
                        .cloned()
                        .collect()
                });
                if stopped.is_empty() {
                    return Err(
                        "no agent in this mission is running; pick one under To to restart it"
                            .into(),
                    );
                }
                for ma in stopped {
                    self.restart(ma, body)?;
                }
            }
        }
        Ok(())
    }

    /// A side question: answered from the mission's context by a one-shot model call, in the same side thread,
    /// as one agent message. Nothing reaches the running agents and no steering or status changes.
    fn side_question(
        &self,
        mission_id: &str,
        to: Option<&str>,
        question: &str,
        side: &str,
        actor: &str,
    ) -> CmdResult {
        if side.is_empty() {
            return Err("the side thread id is empty".into());
        }
        let (answerer, prompt) = read(self.host(), |s| {
            let answerer = match to {
                Some(id) => Some(id.to_string()),
                None => side_answerer(s, mission_id),
            };
            let prompt = s
                .missions
                .get(mission_id)
                .map(|m| side_prompt(s, m, side, question, actor));
            (answerer, prompt)
        });
        let answerer =
            answerer.ok_or_else(|| format!("mission {mission_id} has no agent to answer"))?;
        let prompt = prompt.ok_or_else(|| format!("unknown mission {mission_id}"))?;
        let o = self.clone();
        let (mission_id, side) = (mission_id.to_string(), side.to_string());
        tokio::spawn(async move {
            let body = match o.ask_side(&prompt).await {
                Ok(answer) => answer,
                Err(e) => {
                    tracing::warn!(mission = %mission_id, error = %e, "side question not answered");
                    format!("I can't answer side questions right now: {e}")
                }
            };
            // The mission may have been deleted while the model answered.
            if !read(o.host(), |s| s.missions.contains_key(&mission_id)) {
                return;
            }
            o.emit(vec![OpBody::put(Message {
                id: format!("msg-{}", ulid_suffix()),
                mission_id,
                author: Author::Agent {
                    mission_agent_id: answerer,
                },
                steering: None,
                to: None,
                body,
                plan: vec![],
                log: vec![],
                approval_id: None,
                at: now_rfc3339(),
                side: Some(side),
            })]);
        });
        Ok(())
    }

    /// One model turn without tools.
    async fn ask_side(&self, prompt: &str) -> Result<String, String> {
        let config = &self.inner.config;
        let turn = async {
            let provider = (config.side_provider)(config.secrets.clone()).await?;
            let messages = [ChatMessage::user(prompt)];
            provider
                .turn(
                    TurnRequest {
                        system: SIDE_SYSTEM,
                        tools: &[],
                        messages: &messages,
                    },
                    &|_| {},
                )
                .await
                .map_err(|e| e.to_string())
        };
        let response = tokio::time::timeout(SIDE_TIMEOUT, turn)
            .await
            .map_err(|_| "the model took too long to answer".to_string())??;
        if response.stop_reason == StopReason::Refusal {
            return Err(format!(
                "the model declined: {}",
                response.refusal.unwrap_or_else(|| "no details".into())
            ));
        }
        let answer = response.message.text().trim().to_string();
        if answer.is_empty() {
            return Err("the model gave no answer".into());
        }
        Ok(answer)
    }

    /// Starts a new session for an agent that is not running (it finished, failed, or Astra restarted), in its
    /// mission worktree, with the mission brief and the message it was sent.
    fn restart(&self, ma: MissionAgent, body: &str) -> CmdResult {
        let mission = read(self.host(), |s| s.missions.get(&ma.mission_id).cloned())
            .ok_or_else(|| format!("unknown mission {}", ma.mission_id))?;
        let worktree = self
            .worktree_of(&ma)
            .ok_or_else(|| format!("agent {} is not running and its worktree is gone", ma.id))?;
        let prompt = format!(
            "{}\n\nYou were stopped and are now resumed in the same worktree. The person steering you says:\n\n{body}",
            prompt_for(ma.role, &mission, "")
        );
        let o = self.clone();
        tokio::spawn(async move {
            if let Err(e) = o.run_turn(&ma, &prompt, &worktree, None).await
                && e != HALTED
            {
                o.emit(vec![OpBody::patch(
                    EntityKind::MissionAgent,
                    &ma.id,
                    [
                        ("status", json!("idle")),
                        ("statusDetail", json!(format!("error: {e}"))),
                    ],
                )]);
            }
        });
        Ok(())
    }

    /// The worktree a mission agent works in: the one on its branch (or the mission's only worktree).
    fn worktree_of(&self, ma: &MissionAgent) -> Option<PathBuf> {
        let dir = self
            .inner
            .config
            .data_dir
            .join("worktrees")
            .join(sanitize(&ma.mission_id));
        let dirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join(".git").exists())
            .collect();
        let on_branch = |wt: &PathBuf| -> Option<String> {
            let git = std::fs::read_to_string(wt.join(".git")).ok()?;
            let gitdir = PathBuf::from(git.trim().strip_prefix("gitdir:")?.trim());
            let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
            head.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string)
        };
        match &ma.branch {
            Some(b) => dirs
                .iter()
                .find(|d| on_branch(d).as_deref() == Some(b.as_str()))
                .cloned(),
            None => None,
        }
        .or_else(|| (dirs.len() == 1).then(|| dirs[0].clone()))
    }

    fn deliver(&self, ma_id: &str, body: &str) -> bool {
        let sent = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|a| {
                a.get(ma_id).map(|l| {
                    l.control
                        .send(AgentControl::Message(body.to_string()))
                        .is_ok()
                })
            })
            .unwrap_or(false);
        if sent {
            self.flush(ma_id, true);
            self.emit(vec![OpBody::patch(
                EntityKind::MissionAgent,
                ma_id,
                [("status", json!("working"))],
            )]);
        }
        sent
    }

    fn take_steering(&self, ma_id: &str, actor: &str) -> CmdResult {
        let (ma, ok) = read(self.host(), |s| {
            let ma = s.mission_agents.get(ma_id).cloned();
            let ok = ma
                .as_ref()
                .is_some_and(|m| may_steer(s, &m.mission_id, actor));
            (ma, ok)
        });
        ma.ok_or_else(|| format!("unknown agent {ma_id}"))?;
        if !ok {
            return Err(format!("{actor} may not steer this mission"));
        }
        self.emit(vec![OpBody::patch(
            EntityKind::MissionAgent,
            ma_id,
            [("steeredBy", json!(actor))],
        )]);
        Ok(())
    }

    fn release_steering(&self, ma_id: &str, actor: &str) -> CmdResult {
        let ma = read(self.host(), |s| s.mission_agents.get(ma_id).cloned())
            .ok_or_else(|| format!("unknown agent {ma_id}"))?;
        match ma.steered_by.as_deref() {
            Some(h) if h == actor => {
                self.emit(vec![OpBody::patch(
                    EntityKind::MissionAgent,
                    ma_id,
                    [("steeredBy", Value::Null)],
                )]);
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
                a.get(ma_id).map(|l| {
                    l.control
                        .send(if pause {
                            AgentControl::Pause
                        } else {
                            AgentControl::Resume
                        })
                        .is_ok()
                })
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
        let agent = read(self.host(), |s| s.agents.get(agent_id).cloned())
            .ok_or_else(|| format!("unknown agent {agent_id}"))?;
        let check: Result<(), String> = match &agent.adapter {
            None => Err("no adapter configured".into()),
            Some(Adapter::Acp { command, args }) | Some(Adapter::Command { command, args }) => {
                crate::process::resolve_program(command, args).map(|_| ())
            }
            Some(Adapter::Langgraph { endpoint, .. }) => {
                let url = format!("{}/ok", endpoint.trim_end_matches('/'));
                match reqwest::Client::builder()
                    .timeout(Duration::from_secs(3))
                    .build()
                {
                    Ok(http) => http
                        .get(url)
                        .send()
                        .await
                        .map(|_| ())
                        .map_err(|_| format!("{endpoint} is not reachable")),
                    Err(e) => Err(e.to_string()),
                }
            }
            Some(Adapter::Builtin { provider, model }) => {
                let b = adapters::builtin::BuiltinAdapter {
                    policy: *provider,
                    model: model.clone(),
                    router_override: None,
                };
                astra_llm::Router::new(b.router_config(self.inner.config.secrets.as_ref()))
                    .select()
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };
        let status = if check.is_ok() {
            "connected"
        } else {
            "notConnected"
        };
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
            .map(|mut l| {
                l.entry(ma_id.to_string())
                    .or_insert_with(|| LogRing::new(500))
                    .clone()
            })
            .unwrap_or_else(|_| LogRing::new(500))
    }

    /// Start a session for a mission agent (or reuse the live one).
    fn ensure_session(
        &self,
        ma: &MissionAgent,
        prompt: &str,
        worktree: &std::path::Path,
    ) -> Result<bool, String> {
        if self
            .inner
            .agents
            .lock()
            .map(|a| a.contains_key(&ma.id))
            .unwrap_or(false)
        {
            return Ok(false);
        }
        // The agent may have been switched since the caller read `ma`: use the current one.
        let agent_id = read(self.host(), |s| {
            s.mission_agents
                .get(&ma.id)
                .map(|m| m.agent_id.clone())
                .unwrap_or_else(|| ma.agent_id.clone())
        });
        let (adapter, policy) = read(self.host(), |s| {
            s.agents
                .get(&agent_id)
                .map(|a| (a.adapter.clone(), a.command_policy))
        })
        .ok_or_else(|| format!("unknown agent {agent_id}"))?;
        let adapter =
            adapter.ok_or_else(|| format!("agent {agent_id} has no adapter configured"))?;
        let approver = Arc::new(MissionApprover {
            orch: self.clone(),
            mission_id: ma.mission_id.clone(),
            ma_id: ma.id.clone(),
        });
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
        let session = (self.inner.config.adapters)(&adapter)
            .start(ctx)
            .map_err(|e| e.to_string())?;
        let (turn_tx, turn_rx) = watch::channel(TurnState::default());
        if let Ok(mut a) = self.inner.agents.lock() {
            a.insert(
                ma.id.clone(),
                LiveAgent {
                    mission_id: ma.mission_id.clone(),
                    control: session.control,
                    turns: turn_rx,
                },
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
            // Gone from the live map before anyone waiting on `exited` starts a new session.
            if let Ok(mut a) = o.inner.agents.lock() {
                a.remove(&ma_id);
            }
            turn_tx.send_modify(|t| t.exited = true);
        });
        Ok(true)
    }

    fn on_agent_event(
        &self,
        mission_id: &str,
        ma_id: &str,
        ev: AgentEvent,
        turns: &watch::Sender<TurnState>,
    ) {
        let mut flush_now = false;
        let mut extra = Vec::new();
        {
            let Ok(mut streams) = self.inner.streams.lock() else {
                return;
            };
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
                        [(
                            "latest",
                            json!(st.latest.iter().cloned().collect::<Vec<_>>()),
                        )],
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
                AgentEvent::Error(_) if self.is_stopping(ma_id) => {
                    drop(streams);
                    self.flush(ma_id, true);
                    turns.send_modify(|t| {
                        t.seq += 1;
                        t.error = Some("stopped by a person".into());
                    });
                    return;
                }
                AgentEvent::Error(e) => {
                    st.body.push_str(&format!(
                        "{}Error: {e}",
                        if st.body.is_empty() { "" } else { "\n\n" }
                    ));
                    st.dirty = true;
                    drop(streams);
                    self.flush(ma_id, true);
                    self.emit(vec![OpBody::patch(
                        EntityKind::MissionAgent,
                        ma_id,
                        [
                            ("status", json!("idle")),
                            ("statusDetail", json!(format!("error: {e}"))),
                        ],
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
            let Ok(mut streams) = self.inner.streams.lock() else {
                return;
            };
            let Some(st) = streams.get_mut(ma_id) else {
                return;
            };
            let mut ops = Vec::new();
            if st.dirty {
                st.dirty = false;
                match &st.message_id {
                    Some(id) => ops.push(OpBody::patch(
                        EntityKind::Message,
                        id,
                        [
                            ("body", json!(st.body)),
                            ("plan", json!(st.plan)),
                            ("log", json!(st.log)),
                        ],
                    )),
                    None => {
                        let mission_id = read(self.host(), |s| {
                            s.mission_agents.get(ma_id).map(|m| m.mission_id.clone())
                        })
                        .unwrap_or_default();
                        let id = format!("msg-{}", ulid_suffix());
                        ops.push(OpBody::put(Message {
                            id: id.clone(),
                            mission_id,
                            author: Author::Agent {
                                mission_agent_id: ma_id.into(),
                            },
                            steering: None,
                            to: None,
                            body: st.body.trim_end().to_string(),
                            plan: st.plan.clone(),
                            log: st.log.clone(),
                            approval_id: None,
                            at: now_rfc3339(),
                            side: None,
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
            .and_then(|mut s| {
                s.get_mut(ma_id)
                    .map(|st| (std::mem::take(&mut st.turn_text), st.last_plan.clone()))
            })
            .unwrap_or_default()
    }

    /// Run one prompt turn of a mission agent in `worktree` and wait for it to finish.
    async fn run_turn(
        &self,
        ma: &MissionAgent,
        prompt: &str,
        worktree: &std::path::Path,
        task_id: Option<&str>,
    ) -> Result<String, String> {
        if let Some(t) = task_id {
            if let Ok(mut m) = self.inner.task_of.lock() {
                m.insert(ma.id.clone(), t.to_string());
            }
            self.emit(vec![OpBody::patch(
                EntityKind::Task,
                t,
                [("column", json!("working"))],
            )]);
        }
        let mut fields = vec![("status", json!("working")), ("statusDetail", Value::Null)];
        if ma.started_at.is_none() {
            fields.push(("startedAt", json!(now_rfc3339())));
        }
        self.emit(vec![OpBody::patch(
            EntityKind::MissionAgent,
            &ma.id,
            fields,
        )]);
        if self.is_halted(&ma.mission_id) {
            return Err(HALTED.into());
        }
        let _ = self.take_turn_text(&ma.id);
        if let Ok(mut t) = self.inner.in_turn.lock() {
            t.insert(ma.id.clone());
        }
        let mut prompt = self.with_agent_type(&ma.id, prompt);
        let outcome = loop {
            let outcome = self.turn_once(ma, &prompt, worktree).await;
            if outcome.is_ok() {
                break outcome;
            }
            match self.take_handoff(&ma.id) {
                Some(Handoff::Switch) => {
                    // The new agent picks the turn up where the old one stopped.
                    self.wait_stopped(&ma.id).await;
                    self.emit(vec![OpBody::patch(
                        EntityKind::MissionAgent,
                        &ma.id,
                        [("status", json!("working"))],
                    )]);
                    prompt = format!(
                        "{prompt}\n\nAnother agent started this task in this worktree and was stopped part-way. \
                         Check what is already done (git status, git diff), then finish the task."
                    );
                }
                Some(Handoff::Remove) => break Err(REMOVED.to_string()),
                Some(Handoff::Halt) => break Err(HALTED.to_string()),
                None => break outcome,
            }
        };
        if let Ok(mut t) = self.inner.in_turn.lock() {
            t.remove(&ma.id);
        }
        if matches!(&outcome, Err(e) if e == HALTED) {
            // Archived or deleted: write nothing more about this turn.
            let _ = self.take_turn_text(&ma.id);
            return Err(HALTED.into());
        }
        if matches!(&outcome, Err(e) if e == REMOVED) {
            // Leaving the mission is not a failure: its task closes and the mission carries on.
            if let Some(t) = task_id {
                self.emit(vec![OpBody::patch(
                    EntityKind::Task,
                    t,
                    [
                        ("column", json!("done")),
                        ("note", json!("Removed from the mission")),
                    ],
                )]);
            }
            let (text, _) = self.take_turn_text(&ma.id);
            return Ok(text);
        }
        let (text, _) = self.take_turn_text(&ma.id);
        match &outcome {
            Ok(()) => {
                let mut ops = vec![OpBody::patch(
                    EntityKind::MissionAgent,
                    &ma.id,
                    [("status", json!("done"))],
                )];
                if let Some(t) = task_id {
                    ops.push(OpBody::patch(
                        EntityKind::Task,
                        t,
                        [("column", json!("done"))],
                    ));
                }
                self.emit(ops);
            }
            Err(e) => {
                if let Some(t) = task_id {
                    self.emit(vec![OpBody::patch(
                        EntityKind::Task,
                        t,
                        [("note", json!(format!("Failed: {e}")))],
                    )]);
                }
            }
        }
        outcome.map(|_| text)
    }

    /// Adds the instructions of the mission agent's custom agent type (if any) to its brief.
    fn with_agent_type(&self, ma_id: &str, prompt: &str) -> String {
        let ty = read(self.host(), |s| {
            s.mission_agents
                .get(ma_id)
                .and_then(|m| m.agent_type_id.as_ref())
                .and_then(|id| s.agent_types.get(id))
                .cloned()
        });
        match ty {
            Some(t) if !t.instructions.trim().is_empty() => format!(
                "{prompt}\n\nYou are working as \"{}\". Follow these instructions:\n{}",
                t.name,
                t.instructions.trim()
            ),
            _ => prompt.to_string(),
        }
    }

    /// One attempt at a turn on the mission agent's current agent.
    async fn turn_once(
        &self,
        ma: &MissionAgent,
        prompt: &str,
        worktree: &std::path::Path,
    ) -> Result<(), String> {
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
        loop {
            {
                let t = rx.borrow();
                if t.seq > before {
                    return match &t.error {
                        Some(e) => Err(e.clone()),
                        None => Ok(()),
                    };
                }
                if t.exited {
                    return Err("agent exited before finishing".to_string());
                }
            }
            if rx.changed().await.is_err() {
                return Err("agent exited before finishing".to_string());
            }
        }
    }

    fn is_stopping(&self, ma_id: &str) -> bool {
        self.inner
            .stopping
            .lock()
            .map(|s| s.contains_key(ma_id))
            .unwrap_or(false)
    }

    fn take_handoff(&self, ma_id: &str) -> Option<Handoff> {
        self.inner.stopping.lock().ok()?.remove(ma_id)
    }

    /// Waits (bounded) until the mission agent has no live session.
    async fn wait_stopped(&self, ma_id: &str) {
        let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
        while self.is_running(ma_id) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Ends a mission agent's live session on purpose. A turn in progress sees `why` (see `run_turn`); with no
    /// turn in progress the mark is cleared once the session is gone. Returns whether a session was running.
    async fn stop_session(&self, ma_id: &str, why: Handoff) -> Result<bool, String> {
        let live = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|a| a.get(ma_id).map(|l| (l.control.clone(), l.turns.clone())));
        let Some((control, mut turns)) = live else {
            return Ok(false);
        };
        if let Ok(mut s) = self.inner.stopping.lock() {
            s.insert(ma_id.to_string(), why);
        }
        let _ = control.send(AgentControl::Cancel);
        // Wait for this session to end (a handed-over turn may already be starting the next one).
        let stopped = tokio::time::timeout(STOP_TIMEOUT, async {
            while !turns.borrow().exited {
                if turns.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .is_ok();
        let in_turn = self
            .inner
            .in_turn
            .lock()
            .map(|t| t.contains(ma_id))
            .unwrap_or(false);
        if !in_turn {
            self.take_handoff(ma_id);
        }
        if !stopped {
            return Err(format!(
                "agent {ma_id} did not stop within {} s",
                STOP_TIMEOUT.as_secs()
            ));
        }
        Ok(true)
    }

    /// Checks shared by the commands that change a mission's agents: the mission exists, isn't merged, and
    /// the actor may steer it.
    fn editable_mission(&self, mission_id: &str, actor: &str) -> Result<Mission, String> {
        let (mission, steer) = read(self.host(), |s| {
            (
                s.missions.get(mission_id).cloned(),
                may_steer(s, mission_id, actor),
            )
        });
        let mission = mission.ok_or_else(|| format!("unknown mission {mission_id}"))?;
        if !steer {
            return Err(format!("{actor} may not steer mission {mission_id}"));
        }
        if mission.status == MissionStatus::Merged {
            return Err("the mission is already merged".into());
        }
        if mission.archived_at.is_some() {
            return Err("the mission is archived".into());
        }
        Ok(mission)
    }

    /// A connected agent with an adapter, ready to take a role.
    fn usable_agent(&self, agent_id: &str) -> Result<Agent, String> {
        let agent = read(self.host(), |s| s.agents.get(agent_id).cloned())
            .ok_or_else(|| format!("unknown agent {agent_id}"))?;
        if agent.adapter.is_none() {
            return Err(format!("{} has no adapter configured", agent.name));
        }
        if agent.status != AgentStatus::Connected {
            return Err(format!("{} is not connected", agent.name));
        }
        Ok(agent)
    }

    fn has_pending_approval(&self, ma_id: &str) -> bool {
        read(self.host(), |s| {
            s.approvals
                .values()
                .any(|a| a.mission_agent_id == ma_id && a.state == ApprovalState::Pending)
        })
    }

    /// Runs a mission agent's role on another agent (vendor or harness). A turn in progress is handed over in
    /// the same worktree; an agent that hasn't started yet simply starts on the new one.
    async fn switch_agent(&self, ma_id: &str, agent_id: &str, actor: &str) -> CmdResult {
        let ma = read(self.host(), |s| s.mission_agents.get(ma_id).cloned())
            .ok_or_else(|| format!("unknown agent {ma_id}"))?;
        self.editable_mission(&ma.mission_id, actor)?;
        let agent = self.usable_agent(agent_id)?;
        if ma.agent_id == agent_id {
            return Ok(());
        }
        if self.has_pending_approval(ma_id) {
            return Err("decide its pending approval first".into());
        }
        let old = read(self.host(), |s| {
            s.agents.get(&ma.agent_id).map(|a| a.name.clone())
        })
        .unwrap_or_else(|| ma.agent_id.clone());
        // Switch first, so whoever starts the next session (a handed-over turn included) uses the new agent.
        self.emit(vec![OpBody::patch(
            EntityKind::MissionAgent,
            ma_id,
            [
                ("agentId", json!(agent.id)),
                ("statusDetail", json!(format!("switched from {old}"))),
            ],
        )]);
        self.stop_session(ma_id, Handoff::Switch).await?;
        Ok(())
    }

    /// Whether nothing may run in this mission any more: it is archived, deleted, or being deleted.
    fn is_halted(&self, mission_id: &str) -> bool {
        self.inner
            .halted
            .lock()
            .map(|h| h.contains(mission_id))
            .unwrap_or(false)
            || read(self.host(), |s| {
                s.missions
                    .get(mission_id)
                    .is_none_or(|m| m.archived_at.is_some())
            })
    }

    /// Stops everything running in a mission: pending approvals are denied (the agents waiting on them give
    /// up) and live sessions end. Returns the mission agents that were running.
    async fn stop_mission(&self, mission_id: &str, actor: &str) -> Vec<String> {
        let pending: Vec<String> = read(self.host(), |s| {
            s.approvals
                .values()
                .filter(|a| a.mission_id == mission_id && a.state == ApprovalState::Pending)
                .map(|a| a.id.clone())
                .collect()
        });
        for id in &pending {
            if let Some(tx) = self
                .inner
                .approvals
                .lock()
                .ok()
                .and_then(|mut a| a.remove(id))
            {
                let _ = tx.send(false);
            }
            self.emit(vec![OpBody::patch(
                EntityKind::Approval,
                id,
                [
                    ("state", json!("denied")),
                    ("decidedBy", json!(actor)),
                    ("decidedAt", json!(now_rfc3339())),
                ],
            )]);
        }
        let live: Vec<String> = self
            .inner
            .agents
            .lock()
            .map(|a| {
                a.iter()
                    .filter(|(_, l)| l.mission_id == mission_id)
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in &live {
            if let Err(e) = self.stop_session(id, Handoff::Halt).await {
                tracing::warn!(agent = %id, error = %e, "could not stop agent");
            }
        }
        live
    }

    fn owned_mission(&self, mission_id: &str, actor: &str) -> Result<Mission, String> {
        let (mission, manage) = read(self.host(), |s| {
            (
                s.missions.get(mission_id).cloned(),
                may_manage(s, mission_id, actor),
            )
        });
        let mission = mission.ok_or_else(|| format!("unknown mission {mission_id}"))?;
        if !manage {
            return Err(format!(
                "only the owner can archive or delete mission {mission_id}"
            ));
        }
        Ok(mission)
    }

    /// Archives a mission (its agents stop; it leaves the active lists) or brings it back.
    async fn archive_mission(&self, mission_id: &str, archived: bool, actor: &str) -> CmdResult {
        let mission = self.owned_mission(mission_id, actor)?;
        if !archived {
            if mission.archived_at.is_some() {
                self.emit(vec![OpBody::patch(
                    EntityKind::Mission,
                    mission_id,
                    [("archivedAt", Value::Null)],
                )]);
            }
            return Ok(());
        }
        if mission.archived_at.is_some() {
            return Ok(());
        }
        // Archive first, so nothing new starts while the running agents stop.
        let mut fields = vec![("archivedAt", json!(now_rfc3339()))];
        if mission.status == MissionStatus::Working {
            fields.push(("status", json!("needsYou")));
        }
        self.emit(vec![OpBody::patch(EntityKind::Mission, mission_id, fields)]);
        let stopped = self.stop_mission(mission_id, actor).await;
        self.emit(
            stopped
                .iter()
                .map(|id| {
                    OpBody::patch(
                        EntityKind::MissionAgent,
                        id,
                        [
                            ("status", json!("idle")),
                            ("statusDetail", json!("stopped: archived")),
                        ],
                    )
                })
                .collect(),
        );
        Ok(())
    }

    /// Deletes a mission and everything in it. Its worktrees are removed; its branches stay in the repository.
    async fn delete_mission(&self, mission_id: &str, actor: &str) -> CmdResult {
        let mission = self.owned_mission(mission_id, actor)?;
        if let Ok(mut h) = self.inner.halted.lock() {
            h.insert(mission_id.to_string());
        }
        let stopped = self.stop_mission(mission_id, actor).await;
        if let Ok(mut streams) = self.inner.streams.lock() {
            for id in &stopped {
                streams.remove(id);
            }
        }
        let repo_path = read(self.host(), |s| {
            s.repositories
                .get(&mission.repo_id)
                .and_then(|r| r.path.clone())
        })
        .map(|p| git::expand_home(&p));
        let wt_root = self
            .inner
            .config
            .data_dir
            .join("worktrees")
            .join(sanitize(mission_id));
        let _ = blocking(move || -> Result<(), String> {
            if let (Some(repo), Ok(rd)) = (repo_path, std::fs::read_dir(&wt_root)) {
                for e in rd.flatten() {
                    let _ = git::remove_worktree(&repo, &e.path());
                }
            }
            let _ = std::fs::remove_dir_all(&wt_root);
            Ok(())
        })
        .await;
        let mut ops: Vec<OpBody> = read(self.host(), |s| {
            let mut ids: Vec<(EntityKind, String)> = Vec::new();
            ids.extend(
                s.mission_agents
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::MissionAgent, e.id.clone())),
            );
            ids.extend(
                s.tasks
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Task, e.id.clone())),
            );
            ids.extend(
                s.messages
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Message, e.id.clone())),
            );
            ids.extend(
                s.approvals
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Approval, e.id.clone())),
            );
            ids.extend(
                s.results
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Result, e.id.clone())),
            );
            ids.extend(
                s.merges
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Merge, e.id.clone())),
            );
            ids.extend(
                s.memberships
                    .values()
                    .filter(|e| e.mission_id == mission_id)
                    .map(|e| (EntityKind::Membership, e.id.clone())),
            );
            ids.into_iter()
                .map(|(entity_kind, id)| OpBody::Delete { entity_kind, id })
                .collect()
        });
        ops.push(OpBody::Delete {
            entity_kind: EntityKind::Mission,
            id: mission_id.to_string(),
        });
        self.emit(ops);
        Ok(())
    }

    /// Starts an agent the client just put on a mission that is already under way. One added before the
    /// mission starts is picked up by `startMission`.
    fn add_agent(&self, ma_id: &str, actor: &str) -> CmdResult {
        let ma = read(self.host(), |s| s.mission_agents.get(ma_id).cloned())
            .ok_or_else(|| format!("unknown agent {ma_id}"))?;
        let mission = self.editable_mission(&ma.mission_id, actor)?;
        self.usable_agent(&ma.agent_id)?;
        if mission.status == MissionStatus::Draft {
            return Ok(());
        }
        if mission.mode == MissionMode::Race {
            return Err("a race's entrants are fixed once it starts".into());
        }
        let worktree = self
            .worktree_of(&ma)
            .ok_or_else(|| "the mission has no worktree to work in".to_string())?;
        let prompt = prompt_for(ma.role, &mission, "");
        let o = self.clone();
        tokio::spawn(async move {
            if let Err(e) = o.run_turn(&ma, &prompt, &worktree, None).await
                && e != HALTED
            {
                o.emit(vec![OpBody::patch(
                    EntityKind::MissionAgent,
                    &ma.id,
                    [
                        ("status", json!("idle")),
                        ("statusDetail", json!(format!("error: {e}"))),
                    ],
                )]);
            }
        });
        Ok(())
    }

    /// Takes an agent off a mission: its session stops, it stays in the thread's history, and the mission
    /// carries on without it.
    async fn remove_agent(&self, ma_id: &str, actor: &str) -> CmdResult {
        let ma = read(self.host(), |s| s.mission_agents.get(ma_id).cloned())
            .ok_or_else(|| format!("unknown agent {ma_id}"))?;
        self.editable_mission(&ma.mission_id, actor)?;
        if !ma.enabled {
            return Ok(());
        }
        let others = read(self.host(), |s| {
            s.mission_agents
                .values()
                .filter(|m| m.mission_id == ma.mission_id && m.enabled && m.id != ma_id)
                .count()
        });
        if others == 0 {
            return Err("a mission needs at least one agent".into());
        }
        if self.has_pending_approval(ma_id) {
            return Err("decide its pending approval first".into());
        }
        self.emit(vec![OpBody::patch(
            EntityKind::MissionAgent,
            ma_id,
            [
                ("enabled", json!(false)),
                ("status", json!("done")),
                ("statusDetail", json!("removed")),
                ("steeredBy", Value::Null),
            ],
        )]);
        self.stop_session(ma_id, Handoff::Remove).await?;
        Ok(())
    }

    // -------------------------------------------------------------------------------------- missions

    fn mission_context(
        &self,
        mission_id: &str,
    ) -> Result<(Mission, Repository, Vec<MissionAgent>, PathBuf), String> {
        let (mission, repo, agents, adapters_ok) = read(self.host(), |s| {
            let m = s.missions.get(mission_id).cloned();
            let r = m
                .as_ref()
                .and_then(|m| s.repositories.get(&m.repo_id).cloned());
            let mut a: Vec<MissionAgent> = s
                .mission_agents
                .values()
                .filter(|ma| ma.mission_id == mission_id && ma.enabled)
                .cloned()
                .collect();
            a.sort_by_key(|ma| (role_rank(ma.role), ma.id.clone()));
            let missing: Vec<String> = a
                .iter()
                .filter(|ma| {
                    s.agents
                        .get(&ma.agent_id)
                        .and_then(|ag| ag.adapter.as_ref())
                        .is_none()
                })
                .map(|ma| ma.agent_id.clone())
                .collect();
            (m, r, a, missing)
        });
        let mission = mission.ok_or_else(|| format!("unknown mission {mission_id}"))?;
        let repo = repo.ok_or_else(|| format!("unknown repository {}", mission.repo_id))?;
        let path = repo
            .path
            .as_deref()
            .map(git::expand_home)
            .ok_or_else(|| format!("repository {} has no local path", repo.name))?;
        if !adapters_ok.is_empty() {
            return Err(format!(
                "agents without an adapter: {}",
                adapters_ok.join(", ")
            ));
        }
        if agents.is_empty() {
            return Err("the mission has no enabled agents".into());
        }
        git2_check(&path, &mission.base_branch)?;
        Ok((mission, repo, agents, path))
    }

    fn worktree_dir(&self, mission_id: &str, name: &str) -> PathBuf {
        self.inner
            .config
            .data_dir
            .join("worktrees")
            .join(sanitize(mission_id))
            .join(sanitize(name))
    }

    fn start_mission(&self, mission_id: &str) -> CmdResult {
        let (mission, repo, agents, path) = self.mission_context(mission_id)?;
        if mission.archived_at.is_some() {
            return Err("the mission is archived".into());
        }
        if !matches!(
            mission.status,
            MissionStatus::Draft | MissionStatus::NeedsYou | MissionStatus::Working
        ) {
            return Err(format!("mission {mission_id} is {:?}", mission.status));
        }
        if mission.mode == MissionMode::Race && !agents.iter().any(|a| a.role == Role::Builder) {
            return Err("a race needs at least one builder".into());
        }
        self.emit(vec![OpBody::patch(
            EntityKind::Mission,
            mission_id,
            [
                ("status", json!("working")),
                ("nodeId", json!(self.host().node_id())),
            ],
        )]);
        let o = self.clone();
        tokio::spawn(async move {
            let r = match mission.mode {
                MissionMode::Split => o.run_split(&mission, &repo, &agents, &path).await,
                MissionMode::Race => o.run_race(&mission, &repo, &agents, &path).await,
            };
            if let Err(e) = r {
                if e == HALTED {
                    tracing::info!(mission = %mission.id, "mission stopped: archived or deleted");
                    return;
                }
                tracing::warn!(mission = %mission.id, error = %e, "mission failed");
                let author = agents.first().map(|a| a.id.clone()).unwrap_or_default();
                o.emit(vec![
                    OpBody::put(Message {
                        id: format!("msg-{}", ulid_suffix()),
                        mission_id: mission.id.clone(),
                        author: Author::Agent {
                            mission_agent_id: author,
                        },
                        steering: None,
                        to: None,
                        body: format!("The mission stopped: {e}"),
                        plan: vec![],
                        log: vec![],
                        approval_id: None,
                        at: now_rfc3339(),
                        side: None,
                    }),
                    OpBody::patch(
                        EntityKind::Mission,
                        &mission.id,
                        [("status", json!("needsYou"))],
                    ),
                ]);
            }
        });
        Ok(())
    }

    fn new_task(
        &self,
        mission: &Mission,
        ma: Option<&MissionAgent>,
        role: Role,
        title: String,
        column: TaskColumn,
        order: i64,
    ) -> String {
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

    async fn run_split(
        &self,
        mission: &Mission,
        _repo: &Repository,
        agents: &[MissionAgent],
        path: &std::path::Path,
    ) -> CmdResult {
        let wt_dir = self.worktree_dir(&mission.id, "main");
        let (p, base, branch, d) = (
            path.to_path_buf(),
            mission.base_branch.clone(),
            mission.branch.clone(),
            wt_dir.clone(),
        );
        let worktree = blocking(move || {
            git::create_worktree(&p, &base, &branch, &d).map_err(|e| e.to_string())
        })
        .await?;
        self.emit(
            agents
                .iter()
                .map(|a| {
                    OpBody::patch(
                        EntityKind::MissionAgent,
                        &a.id,
                        [("branch", json!(mission.branch))],
                    )
                })
                .collect(),
        );

        let mut plan_text = String::new();
        let mut order = 1;
        if let Some(planner) = agents.iter().find(|a| a.role == Role::Planner) {
            let title = planner
                .task
                .clone()
                .unwrap_or_else(|| format!("Plan: {}", mission.title));
            let task = self.new_task(
                mission,
                Some(planner),
                Role::Planner,
                title,
                TaskColumn::Queued,
                order,
            );
            order += 1;
            let text = self
                .run_turn(
                    planner,
                    &prompt_for(Role::Planner, mission, ""),
                    &worktree,
                    Some(&task),
                )
                .await?;
            let (_, plan) = self.take_turn_text(&planner.id);
            plan_text = render_plan(&plan, &text);
        }
        let workers: Vec<&MissionAgent> =
            agents.iter().filter(|a| a.role != Role::Planner).collect();
        let mut queue = Vec::new();
        for ma in &workers {
            let title = ma
                .task
                .clone()
                .unwrap_or_else(|| default_task_title(ma.role, &mission.title));
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
            self.run_turn(
                &ma,
                &prompt_for(ma.role, mission, &plan_text),
                &worktree,
                Some(&task),
            )
            .await?;
        }

        let (wt, msg) = (worktree.clone(), format!("astra: {}", mission.title));
        blocking(move || git::commit_all(&wt, &msg).map_err(|e| e.to_string())).await?;
        let (p, base, branch) = (
            path.to_path_buf(),
            mission.base_branch.clone(),
            mission.branch.clone(),
        );
        let (stats, diff) =
            blocking(move || git::diff_branch(&p, &base, &branch).map_err(|e| e.to_string()))
                .await?;
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
            OpBody::patch(
                EntityKind::Mission,
                &mission.id,
                [("stats", json!(stats)), ("status", json!("needsYou"))],
            ),
        ]);
        self.new_task(
            mission,
            None,
            Role::Reviewer,
            format!("Review and merge {}", mission.short_name),
            TaskColumn::NeedsYou,
            order,
        );
        Ok(())
    }

    async fn run_race(
        &self,
        mission: &Mission,
        _repo: &Repository,
        agents: &[MissionAgent],
        path: &std::path::Path,
    ) -> CmdResult {
        let builders: Vec<&MissionAgent> =
            agents.iter().filter(|a| a.role == Role::Builder).collect();
        let mut runs = Vec::new();
        let mut entries = Vec::new();
        for (i, ma) in builders.iter().enumerate() {
            let label = label_for(i);
            let branch = format!("{}-{}", mission.branch, label.to_lowercase());
            let dir = self.worktree_dir(&mission.id, &label);
            let (p, base, b, d) = (
                path.to_path_buf(),
                mission.base_branch.clone(),
                branch.clone(),
                dir,
            );
            let wt = blocking(move || {
                git::create_worktree(&p, &base, &b, &d).map_err(|e| e.to_string())
            })
            .await?;
            self.emit(vec![OpBody::patch(
                EntityKind::MissionAgent,
                &ma.id,
                [("branch", json!(branch))],
            )]);
            let title = ma.task.clone().unwrap_or_else(|| mission.title.clone());
            let task = self.new_task(
                mission,
                Some(ma),
                Role::Builder,
                title,
                TaskColumn::Queued,
                i as i64 + 1,
            );
            entries.push(((*ma).clone(), label, branch, wt.clone()));
            let o = self.clone();
            let ma = (*ma).clone();
            let prompt = prompt_for(Role::Builder, mission, "");
            runs.push(async move { o.run_turn(&ma, &prompt, &wt, Some(&task)).await });
        }
        let outcomes = futures::future::join_all(runs).await;
        if self.is_halted(&mission.id) {
            return Err(HALTED.into());
        }

        let mut results = Vec::new();
        for ((ma, label, branch, wt), outcome) in entries.into_iter().zip(outcomes) {
            if let Err(e) = outcome {
                tracing::warn!(agent = %ma.id, error = %e, "race builder failed");
                continue;
            }
            let msg = format!("astra: {} ({label})", mission.title);
            let w = wt.clone();
            blocking(move || git::commit_all(&w, &msg).map_err(|e| e.to_string())).await?;
            let (p, base, b) = (
                path.to_path_buf(),
                mission.base_branch.clone(),
                branch.clone(),
            );
            let (stats, diff) =
                blocking(move || git::diff_branch(&p, &base, &b).map_err(|e| e.to_string()))
                    .await?;
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
            let title = reviewer
                .task
                .clone()
                .unwrap_or_else(|| format!("Compare the {} results", results.len()));
            let task = self.new_task(
                mission,
                Some(reviewer),
                Role::Reviewer,
                title,
                TaskColumn::Queued,
                results.len() as i64 + 1,
            );
            let prompt = review_prompt(mission, &results);
            match self.run_turn(reviewer, &prompt, &dir, Some(&task)).await {
                Ok(text) => {
                    let labels: Vec<String> = results.iter().map(|r| r.label.clone()).collect();
                    if let Some(pick) = parse_pick(&text, &labels)
                        && let Some(r) = results.iter().find(|r| r.label == pick)
                    {
                        self.emit(vec![OpBody::patch(
                            EntityKind::Result,
                            &r.id,
                            [("reviewerPick", json!(true))],
                        )]);
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
        self.emit(vec![OpBody::patch(
            EntityKind::Mission,
            &mission.id,
            [("status", json!("needsYou"))],
        )]);
        Ok(())
    }

    async fn merge(&self, result_id: &str, actor: &str) -> CmdResult {
        let (result, mission, repo, others, tasks, allowed) = read(self.host(), |s| {
            let r = s.results.get(result_id).cloned();
            let m = r
                .as_ref()
                .and_then(|r| s.missions.get(&r.mission_id).cloned());
            let repo = m
                .as_ref()
                .and_then(|m| s.repositories.get(&m.repo_id).cloned());
            let others: Vec<String> = r
                .as_ref()
                .map(|r| {
                    s.results
                        .values()
                        .filter(|o| o.mission_id == r.mission_id && o.id != r.id)
                        .map(|o| o.branch.clone())
                        .collect()
                })
                .unwrap_or_default();
            let tasks: Vec<String> = r
                .as_ref()
                .map(|r| {
                    s.tasks
                        .values()
                        .filter(|t| {
                            t.mission_id == r.mission_id
                                && t.column == TaskColumn::NeedsYou
                                && t.approval_id.is_none()
                        })
                        .map(|t| t.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            let allowed = r
                .as_ref()
                .is_some_and(|r| may_decide(s, &r.mission_id, actor, None));
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
        let path = repo
            .path
            .as_deref()
            .map(git::expand_home)
            .ok_or_else(|| format!("repository {} has no local path", repo.name))?;

        // Stop the mission's agents and remove its worktrees (branches are kept).
        let live: Vec<String> = self
            .inner
            .agents
            .lock()
            .map(|a| {
                a.iter()
                    .filter(|(_, l)| l.mission_id == mission.id)
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in live {
            if let Some(l) = self
                .inner
                .agents
                .lock()
                .ok()
                .and_then(|mut a| a.remove(&id))
            {
                let _ = l.control.send(AgentControl::Cancel);
            }
        }
        let message = format!(
            "Merge {} (mission {}: {})",
            result.branch, mission.number, mission.title
        );
        let (p, b, into) = (
            path.clone(),
            result.branch.clone(),
            mission.base_branch.clone(),
        );
        blocking(move || git::merge_branch(&p, &b, &into, &message).map_err(|e| e.to_string()))
            .await?;
        let wt_root = self
            .inner
            .config
            .data_dir
            .join("worktrees")
            .join(sanitize(&mission.id));
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
            OpBody::patch(
                EntityKind::Mission,
                &mission.id,
                [("status", json!("merged")), ("stats", json!(result.stats))],
            ),
        ];
        for t in tasks {
            ops.push(OpBody::patch(
                EntityKind::Task,
                t,
                [("column", json!("done")), ("mergedBy", json!(actor))],
            ));
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
        self.orch
            .request_approval(&self.mission_id, &self.ma_id, req)
            .await
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

fn git2_check(path: &std::path::Path, base: &str) -> Result<(), String> {
    if !git::branch_exists(path, base) {
        return Err(format!(
            "{} is not a git repository with branch {base}",
            path.display()
        ));
    }
    Ok(())
}

/// The system prompt for side questions.
const SIDE_SYSTEM: &str = "You answer a side question about a coding mission in Astra, asked in a side chat next to \
the mission's main thread. The mission's agents keep working and never see this side chat. You are read-only: you \
cannot read files, run commands, message the agents or take any other action, so never offer to. Answer from the \
mission context you are given, concisely and plainly. If the context does not tell, say so instead of guessing.";
/// How long a side question may take.
const SIDE_TIMEOUT: Duration = Duration::from_secs(180);
/// Main-thread messages included in a side question's context.
const SIDE_CONTEXT_MESSAGES: usize = 30;

/// Who answers a side question not addressed to an agent: the planner, else the first enabled agent.
fn side_answerer(s: &State, mission_id: &str) -> Option<String> {
    let mut agents: Vec<&MissionAgent> = s
        .mission_agents
        .values()
        .filter(|m| m.mission_id == mission_id)
        .collect();
    agents.sort_by_key(|m| (!m.enabled, m.role != Role::Planner, role_rank(m.role)));
    agents.first().map(|m| m.id.clone())
}

fn role_title(r: Role) -> &'static str {
    match r {
        Role::Planner => "Planner",
        Role::Builder => "Builder",
        Role::Tester => "Tester",
        Role::Writer => "Writer",
        Role::Reviewer => "Reviewer",
    }
}

fn author_name(s: &State, author: &Author) -> String {
    match author {
        Author::Person { person_id } => s
            .people
            .get(person_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| person_id.clone()),
        Author::Agent { mission_agent_id } => match s.mission_agents.get(mission_agent_id) {
            Some(ma) => match s.agents.get(&ma.agent_id) {
                Some(a) => format!("{} ({})", role_title(ma.role), a.name),
                None => role_title(ma.role).to_string(),
            },
            None => "Agent".into(),
        },
    }
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// One thread message as context lines: `Name: body`, then its plan and activity in brief.
fn side_line(s: &State, m: &Message) -> String {
    let mut line = format!("{}: {}", author_name(s, &m.author), clip(&m.body, 1200));
    if !m.plan.is_empty() {
        let plan: Vec<String> = m
            .plan
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}. {}", i + 1, clip(&p.text, 160)))
            .collect();
        line.push_str(&format!("\n  plan: {}", plan.join("; ")));
    }
    if !m.log.is_empty() {
        let shown: Vec<String> = m
            .log
            .iter()
            .take(8)
            .map(|l| format!("{} {}", l.verb, clip(&l.target, 120)))
            .collect();
        let more = m.log.len().saturating_sub(shown.len());
        let more = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        line.push_str(&format!("\n  activity: {}{more}", shown.join("; ")));
    }
    line
}

/// The one-shot prompt for a side question: the mission, its agents, the recent main thread, this side thread
/// so far and the question.
fn side_prompt(s: &State, mission: &Mission, side: &str, question: &str, asker: &str) -> String {
    let mut thread: Vec<&Message> = s
        .messages
        .values()
        .filter(|m| m.mission_id == mission.id)
        .collect();
    thread.sort_by(|a, b| (&a.at, &a.id).cmp(&(&b.at, &b.id)));
    let main: Vec<&Message> = thread
        .iter()
        .copied()
        .filter(|m| m.side.is_none())
        .collect();
    let main = &main[main.len().saturating_sub(SIDE_CONTEXT_MESSAGES)..];
    let mut earlier: Vec<&Message> = thread
        .iter()
        .copied()
        .filter(|m| m.side.as_deref() == Some(side))
        .collect();
    // The client puts the question itself before the command; it is quoted below instead.
    if let Some(i) = earlier.iter().rposition(|m| {
        matches!(&m.author, Author::Person { .. }) && m.body.trim() == question.trim()
    }) {
        earlier.remove(i);
    }

    let status = serde_json::to_value(mission.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut out = format!(
        "Mission #{}: {}\nGoal: {}\nStatus: {status}\n",
        mission.number, mission.title, mission.goal
    );
    let mut agents: Vec<&MissionAgent> = s
        .mission_agents
        .values()
        .filter(|m| m.mission_id == mission.id && m.enabled)
        .collect();
    agents.sort_by_key(|m| (role_rank(m.role), m.id.clone()));
    if !agents.is_empty() {
        out.push_str("\nAgents:\n");
        for ma in agents {
            let status = serde_json::to_value(ma.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {}: {status}",
                author_name(
                    s,
                    &Author::Agent {
                        mission_agent_id: ma.id.clone()
                    }
                )
            ));
            if let Some(task) = &ma.task {
                out.push_str(&format!(", on: {}", clip(task, 200)));
            }
            if let Some(detail) = &ma.status_detail {
                out.push_str(&format!(" ({})", clip(detail, 200)));
            }
            out.push('\n');
        }
    }
    out.push_str("\nMain thread, oldest first:\n");
    if main.is_empty() {
        out.push_str("(no messages yet)\n");
    }
    for m in main {
        out.push_str(&side_line(s, m));
        out.push('\n');
    }
    if !earlier.is_empty() {
        out.push_str("\nThis side chat so far:\n");
        for m in earlier {
            out.push_str(&side_line(s, m));
            out.push('\n');
        }
    }
    let asker = s
        .people
        .get(asker)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| asker.to_string());
    out.push_str(&format!(
        "\nSide question from {asker}:\n{}",
        question.trim()
    ));
    out
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
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
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
    plan.iter()
        .enumerate()
        .map(|(i, p)| format!("{}. {}", i + 1, p.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prompt for a role.
pub fn prompt_for(role: Role, mission: &Mission, plan: &str) -> String {
    let plan = if plan.is_empty() {
        String::new()
    } else {
        format!("\n\nThe plan:\n{plan}")
    };
    match role {
        Role::Planner => format!(
            "Goal: {}\n\nYou are the planner. Read the code and write a short, numbered plan that the builder, tester \
             and reviewer will follow. Do not change files.",
            mission.goal
        ),
        Role::Builder => format!(
            "Goal: {}{plan}\n\nYou are the builder. Make the change in this worktree.",
            mission.goal
        ),
        Role::Tester => format!(
            "Goal: {}{plan}\n\nYou are the tester. Add or run tests for the change and fix what fails.",
            mission.goal
        ),
        Role::Writer => format!(
            "Goal: {}{plan}\n\nYou are the writer. Update the documentation for the change.",
            mission.goal
        ),
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
    let alts = labels
        .iter()
        .map(|l| regex::escape(l))
        .collect::<Vec<_>>()
        .join("|");
    let strong = regex::Regex::new(&format!(
        r"(?i)\b(?:pick|merge|choose|prefer|recommend|go with)\b[^A-Za-z0-9]*(?:result\s+)?\b({alts})\b"
    ))
    .ok()?;
    if let Some(c) = strong.captures(text) {
        let found = c.get(1)?.as_str();
        return labels
            .iter()
            .find(|l| l.eq_ignore_ascii_case(found))
            .cloned();
    }
    let weak = regex::Regex::new(&format!(r"\b(?:result\s+)?({alts})\b")).ok()?;
    weak.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
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
        assert_eq!(
            parse_pick("Pick: B\nIt is smaller.", &labels).as_deref(),
            Some("B")
        );
        assert_eq!(
            parse_pick("I would merge A now and keep B as a follow-up.", &labels).as_deref(),
            Some("A")
        );
        assert_eq!(
            parse_pick("Result B is cleaner.", &labels).as_deref(),
            Some("B")
        );
        assert_eq!(parse_pick("no idea", &labels), None);
    }

    fn message(id: &str, author: Author, body: &str, at: &str, side: Option<&str>) -> Entity {
        Entity::Message(Message {
            id: id.into(),
            mission_id: "m-1".into(),
            author,
            steering: None,
            to: None,
            body: body.into(),
            plan: vec![],
            log: vec![LogLine {
                verb: "edit".into(),
                target: "src/hello.rs".into(),
            }],
            approval_id: None,
            at: at.into(),
            side: side.map(str::to_string),
        })
    }

    #[test]
    fn side_prompt_has_the_main_thread_this_side_thread_and_the_question() {
        let mut s = State::new("ws", "p-owner");
        let mission: Mission = serde_json::from_value(json!({
            "id": "m-1", "number": 7, "title": "Add a greeting", "shortName": "GREETING",
            "goal": "Add a greeting file.", "mode": "split", "repoId": "r-1", "baseBranch": "main",
            "branch": "astra/greeting", "status": "working", "approvalPolicy": "owner", "ownerId": "p-owner",
            "nodeId": null, "stats": null, "createdAt": "2026-11-21T09:00:00Z", "archivedAt": null
        }))
        .expect("mission");
        let ma: MissionAgent = serde_json::from_value(json!({
            "id": "ma-b", "missionId": "m-1", "agentId": "a-1", "role": "builder", "enabled": true,
            "status": "working", "statusDetail": null, "steeredBy": null, "task": "Add hello.rs",
            "startedAt": null, "branch": null, "latest": [], "agentTypeId": null
        }))
        .expect("mission agent");
        let person = |id: &str, name: &str| {
            Entity::Person(Person {
                id: id.into(),
                name: name.into(),
                initials: "X".into(),
            })
        };
        let me = || Author::Person {
            person_id: "p-owner".into(),
        };
        let builder = || Author::Agent {
            mission_agent_id: "ma-b".into(),
        };
        for e in [
            Entity::Mission(mission.clone()),
            Entity::MissionAgent(ma),
            person("p-owner", "Mark"),
            message(
                "msg-1",
                me(),
                "Please add a greeting",
                "2026-11-21T09:01:00Z",
                None,
            ),
            message(
                "msg-2",
                builder(),
                "Adding it now",
                "2026-11-21T09:02:00Z",
                None,
            ),
            message(
                "msg-3",
                me(),
                "Earlier side question",
                "2026-11-21T09:03:00Z",
                Some("side-a"),
            ),
            message(
                "msg-4",
                builder(),
                "Earlier side answer",
                "2026-11-21T09:03:30Z",
                Some("side-a"),
            ),
            message(
                "msg-5",
                me(),
                "Another side thread",
                "2026-11-21T09:04:00Z",
                Some("side-b"),
            ),
            message(
                "msg-6",
                me(),
                "Is it done yet?",
                "2026-11-21T09:05:00Z",
                Some("side-a"),
            ),
        ] {
            s.insert(e);
        }
        let p = side_prompt(&s, &mission, "side-a", "Is it done yet?", "p-owner");
        assert!(p.starts_with(
            "Mission #7: Add a greeting\nGoal: Add a greeting file.\nStatus: working"
        ));
        assert!(p.contains("- Builder: working, on: Add hello.rs"));
        assert!(p.contains("Mark: Please add a greeting"));
        assert!(p.contains("Builder: Adding it now\n  activity: edit src/hello.rs"));
        assert!(p.contains("This side chat so far:\nMark: Earlier side question"));
        assert!(p.contains("Builder: Earlier side answer"));
        assert!(!p.contains("Another side thread"));
        // The question is quoted once, at the end, not as part of the side chat so far.
        assert_eq!(p.matches("Is it done yet?").count(), 1);
        assert!(p.ends_with("Side question from Mark:\nIs it done yet?"));
        let main_at = p.find("Main thread").expect("main");
        assert!(p.find("Please add a greeting").expect("msg-1") > main_at);
        assert!(
            p.find("Earlier side question").expect("msg-3")
                > p.find("Adding it now").expect("msg-2")
        );
    }
}
