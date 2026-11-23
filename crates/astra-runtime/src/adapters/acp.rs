//! Agent Client Protocol adapter (JSON-RPC 2.0, newline-delimited, over the agent's stdio).
//!
//! Implements the client side of ACP v1: `initialize`, `session/new`, `session/prompt`, `session/cancel`;
//! serves the agent's `session/request_permission`, `fs/read_text_file`, `fs/write_text_file` and
//! `terminal/*` requests (confined to the worktree, commands subject to the agent's command policy), and maps
//! `session/update` notifications to agent events.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use astra_llm::Workspace;
use astra_protocol::{LogLine, PlanState, PlanStep};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

use super::*;
use crate::policy::command_needs_approval;
use crate::process::{Supervised, pump_lines};

/// ACP protocol version spoken by this client.
pub const PROTOCOL_VERSION: u64 = 1;
/// Default byte limit of terminal output kept.
pub const TERMINAL_OUTPUT_LIMIT: usize = 1024 * 1024;

/// Spawns `command args` as an ACP agent.
pub struct AcpAdapter {
    pub command: String,
    pub args: Vec<String>,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;

struct Rpc {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
}

impl Rpc {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.out
            .send(msg.to_string())
            .map_err(|_| "agent stdin closed".to_string())?;
        rx.await.map_err(|_| "agent exited".to_string())?
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self
            .out
            .send(json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string());
    }

    fn respond(&self, id: Value, result: Result<Value, (i64, String)>) {
        let msg = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        };
        let _ = self.out.send(msg.to_string());
    }

    fn fail_all(&self) {
        if let Ok(mut p) = self.pending.lock() {
            for (_, tx) in p.drain() {
                let _ = tx.send(Err("agent exited".into()));
            }
        }
    }
}

struct Terminal {
    output: Arc<Mutex<(String, bool)>>,
    exit: watch::Receiver<Option<Value>>,
    kill: Option<oneshot::Sender<()>>,
}

struct Shared {
    ctx: AgentContext,
    workspace: Workspace,
    rpc: Rpc,
    events: mpsc::UnboundedSender<AgentEvent>,
    terminals: Mutex<HashMap<String, Terminal>>,
    next_terminal: AtomicU64,
    pre_approved: AtomicUsize,
    logged: Mutex<HashSet<(String, String)>>,
}

impl Shared {
    fn log(&self, verb: &str, target: &str) {
        let key = (verb.to_string(), target.to_string());
        if self
            .logged
            .lock()
            .map(|mut l| l.insert(key))
            .unwrap_or(true)
        {
            let _ = self.events.send(AgentEvent::Log(LogLine {
                verb: verb.into(),
                target: target.into(),
            }));
        }
    }

    fn rel(&self, p: &Path) -> String {
        self.workspace.display(p)
    }
}

impl AgentAdapter for AcpAdapter {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError> {
        let workspace = Workspace::new(&ctx.worktree)
            .map_err(|e| AdapterError::Other(format!("worktree: {e}")))?;
        let mut proc = Supervised::spawn(&self.command, &self.args, workspace.root())
            .map_err(|e| AdapterError::Spawn(self.command.clone(), e.to_string()))?;
        let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

        let stdin = proc.child.stdin.take();
        let stdout = proc.child.stdout.take();
        if let Some(err) = proc.child.stderr.take() {
            pump_lines(err, ctx.logs.clone(), "stderr: ");
        }
        let (Some(mut stdin), Some(stdout)) = (stdin, stdout) else {
            return Err(AdapterError::Other("agent stdio unavailable".into()));
        };
        let logs = ctx.logs.clone();
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                logs.push(format!("> {line}"));
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        let shared = Arc::new(Shared {
            ctx,
            workspace,
            rpc: Rpc {
                out: out_tx,
                pending: Mutex::new(HashMap::new()),
                next: AtomicU64::new(0),
            },
            events: ev_tx,
            terminals: Mutex::new(HashMap::new()),
            next_terminal: AtomicU64::new(1),
            pre_approved: AtomicUsize::new(0),
            logged: Mutex::new(HashSet::new()),
        });

        // Reader: responses, requests from the agent, notifications.
        let reader_shared = shared.clone();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                reader_shared.ctx.logs.push(format!("< {line}"));
                let Ok(msg) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                handle_incoming(&reader_shared, msg);
            }
            reader_shared.rpc.fail_all();
        });

        tokio::spawn(drive(shared, proc, ctl_rx, reader));
        Ok(AgentSession {
            control: ctl_tx,
            events: ev_rx,
        })
    }
}

fn handle_incoming(shared: &Arc<Shared>, msg: Value) {
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = msg.get("id").cloned();
    match (method, id) {
        (Some(method), Some(id)) => {
            let s = shared.clone();
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            tokio::spawn(async move {
                let result = handle_request(&s, &method, params).await;
                s.rpc.respond(id, result);
            });
        }
        (Some(method), None) => {
            if method == "session/update" {
                handle_update(shared, &msg["params"]["update"]);
            }
        }
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else { return };
            let tx = shared
                .rpc
                .pending
                .lock()
                .ok()
                .and_then(|mut p| p.remove(&id));
            if let Some(tx) = tx {
                let r = match msg.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(r);
            }
        }
        (None, None) => {}
    }
}

fn handle_update(shared: &Shared, update: &Value) {
    match update["sessionUpdate"].as_str().unwrap_or_default() {
        "agent_message_chunk" => {
            if update["content"]["type"] == "text"
                && let Some(t) = update["content"]["text"].as_str()
            {
                let _ = shared.events.send(AgentEvent::Text(t.to_string()));
            }
        }
        "plan" => {
            let steps = update["entries"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|e| PlanStep {
                            text: e["content"].as_str().unwrap_or_default().to_string(),
                            state: match e["status"].as_str() {
                                Some("completed") => PlanState::Done,
                                Some("in_progress") => PlanState::Active,
                                _ => PlanState::Pending,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            let _ = shared.events.send(AgentEvent::Plan(steps));
        }
        "tool_call" => {
            let verb = match update["kind"].as_str().unwrap_or("other") {
                "read" => "read",
                "edit" | "delete" | "move" => "edit",
                "execute" => "run",
                "search" => "search",
                "fetch" => "fetch",
                _ => return,
            };
            let target = update["locations"][0]["path"]
                .as_str()
                .map(|p| shared.rel(Path::new(p)))
                .or_else(|| raw_command(&update["rawInput"]))
                .or_else(|| update["title"].as_str().map(str::to_string))
                .unwrap_or_default();
            shared.log(verb, &target);
        }
        _ => {}
    }
}

fn raw_command(raw: &Value) -> Option<String> {
    match &raw["command"] {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

type RpcResult = Result<Value, (i64, String)>;

fn invalid(msg: impl Into<String>) -> (i64, String) {
    (-32602, msg.into())
}

async fn handle_request(s: &Arc<Shared>, method: &str, p: Value) -> RpcResult {
    match method {
        "session/request_permission" => {
            let tool = &p["toolCall"];
            let command = if tool["kind"] == "execute" {
                raw_command(&tool["rawInput"])
            } else {
                None
            };
            let cwd = tool["rawInput"]["cwd"].as_str().map(PathBuf::from);
            let title = tool["title"]
                .as_str()
                .unwrap_or("The agent asks for permission")
                .to_string();
            let needs = match &command {
                Some(_) => {
                    command_needs_approval(s.ctx.command_policy, cwd.as_deref(), s.workspace.root())
                }
                None => s.ctx.command_policy != astra_protocol::CommandPolicy::Auto,
            };
            let allowed = if needs {
                s.ctx
                    .approver
                    .request(PermissionRequest {
                        prompt: title,
                        command: command.clone(),
                        cwd,
                    })
                    .await
            } else {
                true
            };
            if allowed && command.is_some() {
                s.pre_approved.fetch_add(1, Ordering::SeqCst);
            }
            let options = p["options"].as_array().cloned().unwrap_or_default();
            let pick = |kinds: &[&str]| {
                kinds.iter().find_map(|k| {
                    options
                        .iter()
                        .find(|o| o["kind"] == *k)
                        .and_then(|o| o["optionId"].as_str())
                })
            };
            let choice = if allowed {
                pick(&["allow_once", "allow_always"])
            } else {
                pick(&["reject_once", "reject_always"])
            };
            Ok(match choice {
                Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
                None => json!({"outcome": {"outcome": "cancelled"}}),
            })
        }
        "fs/read_text_file" => {
            let path = p["path"].as_str().ok_or_else(|| invalid("path required"))?;
            let full = s.workspace.resolve(path).map_err(invalid)?;
            let text = tokio::fs::read_to_string(&full)
                .await
                .map_err(|e| (-32000, format!("{path}: {e}")))?;
            let line = p["line"].as_u64().unwrap_or(1).max(1) as usize;
            let content = match p["limit"].as_u64() {
                Some(limit) => text
                    .lines()
                    .skip(line - 1)
                    .take(limit as usize)
                    .collect::<Vec<_>>()
                    .join("\n"),
                None if line > 1 => text.lines().skip(line - 1).collect::<Vec<_>>().join("\n"),
                None => text,
            };
            s.log("read", &s.rel(&full));
            Ok(json!({"content": content}))
        }
        "fs/write_text_file" => {
            let path = p["path"].as_str().ok_or_else(|| invalid("path required"))?;
            let content = p["content"]
                .as_str()
                .ok_or_else(|| invalid("content required"))?;
            let full = s.workspace.resolve(path).map_err(invalid)?;
            if let Some(dir) = full.parent() {
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| (-32000, e.to_string()))?;
            }
            tokio::fs::write(&full, content)
                .await
                .map_err(|e| (-32000, format!("{path}: {e}")))?;
            s.log("edit", &s.rel(&full));
            Ok(Value::Null)
        }
        "terminal/create" => create_terminal(s, &p).await,
        "terminal/output" => {
            let t = terminal_parts(s, &p)?;
            let (output, truncated) = t.0.lock().map(|o| o.clone()).unwrap_or_default();
            let exit = t.1.borrow().clone();
            Ok(json!({"output": output, "truncated": truncated, "exitStatus": exit}))
        }
        "terminal/wait_for_exit" => {
            let mut rx = terminal_parts(s, &p)?.1;
            loop {
                if let Some(st) = rx.borrow().clone() {
                    return Ok(st);
                }
                if rx.changed().await.is_err() {
                    return Ok(json!({"exitCode": null, "signal": "SIGKILL"}));
                }
            }
        }
        "terminal/kill" | "terminal/release" => {
            let id = p["terminalId"]
                .as_str()
                .ok_or_else(|| invalid("terminalId required"))?;
            let mut terms = s
                .terminals
                .lock()
                .map_err(|_| (-32603, "poisoned".to_string()))?;
            if method == "terminal/release" {
                if let Some(mut t) = terms.remove(id)
                    && let Some(k) = t.kill.take()
                {
                    let _ = k.send(());
                }
            } else if let Some(t) = terms.get_mut(id)
                && let Some(k) = t.kill.take()
            {
                let _ = k.send(());
            }
            Ok(Value::Null)
        }
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

type TerminalParts = (Arc<Mutex<(String, bool)>>, watch::Receiver<Option<Value>>);

fn terminal_parts(s: &Shared, p: &Value) -> Result<TerminalParts, (i64, String)> {
    let id = p["terminalId"]
        .as_str()
        .ok_or_else(|| invalid("terminalId required"))?;
    let terms = s
        .terminals
        .lock()
        .map_err(|_| (-32603, "poisoned".to_string()))?;
    let t = terms
        .get(id)
        .ok_or_else(|| invalid(format!("unknown terminal {id}")))?;
    Ok((t.output.clone(), t.exit.clone()))
}

async fn create_terminal(s: &Arc<Shared>, p: &Value) -> RpcResult {
    let command = p["command"]
        .as_str()
        .ok_or_else(|| invalid("command required"))?
        .to_string();
    let args: Vec<String> = p["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let cwd = match p["cwd"].as_str() {
        Some(c) => PathBuf::from(c),
        None => s.workspace.root().to_path_buf(),
    };
    let line = if args.is_empty() {
        command.clone()
    } else {
        format!("{command} {}", args.join(" "))
    };
    if command_needs_approval(s.ctx.command_policy, Some(&cwd), s.workspace.root()) {
        let pre = s
            .pre_approved
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        let inside = crate::policy::within(&cwd, s.workspace.root());
        // A prior approval only covers commands inside the worktree.
        let allowed = (pre && inside)
            || s.ctx
                .approver
                .request(PermissionRequest {
                    prompt: format!("Run `{line}`"),
                    command: Some(line.clone()),
                    cwd: Some(cwd.clone()),
                })
                .await;
        if !allowed {
            return Err((-32000, "the user denied running this command".into()));
        }
    }
    if !cwd.is_dir() {
        return Err(invalid(format!("cwd does not exist: {}", cwd.display())));
    }
    let (program, argv) = if args.is_empty() && command.contains(char::is_whitespace) {
        ("sh".to_string(), vec!["-c".to_string(), command.clone()])
    } else {
        (command.clone(), args)
    };
    let mut proc = Supervised::spawn(&program, &argv, &cwd)
        .map_err(|e| (-32000, format!("spawn {program}: {e}")))?;
    drop(proc.child.stdin.take());
    let limit = p["outputByteLimit"]
        .as_u64()
        .map(|l| l as usize)
        .unwrap_or(TERMINAL_OUTPUT_LIMIT);
    let output = Arc::new(Mutex::new((String::new(), false)));
    for reader in [
        proc.child
            .stdout
            .take()
            .map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        proc.child
            .stderr
            .take()
            .map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let output = output.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if let Ok(mut o) = output.lock() {
                    o.0.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if o.0.len() > limit {
                        let mut cut = o.0.len() - limit;
                        while !o.0.is_char_boundary(cut) {
                            cut += 1;
                        }
                        o.0.drain(..cut);
                        o.1 = true;
                    }
                }
            }
        });
    }
    let (exit_tx, exit_rx) = watch::channel(None);
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            status = proc.child.wait() => {
                let v = match status {
                    Ok(st) => json!({"exitCode": st.code(), "signal": exit_signal(&st)}),
                    Err(_) => json!({"exitCode": null, "signal": null}),
                };
                let _ = exit_tx.send(Some(v));
            }
            _ = kill_rx => {
                proc.kill();
                let _ = exit_tx.send(Some(json!({"exitCode": null, "signal": "SIGKILL"})));
            }
        }
    });
    let id = format!("term-{}", s.next_terminal.fetch_add(1, Ordering::Relaxed));
    if let Ok(mut t) = s.terminals.lock() {
        t.insert(
            id.clone(),
            Terminal {
                output,
                exit: exit_rx,
                kill: Some(kill_tx),
            },
        );
    }
    s.log("run", &line);
    Ok(json!({"terminalId": id}))
}

#[cfg(unix)]
fn exit_signal(st: &std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    st.signal().map(|s| format!("SIG{s}"))
}

#[cfg(not(unix))]
fn exit_signal(_st: &std::process::ExitStatus) -> Option<String> {
    None
}

async fn drive(
    s: Arc<Shared>,
    mut proc: Supervised,
    mut ctl: mpsc::UnboundedReceiver<AgentControl>,
    reader: tokio::task::JoinHandle<()>,
) {
    let result = async {
        let init = s
            .rpc
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": true},
                    "clientInfo": {"name": "astra", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        if init["protocolVersion"].as_u64().is_some_and(|v| v != PROTOCOL_VERSION) {
            return Err(format!("unsupported ACP protocol version {}", init["protocolVersion"]));
        }
        let session = s
            .rpc
            .request("session/new", json!({"cwd": s.workspace.root().to_string_lossy(), "mcpServers": []}))
            .await?;
        let session_id = session["sessionId"].as_str().ok_or("session/new returned no sessionId")?.to_string();
        let mut prompt = Some(s.ctx.prompt.clone());
        let mut queued: Vec<String> = Vec::new();
        loop {
            let Some(text) = prompt.take() else {
                match ctl.recv().await {
                    Some(AgentControl::Message(m)) => prompt = Some(m),
                    Some(AgentControl::Pause) => proc.pause(),
                    Some(AgentControl::Resume) => proc.resume(),
                    Some(AgentControl::Cancel) | None => return Ok(()),
                }
                continue;
            };
            if let Ok(mut l) = s.logged.lock() {
                l.clear();
            }
            let req = s.rpc.request(
                "session/prompt",
                json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
            );
            tokio::pin!(req);
            let outcome = loop {
                tokio::select! {
                    r = &mut req => break r,
                    c = ctl.recv() => match c {
                        Some(AgentControl::Message(m)) => queued.push(m),
                        Some(AgentControl::Pause) => proc.pause(),
                        Some(AgentControl::Resume) => proc.resume(),
                        Some(AgentControl::Cancel) | None => {
                            s.rpc.notify("session/cancel", json!({"sessionId": session_id}));
                            return Ok(());
                        }
                    }
                }
            };
            let result = outcome?;
            if result["stopReason"] == "refusal" {
                let _ = s.events.send(AgentEvent::Error("the agent refused".into()));
            } else {
                let _ = s.events.send(AgentEvent::TurnDone);
            }
            if !queued.is_empty() {
                prompt = Some(std::mem::take(&mut queued).join("\n\n"));
            }
        }
    }
    .await;
    if let Err(e) = result {
        let _ = s.events.send(AgentEvent::Error(e));
    }
    proc.kill();
    reader.abort();
    if let Ok(mut t) = s.terminals.lock() {
        for (_, mut term) in t.drain() {
            if let Some(k) = term.kill.take() {
                let _ = k.send(());
            }
        }
    }
    let _ = s.events.send(AgentEvent::Exited);
}
