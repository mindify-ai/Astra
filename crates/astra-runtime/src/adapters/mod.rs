//! Agent adapters: one session per mission agent, driven through [`AgentControl`], reporting
//! [`AgentEvent`]s. Permission requests go through the [`Approver`] (which may ask a human).

use std::path::PathBuf;
use std::sync::Arc;

use astra_protocol::{Adapter, CommandPolicy, LogLine, PlanStep, Role};
use tokio::sync::mpsc;

use crate::process::LogRing;
use crate::secrets::SecretStore;

pub mod acp;
pub mod builtin;
pub mod command;
pub mod langgraph;

/// Events from a running agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// Message text (a chunk; consecutive chunks form one message).
    Text(String),
    /// The agent's current plan.
    Plan(Vec<PlanStep>),
    /// An activity line (`edit src/x.rs`, `run cargo test`).
    Log(LogLine),
    /// The agent finished the current prompt.
    TurnDone,
    /// The agent failed.
    Error(String),
    /// The agent process/session ended.
    Exited,
}

/// Controls sent to a running agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentControl {
    /// A user message (a new prompt turn).
    Message(String),
    /// Stop the current work and end the session.
    Cancel,
    Pause,
    Resume,
}

/// A permission request from an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// What the agent wants to do, for humans.
    pub prompt: String,
    /// The command line, when the request is to run a command.
    pub command: Option<String>,
    /// Working directory of the command, when known.
    pub cwd: Option<PathBuf>,
}

/// Decides agent permission requests (policy first, then a human via an `approval` entity).
#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    /// Return true to allow.
    async fn request(&self, req: PermissionRequest) -> bool;
}

/// Everything an adapter needs to start a session.
#[derive(Clone)]
pub struct AgentContext {
    pub mission_agent_id: String,
    pub role: Role,
    /// The initial prompt.
    pub prompt: String,
    /// The agent's git worktree (all file access is confined to it).
    pub worktree: PathBuf,
    pub command_policy: CommandPolicy,
    pub approver: Arc<dyn Approver>,
    pub secrets: Arc<dyn SecretStore>,
    pub logs: LogRing,
}

/// A started session.
pub struct AgentSession {
    pub control: mpsc::UnboundedSender<AgentControl>,
    pub events: mpsc::UnboundedReceiver<AgentEvent>,
}

/// Adapter errors.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("failed to start {0}: {1}")]
    Spawn(String, String),
    #[error("{0}")]
    Other(String),
}

/// Starts agent sessions.
pub trait AgentAdapter: Send + Sync {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError>;
}

/// The adapter implementation for a protocol `Adapter` config.
pub fn for_config(adapter: &Adapter) -> Box<dyn AgentAdapter> {
    match adapter {
        Adapter::Acp { command, args } => Box::new(acp::AcpAdapter {
            command: command.clone(),
            args: args.clone(),
        }),
        Adapter::Command { command, args } => Box::new(command::CommandAdapter {
            command: command.clone(),
            args: args.clone(),
        }),
        Adapter::Langgraph { endpoint, graph } => Box::new(langgraph::LangGraphAdapter {
            endpoint: endpoint.clone(),
            graph: graph.clone(),
        }),
        Adapter::Builtin { provider, model } => Box::new(builtin::BuiltinAdapter {
            policy: *provider,
            model: model.clone(),
            router_override: None,
        }),
    }
}
