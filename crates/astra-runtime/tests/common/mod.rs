#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_protocol::*;
use astra_runtime::{MemorySecrets, NodeHost, Orchestrator, OrchestratorConfig};

pub const NODE: &str = "n-test";

/// An in-memory node for orchestrator tests.
pub struct TestHost {
    pub inner: Mutex<(Reducer, HlcClock, Vec<Op>)>,
}

impl TestHost {
    pub fn new(state: State) -> Arc<TestHost> {
        Arc::new(TestHost { inner: Mutex::new((Reducer::new(state), HlcClock::new(NODE), vec![])) })
    }

    pub fn state(&self) -> State {
        self.inner.lock().expect("lock").0.state.clone()
    }

    pub fn ops(&self) -> Vec<Op> {
        self.inner.lock().expect("lock").2.clone()
    }

    /// An op made by a client (person) — applied locally like a synced op.
    pub fn client_op(&self, actor: &str, body: OpBody) -> Op {
        let mut g = self.inner.lock().expect("lock");
        let hlc = g.1.tick();
        let op = Op::new(hlc, actor, body);
        g.0.apply(&op);
        g.2.push(op.clone());
        op
    }

    pub fn command(&self, orch: &Orchestrator, actor: &str, command: Command) -> Op {
        let op = self.client_op(actor, OpBody::Command { node: NODE.into(), command });
        orch.submit(&op);
        op
    }

    pub fn result_of(&self, op: &Op) -> Option<(bool, Option<String>)> {
        self.ops().into_iter().find_map(|o| match o.body {
            OpBody::CommandResult { command_op, ok, error } if command_op == op.id => Some((ok, error)),
            _ => None,
        })
    }
}

impl NodeHost for TestHost {
    fn node_id(&self) -> String {
        NODE.into()
    }
    fn with_state(&self, f: &mut dyn FnMut(&State)) {
        let g = self.inner.lock().expect("lock");
        f(&g.0.state);
    }
    fn emit(&self, bodies: Vec<OpBody>) -> Result<Vec<Op>, String> {
        let mut g = self.inner.lock().map_err(|e| e.to_string())?;
        let mut out = vec![];
        for b in bodies {
            let hlc = g.1.tick();
            let op = Op::new(hlc, NODE, b);
            g.0.apply(&op);
            g.2.push(op.clone());
            out.push(op);
        }
        Ok(out)
    }
    fn has_command_result(&self, command_op: &str) -> bool {
        self.inner
            .lock()
            .expect("lock")
            .2
            .iter()
            .any(|o| matches!(&o.body, OpBody::CommandResult { command_op: c, .. } if c == command_op))
    }
}

pub async fn wait_for<T>(host: &TestHost, what: &str, mut f: impl FnMut(&State, &[Op]) -> Option<T>) -> T {
    for _ in 0..600 {
        {
            let g = host.inner.lock().expect("lock");
            if let Some(v) = f(&g.0.state, &g.2) {
                return v;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let g = host.inner.lock().expect("lock");
    panic!("timed out waiting for {what}; state: {:#?}", g.0.state);
}

pub fn orchestrator(host: Arc<TestHost>, data_dir: &Path) -> Orchestrator {
    Orchestrator::start(host, OrchestratorConfig::new(data_dir, Arc::new(MemorySecrets::new())))
}

pub fn person(id: &str) -> Entity {
    Entity::Person(Person { id: id.into(), name: id.into(), initials: "XX".into() })
}

pub fn repo(path: &Path) -> Entity {
    Entity::Repository(Repository {
        id: "r-1".into(),
        name: "acme/app".into(),
        source: RepoSource::Local,
        default_branch: "main".into(),
        path: Some(path.to_string_lossy().into_owned()),
        remote_url: None,
    })
}

pub fn agent(id: &str, adapter: Adapter, policy: CommandPolicy) -> Entity {
    Entity::Agent(Agent {
        id: id.into(),
        name: id.into(),
        vendor: "TEST".into(),
        category: AgentCategory::CodingAgent,
        transport: "CLI".into(),
        adapter: Some(adapter),
        runs_on: RunsOn::Local,
        node_id: Some(NODE.into()),
        command_policy: policy,
        status: AgentStatus::Connected,
        requires_network: false,
    })
}

pub fn mission(id: &str, mode: MissionMode, branch: &str) -> Entity {
    Entity::Mission(Mission {
        id: id.into(),
        number: 1,
        title: "Add a greeting".into(),
        short_name: "GREETING".into(),
        goal: "Add a greeting file.".into(),
        mode,
        repo_id: "r-1".into(),
        base_branch: "main".into(),
        branch: branch.into(),
        status: MissionStatus::Draft,
        approval_policy: ApprovalPolicy::Owner,
        owner_id: "p-owner".into(),
        node_id: Some(NODE.into()),
        stats: None,
        created_at: "2026-11-21T09:00:00Z".into(),
    })
}

pub fn mission_agent(id: &str, mission: &str, agent: &str, role: Role) -> Entity {
    Entity::MissionAgent(MissionAgent {
        id: id.into(),
        mission_id: mission.into(),
        agent_id: agent.into(),
        role,
        enabled: true,
        status: MissionAgentStatus::Queued,
        status_detail: None,
        steered_by: None,
        task: None,
        started_at: None,
        branch: None,
        latest: vec![],
    })
}

pub fn membership(mission: &str, person: &str, access: Access) -> Entity {
    Entity::Membership(Membership {
        id: Membership::id_for(mission, person),
        mission_id: mission.into(),
        person_id: person.into(),
        access,
    })
}
