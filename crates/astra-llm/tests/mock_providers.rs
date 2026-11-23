//! Provider + agent loop tests against a local mock HTTP server replaying canned SSE.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use astra_llm::*;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct Sequence {
    bodies: Vec<String>,
    n: AtomicUsize,
}

impl Respond for Sequence {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        let i = self
            .n
            .fetch_add(1, Ordering::SeqCst)
            .min(self.bodies.len() - 1);
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(self.bodies[i].clone())
    }
}

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for e in events {
        out.push_str(&format!(
            "event: {}\ndata: {}\n\n",
            e["type"].as_str().expect("type"),
            e
        ));
    }
    out
}

fn start() -> Value {
    json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"m","stop_reason":null,"usage":{"input_tokens":1,"output_tokens":1}}})
}

fn text_turn(text: &str) -> String {
    let (a, b) = text.split_at(text.len() / 2);
    sse(&[
        start(),
        json!({"type":"ping"}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":a}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":b}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_details":null},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ])
}

fn tool_turn(id: &str, name: &str, input_parts: &[&str], stop_reason: &str) -> String {
    let mut ev = vec![
        start(),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-1"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":id,"name":name,"input":{}}}),
    ];
    for p in input_parts {
        ev.push(json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":p}}));
    }
    ev.push(json!({"type":"content_block_stop","index":1}));
    let details = if stop_reason == "refusal" {
        json!({"type":"refusal","category":"cyber","explanation":"declined"})
    } else {
        Value::Null
    };
    ev.push(json!({"type":"message_delta","delta":{"stop_reason":stop_reason,"stop_details":details},"usage":{"output_tokens":5}}));
    ev.push(json!({"type":"message_stop"}));
    sse(&ev)
}

async fn mock(bodies: Vec<String>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(header("anthropic-version", "2026-10-21"))
        .and(header("anthropic-beta", "server-side-fallback-2026-07-01"))
        .respond_with(Sequence {
            bodies,
            n: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;
    server
}

fn provider(server: &MockServer) -> Arc<dyn Provider> {
    let mut cfg = AnthropicConfig::new("test-key");
    cfg.base_url = server.uri();
    Arc::new(AnthropicProvider::new(cfg).expect("provider"))
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| !r.body.is_empty())
        .map(|r| serde_json::from_slice(&r.body).expect("json body"))
        .collect()
}

fn agent(
    server: &MockServer,
    dir: &std::path::Path,
    approver: Arc<dyn CommandApprover>,
) -> AgentLoop {
    let tools = Toolset::new(Workspace::new(dir).expect("ws"), approver);
    AgentLoop::new(provider(server), tools, astra_llm::agent::DEFAULT_SYSTEM).expect("loop")
}

#[tokio::test]
async fn text_only_turn_and_request_shape() {
    let server = mock(vec![text_turn("Hello there")]).await;
    let dir = tempfile::tempdir().expect("tmp");
    let mut a = agent(&server, dir.path(), Arc::new(DenyAllCommands));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let out = a.run("hi", &tx).await.expect("run");
    assert_eq!(
        out,
        LoopOutcome::Done {
            text: "Hello there".into()
        }
    );
    let mut deltas = String::new();
    while let Ok(e) = rx.try_recv() {
        if let LoopEvent::TextDelta(t) = e {
            deltas.push_str(&t);
        }
    }
    assert_eq!(deltas, "Hello there");

    let body = &bodies(&server).await[0];
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["max_tokens"], 64000);
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"], json!({"type":"adaptive"}));
    assert_eq!(body["output_config"], json!({"effort":"high"}));
    assert_eq!(body["fallbacks"], "default");
    assert_eq!(body["tool_choice"], json!({"type":"auto"}));
    for forbidden in ["temperature", "top_p", "top_k"] {
        assert!(body.get(forbidden).is_none());
    }
    assert_eq!(
        body["system"][0]["cache_control"],
        json!({"type":"ephemeral"})
    );
    let tools = body["tools"].as_array().expect("tools");
    assert!(tools.iter().all(|t| t["eager_input_streaming"] == true));
    assert!(
        tools[..tools.len() - 1]
            .iter()
            .all(|t| t.get("cache_control").is_none())
    );
    assert_eq!(
        tools[tools.len() - 1]["cache_control"],
        json!({"type":"ephemeral"})
    );
    // Thinking block (with signature) kept in history unchanged.
    let raw = &a.history()[1].raw.as_ref().expect("raw").1;
    assert_eq!(
        raw[0],
        json!({"type":"thinking","thinking":"","signature":"sig-abc"})
    );
}

#[tokio::test]
async fn tool_use_then_result_loop() {
    let server = mock(vec![
        tool_turn(
            "toolu_1",
            "write_file",
            &["{\"path\": \"notes/a.txt\", ", "\"content\": \"hi\"}"],
            "tool_use",
        ),
        text_turn("Wrote the file"),
    ])
    .await;
    let dir = tempfile::tempdir().expect("tmp");
    let mut a = agent(&server, dir.path(), Arc::new(DenyAllCommands));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let out = a.run("write a note", &tx).await.expect("run");
    assert_eq!(
        out,
        LoopOutcome::Done {
            text: "Wrote the file".into()
        }
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes/a.txt")).expect("file"),
        "hi"
    );
    let mut saw_log = false;
    while let Ok(e) = rx.try_recv() {
        if let LoopEvent::ToolFinished {
            log: Some((verb, target)),
            is_error: false,
            ..
        } = e
        {
            saw_log = verb == "edit" && target == "notes/a.txt";
        }
    }
    assert!(saw_log);

    let reqs = bodies(&server).await;
    assert_eq!(reqs.len(), 2);
    let msgs = reqs[1]["messages"].as_array().expect("messages");
    assert_eq!(msgs.len(), 3);
    // Assistant content replayed verbatim, thinking block included.
    assert_eq!(
        msgs[1]["content"][0],
        json!({"type":"thinking","thinking":"","signature":"sig-1"})
    );
    assert_eq!(
        msgs[1]["content"][1]["input"],
        json!({"path":"notes/a.txt","content":"hi"})
    );
    // One user message with the tool result.
    assert_eq!(msgs[2]["role"], "user");
    assert_eq!(msgs[2]["content"].as_array().expect("content").len(), 1);
    assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(msgs[2]["content"][0]["is_error"], false);
}

#[tokio::test]
async fn invalid_tool_input_returns_is_error() {
    let server = mock(vec![
        // Missing required `content`, plus an unknown property.
        tool_turn(
            "toolu_2",
            "write_file",
            &["{\"path\": \"x.txt\", \"bogus\": 1}"],
            "tool_use",
        ),
        // Truncated / invalid JSON.
        tool_turn(
            "toolu_3",
            "write_file",
            &["{\"path\": \"y.txt\", \"content\": \"unterminated"],
            "tool_use",
        ),
        text_turn("Giving up"),
    ])
    .await;
    let dir = tempfile::tempdir().expect("tmp");
    let mut a = agent(&server, dir.path(), Arc::new(DenyAllCommands));
    let (tx, _rx) = mpsc::unbounded_channel();
    let out = a.run("go", &tx).await.expect("run");
    assert_eq!(
        out,
        LoopOutcome::Done {
            text: "Giving up".into()
        }
    );
    assert!(!dir.path().join("x.txt").exists());
    assert!(!dir.path().join("y.txt").exists());
    let reqs = bodies(&server).await;
    let r1 = &reqs[1]["messages"][2]["content"][0];
    assert_eq!(r1["is_error"], true);
    assert!(r1["content"].as_str().expect("s").contains("bogus"), "{r1}");
    let r2 = &reqs[2]["messages"][4]["content"][0];
    assert_eq!(r2["is_error"], true);
    assert!(
        r2["content"].as_str().expect("s").contains("unterminated"),
        "{r2}"
    );
    // The unparseable tool_use is replayed with an object input.
    assert_eq!(reqs[2]["messages"][3]["content"][1]["input"], json!({}));
}

#[tokio::test]
async fn refusal_stops_without_running_tools() {
    let server = mock(vec![tool_turn(
        "toolu_4",
        "write_file",
        &["{\"path\":\"r.txt\",\"content\":\"x\"}"],
        "refusal",
    )])
    .await;
    let dir = tempfile::tempdir().expect("tmp");
    let mut a = agent(&server, dir.path(), Arc::new(DenyAllCommands));
    let (tx, _rx) = mpsc::unbounded_channel();
    let out = a.run("go", &tx).await.expect("run");
    assert_eq!(
        out,
        LoopOutcome::Refused {
            explanation: Some("declined".into())
        }
    );
    assert!(!dir.path().join("r.txt").exists());
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn max_tokens_with_tool_use_is_not_run_and_retried() {
    let server = mock(vec![
        tool_turn(
            "toolu_5",
            "write_file",
            &["{\"path\":\"m.txt\",\"content\":\"partial"],
            "max_tokens",
        ),
        text_turn("ok"),
    ])
    .await;
    let dir = tempfile::tempdir().expect("tmp");
    let mut a = agent(&server, dir.path(), Arc::new(DenyAllCommands));
    let (tx, _rx) = mpsc::unbounded_channel();
    assert_eq!(
        a.run("go", &tx).await.expect("run"),
        LoopOutcome::Done { text: "ok".into() }
    );
    assert!(!dir.path().join("m.txt").exists());
    let reqs = bodies(&server).await;
    let note = &reqs[1]["messages"][2]["content"][0];
    assert_eq!(note["is_error"], true);
    assert!(note["content"].as_str().expect("s").contains("max_tokens"));
}

#[tokio::test]
async fn api_errors_surface_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"type":"error","error":{"type":"invalid_request_error","message":"bad"}}),
        ))
        .mount(&server)
        .await;
    let p = provider(&server);
    let err = p
        .turn(
            TurnRequest {
                system: "s",
                tools: &[],
                messages: &[ChatMessage::user("x")],
            },
            &|_| {},
        )
        .await
        .expect_err("should fail");
    let msg = err.to_string();
    assert!(msg.contains("400") && msg.contains("bad"));
    assert!(!msg.contains("test-key"));
    assert!(!format!("{:?}", AnthropicConfig::new("secret-key")).contains("secret-key"));
}

#[tokio::test]
async fn openai_compat_streaming_tool_call_and_detection() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"qwen3-coder"}]})),
        )
        .mount(&server)
        .await;
    let chunk = |d: Value| format!("data: {}\n\n", json!({"choices":[d]}));
    let first = [
        chunk(json!({"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"write_file","arguments":"{\"path\":"}}]}})),
        chunk(json!({"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"o.txt\",\"content\":\"local\"}"}}]}})),
        chunk(json!({"delta":{},"finish_reason":"tool_calls"})),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat();
    let second = [
        chunk(json!({"delta":{"content":"Done "}})),
        chunk(json!({"delta":{"content":"locally"},"finish_reason":"stop"})),
        "data: [DONE]\n\n".to_string(),
    ]
    .concat();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Sequence {
            bodies: vec![first, second],
            n: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;

    let base = format!("{}/v1", server.uri());
    let found = astra_llm::openai::detect_servers(
        &[
            ("dead".into(), "http://127.0.0.1:9/v1".into()),
            ("mock".into(), base.clone()),
        ],
        std::time::Duration::from_millis(500),
    )
    .await;
    assert_eq!(
        found,
        vec![LocalServer {
            kind: "mock".into(),
            base_url: base.clone(),
            models: vec!["qwen3-coder".into()]
        }]
    );

    let mut rc = RouterConfig::new(RoutingPolicy::Auto, None);
    rc.local_candidates = vec![("mock".into(), base)];
    let p = Router::new(rc).select().await.expect("router picks local");
    assert_eq!(p.name(), "mock:qwen3-coder");

    let dir = tempfile::tempdir().expect("tmp");
    let tools = Toolset::new(
        Workspace::new(dir.path()).expect("ws"),
        Arc::new(DenyAllCommands),
    );
    let mut a = AgentLoop::new(p, tools, "sys").expect("loop");
    let (tx, _rx) = mpsc::unbounded_channel();
    assert_eq!(
        a.run("go", &tx).await.expect("run"),
        LoopOutcome::Done {
            text: "Done locally".into()
        }
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("o.txt")).expect("file"),
        "local"
    );
    let reqs = bodies(&server).await;
    let chat: Vec<&Value> = reqs
        .iter()
        .filter(|b| b.get("messages").is_some())
        .collect();
    let msgs = chat[1]["messages"].as_array().expect("messages");
    assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_a");
    assert_eq!(msgs[3]["role"], "tool");
    assert_eq!(msgs[3]["tool_call_id"], "call_a");
}

#[tokio::test]
async fn router_cloud_policy_requires_key() {
    let r = Router::new(RouterConfig::new(RoutingPolicy::Cloud, None));
    assert!(matches!(r.select().await, Err(LlmError::NoProvider(_))));
    let mut rc = RouterConfig::new(RoutingPolicy::Local, None);
    rc.local_candidates = vec![("dead".into(), "http://127.0.0.1:9/v1".into())];
    assert!(matches!(
        Router::new(rc).select().await,
        Err(LlmError::NoProvider(_))
    ));
}
