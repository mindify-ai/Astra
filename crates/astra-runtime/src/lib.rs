//! The Astra agent runtime for runner nodes: agent adapters (ACP, command, LangGraph, built-in), process
//! supervision, git worktrees, command policy, secrets and the orchestrator that executes protocol commands.

pub mod adapters;
pub mod git;
pub mod orchestrator;
pub mod policy;
pub mod process;
pub mod secrets;

pub use orchestrator::{NodeHost, Orchestrator, OrchestratorConfig};
pub use secrets::{MemorySecrets, SecretStore};
