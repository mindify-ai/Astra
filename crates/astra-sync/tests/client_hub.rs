//! The async SyncClient against an in-process hub.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_protocol::{Command, EntityKind, HlcClock, Op, OpBody, Person, Presence, State};
use astra_store::{Origin, Store};
use astra_sync::{
    AllowAll, Hub, HubConnector, MemoryOpLog, OpLog, StoreHost, SyncClient, SyncConfig, SyncHost,
};

struct Recording {
    inner: StoreHost,
    presence: Mutex<Vec<Vec<Presence>>>,
}

impl SyncHost for Recording {
    fn cursor(&self) -> astra_sync::Result<u64> {
        self.inner.cursor()
    }
    fn pending(&self) -> astra_sync::Result<Vec<Op>> {
        self.inner.pending()
    }
    fn apply_remote(&self, ops: Vec<Op>, cursor: u64) -> astra_sync::Result<()> {
        self.inner.apply_remote(ops, cursor)
    }
    fn acked(&self, ids: &[String]) -> astra_sync::Result<()> {
        self.inner.acked(ids)
    }
    fn remote_presence(&self, p: Vec<Presence>) {
        self.presence.lock().expect("lock").push(p);
    }
}

fn store() -> Arc<Mutex<Store>> {
    let mut s = Store::open_in_memory().expect("store");
    s.set_identity("ws", "me").expect("id");
    Arc::new(Mutex::new(s))
}

async fn eventually(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn two_clients_sync_through_hub() {
    let log = Arc::new(MemoryOpLog::new());
    let hub = Hub::new(log.clone(), Arc::new(AllowAll));
    let connector = Arc::new(HubConnector { hub: hub.clone() });

    let (sa, sb) = (store(), store());
    let ha = Arc::new(Recording {
        inner: StoreHost { store: sa.clone() },
        presence: Mutex::new(vec![]),
    });
    let hb = Arc::new(Recording {
        inner: StoreHost { store: sb.clone() },
        presence: Mutex::new(vec![]),
    });
    let (ca, handle_a) = SyncClient::new(SyncConfig::new("ws", "n-a", None), ha.clone());
    let (cb, handle_b) = SyncClient::new(SyncConfig::new("ws", "n-b", None), hb.clone());
    tokio::spawn(ca.run(connector.clone()));
    tokio::spawn(cb.run(connector.clone()));

    let mut clock = HlcClock::new("n-a");
    let put = Op::new(
        clock.tick(),
        "p-a",
        OpBody::put(Person {
            id: "p-1".into(),
            name: "One".into(),
            initials: "O".into(),
        }),
    );
    let cmd = Op::new(
        clock.tick(),
        "p-a",
        OpBody::Command {
            node: "n-b".into(),
            command: Command::TestAgent {
                agent_id: "a".into(),
            },
        },
    );
    sa.lock()
        .expect("lock")
        .append(&[put, cmd.clone()], Origin::Local)
        .expect("append");
    handle_a.notify_local();

    eventually(|| {
        sb.lock()
            .expect("lock")
            .state()
            .contains(EntityKind::Person, "p-1")
    })
    .await;
    // The command op is routed to node n-b.
    eventually(|| {
        sb.lock()
            .expect("lock")
            .unexecuted_commands("n-b")
            .map(|c| c.len() == 1)
            .unwrap_or(false)
    })
    .await;
    eventually(|| {
        sa.lock()
            .expect("lock")
            .pending_outbound()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    })
    .await;

    handle_b.set_presence(vec![Presence {
        person_id: "p-b".into(),
        mission_id: None,
        activity: "Looking".into(),
        at: "2026-01-01T00:00:00Z".into(),
    }]);
    eventually(|| {
        ha.presence
            .lock()
            .expect("lock")
            .iter()
            .any(|p| p.len() == 1)
    })
    .await;

    // B goes away; A keeps writing; B comes back and catches up from its cursor.
    handle_b.shutdown();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let patch = Op::new(
        clock.tick(),
        "p-a",
        OpBody::patch(EntityKind::Person, "p-1", [("name", "Uno".into())]),
    );
    sa.lock()
        .expect("lock")
        .append(&[patch], Origin::Local)
        .expect("append");
    handle_a.notify_local();
    eventually(|| {
        sa.lock()
            .expect("lock")
            .pending_outbound()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    })
    .await;

    let (cb2, _h) = SyncClient::new(SyncConfig::new("ws", "n-b", None), hb.clone());
    tokio::spawn(cb2.run(connector.clone()));
    eventually(|| {
        let a: State = sa.lock().expect("lock").state().clone();
        let b: State = sb.lock().expect("lock").state().clone();
        a == b && b.people.get("p-1").is_some_and(|p| p.name == "Uno")
    })
    .await;
    let head = log.head("ws").await.expect("head");
    assert_eq!(sb.lock().expect("lock").cursor().expect("cursor"), head);
    assert_eq!(log.head("ws").await.expect("head"), 3);
}
