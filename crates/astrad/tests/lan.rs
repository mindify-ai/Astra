//! LAN pairing and sync over the Noise WebSocket.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_node::noise::{self, NoiseWsConnector};
use astra_node::{Node, NodeConfig};
use astra_protocol::*;
use astra_store::{Origin, Store};
use astra_sync::{Connector, StoreHost, SyncClient, SyncConfig};
use astrad::lan::{LanState, router};
use astrad::pairing::LanFiles;

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("timed out: {what}");
}

fn phone_store() -> Arc<Mutex<Store>> {
    let mut s = Store::open_in_memory().expect("store");
    s.set_identity("ws-home", "p-me").expect("id");
    Arc::new(Mutex::new(s))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_and_sync_over_noise() {
    let tmp = tempfile::tempdir().expect("tmp");
    let node = Node::open(NodeConfig::new(
        tmp.path().join("desk"),
        "n-desk",
        "ws-home",
        "p-me",
    ))
    .await
    .expect("node");
    let files = LanFiles::new(&tmp.path().join("desk"));
    let key = files.key().expect("key");
    let fp = noise::fingerprint(&key.public);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("ws://{}/v1/lan", listener.local_addr().expect("addr"));
    tokio::spawn(
        axum::serve(
            listener,
            router(LanState::new(node.clone(), files.clone(), key)),
        )
        .into_future(),
    );

    // The desktop already has data.
    let op = node.make_op(
        "p-me",
        OpBody::put(Person {
            id: "p-a".into(),
            name: "Ann".into(),
            initials: "AN".into(),
        }),
    );
    node.apply_ops(vec![op]).expect("apply");

    let phone_key = noise::generate_key().expect("key");
    // Wrong fingerprint: refused before any frame is sent.
    let bad = NoiseWsConnector {
        url: url.clone(),
        fingerprint: "00".repeat(32),
        key: phone_key.clone(),
    };
    assert!(bad.connect().await.is_err());

    // Unpaired key and a wrong code: unauthorized.
    {
        let store = phone_store();
        let host = Arc::new(StoreHost {
            store: store.clone(),
        });
        let (c, h) = SyncClient::new(
            SyncConfig::new("ws-home", "n-phone", Some("pair:NOPE1234".into())),
            host,
        );
        tokio::spawn(c.run(Arc::new(NoiseWsConnector {
            url: url.clone(),
            fingerprint: fp.clone(),
            key: phone_key.clone(),
        })));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(store.lock().expect("lock").state().people.is_empty());
        h.shutdown();
    }

    // Pair with a one-time code.
    let code = files.create_code(60).expect("code");
    let store = phone_store();
    let host = Arc::new(StoreHost {
        store: store.clone(),
    });
    let mut cfg = SyncConfig::new("ws-home", "n-phone", Some(format!("pair:{code}")));
    cfg.min_backoff = Duration::from_millis(50);
    let (c, h) = SyncClient::new(cfg, host);
    tokio::spawn(c.run(Arc::new(NoiseWsConnector {
        url: url.clone(),
        fingerprint: fp.clone(),
        key: phone_key.clone(),
    })));
    eventually("replay to phone", || {
        store
            .lock()
            .expect("lock")
            .state()
            .people
            .contains_key("p-a")
    })
    .await;
    assert!(files.is_paired(&phone_key.public));

    // Phone → desktop.
    let mut clock = HlcClock::new("n-phone");
    let op = Op::new(
        clock.tick(),
        "p-me",
        OpBody::put(Person {
            id: "p-b".into(),
            name: "Bo".into(),
            initials: "BO".into(),
        }),
    );
    store
        .lock()
        .expect("lock")
        .append(&[op], Origin::Local)
        .expect("append");
    h.notify_local();
    eventually("push to desktop", || {
        node.with_state(|s| s.people.contains_key("p-b"))
    })
    .await;
    eventually("acked", || {
        store
            .lock()
            .expect("lock")
            .pending_outbound()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    })
    .await;

    // Desktop → phone, live.
    let op = node.make_op(
        "p-me",
        OpBody::patch(
            EntityKind::Person,
            "p-a",
            [("name", serde_json::json!("Anna"))],
        ),
    );
    node.apply_ops(vec![op]).expect("apply");
    eventually("live update", || {
        store
            .lock()
            .expect("lock")
            .state()
            .people
            .get("p-a")
            .is_some_and(|p| p.name == "Anna")
    })
    .await;

    // Presence relays to the desktop node.
    h.set_presence(vec![Presence {
        person_id: "p-me".into(),
        mission_id: None,
        activity: "On the phone".into(),
        at: now_rfc3339(),
    }]);
    eventually("presence", || {
        node.presence().iter().any(|p| p.activity == "On the phone")
    })
    .await;
    h.shutdown();

    // Reconnect later without a code: the paired key is enough; replay resumes from the cursor.
    let op = node.make_op(
        "p-me",
        OpBody::put(Person {
            id: "p-c".into(),
            name: "Cy".into(),
            initials: "CY".into(),
        }),
    );
    node.apply_ops(vec![op]).expect("apply");
    let (c, _h2) = SyncClient::new(
        SyncConfig::new("ws-home", "n-phone", None),
        Arc::new(StoreHost {
            store: store.clone(),
        }),
    );
    tokio::spawn(c.run(Arc::new(NoiseWsConnector {
        url,
        fingerprint: fp,
        key: phone_key,
    })));
    eventually("catch up", || {
        store
            .lock()
            .expect("lock")
            .state()
            .people
            .contains_key("p-c")
    })
    .await;
    let desk = node.with_state(|s| s.people.clone());
    eventually("converged", || {
        store.lock().expect("lock").state().people == desk
    })
    .await;
}
