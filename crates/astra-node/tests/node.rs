use std::path::PathBuf;
use std::time::Duration;

use astra_node::{Connectivity, Node, NodeConfig, NodeEvent};
use astra_protocol::*;

fn seed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/seed.json")
}

#[tokio::test]
async fn seed_snapshot_apply_events_and_reopen() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cfg = NodeConfig::new(tmp.path(), "n-desk", "ws-mindify", "p-mark");
    cfg.seed_if_empty = Some(seed_path());
    let node = Node::open(cfg.clone()).await.expect("open");
    let snap = node.snapshot().await.expect("snapshot");
    assert_eq!(snap.cursor, 0);
    assert_eq!(snap.state.missions.len(), 3);
    assert_eq!(snap.state.workspace_id, "ws-mindify");
    assert_eq!(
        node.connectivity(),
        Connectivity {
            online: false,
            relay: RelayStatus::Disabled
        }
    );

    let mut events = node.subscribe();
    let op = node.make_op(
        "p-mark",
        OpBody::patch(
            EntityKind::Mission,
            "m-014",
            [("title", serde_json::json!("Rate limits"))],
        ),
    );
    let fresh = node.apply_ops(vec![op.clone(), op.clone()]).expect("apply");
    assert_eq!(fresh.len(), 1);
    assert!(
        node.apply_ops(vec![op.clone()]).expect("apply").is_empty(),
        "idempotent"
    );
    match events.recv().await.expect("event") {
        NodeEvent::Ops { ops, cursor } => {
            assert_eq!(ops, vec![op.clone()]);
            assert_eq!(cursor, 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    node.set_presence(Some(Presence {
        person_id: "p-mark".into(),
        mission_id: Some("m-014".into()),
        activity: "Reading".into(),
        at: now_rfc3339(),
    }))
    .await
    .expect("presence");
    assert!(
        matches!(events.recv().await.expect("event"), NodeEvent::Presence(presence) if presence.len() == 1)
    );
    assert_eq!(node.snapshot().await.expect("snap").state.presence.len(), 1);
    let payload = serde_json::to_value(node.connectivity()).expect("json");
    assert_eq!(
        payload,
        serde_json::json!({"online":false,"relay":"disabled"})
    );
    node.shutdown();
    drop(node);

    let node = Node::open(cfg).await.expect("reopen");
    let snap = node.snapshot().await.expect("snapshot");
    assert_eq!(snap.cursor, 1);
    assert_eq!(snap.state.missions["m-014"].title, "Rate limits");
    assert_eq!(node.ops_since(0, 10).expect("ops").len(), 1);
    // The clock continues after the stored ops.
    let next = node.make_op(
        "p-mark",
        OpBody::Delete {
            entity_kind: EntityKind::Person,
            id: "p-jo".into(),
        },
    );
    assert!(next.hlc > op.hlc);
}

#[tokio::test]
async fn runtime_executes_commands_addressed_to_this_node() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cfg = NodeConfig::new(tmp.path(), "n-desk", "ws", "p-me");
    cfg.runtime = true;
    let node = Node::open(cfg).await.expect("open");
    let agent = Agent {
        id: "a-sh".into(),
        name: "Shell".into(),
        vendor: "ANY CLI".into(),
        category: AgentCategory::Protocol,
        transport: "Reads a prompt, writes text".into(),
        adapter: Some(Adapter::Command {
            command: "sh".into(),
            args: vec![],
        }),
        runs_on: RunsOn::Local,
        node_id: None,
        command_policy: CommandPolicy::AskMe,
        status: AgentStatus::NotConnected,
        requires_network: false,
    };
    let other = node.make_op(
        "p-me",
        OpBody::Command {
            node: "n-elsewhere".into(),
            command: Command::ConnectAgent {
                agent_id: "a-sh".into(),
            },
        },
    );
    let mine = node.make_op(
        "p-me",
        OpBody::Command {
            node: "n-desk".into(),
            command: Command::ConnectAgent {
                agent_id: "a-sh".into(),
            },
        },
    );
    node.apply_ops(vec![
        node.make_op("p-me", OpBody::put(agent)),
        other.clone(),
        mine.clone(),
    ])
    .expect("apply");
    for _ in 0..200 {
        if node.with_state(|s| s.agents["a-sh"].status == AgentStatus::Connected) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let s = node.snapshot().await.expect("snap").state;
    assert_eq!(s.agents["a-sh"].status, AgentStatus::Connected);
    assert_eq!(s.agents["a-sh"].node_id.as_deref(), Some("n-desk"));
    let results: Vec<(String, bool)> = node
        .ops_since(0, 100)
        .expect("ops")
        .into_iter()
        .filter_map(|(_, o)| match o.body {
            OpBody::CommandResult { command_op, ok, .. } => Some((command_op, ok)),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        vec![(mine.id.clone(), true)],
        "only the command for this node runs, once"
    );
}
