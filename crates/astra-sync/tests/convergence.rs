//! N clients making concurrent ops with random delivery and reconnects converge to identical state.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use astra_protocol::{
    ClientFrame, EntityKind, HlcClock, Op, OpBody, Person, Reducer, ServerFrame, State,
};
use astra_store::{Origin, Store};
use astra_sync::{AllowAll, ClientCore, ConnId, Hub, MemoryOpLog, OpLog, StoreHost};
use proptest::prelude::*;
use tokio::sync::mpsc::UnboundedReceiver;

const WS: &str = "ws";

struct SimClient {
    store: Arc<Mutex<Store>>,
    host: StoreHost,
    clock: HlcClock,
    core: ClientCore,
    conn: Option<(ConnId, UnboundedReceiver<ServerFrame>)>,
    outbox: VecDeque<ClientFrame>,
}

impl SimClient {
    fn new(i: usize) -> SimClient {
        let mut store = Store::open_in_memory().expect("store");
        store.set_identity(WS, &format!("p{i}")).expect("identity");
        let store = Arc::new(Mutex::new(store));
        SimClient {
            host: StoreHost { store: store.clone() },
            store,
            clock: HlcClock::new(format!("n{i}")),
            core: ClientCore::new(WS, format!("n{i}"), None),
            conn: None,
            outbox: VecDeque::new(),
        }
    }

    /// Shared state (`me` is per-client and excluded).
    fn state(&self) -> State {
        let mut s = self.store.lock().expect("lock").state().clone();
        s.me.clear();
        s
    }
}

#[derive(Debug, Clone)]
enum Action {
    Put { client: usize, person: u8, name: u8 },
    Patch { client: usize, person: u8, name: u8, initials: bool },
    Delete { client: usize, person: u8 },
    ToHub { client: usize },
    ToClient { client: usize },
    Disconnect { client: usize },
    Connect { client: usize },
}

fn action(n: usize) -> impl Strategy<Value = Action> {
    let c = 0..n;
    prop_oneof![
        1 => (c.clone(), 0u8..4, 0u8..8).prop_map(|(client, person, name)| Action::Put { client, person, name }),
        1 => (c.clone(), 0u8..4, 0u8..8, any::<bool>())
            .prop_map(|(client, person, name, initials)| Action::Patch { client, person, name, initials }),
        1 => (c.clone(), 0u8..4).prop_map(|(client, person)| Action::Delete { client, person }),
        3 => c.clone().prop_map(|client| Action::ToHub { client }),
        3 => c.clone().prop_map(|client| Action::ToClient { client }),
        1 => c.clone().prop_map(|client| Action::Disconnect { client }),
        1 => c.prop_map(|client| Action::Connect { client }),
    ]
}

fn local(c: &mut SimClient, body: OpBody) {
    let op = Op::new(c.clock.tick(), c.clock.node().to_string(), body);
    c.store.lock().expect("lock").append(&[op], Origin::Local).expect("append");
    let frames = c.core.on_local_change(&c.host).expect("local change");
    c.outbox.extend(frames);
}

async fn step(hub: &Hub, clients: &mut [SimClient], a: &Action) {
    match *a {
        Action::Put { client, person, name } => {
            let c = &mut clients[client];
            let p = Person { id: format!("p-{person}"), name: format!("name-{name}"), initials: "X".into() };
            local(c, OpBody::put(p));
        }
        Action::Patch { client, person, name, initials } => {
            let c = &mut clients[client];
            let id = format!("p-{person}");
            // Clients only patch entities they know (patches are causally after the put).
            if !c.state().contains(EntityKind::Person, &id) {
                return;
            }
            let field = if initials { "initials" } else { "name" };
            local(c, OpBody::patch(EntityKind::Person, id, [(field, format!("v{name}").into())]));
        }
        Action::Delete { client, person } => {
            let c = &mut clients[client];
            local(c, OpBody::Delete { entity_kind: EntityKind::Person, id: format!("p-{person}") });
        }
        Action::ToHub { client } => {
            let c = &mut clients[client];
            let Some((conn, _)) = &c.conn else { return };
            if let Some(frame) = c.outbox.pop_front() {
                hub.handle(*conn, frame).await.expect("hub handles frame");
            }
        }
        Action::ToClient { client } => {
            let c = &mut clients[client];
            let Some((_, rx)) = &mut c.conn else { return };
            if let Ok(frame) = rx.try_recv() {
                if let ServerFrame::Ops { ops, .. } = &frame {
                    for op in ops {
                        c.clock.observe(&op.hlc);
                    }
                }
                let out = c.core.on_frame(&c.host, frame).expect("client handles frame");
                c.outbox.extend(out);
            }
        }
        Action::Disconnect { client } => {
            let c = &mut clients[client];
            if let Some((conn, _)) = c.conn.take() {
                hub.disconnect(conn);
                c.core.on_disconnected();
                c.outbox.clear();
            }
        }
        Action::Connect { client } => {
            let c = &mut clients[client];
            if c.conn.is_none() {
                let (conn, rx) = hub.connect();
                c.conn = Some((conn, rx));
                let hello = c.core.on_connected(&c.host).expect("hello");
                c.outbox.extend(hello);
            }
        }
    }
}

async fn quiesce(hub: &Hub, clients: &mut [SimClient]) {
    for i in 0..clients.len() {
        step(hub, clients, &Action::Connect { client: i }).await;
    }
    for _ in 0..10_000 {
        let mut progressed = false;
        for i in 0..clients.len() {
            while !clients[i].outbox.is_empty() {
                step(hub, clients, &Action::ToHub { client: i }).await;
                progressed = true;
            }
            loop {
                let has = clients[i].conn.as_mut().is_some_and(|(_, rx)| !rx.is_empty());
                if !has {
                    break;
                }
                step(hub, clients, &Action::ToClient { client: i }).await;
                progressed = true;
            }
        }
        if !progressed {
            return;
        }
    }
    panic!("did not quiesce");
}

fn run_case(n: usize, actions: Vec<Action>) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");
    rt.block_on(async move {
        let log = Arc::new(MemoryOpLog::new());
        let hub = Hub::new(log.clone(), Arc::new(AllowAll));
        let mut clients: Vec<SimClient> = (0..n).map(SimClient::new).collect();
        for a in &actions {
            step(&hub, &mut clients, a).await;
        }
        quiesce(&hub, &mut clients).await;

        let reference = clients[0].state();
        for (i, c) in clients.iter().enumerate() {
            assert_eq!(c.state(), reference, "client {i} diverged");
            assert!(c.store.lock().expect("lock").pending_outbound().expect("pending").is_empty());
            assert_eq!(
                c.store.lock().expect("lock").cursor().expect("cursor"),
                log.head(WS).await.expect("head"),
                "client {i} cursor"
            );
        }
        // The server log, reduced in server order, gives the same state.
        let all = log.since(WS, 0, usize::MAX).await.expect("since");
        let mut r = Reducer::new(State::new(WS, "p0"));
        r.apply_all(all.iter().map(|(_, op)| op));
        let mut expected = r.state;
        expected.me.clear();
        assert_eq!(expected, reference);
    });
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, .. ProptestConfig::default() })]

    #[test]
    fn clients_converge(n in 2usize..5, actions in prop::collection::vec(action(4), 1..120)) {
        let actions: Vec<Action> = actions.into_iter().filter(|a| match a {
            Action::Put { client, .. } | Action::Patch { client, .. } | Action::Delete { client, .. }
            | Action::ToHub { client } | Action::ToClient { client } | Action::Disconnect { client }
            | Action::Connect { client } => *client < n,
        }).collect();
        run_case(n, actions);
    }
}
