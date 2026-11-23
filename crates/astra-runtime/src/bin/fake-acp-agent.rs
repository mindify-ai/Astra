//! A tiny ACP agent for tests. It speaks just enough of the Agent Client Protocol to: send a plan update,
//! write a file in the worktree through the client, ask permission to run a command, run it through a client
//! terminal when allowed, and finish the turn.
//!
//! Env: `FAKE_ACP_FILE` (default `hello.txt`), `FAKE_ACP_CONTENT` (default `hello from the fake agent\n`).

use std::io::{BufRead, Write};

use serde_json::{Value, json};

struct Io {
    lines: std::io::Lines<std::io::StdinLock<'static>>,
    next_id: u64,
}

impl Io {
    fn send(&self, v: Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }

    fn notify(&self, session: &str, update: Value) {
        self.send(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":update}}));
    }

    /// Send a request and wait for its response (exits if the client cancels or closes stdin).
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let Some(Ok(line)) = self.lines.next() else { std::process::exit(0) };
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if msg["method"] == "session/cancel" {
                std::process::exit(0);
            }
            if msg["id"].as_u64() == Some(id) && msg.get("method").is_none() {
                return match msg.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(msg["result"].clone()),
                };
            }
        }
    }
}

fn main() {
    let stdin: &'static std::io::Stdin = Box::leak(Box::new(std::io::stdin()));
    let mut io = Io { lines: stdin.lock().lines(), next_id: 100 };
    let file = std::env::var("FAKE_ACP_FILE").unwrap_or_else(|_| "hello.txt".into());
    let content = std::env::var("FAKE_ACP_CONTENT").unwrap_or_else(|_| "hello from the fake agent\n".into());
    let mut cwd = String::new();
    let mut turns = 0;
    while let Some(Ok(line)) = io.lines.next() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg["id"].clone();
        match msg["method"].as_str().unwrap_or_default() {
            "initialize" => io.send(json!({"jsonrpc":"2.0","id":id,"result":{
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": false, "promptCapabilities": {}},
                "authMethods": []
            }})),
            "session/new" => {
                cwd = msg["params"]["cwd"].as_str().unwrap_or(".").to_string();
                io.send(json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":"fake-1"}}));
            }
            "session/prompt" => {
                turns += 1;
                let s = "fake-1";
                let text = msg["params"]["prompt"][0]["text"].as_str().unwrap_or_default().to_string();
                if turns > 1 {
                    io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("Got: {text}")}}));
                    io.send(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn"}}));
                    continue;
                }
                io.notify(s, json!({"sessionUpdate":"plan","entries":[
                    {"content":"Read the code","priority":"high","status":"completed"},
                    {"content":format!("Write {file}"),"priority":"high","status":"in_progress"},
                    {"content":"Run a check","priority":"medium","status":"pending"}
                ]}));
                io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Working on it. "}}));
                let path = format!("{cwd}/{file}");
                io.notify(s, json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":format!("Write {file}"),
                    "kind":"edit","status":"pending","locations":[{"path":path}]}));
                if let Err(e) = io.request("fs/write_text_file", json!({"sessionId":s,"path":path,"content":content})) {
                    io.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":format!("write failed: {e}")}}));
                    continue;
                }
                // An escape attempt must be refused by the client.
                let escaped = io.request("fs/write_text_file", json!({"sessionId":s,"path":format!("{cwd}/../escape.txt"),"content":"x"}));
                if escaped.is_ok() {
                    io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ESCAPED "}}));
                }
                let cmd = "echo checked > check.txt";
                let perm = io.request("session/request_permission", json!({"sessionId":s,
                    "toolCall":{"toolCallId":"t2","title":format!("Run `{cmd}`"),"kind":"execute","status":"pending","rawInput":{"command":cmd}},
                    "options":[{"optionId":"allow","name":"Allow","kind":"allow_once"},{"optionId":"reject","name":"Reject","kind":"reject_once"}]}));
                let allowed = matches!(&perm, Ok(r) if r["outcome"]["outcome"] == "selected" && r["outcome"]["optionId"] == "allow");
                if allowed {
                    let term = io.request("terminal/create", json!({"sessionId":s,"command":"sh","args":["-c",cmd],"cwd":cwd}));
                    match term {
                        Ok(t) => {
                            let tid = t["terminalId"].clone();
                            let _ = io.request("terminal/wait_for_exit", json!({"sessionId":s,"terminalId":tid}));
                            let _ = io.request("terminal/output", json!({"sessionId":s,"terminalId":tid}));
                            let _ = io.request("terminal/release", json!({"sessionId":s,"terminalId":tid}));
                            io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Check passed."}}));
                        }
                        Err(_) => io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Terminal refused."}})),
                    }
                } else {
                    io.notify(s, json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Command was denied."}}));
                }
                io.send(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn"}}));
            }
            "session/cancel" => std::process::exit(0),
            _ => {
                if !id.is_null() {
                    io.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}));
                }
            }
        }
    }
}
