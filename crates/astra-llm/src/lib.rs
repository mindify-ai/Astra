//! Model providers and the provider-agnostic tool loop of the built-in Astra agent.
//!
//! - [`anthropic::AnthropicProvider`]: Messages API over raw HTTP + SSE.
//! - [`openai::OpenAiCompatProvider`]: OpenAI-compatible local servers (Ollama, LM Studio, llama.cpp).
//! - [`router::Router`]: `auto | cloud | local` provider selection.
//! - [`agent::AgentLoop`]: manual tool loop with sandboxed workspace tools ([`tools`]).

pub mod agent;
pub mod anthropic;
pub mod openai;
pub mod router;
pub mod sse;
pub mod tools;
pub mod types;

pub use agent::{AgentLoop, LoopEvent, LoopOutcome};
pub use anthropic::{AnthropicConfig, AnthropicProvider, Effort};
pub use openai::{LocalServer, OpenAiCompatProvider, detect_local_servers};
pub use router::{Router, RouterConfig, RoutingPolicy};
pub use tools::{AllowAllCommands, CommandApprover, DenyAllCommands, ToolOutcome, Toolset, Workspace};
pub use types::*;

/// Errors from providers and the loop.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("http: {0}")]
    Http(String),
    #[error("api error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("stream error: {0}")]
    Stream(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no model provider available: {0}")]
    NoProvider(String),
    #[error("tool schema: {0}")]
    Schema(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        // Never include request details (headers carry the API key).
        LlmError::Http(e.without_url().to_string())
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, LlmError>;
