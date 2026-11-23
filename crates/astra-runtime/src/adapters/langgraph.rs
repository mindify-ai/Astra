//! LangGraph Server adapter: `POST /threads`, then `POST /threads/{id}/runs/stream` per prompt (SSE,
//! `stream_mode: values`); the newest AI message of each `values` event becomes message text.

use std::time::Duration;

use astra_llm::sse::SseParser;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::*;

/// Talks to a LangGraph Server.
pub struct LangGraphAdapter {
    pub endpoint: String,
    pub graph: String,
}

fn message_text(m: &Value) -> String {
    match &m["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn is_ai(m: &Value) -> bool {
    m["type"] == "ai" || m["role"] == "assistant" || m["type"] == "AIMessageChunk"
}

async fn run_stream(
    http: &reqwest::Client,
    base: &str,
    thread: &str,
    graph: &str,
    prompt: &str,
    events: &mpsc::UnboundedSender<AgentEvent>,
    ctl: &mut mpsc::UnboundedReceiver<AgentControl>,
    queued: &mut Vec<String>,
) -> Result<bool, String> {
    let resp = http
        .post(format!("{base}/threads/{thread}/runs/stream"))
        .json(&json!({
            "assistant_id": graph,
            "input": {"messages": [{"role": "user", "content": prompt}]},
            "stream_mode": ["values"],
        }))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!("LangGraph run failed: HTTP {}", resp.status()));
    }
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::new();
    let mut seen: Option<(usize, String)> = None;
    let mut handle = |ev: astra_llm::sse::SseEvent| -> Result<bool, String> {
        match ev.event.as_deref() {
            Some("error") => return Err(format!("LangGraph error: {}", ev.data)),
            Some("end") => return Ok(true),
            Some("values") | None => {
                let Ok(v) = serde_json::from_str::<Value>(&ev.data) else { return Ok(false) };
                let Some(msgs) = v["messages"].as_array() else { return Ok(false) };
                if let Some((i, m)) = msgs.iter().enumerate().rev().find(|(_, m)| is_ai(m)) {
                    let text = message_text(m);
                    let new = match &seen {
                        Some((si, st)) if *si == i => text.strip_prefix(st.as_str()).map(str::to_string),
                        _ => Some(text.clone()),
                    };
                    match new {
                        Some(delta) if !delta.is_empty() => {
                            let _ = events.send(AgentEvent::Text(delta));
                        }
                        Some(_) => {}
                        None => {
                            // Rewritten message: emit in full as a new message.
                            let _ = events.send(AgentEvent::Text(format!("\n{text}")));
                        }
                    }
                    seen = Some((i, text));
                }
            }
            _ => {}
        }
        Ok(false)
    };
    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    for ev in parser.push(&bytes) {
                        if handle(ev)? {
                            return Ok(true);
                        }
                    }
                }
                Some(Err(e)) => return Err(e.without_url().to_string()),
                None => {
                    if let Some(ev) = parser.finish() {
                        handle(ev)?;
                    }
                    return Ok(true);
                }
            },
            c = ctl.recv() => match c {
                Some(AgentControl::Message(m)) => queued.push(m),
                Some(AgentControl::Pause) | Some(AgentControl::Resume) => {}
                Some(AgentControl::Cancel) | None => return Ok(false),
            }
        }
    }
}

impl AgentAdapter for LangGraphAdapter {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError> {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let base = self.endpoint.trim_end_matches('/').to_string();
        let graph = self.graph.clone();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| AdapterError::Other(e.to_string()))?;
        tokio::spawn(async move {
            let result: Result<(), String> = async {
                let thread: Value = http
                    .post(format!("{base}/threads"))
                    .json(&json!({}))
                    .send()
                    .await
                    .map_err(|e| e.without_url().to_string())?
                    .json()
                    .await
                    .map_err(|e| e.without_url().to_string())?;
                let thread_id = thread["thread_id"].as_str().ok_or("LangGraph returned no thread_id")?.to_string();
                ctx.logs.push(format!("langgraph thread {thread_id}"));
                let mut prompt = Some(ctx.prompt.clone());
                let mut queued = Vec::new();
                loop {
                    let Some(p) = prompt.take() else {
                        match ctl_rx.recv().await {
                            Some(AgentControl::Message(m)) => prompt = Some(m),
                            Some(AgentControl::Cancel) | None => return Ok(()),
                            _ => {}
                        }
                        continue;
                    };
                    let finished =
                        run_stream(&http, &base, &thread_id, &graph, &p, &ev_tx, &mut ctl_rx, &mut queued).await?;
                    if !finished {
                        return Ok(());
                    }
                    let _ = ev_tx.send(AgentEvent::TurnDone);
                    if !queued.is_empty() {
                        prompt = Some(queued.drain(..).collect::<Vec<_>>().join("\n\n"));
                    }
                }
            }
            .await;
            if let Err(e) = result {
                let _ = ev_tx.send(AgentEvent::Error(e));
            }
            let _ = ev_tx.send(AgentEvent::Exited);
        });
        Ok(AgentSession { control: ctl_tx, events: ev_rx })
    }
}
