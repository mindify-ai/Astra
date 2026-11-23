//! astra-server integration tests: sync over real WebSockets, auth, device flow.
//! The Postgres variant runs when `ASTRA_TEST_DATABASE_URL` is set.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use astra_node::WsConnector;
use astra_protocol::*;
use astra_server::{ServerConfig, build_state, serve};
use astra_store::{Origin, Store};
use astra_sync::{StoreHost, SyncClient, SyncConfig, SyncHandle, SyncHost};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

struct Server {
    http: String,
    ws: String,
    shutdown: watch::Sender<bool>,
    state: astra_server::AppState,
}

async fn start(config: ServerConfig) -> Server {
    let (tx, rx) = watch::channel(false);
    let state = build_state(config, rx.clone()).await.expect("state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve(listener, state.clone(), rx));
    Server {
        http: format!("http://{addr}"),
        ws: format!("ws://{addr}/v1/sync"),
        shutdown: tx,
        state,
    }
}

fn dev_config() -> ServerConfig {
    ServerConfig {
        dev_auth: true,
        jwt_secret: Some("test-secret".into()),
        ..ServerConfig::default()
    }
}

async fn dev_session(s: &Server, name: &str) -> Value {
    let v: Value = reqwest::Client::new()
        .post(format!("{}/v1/auth/dev-token", s.http))
        .json(&json!({"name": name}))
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    v["session"].clone()
}

struct Client {
    host: Arc<Recorder>,
    handle: SyncHandle,
}

struct Recorder {
    inner: StoreHost,
    presence: Mutex<Vec<Presence>>,
}

impl SyncHost for Recorder {
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
        *self.presence.lock().expect("lock") = p;
    }
}

fn connect(s: &Server, store: Arc<Mutex<Store>>, node: &str, token: &str, ws: &str) -> Client {
    let host = Arc::new(Recorder {
        inner: StoreHost {
            store: store.clone(),
        },
        presence: Mutex::new(vec![]),
    });
    let mut cfg = SyncConfig::new(ws, node, Some(token.to_string()));
    cfg.min_backoff = Duration::from_millis(50);
    let (client, handle) = SyncClient::new(cfg, host.clone());
    tokio::spawn(client.run(Arc::new(WsConnector::new(s.ws.clone()))));
    Client { host, handle }
}

fn new_store() -> Arc<Mutex<Store>> {
    let mut st = Store::open_in_memory().expect("store");
    st.set_identity("ws-dev", "me").expect("identity");
    Arc::new(Mutex::new(st))
}

async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("timed out: {what}");
}

fn shared(store: &Arc<Mutex<Store>>) -> State {
    let mut s = store.lock().expect("lock").state().clone();
    s.me.clear();
    s
}

async fn sync_scenario(config: ServerConfig, node_id: &str) {
    let s = start(config).await;
    let mark = dev_session(&s, "Mark").await;
    let lin = dev_session(&s, "Lin").await;
    assert_eq!(mark["workspaceId"], "ws-dev");
    assert_eq!(
        mark["person"],
        json!({"kind":"person","id":"p-dev-mark","name":"Mark","initials":"MA"})
    );
    let (mark_token, lin_token) = (
        mark["token"].as_str().expect("t"),
        lin["token"].as_str().expect("t"),
    );

    let (web_store, node_store) = (new_store(), new_store());
    let web = connect(&s, web_store.clone(), "web-lin", lin_token, "ws-dev");
    let node = connect(&s, node_store.clone(), node_id, mark_token, "ws-dev");

    // Server-emitted person entities reach both clients.
    eventually("people", || {
        let st = shared(&node_store);
        st.people.contains_key("p-dev-mark") && st.people.contains_key("p-dev-lin")
    })
    .await;

    // A web client issues a command for the runner node; it is routed to the node.
    let mut clock = HlcClock::new("web-lin");
    let put = Op::new(
        clock.tick(),
        "p-dev-lin",
        OpBody::put(Person {
            id: "p-x".into(),
            name: "X".into(),
            initials: "X".into(),
        }),
    );
    let cmd = Op::new(
        clock.tick(),
        "p-dev-lin",
        OpBody::Command {
            node: node_id.into(),
            command: Command::TestAgent {
                agent_id: "a-1".into(),
            },
        },
    );
    web_store
        .lock()
        .expect("lock")
        .append(&[put, cmd.clone()], Origin::Local)
        .expect("append");
    web.handle.notify_local();
    eventually("command routed", || {
        node_store
            .lock()
            .expect("lock")
            .unexecuted_commands(node_id)
            .map(|c| c.iter().any(|o| o.id == cmd.id))
            .unwrap_or(false)
    })
    .await;
    eventually("acked", || {
        web_store
            .lock()
            .expect("lock")
            .pending_outbound()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    })
    .await;

    // Presence relay.
    node.handle.set_presence(vec![Presence {
        person_id: "p-dev-mark".into(),
        mission_id: None,
        activity: "Reviewing".into(),
        at: now_rfc3339(),
    }]);
    eventually("presence", || {
        web.host
            .presence
            .lock()
            .expect("lock")
            .iter()
            .any(|p| p.activity == "Reviewing")
    })
    .await;

    // The node goes offline; the web client keeps writing; the node reconnects and replays from its cursor.
    node.handle.shutdown();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let patch = Op::new(
        clock.tick(),
        "p-dev-lin",
        OpBody::patch(EntityKind::Person, "p-x", [("name", json!("Xavier"))]),
    );
    web_store
        .lock()
        .expect("lock")
        .append(&[patch], Origin::Local)
        .expect("append");
    web.handle.notify_local();
    eventually("acked 2", || {
        web_store
            .lock()
            .expect("lock")
            .pending_outbound()
            .map(|p| p.is_empty())
            .unwrap_or(false)
    })
    .await;
    assert!(
        shared(&node_store)
            .people
            .get("p-x")
            .is_some_and(|p| p.name == "X")
    );
    let node2 = connect(&s, node_store.clone(), node_id, mark_token, "ws-dev");
    eventually("converged", || {
        let (a, b) = (shared(&web_store), shared(&node_store));
        a == b && b.people.get("p-x").is_some_and(|p| p.name == "Xavier")
    })
    .await;
    let head = s.state.hub.log().head("ws-dev").await.expect("head");
    assert_eq!(
        node_store.lock().expect("lock").cursor().expect("cursor"),
        head
    );
    drop(node2);
    let _ = node.host;
    let _ = s.shutdown.send(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_backend_sync_converges_routes_and_replays() {
    sync_scenario(dev_config(), "n-desk").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_backend_sync_converges_routes_and_replays() {
    let Ok(url) = std::env::var("ASTRA_TEST_DATABASE_URL") else {
        eprintln!("ASTRA_TEST_DATABASE_URL not set; skipping Postgres test");
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.expect("connect");
    astra_server::pg::migrate(&pool).await.expect("migrate");
    sqlx::query("TRUNCATE ops, nodes, memberships, people, workspaces CASCADE")
        .execute(&pool)
        .await
        .expect("truncate");
    sync_scenario(
        ServerConfig {
            database_url: Some(url),
            ..dev_config()
        },
        "n-pg",
    )
    .await;
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM nodes WHERE id = 'n-pg'")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(n.0, 1);
    // Sequence numbers are gap-free.
    let (max, count): (i64, i64) =
        sqlx::query_as("SELECT MAX(seq), COUNT(*) FROM ops WHERE workspace_id = 'ws-dev'")
            .fetch_one(&pool)
            .await
            .expect("q");
    assert_eq!(max, count);
}

#[tokio::test]
async fn bad_tokens_get_unauthorized_and_4401() {
    let s = start(dev_config()).await;
    let session = dev_session(&s, "Mark").await;
    let token = session["token"].as_str().expect("token").to_string();
    for (ws, tok) in [
        ("ws-dev", Some("garbage".to_string())),
        ("ws-other", Some(token.clone())),
        ("ws-dev", None),
    ] {
        let (mut sock, _) = tokio_tungstenite::connect_async(format!("{}?format=json", s.ws))
            .await
            .expect("connect");
        let hello = json!({"t":"hello","workspaceId":ws,"nodeId":"n","cursor":0,"token":tok});
        sock.send(Message::Text(hello.to_string().into()))
            .await
            .expect("send");
        let mut got_error = false;
        let mut close_code = None;
        while let Some(Ok(m)) = sock.next().await {
            match m {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t).expect("json");
                    assert_eq!(v, json!({"t":"error","message":"unauthorized"}));
                    got_error = true;
                }
                Message::Close(Some(f)) => {
                    close_code = Some(u16::from(f.code));
                    break;
                }
                _ => {}
            }
        }
        assert!(got_error, "error frame for {ws}");
        assert_eq!(close_code, Some(4401));
    }
    // A good token works (JSON format): welcome arrives.
    let (mut sock, _) = tokio_tungstenite::connect_async(format!("{}?format=json", s.ws))
        .await
        .expect("connect");
    let hello = json!({"t":"hello","workspaceId":"ws-dev","nodeId":"n","cursor":0,"token":token});
    sock.send(Message::Text(hello.to_string().into()))
        .await
        .expect("send");
    let Some(Ok(Message::Text(t))) = sock.next().await else {
        panic!("no welcome")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&t).expect("json")["t"],
        "welcome"
    );
}

#[tokio::test]
async fn http_endpoints() {
    let s = start(dev_config()).await;
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{}/healthz", s.http))
            .send()
            .await
            .expect("h")
            .text()
            .await
            .expect("t"),
        "ok"
    );
    assert_eq!(
        http.get(format!("{}/readyz", s.http))
            .send()
            .await
            .expect("r")
            .status(),
        200
    );
    let session = dev_session(&s, "Jo Kim").await;
    assert_eq!(session["person"]["initials"], "JK");
    let me: Value = http
        .get(format!("{}/v1/me", s.http))
        .bearer_auth(session["token"].as_str().expect("t"))
        .send()
        .await
        .expect("me")
        .json()
        .await
        .expect("json");
    assert_eq!(
        me,
        json!({"person": session["person"], "workspaceId": "ws-dev"})
    );
    let r = http
        .get(format!("{}/v1/me", s.http))
        .send()
        .await
        .expect("me");
    assert_eq!(r.status(), 401);
    assert_eq!(
        r.json::<Value>().await.expect("j"),
        json!({"error":"unauthorized"})
    );
    let r = http
        .post(format!("{}/v1/auth/device/start", s.http))
        .json(&json!({"provider":"github"}))
        .send()
        .await
        .expect("s");
    assert_eq!(r.status(), 503);
    assert_eq!(
        r.json::<Value>().await.expect("j"),
        json!({"error":"notConfigured"})
    );
    let r = http
        .post(format!("{}/v1/auth/device/start", s.http))
        .json(&json!({"provider":"bitbucket"}))
        .send()
        .await
        .expect("s");
    assert_eq!(r.status(), 400);
    assert_eq!(
        r.json::<Value>().await.expect("j"),
        json!({"error":"badProvider"})
    );

    let prod = start(ServerConfig {
        jwt_secret: Some("x".into()),
        ..ServerConfig::default()
    })
    .await;
    let r = http
        .post(format!("{}/v1/auth/dev-token", prod.http))
        .json(&json!({"name":"x"}))
        .send()
        .await
        .expect("d");
    assert_eq!(r.status(), 404);
    let (tx, rx) = watch::channel(false);
    assert!(matches!(
        build_state(ServerConfig::default(), rx).await,
        Err(astra_server::StartupError::MissingJwtSecret)
    ));
    drop(tx);
}

#[tokio::test]
async fn github_device_flow_status_mapping() {
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let gh = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .and(body_string_contains("client_id=gh-client"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://github.com/login/device",
            "expires_in": 899, "interval": 5
        })))
        .mount(&gh)
        .await;
    for (code, body) in [
        ("pending", json!({"error": "authorization_pending"})),
        ("slow", json!({"error": "slow_down", "interval": 10})),
        ("expired", json!({"error": "expired_token"})),
        ("denied", json!({"error": "access_denied"})),
        (
            "good",
            json!({"access_token": "gho_secret", "token_type": "bearer", "scope": "read:user"}),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_string_contains(format!("device_code={code}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&gh)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer gho_secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"login": "OctoCat", "name": "The Octocat", "id": 1})),
        )
        .mount(&gh)
        .await;

    let mut config = dev_config();
    config.providers.github_client_id = Some("gh-client".into());
    config.providers.github_oauth_base = gh.uri();
    config.providers.github_api_base = gh.uri();
    let s = start(config).await;
    let http = reqwest::Client::new();
    let start: Value = http
        .post(format!("{}/v1/auth/device/start", s.http))
        .json(&json!({"provider":"github"}))
        .send()
        .await
        .expect("start")
        .json()
        .await
        .expect("json");
    assert_eq!(
        start,
        json!({"deviceCode":"dc","userCode":"ABCD-1234","verificationUri":"https://github.com/login/device",
               "verificationUriComplete":null,"interval":5,"expiresIn":899})
    );
    let poll = |code: &str| {
        let http = http.clone();
        let url = format!("{}/v1/auth/device/poll", s.http);
        let code = code.to_string();
        async move {
            http.post(url)
                .json(&json!({"provider":"github","deviceCode":code}))
                .send()
                .await
                .expect("poll")
                .json::<Value>()
                .await
                .expect("json")
        }
    };
    assert_eq!(poll("pending").await, json!({"status":"pending"}));
    assert_eq!(
        poll("slow").await,
        json!({"status":"slowDown","interval":10})
    );
    assert_eq!(poll("expired").await, json!({"status":"expired"}));
    assert_eq!(poll("denied").await, json!({"status":"denied"}));
    let done = poll("good").await;
    assert_eq!(done["status"], "complete");
    let session = &done["session"];
    assert_eq!(session["workspaceId"], "ws-gh-octocat");
    assert_eq!(
        session["person"],
        json!({"kind":"person","id":"p-gh-octocat","name":"The Octocat","initials":"TO"})
    );
    assert!(session["expiresAt"].as_str().expect("exp").ends_with('Z'));
    let claims = s
        .state
        .jwt
        .verify(session["token"].as_str().expect("t"))
        .expect("valid");
    assert_eq!(
        (claims.sub.as_str(), claims.ws.as_str()),
        ("p-gh-octocat", "ws-gh-octocat")
    );
    // The person was put into the workspace log once (first sign-in only).
    poll("good").await;
    let ops = s
        .state
        .hub
        .log()
        .since("ws-gh-octocat", 0, 100)
        .await
        .expect("since");
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(&ops[0].1.body, OpBody::Put { entity: Entity::Person(p) } if p.id == "p-gh-octocat")
    );
}

/// Reads JSON frames until one with tag `t` arrives (or the socket closes).
async fn next_frame<S>(sock: &mut S, t: &str) -> Option<Value>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match tokio::time::timeout(Duration::from_secs(5), sock.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                let v: Value = serde_json::from_str(&txt).expect("json");
                if v["t"] == t {
                    return Some(v);
                }
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Err(_) => return None,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) => return None,
        }
    }
}

#[tokio::test]
async fn pushed_ops_cannot_claim_someone_else() {
    let s = start(dev_config()).await;
    let session = dev_session(&s, "Mark").await;
    let token = session["token"].as_str().expect("token").to_string();
    let me = session["person"]["id"]
        .as_str()
        .expect("person")
        .to_string();
    let hlc = |n: u64| format!("{:016}:000000:n-web", 1_732_180_000_000u64 + n);
    let command = |id: &str, actor: &str, n: u64| {
        json!({"id": id, "hlc": hlc(n), "actor": actor,
               "body": {"type":"command","node":"n-desktop","command":{"type":"approve","approvalId":"ap-1"}}})
    };

    let open = || async {
        let (mut sock, _) = tokio_tungstenite::connect_async(format!("{}?format=json", s.ws))
            .await
            .expect("connect");
        let hello =
            json!({"t":"hello","workspaceId":"ws-dev","nodeId":"n-web","cursor":0,"token":token});
        sock.send(Message::Text(hello.to_string().into()))
            .await
            .expect("send");
        assert!(next_frame(&mut sock, "welcome").await.is_some());
        sock
    };

    // A command forged as another person is refused, the socket closes, and nothing is stored.
    let mut sock = open().await;
    let forged = command("01JFORGED00000000000000000", "p-dev-lin", 1);
    sock.send(Message::Text(
        json!({"t":"push","ops":[forged]}).to_string().into(),
    ))
    .await
    .expect("push");
    let err = next_frame(&mut sock, "error").await.expect("error frame");
    assert!(
        err["message"]
            .as_str()
            .expect("message")
            .contains("claims actor p-dev-lin")
    );
    assert!(
        next_frame(&mut sock, "ack").await.is_none(),
        "forged op must not be acked"
    );

    // A command node ids may not issue commands either, even the connection's own node id.
    let mut sock = open().await;
    let as_node = command("01JASNODE00000000000000000", "n-web", 2);
    sock.send(Message::Text(
        json!({"t":"push","ops":[as_node]}).to_string().into(),
    ))
    .await
    .expect("push");
    assert!(next_frame(&mut sock, "error").await.is_some());

    // The signed-in person's command, and a node-authored patch from the connection's own node, are accepted.
    let mut sock = open().await;
    let mine = command("01JMINE0000000000000000000", &me, 3);
    let node_patch = json!({"id":"01JNODE0000000000000000000","hlc":hlc(4),"actor":"n-web",
        "body":{"type":"patch","entityKind":"person","id":me,"fields":{"name":"Mark C"}}});
    sock.send(Message::Text(
        json!({"t":"push","ops":[mine, node_patch]})
            .to_string()
            .into(),
    ))
    .await
    .expect("push");
    let ack = next_frame(&mut sock, "ack").await.expect("ack");
    assert_eq!(
        ack["ids"],
        json!(["01JMINE0000000000000000000", "01JNODE0000000000000000000"])
    );

    // Only the accepted ops are in the log.
    let replay_ids = {
        let mut fresh = open().await;
        let mut ids = Vec::new();
        while let Some(f) =
            tokio::time::timeout(Duration::from_millis(500), next_frame(&mut fresh, "ops"))
                .await
                .ok()
                .flatten()
        {
            for op in f["ops"].as_array().expect("ops") {
                ids.push(op["id"].as_str().expect("id").to_string());
            }
        }
        ids
    };
    assert!(replay_ids.contains(&"01JMINE0000000000000000000".to_string()));
    assert!(
        !replay_ids
            .iter()
            .any(|id| id.starts_with("01JFORGED") || id.starts_with("01JASNODE"))
    );
    let _ = s.shutdown.send(true);
}
