//! Provider-agnostic conversation types.

use serde_json::Value;

/// Conversation role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

/// A content block.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

/// One message of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub blocks: Vec<Block>,
    /// The provider-native content array, replayed verbatim to the provider that produced it
    /// (keeps thinking blocks and signatures unchanged).
    pub raw: Option<(String, Value)>,
}

impl ChatMessage {
    /// A user text message.
    pub fn user(text: impl Into<String>) -> Self {
        ChatMessage { role: ChatRole::User, blocks: vec![Block::Text { text: text.into() }], raw: None }
    }

    /// Concatenated text blocks.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// A user-defined tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Why a turn stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    PauseTurn,
    Refusal,
    StopSequence,
    Other(String),
}

impl StopReason {
    /// Parse an Anthropic `stop_reason`.
    pub fn parse(s: &str) -> StopReason {
        match s {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "pause_turn" => StopReason::PauseTurn,
            "refusal" => StopReason::Refusal,
            "stop_sequence" => StopReason::StopSequence,
            other => StopReason::Other(other.to_string()),
        }
    }
}

/// A tool call requested by the model. `input` is `Err(raw text)` when it was not valid JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Result<Value, String>,
}

/// The result of one model turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResponse {
    pub message: ChatMessage,
    pub stop_reason: StopReason,
    pub tool_calls: Vec<ToolCall>,
    /// Refusal explanation from `stop_details`, when present.
    pub refusal: Option<String>,
}

/// One request to a provider.
#[derive(Debug, Clone, Copy)]
pub struct TurnRequest<'a> {
    pub system: &'a str,
    pub tools: &'a [ToolSpec],
    pub messages: &'a [ChatMessage],
}

/// Streaming progress.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    Thinking,
    ToolInputDelta { index: usize },
}

/// A model provider.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Human-readable name (`"anthropic:<model>"`, `"ollama:<model>"`).
    fn name(&self) -> String;
    /// Run one streamed turn.
    async fn turn(&self, req: TurnRequest<'_>, on_event: &(dyn Fn(StreamEvent) + Send + Sync)) -> crate::Result<TurnResponse>;
}
