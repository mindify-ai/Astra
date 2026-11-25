//! Changing a mission's agents while it runs: switching a role to another agent, adding and removing agents.

mod common;

use astra_protocol::*;
use astra_runtime::git;
use common::*;

fn sh(script: &str) -> Adapter {
    Adapter::Command {
        command: "sh".into(),
        args: vec!["-c".into(), script.into()],
    }
}

/// A repository plus a slow agent (never finishes on its own) and a fast one that writes a file.
fn setup(tmp: &std::path::Path, extra: Vec<Entity>) -> State {
    let repo_path = tmp.join("repo");
    std::fs::create_dir_all(&repo_path).expect("mkdir");
    git::init_repo(&repo_path, "main", &[("README.md", "# app\n")]).expect("init");
    let mut state = State::new("ws", "p-owner");
    for e in [
        person("p-owner"),
        person("p-watch"),
        repo(&repo_path),
        agent(
            "a-slow",
            sh("cat > /dev/null; sleep 60"),
            CommandPolicy::Auto,
        ),
        agent(
            "a-fast",
            sh("cat > /dev/null; echo fast > fast.txt; echo took over"),
            CommandPolicy::Auto,
        ),
        mission("m-1", MissionMode::Split, "astra/greeting"),
        membership("m-1", "p-owner", Access::Owner),
        membership("m-1", "p-watch", Access::Watch),
    ]
    .into_iter()
    .chain(extra)
    {
        state.insert(e);
    }
    state
}

async fn start_and_wait_working(host: &TestHost, orch: &astra_runtime::Orchestrator, ma: &str) {
    host.command(
        orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    let ma = ma.to_string();
    wait_for(host, "agent working", |_, _| {
        orch.is_running(&ma).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_a_working_agent_hands_its_turn_to_the_new_one() {
    let tmp = tempfile::tempdir().expect("tmp");
    let state = setup(
        tmp.path(),
        vec![mission_agent("ma-b", "m-1", "a-slow", Role::Builder)],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    start_and_wait_working(&host, &orch, "ma-b").await;

    let switch = host.command(
        &orch,
        "p-owner",
        Command::SwitchAgent {
            mission_agent_id: "ma-b".into(),
            agent_id: "a-fast".into(),
        },
    );
    wait_for(&host, "the new agent finishes the mission", |s, ops| {
        s.results.get("res-m-1").and(result_in(ops, &switch))
    })
    .await;
    assert_eq!(host.result_of(&switch), Some((true, None)));
    let s = host.state();
    let ma = &s.mission_agents["ma-b"];
    assert_eq!(ma.agent_id, "a-fast");
    assert_eq!(ma.status, MissionAgentStatus::Done);
    assert!(s.messages.values().any(|m| m.body.contains("took over")));
    assert_eq!(s.results["res-m-1"].diff[0].path, "fast.txt");
    // The stop was on purpose: nothing reports it as an error.
    assert!(!s.messages.values().any(|m| m.body.contains("Error:")));
    assert_ne!(s.missions["m-1"].status, MissionStatus::Working);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_working_agent_lets_the_mission_carry_on() {
    let tmp = tempfile::tempdir().expect("tmp");
    let state = setup(
        tmp.path(),
        vec![
            mission_agent("ma-b", "m-1", "a-slow", Role::Builder),
            mission_agent("ma-t", "m-1", "a-fast", Role::Tester),
        ],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    start_and_wait_working(&host, &orch, "ma-b").await;

    let remove = host.command(
        &orch,
        "p-owner",
        Command::RemoveAgent {
            mission_agent_id: "ma-b".into(),
        },
    );
    wait_for(&host, "the tester runs next", |s, ops| {
        (s.mission_agents["ma-t"].status == MissionAgentStatus::Done)
            .then(|| result_in(ops, &remove))
            .flatten()
    })
    .await;
    assert_eq!(host.result_of(&remove), Some((true, None)));
    let s = host.state();
    let ma = &s.mission_agents["ma-b"];
    assert!(!ma.enabled);
    assert_eq!(ma.status_detail.as_deref(), Some("removed"));
    assert!(!orch.is_running("ma-b"));
    assert!(
        s.tasks
            .values()
            .any(|t| t.note.as_deref() == Some("Removed from the mission"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_changes_are_checked() {
    let tmp = tempfile::tempdir().expect("tmp");
    let Entity::Agent(mut offline) = agent("a-off", sh("true"), CommandPolicy::Auto) else {
        unreachable!()
    };
    offline.status = AgentStatus::NotConnected;
    let state = setup(
        tmp.path(),
        vec![
            mission_agent("ma-b", "m-1", "a-fast", Role::Builder),
            Entity::Agent(offline),
        ],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    let cases = [
        (
            "p-owner",
            Command::SwitchAgent {
                mission_agent_id: "ma-b".into(),
                agent_id: "a-off".into(),
            },
            "not connected",
        ),
        (
            "p-watch",
            Command::SwitchAgent {
                mission_agent_id: "ma-b".into(),
                agent_id: "a-slow".into(),
            },
            "may not steer",
        ),
        (
            "p-owner",
            Command::RemoveAgent {
                mission_agent_id: "ma-b".into(),
            },
            "at least one agent",
        ),
    ];
    for (actor, command, why) in cases {
        let op = host.command(&orch, actor, command);
        let (ok, err) = wait_for(&host, "result", |_, ops| result_in(ops, &op)).await;
        assert!(!ok, "{why}");
        assert!(err.expect("error").contains(why), "{why}");
    }
    assert_eq!(host.state().mission_agents["ma-b"].agent_id, "a-fast");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_added_to_a_mission_under_way_starts_in_its_worktree() {
    let tmp = tempfile::tempdir().expect("tmp");
    let state = setup(
        tmp.path(),
        vec![mission_agent("ma-b", "m-1", "a-fast", Role::Builder)],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    wait_for(&host, "the builder finishes", |s, _| {
        s.results.get("res-m-1").map(|_| ())
    })
    .await;

    let Entity::MissionAgent(tester) = mission_agent("ma-t", "m-1", "a-fast", Role::Tester) else {
        unreachable!()
    };
    host.client_op(
        "p-owner",
        OpBody::Put {
            entity: Entity::MissionAgent(tester),
        },
    );
    let add = host.command(
        &orch,
        "p-owner",
        Command::AddAgent {
            mission_agent_id: "ma-t".into(),
        },
    );
    wait_for(&host, "the added agent works", |s, ops| {
        (s.mission_agents["ma-t"].status == MissionAgentStatus::Done)
            .then(|| result_in(ops, &add))
            .flatten()
    })
    .await;
    assert_eq!(host.result_of(&add), Some((true, None)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_custom_agent_type_adds_its_instructions_to_the_brief() {
    let tmp = tempfile::tempdir().expect("tmp");
    let Entity::MissionAgent(mut ma) = mission_agent("ma-b", "m-1", "a-echo", Role::Builder) else {
        unreachable!()
    };
    ma.agent_type_id = Some("at-sec".into());
    let state = setup(
        tmp.path(),
        vec![
            // Echoes its brief back, so the brief shows up in the thread.
            agent("a-echo", sh("cat"), CommandPolicy::Auto),
            Entity::AgentType(AgentType {
                id: "at-sec".into(),
                name: "Security builder".into(),
                role: Role::Builder,
                agent_id: "a-echo".into(),
                instructions: "Validate every input at the boundary.".into(),
            }),
            Entity::MissionAgent(ma),
        ],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    wait_for(&host, "the brief with the type's instructions", |s, _| {
        s.messages
            .values()
            .find(|m| m.body.contains("Validate every input at the boundary."))
            .map(|m| m.body.clone())
    })
    .await;
    let s = host.state();
    assert!(s.messages.values().any(|m| {
        m.body.contains("You are the builder") && m.body.contains("\"Security builder\"")
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_agent_plays_two_roles_at_the_same_time() {
    let tmp = tempfile::tempdir().expect("tmp");
    let marks = tmp.path().join("marks");
    std::fs::create_dir_all(&marks).expect("marks");
    // Each session leaves a mark and waits (up to 5 s) to see the other's: it only writes its file if both
    // ran at once.
    let script = format!(
        "cat > /dev/null; touch {m}/$$; i=0; \
         while [ $(ls {m} | wc -l) -lt 2 ] && [ $i -lt 100 ]; do sleep 0.05; i=$((i+1)); done; \
         if [ $(ls {m} | wc -l) -ge 2 ]; then echo together > together.txt; echo together; fi",
        m = marks.display()
    );
    let state = setup(
        tmp.path(),
        vec![
            agent("a-shared", sh(&script), CommandPolicy::Auto),
            mission("m-1", MissionMode::Race, "astra/race"),
            mission_agent("ma-a", "m-1", "a-shared", Role::Builder),
            mission_agent("ma-b", "m-1", "a-shared", Role::Builder),
        ],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    wait_for(&host, "both results", |s, _| {
        (s.results.contains_key("res-m-1-a") && s.results.contains_key("res-m-1-b")).then_some(())
    })
    .await;
    let s = host.state();
    for id in ["res-m-1-a", "res-m-1-b"] {
        assert_eq!(s.results[id].diff[0].path, "together.txt", "{id}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn archiving_stops_a_running_mission_quietly_and_unarchiving_brings_it_back() {
    let tmp = tempfile::tempdir().expect("tmp");
    let state = setup(
        tmp.path(),
        vec![
            mission_agent("ma-b", "m-1", "a-slow", Role::Builder),
            mission_agent("ma-t", "m-1", "a-fast", Role::Tester),
            membership("m-1", "p-steer", Access::Steer),
        ],
    );
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    start_and_wait_working(&host, &orch, "ma-b").await;

    // Only the owner may archive.
    let denied = host.command(
        &orch,
        "p-steer",
        Command::ArchiveMission {
            mission_id: "m-1".into(),
            archived: true,
        },
    );
    let (ok, err) = wait_for(&host, "denied", |_, ops| result_in(ops, &denied)).await;
    assert!(!ok && err.expect("err").contains("only the owner"));

    let archive = host.command(
        &orch,
        "p-owner",
        Command::ArchiveMission {
            mission_id: "m-1".into(),
            archived: true,
        },
    );
    wait_for(&host, "archived", |_, ops| result_in(ops, &archive)).await;
    assert_eq!(host.result_of(&archive), Some((true, None)));
    assert!(!orch.is_running("ma-b"));
    // Give a wrongly continuing pipeline time to start the tester.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let s = host.state();
    assert!(s.missions["m-1"].archived_at.is_some());
    assert_eq!(s.missions["m-1"].status, MissionStatus::NeedsYou);
    assert_eq!(
        s.mission_agents["ma-b"].status_detail.as_deref(),
        Some("stopped: archived")
    );
    assert_eq!(s.mission_agents["ma-t"].status, MissionAgentStatus::Queued);
    assert!(
        !s.messages
            .values()
            .any(|m| m.body.starts_with("The mission stopped"))
    );

    let send = host.command(
        &orch,
        "p-owner",
        Command::SendMessage {
            mission_id: "m-1".into(),
            to: Some("ma-b".into()),
            body: "go on".into(),
            side: None,
        },
    );
    let (ok, _) = wait_for(&host, "rejected", |_, ops| result_in(ops, &send)).await;
    assert!(!ok);

    let back = host.command(
        &orch,
        "p-owner",
        Command::ArchiveMission {
            mission_id: "m-1".into(),
            archived: false,
        },
    );
    wait_for(&host, "unarchived", |_, ops| result_in(ops, &back)).await;
    assert!(host.state().missions["m-1"].archived_at.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_a_mission_removes_it_its_records_and_its_worktree() {
    let tmp = tempfile::tempdir().expect("tmp");
    let state = setup(
        tmp.path(),
        vec![mission_agent("ma-b", "m-1", "a-slow", Role::Builder)],
    );
    let host = TestHost::new(state);
    let data = tmp.path().join("data");
    let orch = orchestrator(host.clone(), &data);
    start_and_wait_working(&host, &orch, "ma-b").await;
    let worktrees = data.join("worktrees").join("m-1");
    assert!(worktrees.exists());

    let delete = host.command(
        &orch,
        "p-owner",
        Command::DeleteMission {
            mission_id: "m-1".into(),
        },
    );
    wait_for(&host, "deleted", |_, ops| result_in(ops, &delete)).await;
    assert_eq!(host.result_of(&delete), Some((true, None)));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let s = host.state();
    assert!(!s.missions.contains_key("m-1"));
    assert!(!s.mission_agents.contains_key("ma-b"));
    assert!(!s.tasks.values().any(|t| t.mission_id == "m-1"));
    assert!(!s.messages.values().any(|m| m.mission_id == "m-1"));
    assert!(!s.memberships.values().any(|m| m.mission_id == "m-1"));
    assert!(!worktrees.exists());
    assert!(!orch.is_running("ma-b"));
}
