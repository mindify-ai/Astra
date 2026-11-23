//! Anthropic Messages API provider (raw HTTP + SSE).
//!
//! Request shape: streaming, adaptive thinking, explicit effort, server-side fallbacks, `tool_choice: auto`
//! (forced tool choice is rejected by the default model), no sampling parameters. Tools use
//! `eager_input_streaming`, so tool inputs are parsed strictly here and validated against their schema by the
//! agent loop before anything runs. The tools + system prefix is byte-stable for prompt caching.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::sse::SseParser;
use crate::types::*;
use crate::{LlmError, Result};

/// Default model.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// API version header value.
pub const API_VERSION: &str = "2026-10-21";
/// Beta header value (server-side refusal fallbacks).
pub const BETA: &str = "server-side-fallback-2026-07-01";
/// Default API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Effort level (`output_config.effort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
    Xhigh,
    Max,
}

impl Effort {
    /// Wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }

    /// Parse a wire value.
    pub fn parse(s: &str) -> Option<Effort> {
        Some(match s {
            "low" => Effort::Low,
            "medium" => Effort::Medium,
            "high" => Effort::High,
            "xhigh" => Effort::Xhigh,
            "max" => Effort::Max,
            _ => return None,
        })
    }
}

/// Provider configuration. The API key is never logged (`Debug` redacts it).
#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub effort: Effort,
    pub max_tokens: u32,
    pub base_url: String,
    pub max_retries: u32,
}

impl std::fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("max_tokens", &self.max_tokens)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AnthropicConfig {
    /// Defaults for `api_key`.
    pub fn new(api_key: impl Into<String>) -> Self {
        AnthropicConfig {
            api_key: api_key.into(),
            model: DEFAULT_MODEL.into(),
            effort: Effort::High,
            max_tokens: 64_000,
            base_url: DEFAULT_BASE_URL.into(),
            max_retries: 2,
        }
    }
}

/// The Anthropic provider.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    config: AnthropicConfig,
    http: reqwest::Client,
}

impl AnthropicProvider {
    /// A provider with its own HTTP client.
    pub fn new(config: AnthropicConfig) -> Result<Self> {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build()?;
        Ok(AnthropicProvider { config, http })
    }

    /// The configuration.
    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }

    /// Build the JSON request body.
    pub fn request_body(&self, req: &TurnRequest<'_>) -> Value {
        let mut body = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "stream": true,
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": self.config.effort.as_str()},
            "fallbacks": "default",
            "system": [{"type": "text", "text": req.system, "cache_control": {"type": "ephemeral"}}],
            "messages": req.messages.iter().map(message_json).collect::<Vec<_>>(),
        });
        if !req.tools.is_empty() {
            let n = req.tools.len();
            let tools: Vec<Value> = req
                .tools
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut tool = json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                        "eager_input_streaming": true,
                    });
                    if i + 1 == n {
                        tool["cache_control"] = json!({"type": "ephemeral"});
                    }
                    tool
                })
                .collect();
            body["tools"] = Value::Array(tools);
            body["tool_choice"] = json!({"type": "auto"});
        }
        body
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let mut attempt = 0;
        loop {
            let resp = self
                .http
                .post(&url)
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", API_VERSION)
                .header("anthropic-beta", BETA)
                .header("content-type", "application/json")
                .json(body)
                .send()
                .await;
            let retry_after = match resp {
                Ok(r) if r.status().is_success() => return Ok(r),
                Ok(r) => {
                    let status = r.status().as_u16();
                    let retryable = matches!(status, 408 | 409 | 429 | 500..=599)
                        && r.headers().get("x-should-retry").and_then(|v| v.to_str().ok()) != Some("false");
                    let after = r
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok());
                    let text = r.text().await.unwrap_or_default();
                    let message = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                        .unwrap_or(text);
                    if !retryable || attempt >= self.config.max_retries {
                        return Err(LlmError::Api { status, message });
                    }
                    after
                }
                Err(e) => {
                    if attempt >= self.config.max_retries || !(e.is_connect() || e.is_timeout()) {
                        return Err(e.into());
                    }
                    None
                }
            };
            attempt += 1;
            let wait = retry_after.map(Duration::from_secs).unwrap_or(Duration::from_millis(500 * 2u64.pow(attempt)));
            tokio::time::sleep(wait.min(Duration::from_secs(30))).await;
        }
    }
}

/// Convert a message to the Messages API shape.
pub fn message_json(m: &ChatMessage) -> Value {
    let role = match m.role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    };
    if let Some((provider, raw)) = &m.raw {
        if provider == "anthropic" {
            return json!({"role": role, "content": raw});
        }
    }
    let content: Vec<Value> = m
        .blocks
        .iter()
        .map(|b| match b {
            Block::Text { text } => json!({"type": "text", "text": text}),
            Block::ToolUse { id, name, input } => json!({"type": "tool_use", "id": id, "name": name, "input": input}),
            Block::ToolResult { tool_use_id, content, is_error } => json!({
                "type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error
            }),
        })
        .collect();
    json!({"role": role, "content": content})
}

#[derive(Debug)]
struct BlockAcc {
    block: Value,
    json_buf: String,
    stopped: bool,
}

/// Accumulates a streamed message.
#[derive(Debug, Default)]
pub struct MessageAccumulator {
    blocks: BTreeMap<usize, BlockAcc>,
    stop_reason: Option<String>,
    refusal: Option<String>,
    done: bool,
}

impl MessageAccumulator {
    /// Feed one SSE event (`event` name and JSON `data`).
    pub fn on_event(
        &mut self,
        event: &str,
        data: &str,
        on_event: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<()> {
        if event == "ping" {
            return Ok(());
        }
        let v: Value = serde_json::from_str(data)?;
        match v["type"].as_str().unwrap_or(event) {
            "message_start" => {}
            "content_block_start" => {
                let index = index_of(&v)?;
                self.blocks.insert(
                    index,
                    BlockAcc { block: v["content_block"].clone(), json_buf: String::new(), stopped: false },
                );
            }
            "content_block_delta" => {
                let index = index_of(&v)?;
                let acc = self
                    .blocks
                    .get_mut(&index)
                    .ok_or_else(|| LlmError::Stream(format!("delta for unknown block {index}")))?;
                let delta = &v["delta"];
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        let t = delta["text"].as_str().unwrap_or_default();
                        append_str(&mut acc.block, "text", t);
                        on_event(StreamEvent::TextDelta(t.to_string()));
                    }
                    "input_json_delta" => {
                        acc.json_buf.push_str(delta["partial_json"].as_str().unwrap_or_default());
                        on_event(StreamEvent::ToolInputDelta { index });
                    }
                    "thinking_delta" => {
                        append_str(&mut acc.block, "thinking", delta["thinking"].as_str().unwrap_or_default());
                        on_event(StreamEvent::Thinking);
                    }
                    "signature_delta" => {
                        append_str(&mut acc.block, "signature", delta["signature"].as_str().unwrap_or_default());
                    }
                    "citations_delta" => {
                        if let Some(obj) = acc.block.as_object_mut() {
                            let list = obj.entry("citations").or_insert_with(|| Value::Array(vec![]));
                            if let Some(arr) = list.as_array_mut() {
                                arr.push(delta["citation"].clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = index_of(&v)?;
                if let Some(acc) = self.blocks.get_mut(&index) {
                    acc.stopped = true;
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
                let details = &v["delta"]["stop_details"];
                if details.is_object() {
                    self.refusal = details["explanation"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| details["category"].as_str().map(|c| format!("category: {c}")));
                }
            }
            "message_stop" => self.done = true,
            "error" => {
                return Err(LlmError::Stream(
                    v["error"]["message"].as_str().unwrap_or("unknown stream error").to_string(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// Finish: parse tool inputs strictly and build the turn response.
    pub fn finish(self) -> Result<TurnResponse> {
        if !self.done && self.stop_reason.is_none() {
            return Err(LlmError::Stream("stream ended before message_stop".into()));
        }
        let mut raw = Vec::new();
        let mut blocks = Vec::new();
        let mut tool_calls = Vec::new();
        for (_, mut acc) in self.blocks {
            match acc.block["type"].as_str().unwrap_or_default() {
                "text" => blocks.push(Block::Text { text: acc.block["text"].as_str().unwrap_or_default().to_string() }),
                "tool_use" => {
                    let id = acc.block["id"].as_str().unwrap_or_default().to_string();
                    let name = acc.block["name"].as_str().unwrap_or_default().to_string();
                    let parsed: std::result::Result<Value, String> = if !acc.stopped {
                        Err(acc.json_buf.clone())
                    } else if acc.json_buf.trim().is_empty() {
                        // No deltas: the start block carried the full input.
                        Ok(acc.block.get("input").cloned().unwrap_or_else(|| json!({})))
                    } else {
                        serde_json::from_str::<Value>(&acc.json_buf).map_err(|_| acc.json_buf.clone())
                    };
                    let history_input = match &parsed {
                        Ok(v) if v.is_object() => v.clone(),
                        _ => json!({}),
                    };
                    acc.block["input"] = history_input.clone();
                    blocks.push(Block::ToolUse { id: id.clone(), name: name.clone(), input: history_input });
                    tool_calls.push(ToolCall { id, name, input: parsed });
                }
                _ => {}
            }
            raw.push(acc.block);
        }
        Ok(TurnResponse {
            message: ChatMessage {
                role: ChatRole::Assistant,
                blocks,
                raw: Some(("anthropic".into(), Value::Array(raw))),
            },
            stop_reason: StopReason::parse(self.stop_reason.as_deref().unwrap_or("end_turn")),
            tool_calls,
            refusal: self.refusal,
        })
    }
}

fn index_of(v: &Value) -> Result<usize> {
    v["index"]
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| LlmError::Stream("event without index".into()))
}

fn append_str(block: &mut Value, field: &str, add: &str) {
    if let Some(obj) = block.as_object_mut() {
        let cur = obj.get(field).and_then(Value::as_str).unwrap_or_default().to_string();
        obj.insert(field.to_string(), Value::String(cur + add));
    }
}

#[async_trait::async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> String {
        format!("anthropic:{}", self.config.model)
    }

    async fn turn(&self, req: TurnRequest<'_>, on_event: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<TurnResponse> {
        let body = self.request_body(&req);
        let resp = self.send(&body).await?;
        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::new();
        let mut acc = MessageAccumulator::default();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            for ev in parser.push(&chunk) {
                acc.on_event(ev.event.as_deref().unwrap_or(""), &ev.data, on_event)?;
            }
        }
        if let Some(ev) = parser.finish() {
            acc.on_event(ev.event.as_deref().unwrap_or(""), &ev.data, on_event)?;
        }
        acc.finish()
    }
}
