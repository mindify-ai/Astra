//! LangGraph adapter against a mock LangGraph Server.

use std::sync::Arc;

use astra_protocol::{CommandPolicy, Role};
use astra_runtime::MemorySecrets;
use astra_runtime::adapters::langgraph::LangGraphAdapter;
use astra_runtime::adapters::{
    AgentAdapter, AgentContext, AgentControl, AgentEvent, Approver, PermissionRequest,
};
use astra_runtime::process::LogRing;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct No;
#[async_trait::async_trait]
impl Approver for No {
    async fn request(&self, _req: PermissionRequest) -> bool {
        false
    }
}

#[tokio::test]
async fn streams_values_into_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/threads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"thread_id": "th-1"})))
        .mount(&server)
        .await;
    let sse = [
        "event: metadata\ndata: {\"run_id\":\"r1\"}\n\n".to_string(),
        format!("event: values\ndata: {}\n\n", json!({"messages":[{"type":"human","content":"go"}]})),
        format!("event: values\ndata: {}\n\n", json!({"messages":[{"type":"human","content":"go"},{"type":"ai","content":"Tests "}]})),
        format!("event: values\ndata: {}\n\n", json!({"messages":[{"type":"human","content":"go"},{"type":"ai","content":"Tests pass."}]})),
        "event: end\ndata: null\n\n".to_string(),
    ]
    .concat();
    Mock::given(method("POST"))
        .and(path("/threads/th-1/runs/stream"))
        .and(body_partial_json(
            json!({"assistant_id": "qa-graph", "stream_mode": ["values"]}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tmp");
    let adapter = LangGraphAdapter {
        endpoint: server.uri(),
        graph: "qa-graph".into(),
    };
    let mut session = adapter
        .start(AgentContext {
            mission_agent_id: "ma".into(),
            role: Role::Tester,
            prompt: "Run the tests".into(),
            worktree: tmp.path().to_path_buf(),
            command_policy: CommandPolicy::AskMe,
            approver: Arc::new(No),
            secrets: Arc::new(MemorySecrets::new()),
            logs: LogRing::new(10),
        })
        .expect("start");
    let mut text = String::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), session.events.recv())
            .await
            .expect("event")
        {
            Some(AgentEvent::Text(t)) => text.push_str(&t),
            Some(AgentEvent::TurnDone) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(text, "Tests pass.");
    session.control.send(AgentControl::Cancel).expect("cancel");
    assert_eq!(session.events.recv().await, Some(AgentEvent::Exited));
}
