//! OpenAI-compatible chat-completions provider for local model servers.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::sse::SseParser;
use crate::types::*;
use crate::{LlmError, Result};

/// Well-known local servers: (kind, base URL including `/v1`).
pub const LOCAL_CANDIDATES: &[(&str, &str)] = &[
    ("ollama", "http://127.0.0.1:11434/v1"),
    ("lmstudio", "http://127.0.0.1:1234/v1"),
    ("llamacpp", "http://127.0.0.1:8080/v1"),
];

/// A detected local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalServer {
    pub kind: String,
    pub base_url: String,
    pub models: Vec<String>,
}

/// Probe `GET {base}/models` on each candidate with a short timeout. Returns reachable servers in order.
pub async fn detect_servers(
    candidates: &[(String, String)],
    timeout: Duration,
) -> Vec<LocalServer> {
    let http = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let probes = candidates.iter().map(|(kind, base)| {
        let http = http.clone();
        async move {
            let url = format!("{}/models", base.trim_end_matches('/'));
            let resp = http.get(url).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let v: Value = resp.json().await.ok()?;
            let models = v["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Some(LocalServer {
                kind: kind.clone(),
                base_url: base.clone(),
                models,
            })
        }
    });
    futures::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// [`detect_servers`] over the well-known local ports (400 ms timeout each, probed concurrently).
pub async fn detect_local_servers() -> Vec<LocalServer> {
    let c: Vec<(String, String)> = LOCAL_CANDIDATES
        .iter()
        .map(|(k, b)| (k.to_string(), b.to_string()))
        .collect();
    detect_servers(&c, Duration::from_millis(400)).await
}

/// Provider for an OpenAI-compatible server.
#[derive(Debug, Clone)]
pub struct OpenAiCompatProvider {
    pub base_url: String,
    pub model: String,
    pub kind: String,
    pub api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatProvider {
    /// A provider for `base_url` (including `/v1`) and `model`.
    pub fn new(
        kind: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        Ok(OpenAiCompatProvider {
            base_url: base_url.into(),
            model: model.into(),
            kind: kind.into(),
            api_key: None,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()?,
        })
    }

    /// Build the JSON request body.
    pub fn request_body(&self, req: &TurnRequest<'_>) -> Value {
        let mut messages = vec![json!({"role": "system", "content": req.system})];
        for m in req.messages {
            match m.role {
                ChatRole::User => {
                    let mut text = Vec::new();
                    for b in &m.blocks {
                        match b {
                            Block::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            } => {
                                let content = if *is_error {
                                    format!("ERROR: {content}")
                                } else {
                                    content.clone()
                                };
                                messages.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}));
                            }
                            Block::Text { text: t } => text.push(t.clone()),
                            Block::ToolUse { .. } => {}
                        }
                    }
                    if !text.is_empty() {
                        messages.push(json!({"role": "user", "content": text.join("\n")}));
                    }
                }
                ChatRole::Assistant => {
                    let calls: Vec<Value> = m
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            Block::ToolUse { id, name, input } => Some(json!({
                                "id": id, "type": "function",
                                "function": {"name": name, "arguments": input.to_string()}
                            })),
                            _ => None,
                        })
                        .collect();
                    let mut msg = json!({"role": "assistant", "content": m.text()});
                    if !calls.is_empty() {
                        msg["tool_calls"] = Value::Array(calls);
                    }
                    messages.push(msg);
                }
            }
        }
        let mut body = json!({"model": self.model, "stream": true, "messages": messages});
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.input_schema
                    }})
                })
                .collect();
            body["tool_choice"] = json!("auto");
        }
        body
    }
}

#[derive(Default)]
struct CallAcc {
    id: String,
    name: String,
    args: String,
}

#[async_trait::async_trait]
impl Provider for OpenAiCompatProvider {
    fn name(&self) -> String {
        format!("{}:{}", self.kind, self.model)
    }

    async fn turn(
        &self,
        req: TurnRequest<'_>,
        on_event: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<TurnResponse> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut r = self.http.post(url).json(&self.request_body(&req));
        if let Some(k) = &self.api_key {
            r = r.bearer_auth(k);
        }
        let resp = r.send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(LlmError::Api {
                status,
                message: resp.text().await.unwrap_or_default(),
            });
        }
        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::new();
        let mut text = String::new();
        let mut calls: BTreeMap<u64, CallAcc> = BTreeMap::new();
        let mut finish: Option<String> = None;
        let mut handle = |data: &str| -> Result<()> {
            if data.trim() == "[DONE]" {
                return Ok(());
            }
            let v: Value = serde_json::from_str(data)?;
            if let Some(err) = v.get("error") {
                return Err(LlmError::Stream(err.to_string()));
            }
            let choice = &v["choices"][0];
            if let Some(t) = choice["delta"]["content"].as_str()
                && !t.is_empty()
            {
                text.push_str(t);
                on_event(StreamEvent::TextDelta(t.to_string()));
            }
            if let Some(tc) = choice["delta"]["tool_calls"].as_array() {
                for c in tc {
                    let index = c["index"].as_u64().unwrap_or(0);
                    let acc = calls.entry(index).or_default();
                    if let Some(id) = c["id"].as_str() {
                        acc.id = id.to_string();
                    }
                    if let Some(n) = c["function"]["name"].as_str() {
                        acc.name.push_str(n);
                    }
                    if let Some(a) = c["function"]["arguments"].as_str() {
                        acc.args.push_str(a);
                    }
                    on_event(StreamEvent::ToolInputDelta {
                        index: index as usize,
                    });
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                finish = Some(f.to_string());
            }
            Ok(())
        };
        while let Some(chunk) = stream.next().await {
            for ev in parser.push(&chunk?) {
                handle(&ev.data)?;
            }
        }
        if let Some(ev) = parser.finish() {
            handle(&ev.data)?;
        }
        let mut blocks = Vec::new();
        if !text.is_empty() {
            blocks.push(Block::Text { text });
        }
        let mut tool_calls = Vec::new();
        for (i, c) in calls {
            let id = if c.id.is_empty() {
                format!("call_{i}")
            } else {
                c.id
            };
            let parsed = if c.args.trim().is_empty() {
                Ok(json!({}))
            } else {
                serde_json::from_str::<Value>(&c.args).map_err(|_| c.args.clone())
            };
            let input = match &parsed {
                Ok(v) if v.is_object() => v.clone(),
                _ => json!({}),
            };
            blocks.push(Block::ToolUse {
                id: id.clone(),
                name: c.name.clone(),
                input,
            });
            tool_calls.push(ToolCall {
                id,
                name: c.name,
                input: parsed,
            });
        }
        let stop_reason = match finish.as_deref() {
            Some("length") => StopReason::MaxTokens,
            Some("tool_calls") => StopReason::ToolUse,
            _ if !tool_calls.is_empty() => StopReason::ToolUse,
            Some("content_filter") => StopReason::Refusal,
            _ => StopReason::EndTurn,
        };
        Ok(TurnResponse {
            message: ChatMessage {
                role: ChatRole::Assistant,
                blocks,
                raw: None,
            },
            stop_reason,
            tool_calls,
            refusal: None,
        })
    }
}
