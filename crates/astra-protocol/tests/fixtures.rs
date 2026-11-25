use std::path::PathBuf;

use astra_protocol::*;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("fixture json")
}

#[test]
fn ops_conformance_cases_pass() {
    let doc = fixture("ops-conformance.json");
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let initial: Vec<Entity> =
            serde_json::from_value(case["initial"].clone()).expect("initial entities");
        let ops: Vec<Op> = serde_json::from_value(case["ops"].clone()).expect("ops");
        let mut state = State::new("ws", "me");
        for e in initial {
            state.insert(e);
        }
        let mut reducer = Reducer::new(state);
        reducer.apply_all(&ops);
        for expect in case["expect"].as_array().expect("expect") {
            let kind =
                EntityKind::parse(expect["kind"].as_str().expect("kind")).expect("known kind");
            let id = expect["id"].as_str().expect("id");
            let got = reducer.state.get(kind, id);
            match &expect["fields"] {
                Value::Null => assert!(
                    got.is_none(),
                    "case {name:?}: {kind}/{id} should be absent, got {got:?}"
                ),
                Value::Object(fields) => {
                    let got = got.unwrap_or_else(|| panic!("case {name:?}: {kind}/{id} missing"));
                    let json = got.to_json();
                    for (k, v) in fields {
                        assert_eq!(
                            json.get(k),
                            Some(v),
                            "case {name:?}: field {k} of {kind}/{id}"
                        );
                    }
                }
                other => panic!("bad expect {other}"),
            }
        }
        // Applying the same ops again (duplicate ids) changes nothing.
        let before = reducer.state.clone();
        reducer.apply_all(&ops);
        assert_eq!(
            before, reducer.state,
            "case {name:?}: replay must be idempotent"
        );
    }
}

#[test]
fn ops_conformance_cases_are_order_independent() {
    // Every permutation of each case's ops converges to the same state.
    let doc = fixture("ops-conformance.json");
    for case in doc["cases"].as_array().expect("cases") {
        let initial: Vec<Entity> =
            serde_json::from_value(case["initial"].clone()).expect("initial");
        let ops: Vec<Op> = serde_json::from_value(case["ops"].clone()).expect("ops");
        let run = |order: &[usize]| {
            let mut state = State::new("ws", "me");
            for e in &initial {
                state.insert(e.clone());
            }
            let mut r = Reducer::new(state);
            for &i in order {
                r.apply(&ops[i]);
            }
            r.state
        };
        let base: Vec<usize> = (0..ops.len()).collect();
        let expected = run(&base);
        for perm in permutations(&base)
            .into_iter()
            .filter(|p| causal(&initial, &ops, p))
        {
            assert_eq!(run(&perm), expected, "case {}", case["name"]);
        }
    }
}

/// Patches never precede the put that created their entity (they are causally after it).
fn causal(initial: &[Entity], ops: &[Op], order: &[usize]) -> bool {
    let mut known: std::collections::HashSet<(EntityKind, String)> = initial
        .iter()
        .map(|e| (e.kind(), e.id().to_string()))
        .collect();
    let put_somewhere = |k: EntityKind, id: &str| {
        ops.iter().any(|o| matches!(&o.body, OpBody::Put { entity } if entity.kind() == k && entity.id() == id))
    };
    for &i in order {
        match &ops[i].body {
            OpBody::Put { entity } => {
                known.insert((entity.kind(), entity.id().to_string()));
            }
            OpBody::Patch {
                entity_kind, id, ..
            } if put_somewhere(*entity_kind, id)
                && !known.contains(&(*entity_kind, id.clone())) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}

fn permutations(items: &[usize]) -> Vec<Vec<usize>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let head = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, head);
            out.push(p);
        }
    }
    out
}

#[test]
fn seed_roundtrips_unchanged() {
    let raw = fixture("seed.json");
    let seed: Seed = serde_json::from_value(raw.clone()).expect("seed parses");
    // Seed document round-trips exactly.
    assert_eq!(serde_json::to_value(&seed).expect("ser"), raw);

    // Through State: every entity comes back identical.
    let state = State::from_seed(seed.clone());
    assert_eq!(state.len(), seed.entities.len());
    let back = state.to_seed();
    let mut original: Vec<Value> = raw["entities"].as_array().expect("entities").clone();
    let mut roundtrip: Vec<Value> = back
        .entities
        .iter()
        .map(|e| serde_json::to_value(e).expect("ser"))
        .collect();
    let key = |v: &Value| format!("{}/{}", v["kind"], v["id"]);
    original.sort_by_key(key);
    roundtrip.sort_by_key(key);
    assert_eq!(original, roundtrip);
    assert_eq!(
        serde_json::to_value(&back.presence).expect("ser"),
        raw["presence"]
    );

    // State JSON: entity maps keyed by id, entities carry `kind`.
    let state_json = serde_json::to_value(&state).expect("state json");
    assert_eq!(state_json["workspaceId"], "ws-mindify");
    assert_eq!(
        state_json["missionAgents"]["ma-014-builder"]["kind"],
        "missionAgent"
    );
    assert_eq!(state_json["results"]["res-015-a"]["kind"], "result");
    assert_eq!(state_json["missions"]["m-015"]["stats"], Value::Null);
    let state2: State = serde_json::from_value(state_json.clone()).expect("state parses");
    assert_eq!(state2, state);
    assert_eq!(serde_json::to_value(&state2).expect("ser"), state_json);
}

#[test]
fn msgpack_roundtrip_of_ops_and_frames() {
    let raw = fixture("ops-conformance.json");
    let mut ops: Vec<Op> = Vec::new();
    for case in raw["cases"].as_array().expect("cases") {
        ops.extend(serde_json::from_value::<Vec<Op>>(case["ops"].clone()).expect("ops"));
    }
    ops.push(Op::new(
        Hlc::new(1, 0, "n1"),
        "p-mark",
        OpBody::Command {
            node: "n1".into(),
            command: Command::SendMessage {
                mission_id: "m-1".into(),
                to: None,
                body: "hi".into(),
            },
        },
    ));
    ops.push(Op::new(
        Hlc::new(2, 0, "n1"),
        "n1",
        OpBody::CommandResult {
            command_op: "x".into(),
            ok: false,
            error: Some("nope".into()),
        },
    ));
    let frame = ServerFrame::Ops {
        ops: ops.clone(),
        cursor: 42,
    };
    let bytes = rmp_serde::to_vec_named(&frame).expect("encode");
    let back: ServerFrame = rmp_serde::from_slice(&bytes).expect("decode");
    assert_eq!(back, frame);

    let hello = ClientFrame::Hello {
        workspace_id: "ws".into(),
        node_id: "n".into(),
        cursor: 0,
        token: None,
    };
    let json = serde_json::to_value(&hello).expect("json");
    assert_eq!(
        json,
        serde_json::json!({"t":"hello","workspaceId":"ws","nodeId":"n","cursor":0,"token":null})
    );
    let bytes = rmp_serde::to_vec_named(&hello).expect("encode");
    assert_eq!(
        rmp_serde::from_slice::<ClientFrame>(&bytes).expect("decode"),
        hello
    );
}

#[test]
fn command_and_body_shapes() {
    let op = Op {
        id: "01J0000000000000000000000Z".into(),
        hlc: "0001732180000000:000000:n1".parse().expect("hlc"),
        actor: "p-mark".into(),
        body: OpBody::Command {
            node: "n-1".into(),
            command: Command::Approve {
                approval_id: "ap-1".into(),
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&op).expect("json"),
        serde_json::json!({
            "id": "01J0000000000000000000000Z",
            "hlc": "0001732180000000:000000:n1",
            "actor": "p-mark",
            "body": {"type": "command", "node": "n-1", "command": {"type": "approve", "approvalId": "ap-1"}}
        })
    );
    let r = OpBody::CommandResult {
        command_op: "a".into(),
        ok: true,
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&r).expect("json"),
        serde_json::json!({"type":"commandResult","commandOp":"a","ok":true,"error":null})
    );
    let p = OpBody::patch(
        EntityKind::MissionAgent,
        "ma",
        [("status", serde_json::json!("working"))],
    );
    assert_eq!(
        serde_json::to_value(&p).expect("json"),
        serde_json::json!({"type":"patch","entityKind":"missionAgent","id":"ma","fields":{"status":"working"}})
    );
}

#[test]
fn typescript_interfaces_match_serde_shapes() {
    let raw = fixture("seed.json");
    let seed: Seed = serde_json::from_value(raw).expect("seed");
    let mut seen = std::collections::BTreeSet::new();
    for entity in &seed.entities {
        let ts_name = match entity.kind() {
            EntityKind::Result => "MissionResult".to_string(),
            EntityKind::Node => "RunnerNode".to_string(),
            k => {
                let s = k.as_str();
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
        };
        let mut ts_fields =
            ts::interface_fields(&ts_name).unwrap_or_else(|| panic!("interface {ts_name}"));
        let mut json_fields: Vec<String> = entity.to_json().keys().cloned().collect();
        ts_fields.sort();
        json_fields.sort();
        assert_eq!(ts_fields, json_fields, "TS interface {ts_name}");
        seen.insert(entity.kind());
    }
    assert_eq!(seen.len(), EntityKind::ALL.len(), "seed covers every kind");

    let state_fields: Vec<String> = {
        let v = serde_json::to_value(State::default()).expect("json");
        let mut k: Vec<String> = v.as_object().expect("obj").keys().cloned().collect();
        k.sort();
        k
    };
    let mut ts_state = ts::interface_fields("State").expect("State");
    ts_state.sort();
    assert_eq!(ts_state, state_fields);

    let presence = serde_json::to_value(&seed.presence[0]).expect("json");
    let mut p: Vec<String> = presence.as_object().expect("obj").keys().cloned().collect();
    p.sort();
    let mut tp = ts::interface_fields("Presence").expect("Presence");
    tp.sort();
    assert_eq!(tp, p);
}

/// Formatting-insensitive form (quotes, semicolons and whitespace are a formatter's business).
fn normalize_ts(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ';' && *c != ',')
        .map(|c| if c == '"' { '\'' } else { c })
        .collect()
}

#[test]
fn generated_file_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(ts::OUTPUT_PATH);
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        normalize_ts(&on_disk) == normalize_ts(ts::PROTOCOL_TS),
        "{} is stale; run `cargo run -p astra-protocol --bin export-ts`",
        path.display()
    );
}
