//! End-to-end orchestrator tests on temp git repositories.

mod common;

use astra_protocol::*;
use astra_runtime::git;
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_mission_with_acp_agent_approval_and_merge() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("mkdir");
    git::init_repo(&repo_path, "main", &[("README.md", "# app\n")]).expect("init");

    let mut state = State::new("ws", "p-owner");
    for e in [
        person("p-owner"),
        person("p-lin"),
        person("p-x"),
        repo(&repo_path),
        agent(
            "a-fake",
            Adapter::Acp {
                command: env!("CARGO_BIN_EXE_fake-acp-agent").into(),
                args: vec![],
            },
            CommandPolicy::AskMe,
        ),
        mission("m-1", MissionMode::Split, "astra/greeting"),
        mission_agent("ma-1", "m-1", "a-fake", Role::Builder),
        membership("m-1", "p-owner", Access::Owner),
        membership("m-1", "p-lin", Access::Steer),
    ] {
        state.insert(e);
    }
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));

    let start = host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    let approval_id = wait_for(&host, "approval", |s, _| {
        s.approvals
            .values()
            .find(|a| a.state == ApprovalState::Pending)
            .map(|a| a.id.clone())
    })
    .await;
    assert_eq!(host.result_of(&start), Some((true, None)));
    let s = host.state();
    assert_eq!(s.missions["m-1"].status, MissionStatus::NeedsYou);
    assert_eq!(
        s.mission_agents["ma-1"].status,
        MissionAgentStatus::NeedsYou
    );
    assert_eq!(
        s.mission_agents["ma-1"].branch.as_deref(),
        Some("astra/greeting")
    );
    let approval = &s.approvals[&approval_id];
    assert_eq!(
        approval.command.as_deref(),
        Some("echo checked > check.txt")
    );
    assert!(
        s.messages
            .values()
            .any(|m| m.approval_id.as_deref() == Some(approval_id.as_str()))
    );
    assert!(
        s.tasks
            .values()
            .any(|t| t.approval_id.as_deref() == Some(approval_id.as_str())
                && t.column == TaskColumn::NeedsYou)
    );
    // The plan arrived as a message plan; the edit as a log line.
    assert!(
        s.messages
            .values()
            .any(|m| m.plan.len() == 3 && m.plan[1].state == PlanState::Active)
    );
    assert!(
        s.mission_agents["ma-1"]
            .latest
            .iter()
            .any(|l| l.verb == "edit" && l.target == "hello.txt")
    );
    let worktree = tmp.path().join("data/worktrees/m-1/main");
    assert_eq!(
        std::fs::read_to_string(worktree.join("hello.txt")).expect("written"),
        "hello from the fake agent\n"
    );
    assert!(
        !tmp.path().join("data/worktrees/m-1/escape.txt").exists(),
        "fs escape must be refused"
    );

    // Only the owner may decide (approval policy: owner).
    let bad = host.command(
        &orch,
        "p-lin",
        Command::Approve {
            approval_id: approval_id.clone(),
        },
    );
    let r = wait_for(&host, "rejected approve", |_, ops| result_in(ops, &bad)).await;
    assert!(!r.0 && r.1.expect("error").contains("may not decide"));

    let ok = host.command(
        &orch,
        "p-owner",
        Command::Approve {
            approval_id: approval_id.clone(),
        },
    );
    wait_for(&host, "result", |s, _| s.results.get("res-m-1").cloned()).await;
    assert_eq!(host.result_of(&ok), Some((true, None)));
    let s = host.state();
    let a = &s.approvals[&approval_id];
    assert_eq!(
        (a.state, a.decided_by.as_deref()),
        (ApprovalState::Approved, Some("p-owner"))
    );
    assert_eq!(s.missions["m-1"].status, MissionStatus::NeedsYou);
    let res = &s.results["res-m-1"];
    assert_eq!(res.branch, "astra/greeting");
    assert_eq!(res.stats.files, 2, "{:?}", res.diff);
    assert!(res.diff.iter().any(|f| f.path == "hello.txt"));
    assert!(res.diff.iter().any(|f| f.path == "check.txt"));
    assert!(
        s.messages
            .values()
            .any(|m| m.body.contains("Check passed."))
    );
    assert!(
        s.mission_agents["ma-1"]
            .latest
            .iter()
            .any(|l| l.verb == "run")
    );
    assert_eq!(s.mission_agents["ma-1"].status, MissionAgentStatus::Done);

    // Steering: a message to the agent takes the free lock; another steerer is then rejected.
    host.client_op(
        "p-owner",
        OpBody::put(Message {
            id: "msg-h1".into(),
            mission_id: "m-1".into(),
            author: Author::Person {
                person_id: "p-owner".into(),
            },
            steering: Some("ma-1".into()),
            to: Some("ma-1".into()),
            body: "please also add a newline".into(),
            plan: vec![],
            log: vec![],
            approval_id: None,
            at: now_rfc3339(),
        }),
    );
    let send = host.command(
        &orch,
        "p-owner",
        Command::SendMessage {
            mission_id: "m-1".into(),
            to: Some("ma-1".into()),
            body: "please also add a newline".into(),
        },
    );
    wait_for(&host, "agent reply", |s, _| {
        s.messages
            .values()
            .find(|m| m.body.contains("Got: please"))
            .map(|_| ())
    })
    .await;
    assert_eq!(host.result_of(&send), Some((true, None)));
    assert_eq!(
        host.state().mission_agents["ma-1"].steered_by.as_deref(),
        Some("p-owner")
    );
    let other = host.command(
        &orch,
        "p-lin",
        Command::SendMessage {
            mission_id: "m-1".into(),
            to: Some("ma-1".into()),
            body: "hi".into(),
        },
    );
    let r = wait_for(&host, "steering rejection", |_, ops| result_in(ops, &other)).await;
    assert_eq!(r, (false, Some("steering held by p-owner".into())));
    // Only one human message entity exists (the node does not duplicate it).
    assert_eq!(
        host.state()
            .messages
            .values()
            .filter(|m| matches!(m.author, Author::Person { .. }))
            .count(),
        1
    );

    let merge = host.command(
        &orch,
        "p-owner",
        Command::Merge {
            result_id: "res-m-1".into(),
        },
    );
    wait_for(&host, "merged", |s, _| {
        (s.missions["m-1"].status == MissionStatus::Merged).then_some(())
    })
    .await;
    assert_eq!(host.result_of(&merge), Some((true, None)));
    let s = host.state();
    let m = &s.merges["m-1"];
    assert_eq!(m.result_id.as_deref(), Some("res-m-1"));
    assert_eq!(m.approved_by, "p-owner");
    assert_eq!(m.into, "acme/app · main");
    assert!(
        s.tasks
            .values()
            .filter(|t| t.mission_id == "m-1" && t.approval_id.is_none())
            .all(|t| t.column == TaskColumn::Done)
    );
    assert!(
        s.tasks
            .values()
            .any(|t| t.merged_by.as_deref() == Some("p-owner"))
    );
    assert_eq!(
        git::read_file_at(&repo_path, "main", "hello.txt")
            .expect("read")
            .as_deref(),
        Some("hello from the fake agent\n")
    );
    assert_eq!(
        std::fs::read_to_string(repo_path.join("hello.txt")).expect("checked out"),
        "hello from the fake agent\n"
    );
    assert!(git::branch_exists(&repo_path, "astra/greeting"));
    assert!(!worktree.exists());

    // Exactly once: re-submitting an executed command does nothing.
    orch.submit(&start);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let results = host
        .ops()
        .iter()
        .filter(|o| matches!(&o.body, OpBody::CommandResult { command_op, .. } if *command_op == start.id))
        .count();
    assert_eq!(results, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_command_is_not_run() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("mkdir");
    git::init_repo(&repo_path, "main", &[("README.md", "# app\n")]).expect("init");
    let mut state = State::new("ws", "p-owner");
    for e in [
        person("p-owner"),
        repo(&repo_path),
        agent(
            "a-fake",
            Adapter::Acp {
                command: env!("CARGO_BIN_EXE_fake-acp-agent").into(),
                args: vec![],
            },
            CommandPolicy::AskMe,
        ),
        mission("m-1", MissionMode::Split, "astra/deny"),
        mission_agent("ma-1", "m-1", "a-fake", Role::Builder),
        membership("m-1", "p-owner", Access::Owner),
    ] {
        state.insert(e);
    }
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-1".into(),
        },
    );
    let ap = wait_for(&host, "approval", |s, _| {
        s.approvals.values().next().map(|a| a.id.clone())
    })
    .await;
    host.command(
        &orch,
        "p-owner",
        Command::Deny {
            approval_id: ap.clone(),
        },
    );
    wait_for(&host, "result", |s, _| s.results.get("res-m-1").cloned()).await;
    let s = host.state();
    assert_eq!(s.approvals[&ap].state, ApprovalState::Denied);
    assert!(
        s.messages
            .values()
            .any(|m| m.body.contains("Command was denied."))
    );
    let res = &s.results["res-m-1"];
    assert!(res.diff.iter().all(|f| f.path != "check.txt"));
    assert!(
        s.tasks
            .values()
            .any(|t| t.approval_id.as_deref() == Some(ap.as_str()) && t.column == TaskColumn::Done)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn race_with_command_agents_merge_a_keeps_b() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo_path = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("mkdir");
    git::init_repo(&repo_path, "main", &[("README.md", "# app\n")]).expect("init");
    let sh = |script: &str| Adapter::Command {
        command: "sh".into(),
        args: vec!["-c".into(), script.into()],
    };
    let mut state = State::new("ws", "p-owner");
    for e in [
        person("p-owner"),
        repo(&repo_path),
        agent(
            "a-cmd-a",
            sh("cat > /dev/null; echo A > a.txt; echo wrote a"),
            CommandPolicy::Auto,
        ),
        agent(
            "a-cmd-b",
            sh("cat > /dev/null; mkdir -p src && echo B > src/b.txt; echo wrote b"),
            CommandPolicy::Auto,
        ),
        agent(
            "a-review",
            sh("cat > /dev/null; echo 'Both work.'; echo 'Pick: A'"),
            CommandPolicy::Auto,
        ),
        mission("m-2", MissionMode::Race, "astra/race"),
        mission_agent("ma-a", "m-2", "a-cmd-a", Role::Builder),
        mission_agent("ma-b", "m-2", "a-cmd-b", Role::Builder),
        mission_agent("ma-r", "m-2", "a-review", Role::Reviewer),
        membership("m-2", "p-owner", Access::Owner),
    ] {
        state.insert(e);
    }
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), &tmp.path().join("data"));
    host.command(
        &orch,
        "p-owner",
        Command::StartMission {
            mission_id: "m-2".into(),
        },
    );
    wait_for(&host, "pick task", |s, _| {
        s.tasks
            .values()
            .find(|t| t.title == "Pick one of 2 results" && t.column == TaskColumn::NeedsYou)
            .map(|_| ())
    })
    .await;
    let s = host.state();
    assert_eq!(s.missions["m-2"].status, MissionStatus::NeedsYou);
    let a = &s.results["res-m-2-a"];
    let b = &s.results["res-m-2-b"];
    assert_eq!(
        (a.branch.as_str(), b.branch.as_str()),
        ("astra/race-a", "astra/race-b")
    );
    assert!(a.reviewer_pick && !b.reviewer_pick);
    assert_eq!(a.diff[0].path, "a.txt");
    assert_eq!(b.diff[0].path, "src/b.txt");
    assert!(s.messages.values().any(|m| m.body.contains("wrote a")));
    assert!(s.messages.values().any(|m| m.body.contains("Pick: A")));
    assert_eq!(
        s.mission_agents["ma-b"].branch.as_deref(),
        Some("astra/race-b")
    );

    let merge = host.command(
        &orch,
        "p-owner",
        Command::Merge {
            result_id: "res-m-2-a".into(),
        },
    );
    wait_for(&host, "merged", |s, _| {
        (s.missions["m-2"].status == MissionStatus::Merged).then_some(())
    })
    .await;
    assert_eq!(host.result_of(&merge), Some((true, None)));
    let s = host.state();
    assert_eq!(
        s.merges["m-2"].kept_branches,
        vec!["astra/race-b".to_string()]
    );
    assert_eq!(
        git::read_file_at(&repo_path, "main", "a.txt")
            .expect("read")
            .as_deref(),
        Some("A\n")
    );
    assert_eq!(
        git::read_file_at(&repo_path, "main", "src/b.txt").expect("read"),
        None
    );
    assert!(git::branch_exists(&repo_path, "astra/race-b"));
    assert_eq!(
        git::read_file_at(&repo_path, "astra/race-b", "src/b.txt")
            .expect("read")
            .as_deref(),
        Some("B\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_and_test_agents() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut state = State::new("ws", "p-owner");
    let mut missing = match agent(
        "a-missing",
        Adapter::Acp {
            command: "definitely-not-on-path-xyz".into(),
            args: vec![],
        },
        CommandPolicy::AskMe,
    ) {
        Entity::Agent(a) => a,
        _ => unreachable!(),
    };
    missing.node_id = None;
    state.insert(Entity::Agent(missing));
    state.insert(agent(
        "a-sh",
        Adapter::Command {
            command: "sh".into(),
            args: vec![],
        },
        CommandPolicy::AskMe,
    ));
    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), tmp.path());
    let c1 = host.command(
        &orch,
        "p-owner",
        Command::ConnectAgent {
            agent_id: "a-sh".into(),
        },
    );
    let c2 = host.command(
        &orch,
        "p-owner",
        Command::TestAgent {
            agent_id: "a-missing".into(),
        },
    );
    wait_for(&host, "results", |_, ops| result_in(ops, &c2)).await;
    assert_eq!(host.result_of(&c1), Some((true, None)));
    let (ok, err) = host.result_of(&c2).expect("result");
    assert!(!ok && err.expect("err").contains("not found on PATH"));
    let s = host.state();
    assert_eq!(s.agents["a-sh"].status, AgentStatus::Connected);
    assert_eq!(s.agents["a-sh"].node_id.as_deref(), Some(NODE));
    assert_eq!(s.agents["a-missing"].status, AgentStatus::NotConnected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_agent_is_marked_and_a_message_restarts_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut state = State::new("ws", "p-owner");
    state.insert(person("p-owner"));
    state.insert(membership("m-1", "p-owner", Access::Owner));
    state.insert(agent(
        "a-cat",
        Adapter::Command {
            command: "cat".into(),
            args: vec![],
        },
        CommandPolicy::Auto,
    ));
    state.insert(mission("m-1", MissionMode::Split, "astra/greeting"));
    let Entity::MissionAgent(mut ma) = mission_agent("ma-1", "m-1", "a-cat", Role::Builder) else {
        unreachable!()
    };
    // Astra stopped while this agent was working.
    ma.status = MissionAgentStatus::Working;
    ma.branch = Some("astra/greeting".into());
    state.insert(Entity::MissionAgent(ma));
    // Its worktree is still on disk, checked out on the agent's branch.
    let gitdir = tmp.path().join("gitdir");
    std::fs::create_dir_all(&gitdir).expect("gitdir");
    std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/astra/greeting\n").expect("HEAD");
    let wt = tmp.path().join("worktrees").join("m-1").join("main");
    std::fs::create_dir_all(&wt).expect("worktree");
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).expect(".git");

    let host = TestHost::new(state);
    let orch = orchestrator(host.clone(), tmp.path());
    orch.recover_interrupted();
    let s = host.state();
    assert_eq!(s.mission_agents["ma-1"].status, MissionAgentStatus::Idle);
    assert_eq!(
        s.mission_agents["ma-1"].status_detail.as_deref(),
        Some("interrupted")
    );

    let send = host.command(
        &orch,
        "p-owner",
        Command::SendMessage {
            mission_id: "m-1".into(),
            to: Some("ma-1".into()),
            body: "keep going".into(),
        },
    );
    wait_for(&host, "the restarted agent answers", |s, _| {
        (s.messages.values().any(|m| m.body.contains("keep going"))
            && s.mission_agents["ma-1"].status == MissionAgentStatus::Done)
            .then_some(())
    })
    .await;
    assert_eq!(host.result_of(&send), Some((true, None)));
}
