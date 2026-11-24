//! End to end: astra-server relay, an astrad-style runner node with the agent runtime, and a remote client
//! (web/phone) that starts a mission, approves the agent's command and merges, all over the relay.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_node::{Node, NodeConfig, RelayConfig, WsConnector};
use astra_protocol::*;
use astra_runtime::git;
use astra_server::{ServerConfig, build_state, serve};
use astra_store::{Origin, Store};
use astra_sync::{StoreHost, SyncClient, SyncConfig, SyncHandle};
use serde_json::{Value, json};

fn fake_agent() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let bin = target
        .join("debug")
        .join(format!("fake-acp-agent{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "astra-runtime", "--bin", "fake-acp-agent"])
            .status()
            .expect("cargo build");
        assert!(status.success());
    }
    bin
}

async fn eventually<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..1200 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out: {what}");
}

struct Remote {
    store: Arc<Mutex<Store>>,
    handle: SyncHandle,
    clock: HlcClock,
    actor: String,
}

impl Remote {
    fn send(&mut self, bodies: Vec<OpBody>) -> Vec<Op> {
        let ops: Vec<Op> = bodies
            .into_iter()
            .map(|b| Op::new(self.clock.tick(), self.actor.clone(), b))
            .collect();
        self.store
            .lock()
            .expect("lock")
            .append(&ops, Origin::Local)
            .expect("append");
        self.handle.notify_local();
        ops
    }
    fn state(&self) -> State {
        self.store.lock().expect("lock").state().clone()
    }
    fn result_of(&self, op: &Op) -> Option<(bool, Option<String>)> {
        let store = self.store.lock().expect("lock");
        store
            .ops_since(0, usize::MAX)
            .expect("ops")
            .into_iter()
            .find_map(|(_, o)| match o.body {
                OpBody::CommandResult {
                    command_op,
                    ok,
                    error,
                } if command_op == op.id => Some((ok, error)),
                _ => None,
            })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mission_over_relay_with_remote_approval_and_merge() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    git::init_repo(&repo, "main", &[("README.md", "# demo\n")]).expect("init");

    // Relay.
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let state = build_state(
        ServerConfig {
            dev_auth: true,
            jwt_secret: Some("e2e".into()),
            ..ServerConfig::default()
        },
        rx.clone(),
    )
    .await
    .expect("server");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve(listener, state, rx));
    let session: Value = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/dev-token"))
        .json(&json!({"name": "Mark"}))
        .send()
        .await
        .expect("token")
        .json()
        .await
        .expect("json");
    let token = session["session"]["token"]
        .as_str()
        .expect("token")
        .to_string();
    let ws_url = format!("ws://{addr}/v1/sync");

    // Runner node (what `astrad --relay-url ... --token ...` runs).
    let mut cfg = NodeConfig::new(tmp.path().join("node"), "n-desk", "ws-dev", "p-dev-mark");
    cfg.runtime = true;
    cfg.relay = Some(RelayConfig {
        url: ws_url.clone(),
        token: token.clone(),
    });
    let node = Node::open(cfg).await.expect("node");
    eventually("node online", || node.connectivity().online.then_some(())).await;

    // Remote client (phone/web).
    let mut st = Store::open_in_memory().expect("store");
    st.set_identity("ws-dev", "p-dev-mark").expect("id");
    let store = Arc::new(Mutex::new(st));
    let mut sc = SyncConfig::new("ws-dev", "web-mark", Some(token));
    sc.min_backoff = Duration::from_millis(50);
    let (client, handle) = SyncClient::new(
        sc,
        Arc::new(StoreHost {
            store: store.clone(),
        }),
    );
    tokio::spawn(client.run(Arc::new(WsConnector::new(ws_url))));
    let mut remote = Remote {
        store,
        handle,
        clock: HlcClock::new("web-mark"),
        actor: "p-dev-mark".into(),
    };

    remote.send(vec![
        OpBody::put(Repository {
            id: "r-1".into(),
            name: "acme/demo".into(),
            source: RepoSource::Local,
            default_branch: "main".into(),
            path: Some(repo.to_string_lossy().into_owned()),
            remote_url: None,
        }),
        OpBody::put(Agent {
            id: "a-fake".into(),
            name: "Fake".into(),
            vendor: "TEST".into(),
            category: AgentCategory::CodingAgent,
            transport: "CLI on this machine".into(),
            adapter: Some(Adapter::Acp {
                command: fake_agent().to_string_lossy().into_owned(),
                args: vec![],
            }),
            runs_on: RunsOn::Local,
            node_id: Some("n-desk".into()),
            command_policy: CommandPolicy::AskMe,
            status: AgentStatus::Connected,
            requires_network: false,
        }),
        OpBody::put(Mission {
            id: "m-1".into(),
            number: 1,
            title: "Greet".into(),
            short_name: "GREET".into(),
            goal: "Add a greeting.".into(),
            mode: MissionMode::Split,
            repo_id: "r-1".into(),
            base_branch: "main".into(),
            branch: "astra/greet".into(),
            status: MissionStatus::Draft,
            approval_policy: ApprovalPolicy::Owner,
            owner_id: "p-dev-mark".into(),
            node_id: Some("n-desk".into()),
            stats: None,
            created_at: now_rfc3339(),
        }),
        OpBody::put(MissionAgent {
            id: "ma-1".into(),
            mission_id: "m-1".into(),
            agent_id: "a-fake".into(),
            role: Role::Builder,
            enabled: true,
            status: MissionAgentStatus::Queued,
            status_detail: None,
            steered_by: None,
            task: None,
            started_at: None,
            branch: None,
            latest: vec![],
            agent_type_id: None,
        }),
        OpBody::put(Membership {
            id: "m-1:p-dev-mark".into(),
            mission_id: "m-1".into(),
            person_id: "p-dev-mark".into(),
            access: Access::Owner,
        }),
        OpBody::Command {
            node: "n-desk".into(),
            command: Command::StartMission {
                mission_id: "m-1".into(),
            },
        },
    ]);

    let approval = eventually("approval reaches the remote client", || {
        remote
            .state()
            .approvals
            .values()
            .find(|a| a.state == ApprovalState::Pending)
            .map(|a| a.id.clone())
    })
    .await;
    assert_eq!(
        remote.state().mission_agents["ma-1"].status,
        MissionAgentStatus::NeedsYou
    );
    remote.send(vec![OpBody::Command {
        node: "n-desk".into(),
        command: Command::Approve {
            approval_id: approval,
        },
    }]);
    let res = eventually("result", || remote.state().results.get("res-m-1").cloned()).await;
    assert!(res.diff.iter().any(|f| f.path == "check.txt"));
    let merge = remote.send(vec![OpBody::Command {
        node: "n-desk".into(),
        command: Command::Merge {
            result_id: "res-m-1".into(),
        },
    }]);
    eventually("merged", || {
        (remote.state().missions["m-1"].status == MissionStatus::Merged).then_some(())
    })
    .await;
    eventually("merge result", || remote.result_of(&merge[0])).await;
    assert_eq!(remote.result_of(&merge[0]), Some((true, None)));
    assert_eq!(
        git::read_file_at(&repo, "main", "hello.txt")
            .expect("read")
            .as_deref(),
        Some("hello from the fake agent\n")
    );
    // The node and the remote client agree.
    let node_state = node.snapshot().await.expect("snapshot").state;
    eventually("converged", || {
        let r = remote.state();
        (r.missions == node_state.missions && r.results.len() == node_state.results.len())
            .then_some(())
    })
    .await;
    node.shutdown();
}
