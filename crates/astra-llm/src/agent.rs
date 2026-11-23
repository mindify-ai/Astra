//! The provider-agnostic manual tool loop.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::tools::Toolset;
use crate::types::*;
use crate::{LlmError, Result};

/// Default system prompt (stable: no timestamps or per-run values, so it caches).
pub const DEFAULT_SYSTEM: &str = "You are the Astra agent, a careful software engineer working inside one git \
worktree. Use the tools to read, search, edit and run code in the workspace. Keep changes focused on the goal. \
When you finish, reply with a short summary of what you changed.";

/// Progress events from the loop.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopEvent {
    /// Streaming text.
    TextDelta(String),
    /// Complete text of one assistant turn.
    Text(String),
    /// A tool is about to run.
    ToolStarted { name: String, input: Value },
    /// A tool finished; `log` is the activity line.
    ToolFinished { name: String, is_error: bool, log: Option<(String, String)> },
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopOutcome {
    /// The model ended its turn; final text.
    Done { text: String },
    /// The model refused; no tools of that turn were run.
    Refused { explanation: Option<String> },
    /// Too many model turns.
    TurnLimit,
}

/// A manual tool loop over any [`Provider`].
pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: Toolset,
    specs: Vec<ToolSpec>,
    validators: HashMap<String, jsonschema::Validator>,
    system: String,
    history: Vec<ChatMessage>,
    /// Maximum model turns per [`AgentLoop::run`].
    pub max_turns: usize,
}

impl AgentLoop {
    /// A loop with the standard workspace tools.
    pub fn new(provider: Arc<dyn Provider>, tools: Toolset, system: impl Into<String>) -> Result<Self> {
        let specs = Toolset::specs();
        let mut validators = HashMap::new();
        for s in &specs {
            let v = jsonschema::validator_for(&s.input_schema).map_err(|e| LlmError::Schema(e.to_string()))?;
            validators.insert(s.name.clone(), v);
        }
        Ok(AgentLoop { provider, tools, specs, validators, system: system.into(), history: vec![], max_turns: 50 })
    }

    /// The conversation so far.
    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    /// Validate tool input against its schema. `Err` carries the message for the model.
    fn validate(&self, call: &ToolCall) -> std::result::Result<Value, String> {
        let input = match &call.input {
            Ok(v) => v,
            Err(raw) => return Err(format!("Tool input was not valid JSON. Raw input: {raw}")),
        };
        let Some(validator) = self.validators.get(&call.name) else {
            return Err(format!("Unknown tool: {}", call.name));
        };
        let errors: Vec<String> = validator.iter_errors(input).map(|e| e.to_string()).collect();
        if errors.is_empty() {
            Ok(input.clone())
        } else {
            Err(format!("Tool input does not match the schema: {}. Raw input: {}", errors.join("; "), input))
        }
    }

    /// Send a user message and run the loop until the model ends its turn.
    pub async fn run(&mut self, user: impl Into<String>, events: &UnboundedSender<LoopEvent>) -> Result<LoopOutcome> {
        self.history.push(ChatMessage::user(user));
        let mut truncated_retries = 0;
        for _ in 0..self.max_turns {
            let tx = events.clone();
            let on_event = move |e: StreamEvent| {
                if let StreamEvent::TextDelta(t) = e {
                    let _ = tx.send(LoopEvent::TextDelta(t));
                }
            };
            let resp = self
                .provider
                .turn(TurnRequest { system: &self.system, tools: &self.specs, messages: &self.history }, &on_event)
                .await?;
            // Always keep the full assistant content (thinking blocks included, unchanged).
            self.history.push(resp.message.clone());
            let text = resp.message.text();
            if !text.is_empty() {
                let _ = events.send(LoopEvent::Text(text.clone()));
            }
            match resp.stop_reason {
                StopReason::Refusal => return Ok(LoopOutcome::Refused { explanation: resp.refusal }),
                StopReason::PauseTurn => continue,
                StopReason::MaxTokens if !resp.tool_calls.is_empty() => {
                    truncated_retries += 1;
                    if truncated_retries > 2 {
                        return Err(LlmError::Stream("tool calls keep hitting max_tokens".into()));
                    }
                    let results = resp
                        .tool_calls
                        .iter()
                        .map(|c| Block::ToolResult {
                            tool_use_id: c.id.clone(),
                            content: "Not run: the response hit max_tokens before this tool call was complete. \
                                      Retry with a smaller input (for example, split the edit into parts)."
                                .into(),
                            is_error: true,
                        })
                        .collect();
                    self.history.push(ChatMessage { role: ChatRole::User, blocks: results, raw: None });
                }
                StopReason::ToolUse if !resp.tool_calls.is_empty() => {
                    truncated_retries = 0;
                    let mut results = Vec::new();
                    for call in &resp.tool_calls {
                        let block = match self.validate(call) {
                            Err(msg) => Block::ToolResult { tool_use_id: call.id.clone(), content: msg, is_error: true },
                            Ok(input) => {
                                let _ = events.send(LoopEvent::ToolStarted { name: call.name.clone(), input: input.clone() });
                                let out = self.tools.execute(&call.name, &input).await;
                                let _ = events.send(LoopEvent::ToolFinished {
                                    name: call.name.clone(),
                                    is_error: out.is_error,
                                    log: out.log.clone(),
                                });
                                Block::ToolResult { tool_use_id: call.id.clone(), content: out.content, is_error: out.is_error }
                            }
                        };
                        results.push(block);
                    }
                    // All results of the turn go back in one user message.
                    self.history.push(ChatMessage { role: ChatRole::User, blocks: results, raw: None });
                }
                _ => return Ok(LoopOutcome::Done { text }),
            }
        }
        Ok(LoopOutcome::TurnLimit)
    }
}
