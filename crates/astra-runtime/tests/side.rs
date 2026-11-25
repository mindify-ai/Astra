//! Side questions: `sendMessage` with `side` is answered in its side thread and never reaches the agents.

mod common;

use std::sync::{Arc, Mutex};

use astra_llm::{
    Block, ChatMessage, ChatRole, Provider, StopReason, StreamEvent, TurnRequest, TurnResponse,
};
use astra_protocol::*;
use astra_runtime::adapters::{
    AdapterError, AgentAdapter, AgentContext, AgentControl, AgentSession,
};
use astra_runtime::{MemorySecrets, Orchestrator, OrchestratorConfig, git};
use common::*;
use tokio::sync::mpsc;

/// What a recording agent was sent: its first prompt, then every message delivered to it.
type Received = Arc<Mutex<Vec<String>>>;

/// An agent that never finishes its turn and records what reaches it.
struct RecordingAdapter {
    received: Received,
}

impl AgentAdapter for RecordingAdapter {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError> {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let received = self.received.clone();
        received.lock().expect("lock").push(ctx.prompt.clone());
        tokio::spawn(async move {
            let _keep = ev_tx;
            while let Some(c) = ctl_rx.recv().await {
                match c {
                    AgentControl::Message(m) => received.lock().expect("lock").push(m),
                    AgentControl::Cancel => break,
                    _ => {}
                }
            }
        });
        Ok(AgentSession {
            control: ctl_tx,
            events: ev_rx,
        })
    }
}

/// A model that answers every question the same way and records the requests.
#[derive(Default)]
struct FakeModel {
    requests: Mutex<Vec<(String, usize, String)>>,
}

#[async_trait::async_trait]
impl Provider for FakeModel {
    fn name(&self) -> String {
        "fake".into()
    }
    async fn turn(
        &self,
        req: TurnRequest<'_>,
        _on_event: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> astra_llm::Result<TurnResponse> {
        let prompt = req
            .messages
            .iter()
            .map(ChatMessage::text)
            .collect::<Vec<_>>()
            .join("\n");
        self.requests
            .lock()
            .expect("lock")
            .push((req.system.to_string(), req.tools.len(), prompt));
        Ok(TurnResponse {
            message: ChatMessage {
                role: ChatRole::Assistant,
                blocks: vec![Block::Text {
                    text: "  The builder is adding hello.txt.\n".into(),
                }],
                raw: None,
            },
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            refusal: None,
        })
    }
}

fn setup(tmp: &std::path::Path) -> State {
    let repo_path = tmp.join("repo");
    std::fs::create_dir_all(&repo_path).expect("mkdir");
    git::init_repo(&repo_path, "main", &[("README.md", "# app\n")]).expect("init");
    let mut state = State::new("ws", "p-owner");
    let rec = Adapter::Command {
        command: "recording".into(),
        args: vec![],
    };
    for e in [
        person("p-owner"),
        person("p-watch"),
        repo(&repo_path),
        agent("a-rec", rec, CommandPolicy::Auto),
        mission("m-1", MissionMode::Split, "astra/greeting"),
        mission_agent("ma-p", "m-1", "a-rec", Role::Planner),
        mission_agent("ma-b", "m-1", "a-rec", Role::Builder),
        membership("m-1", "p-owner", Access::Owner),
        membership("m-1", "p-watch", Access::Watch),
    ] {
        state.insert(e);
    }
    state
}

fn start(
    host: Arc<TestHost>,
    data_dir: &std::path::Path,
    received: Received,
    model: Result<Arc<FakeModel>, String>,
) -> Orchestrator {
    let mut config = OrchestratorConfig::new(data_dir, Arc::new(MemorySecrets::new()));
    config.adapters = Arc::new(move |_| {
        Box::new(RecordingAdapter {
            received: received.clone(),
        })
    });
    config.side_provider = Arc::new(move |_secrets| {
        let model = model.clone();
        Box::pin(async move { model.map(|m| m as Arc<dyn Provider>) })
    });
    Orchestrator::start(host, config)
}

/// The person's side message (the client writes it), then the command.
fn ask(host: &TestHost, orch: &Orchestrator, to: Option<&str>, body: &str) -> Op {
    host.client_op(
        "p-owner",
        OpBody::put(Message {
            id: format!("msg-{}", new_op_id().to_lowercase()),
            mission_id: "m-1".into(),
            author: Author::Person {
                person_id: "p-owner".into(),
            },
            steering: None,
            to: to.map(str::to_string),
            body: body.into(),
            plan: vec![],
            log: vec![],
            approval_id: None,
            at: now_rfc3339(),
            side: Some("side-1".into()),
        }),
    );
    host.command(
        orch,
        "p-owner",
        Command::SendMessage {
            mission_id: "m-1".into(),
            to: to.map(str::to_string),
            body: body.into(),
            side: Some("side-1".into()),
        },
    )
}

fn side_replies(s: &State) -> Vec<Message> {
    let mut v: Vec<Message> = s
        .messages
        .values()
        .filter(|m| m.side.is_some() && matches!(m.author, Author::Agent { .. }))
        .cloned()
        .collect();
    v.sort_by(|a, b| a.at.cmp(&b.at));
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_side_question_is_answered_in_its_thread_without_reaching_the_agents() {
    let tmp = tempfile::tempdir().expect("tmp");
    let host = TestHost::new(setup(tmp.path()));
    let received: Received = Arc::default();
    let model = Arc::new(FakeModel::default());
    let orch = start(
        host.clone(),
        &tmp.path().join("data"),
        received.clone(),
        Ok(model.clone()),
    );
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    wait_for(&host, "the planner working", |_, _| {
        orch.is_running("ma-p").then_some(())
    })
    .await;
    let before = host.state();
    let main_before = before
        .messages
        .values()
        .filter(|m| m.side.is_none())
        .count();

    // Not addressed: the planner answers.
    let q1 = ask(&host, &orch, None, "What is the builder doing?");
    let reply = wait_for(&host, "a side reply", |s, _| {
        side_replies(s).first().cloned()
    })
    .await;
    assert_eq!(host.result_of(&q1), Some((true, None)));
    assert_eq!(reply.side.as_deref(), Some("side-1"));
    assert_eq!(
        reply.author,
        Author::Agent {
            mission_agent_id: "ma-p".into()
        }
    );
    assert_eq!(reply.body, "The builder is adding hello.txt.");
    assert_eq!(
        (
            reply.steering.as_ref(),
            reply.to.as_ref(),
            reply.approval_id.as_ref()
        ),
        (None, None, None)
    );
    assert!(reply.plan.is_empty() && reply.log.is_empty());

    // Addressed to the builder, which is not running: it answers, and it is not started.
    let q2 = ask(&host, &orch, Some("ma-b"), "And after that?");
    wait_for(&host, "a second side reply", |s, _| {
        (side_replies(s).len() == 2).then_some(())
    })
    .await;
    assert_eq!(host.result_of(&q2), Some((true, None)));
    let s = host.state();
    let replies = side_replies(&s);
    assert_eq!(replies.len(), 2, "exactly one reply per question");
    assert_eq!(
        replies[1].author,
        Author::Agent {
            mission_agent_id: "ma-b".into()
        }
    );
    assert!(!orch.is_running("ma-b"));

    // Nothing reached the agents: the planner only ever got its brief.
    assert_eq!(received.lock().expect("lock").len(), 1);
    // No steering, status or main-thread change.
    for id in ["ma-p", "ma-b"] {
        let (was, now) = (&before.mission_agents[id], &s.mission_agents[id]);
        assert_eq!(now.steered_by, None);
        assert_eq!((now.status, &now.latest), (was.status, &was.latest));
    }
    assert_eq!(
        s.messages.values().filter(|m| m.side.is_none()).count(),
        main_before
    );

    // One tool-less model call per question, with the mission's context and the side thread so far.
    let requests = model.requests.lock().expect("lock").clone();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|(system, tools, _)| *tools == 0 && system.contains("read-only"))
    );
    let (_, _, second) = &requests[1];
    assert!(second.contains("Goal: Add a greeting file."));
    assert!(second.contains("What is the builder doing?"));
    assert!(second.contains("The builder is adding hello.txt."));
    assert!(second.ends_with("And after that?"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_side_question_without_a_model_gets_a_reply_saying_so() {
    let tmp = tempfile::tempdir().expect("tmp");
    let host = TestHost::new(setup(tmp.path()));
    let orch = start(
        host.clone(),
        &tmp.path().join("data"),
        Arc::default(),
        Err("no model configured".into()),
    );
    let q = ask(&host, &orch, None, "Where are we?");
    let reply = wait_for(&host, "a side reply", |s, _| {
        side_replies(s).first().cloned()
    })
    .await;
    assert_eq!(host.result_of(&q), Some((true, None)));
    assert_eq!(
        reply.body,
        "I can't answer side questions right now: no model configured"
    );
    assert_eq!(reply.side.as_deref(), Some("side-1"));
    assert_eq!(side_replies(&host.state()).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn side_questions_are_checked_like_messages() {
    let tmp = tempfile::tempdir().expect("tmp");
    let host = TestHost::new(setup(tmp.path()));
    let model = Arc::new(FakeModel::default());
    let orch = start(
        host.clone(),
        &tmp.path().join("data"),
        Arc::default(),
        Ok(model.clone()),
    );
    let side = |actor: &str, to: Option<&str>| {
        host.command(
            &orch,
            actor,
            Command::SendMessage {
                mission_id: "m-1".into(),
                to: to.map(str::to_string),
                body: "hi".into(),
                side: Some("side-1".into()),
            },
        )
    };
    let watcher = side("p-watch", None);
    let r = wait_for(&host, "watcher rejected", |_, ops| result_in(ops, &watcher)).await;
    assert!(!r.0 && r.1.expect("error").contains("may not steer"));
    let unknown = side("p-owner", Some("ma-nope"));
    let r = wait_for(&host, "unknown agent rejected", |_, ops| {
        result_in(ops, &unknown)
    })
    .await;
    assert_eq!(r, (false, Some("unknown agent ma-nope".into())));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(side_replies(&host.state()).is_empty());
    assert!(model.requests.lock().expect("lock").is_empty());
}
