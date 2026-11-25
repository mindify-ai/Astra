//! Astra's native shell: one Tauri app for macOS, Windows, Linux, iOS and Android.
//!
//! The webview renders `@astra/app`; this crate embeds an `astra-node` (SQLite store + sync, plus the agent
//! runtime on desktop) and exposes it through a few commands and events that `TauriClient` consumes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use astra_node::{Node, NodeConfig, NodeEvent, RelayConfig};
use astra_protocol::{Op, Presence, Snapshot};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

mod files;
mod github;
mod terminal;

/// A first launch starts with only you and the catalog of agents you can connect: no demo data.
const STARTER: &str = include_str!("../../../../fixtures/starter.json");

struct AppState {
    node: Arc<Node>,
    info: NodeInfo,
    /// The app data directory (settings, node id; the local-only workspace's store).
    root: PathBuf,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeInfo {
    node_id: String,
    workspace_id: String,
    me: String,
    /// The sync server the app signed in to through the device flow; `None` in a local-only workspace.
    account: Option<Account>,
    /// The GitHub account this app signed in with directly (no sync server).
    github: Option<GithubIdentity>,
}

/// Who signed in with GitHub from the app. Only the profile is kept; GitHub's token never is.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubIdentity {
    login: String,
    name: String,
}

/// Who the app signed in as, and where. The relay's session token is kept in `Settings::relay_token`; the
/// provider's (GitHub/GitLab) token never reaches the app.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    /// The relay's HTTP base URL.
    server: String,
    person: serde_json::Value,
    expires_at: String,
}

/// Local settings (not synced). Keys never live here: they stay in the OS keychain.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    relay_url: Option<String>,
    relay_token: Option<String>,
    workspace_id: Option<String>,
    me: Option<String>,
    /// Set by signing in from the app (`sign_in`); its workspace gets its own store under `workspaces/`.
    account: Option<Account>,
    /// Set by signing in with GitHub directly (`github_device_poll`).
    github: Option<GithubIdentity>,
}

/// A relay session from the device flow (`@astra/client` `Session`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionInput {
    token: String,
    workspace_id: String,
    person: serde_json::Value,
    expires_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresenceInput {
    mission_id: Option<String>,
    activity: String,
}

#[tauri::command]
fn node_info(state: State<'_, AppState>) -> NodeInfo {
    state.info.clone()
}

/// Signs the app in to a sync server with a session from the device flow, then restarts on that workspace.
#[tauri::command]
fn sign_in(
    app: AppHandle,
    state: State<'_, AppState>,
    server: String,
    session: SessionInput,
) -> Result<(), String> {
    let server = server.trim().trim_end_matches('/').to_string();
    let relay_url = sync_url(&server)?;
    workspace_dir(&state.root, &session.workspace_id)?;
    let me = session
        .person
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
        .ok_or("the session has no person")?
        .to_string();
    let mut settings = read_settings(&state.root);
    settings.relay_url = Some(relay_url);
    settings.relay_token = Some(session.token);
    settings.workspace_id = Some(session.workspace_id);
    settings.me = Some(me);
    settings.account = Some(Account {
        server,
        person: session.person,
        expires_at: session.expires_at,
    });
    write_settings(&state.root, &settings).map_err(|e| e.to_string())?;
    state.node.shutdown();
    app.restart()
}

/// Starts signing in with GitHub directly: a device code from GitHub, with the built-in client id.
#[tauri::command]
async fn github_device_start() -> Result<github::DeviceCode, String> {
    github::start().await.map_err(str::to_string)
}

/// Polls GitHub once. On approval, you take your GitHub name in this workspace (the `person` entity you sign
/// as), GitHub's token goes to the OS keychain, the login is remembered in the settings, and a `complete` poll
/// result is returned whose session has no token: there is no server session, and GitHub's token stays here.
#[tauri::command]
async fn github_device_poll(
    state: State<'_, AppState>,
    device_code: String,
) -> Result<serde_json::Value, String> {
    let (profile, token) = match github::poll(&device_code).await.map_err(str::to_string)? {
        Ok(done) => done,
        Err(unfinished) => return Ok(github::poll_json(&unfinished)),
    };
    let me = state.info.me.clone();
    let person = astra_protocol::Person {
        id: me.clone(),
        initials: initials(&profile.name),
        name: profile.name.clone(),
    };
    let op = state
        .node
        .make_op(me, astra_protocol::OpBody::put(person.clone()));
    state
        .node
        .apply(vec![op])
        .await
        .map_err(|e| e.to_string())?;
    // Kept for listing and cloning your repositories; the keychain only.
    github_token::set(&token)?;
    drop(token);
    let mut settings = read_settings(&state.root);
    settings.github = Some(GithubIdentity {
        login: profile.login,
        name: profile.name,
    });
    write_settings(&state.root, &settings).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "status": "complete",
        "session": {
            "token": "",
            "workspaceId": state.info.workspace_id,
            "person": astra_protocol::Entity::Person(person),
            "expiresAt": "",
        },
    }))
}

/// Your GitHub repositories, most recently pushed first. `signedOut` when there is no token or GitHub no
/// longer accepts it (the stale token is then forgotten).
#[tauri::command]
async fn github_repos() -> Result<Vec<github::Repo>, String> {
    let token = github_token::get().ok_or("signedOut")?;
    match github::repos(&token).await {
        Err("signedOut") => {
            let _ = github_token::delete();
            Err("signedOut".into())
        }
        other => other.map_err(str::to_string),
    }
}

/// Clones `owner/name` into `<parent>/<name>` (default parent: `~/Astra`) and returns the folder. A folder
/// that already holds that repository is returned as it is.
#[tauri::command]
async fn github_clone(full_name: String, parent: Option<String>) -> Result<String, String> {
    if !github::valid_full_name(&full_name) {
        return Err(format!("not a GitHub repository: {full_name}"));
    }
    let token = github_token::get().ok_or("signedOut")?;
    let parent = match parent.filter(|p| !p.trim().is_empty()) {
        Some(p) => PathBuf::from(p),
        None => default_clone_parent()?,
    };
    let name = full_name
        .rsplit('/')
        .next()
        .unwrap_or(&full_name)
        .to_string();
    let dest = parent.join(&name);
    tauri::async_runtime::spawn_blocking(move || {
        if dest.exists() {
            return if same_origin(&dest, &full_name) {
                Ok(dest.to_string_lossy().into_owned())
            } else {
                Err(format!(
                    "{} already exists. Pick another folder to clone into.",
                    dest.display()
                ))
            };
        }
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        github::clone(&token, &full_name, &dest)?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets the GitHub account: its token leaves the keychain. Your name in the workspace stays.
#[tauri::command]
fn github_sign_out(state: State<'_, AppState>) -> Result<(), String> {
    github_token::delete()?;
    let mut settings = read_settings(&state.root);
    settings.github = None;
    write_settings(&state.root, &settings).map_err(|e| e.to_string())
}

/// `~/Astra`, where clones go unless you pick another folder.
fn default_clone_parent() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Astra"))
        .ok_or_else(|| "no home folder".into())
}

/// Whether `dir` is a clone of `github.com/<full_name>` (its `origin`, with or without `.git`).
fn same_origin(dir: &Path, full_name: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
        .is_some_and(|url| {
            let want = format!("github.com/{}", full_name.to_lowercase());
            url.trim_end_matches(".git").ends_with(&want)
        })
}

/// GitHub's token in the OS keychain (desktop). Phones don't clone, so they keep none.
mod github_token {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("ai.mindify.astra", "github").map_err(|e| e.to_string())
    }

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    pub fn get() -> Option<String> {
        entry().ok()?.get_password().ok()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    pub fn set(token: &str) -> Result<(), String> {
        entry()?.set_password(token).map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    pub fn delete() -> Result<(), String> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub fn get() -> Option<String> {
        None
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub fn set(_token: &str) -> Result<(), String> {
        Ok(())
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub fn delete() -> Result<(), String> {
        Ok(())
    }
}

/// Forgets the sync server session and restarts on the local-only workspace (its store is untouched).
#[tauri::command]
fn sign_out(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = read_settings(&state.root);
    settings.relay_url = None;
    settings.relay_token = None;
    settings.workspace_id = None;
    settings.me = None;
    settings.account = None;
    write_settings(&state.root, &settings).map_err(|e| e.to_string())?;
    state.node.shutdown();
    app.restart()
}

/// `https://sync.example.com` → `wss://sync.example.com/v1/sync` (http → ws).
fn sync_url(server: &str) -> Result<String, String> {
    let rest = |scheme: &str| server.strip_prefix(scheme).filter(|r| !r.is_empty());
    match (rest("https://"), rest("http://")) {
        (Some(r), _) => Ok(format!("wss://{r}/v1/sync")),
        (None, Some(r)) => Ok(format!("ws://{r}/v1/sync")),
        _ => Err(format!("not a sync server address: {server}")),
    }
}

/// Where a signed-in workspace keeps its store: `workspaces/<id>`, apart from the local-only workspace, whose
/// unsynced ops must never be pushed to a team's server.
fn workspace_dir(root: &Path, workspace_id: &str) -> Result<PathBuf, String> {
    let ok = !workspace_id.is_empty()
        && workspace_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!("unexpected workspace id: {workspace_id}"));
    }
    Ok(root.join("workspaces").join(workspace_id))
}

#[tauri::command]
async fn snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    state.node.snapshot().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn apply(state: State<'_, AppState>, ops: Vec<Op>) -> Result<(), String> {
    state.node.apply(ops).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_presence(
    state: State<'_, AppState>,
    presence: Option<PresenceInput>,
) -> Result<(), String> {
    let p = presence.map(|p| Presence {
        person_id: state.info.me.clone(),
        mission_id: p.mission_id,
        activity: p.activity,
        at: now_rfc3339(),
    });
    state.node.set_presence(p).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn connectivity(state: State<'_, AppState>) -> serde_json::Value {
    serde_json::to_value(state.node.connectivity()).unwrap_or(serde_json::Value::Null)
}

fn now_rfc3339() -> String {
    // Seconds precision is enough for presence.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

/// Formats unix seconds as an RFC 3339 UTC timestamp (civil-from-days, no date crate needed).
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

fn read_settings(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_settings(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    let tmp = dir.join("settings.json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, dir.join("settings.json"))
}

/// A stable id for this install, created on first launch.
fn node_id(dir: &Path) -> std::io::Result<String> {
    let path = dir.join("node-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let id = format!("n-{}", astra_protocol::new_op_id().to_lowercase());
    std::fs::write(&path, &id)?;
    Ok(id)
}

/// Builds this install's starter workspace: the catalog's agents run on this node, and you are named
/// after your git (or OS) user.
fn starter_seed(workspace_id: &str, me: &str, node_id: &str) -> Result<String, String> {
    let mut seed: serde_json::Value = serde_json::from_str(STARTER).map_err(|e| e.to_string())?;
    seed["workspaceId"] = workspace_id.into();
    seed["me"] = me.into();
    let name = display_name();
    for entity in seed["entities"].as_array_mut().into_iter().flatten() {
        match entity["kind"].as_str() {
            Some("person") => {
                entity["id"] = me.into();
                entity["initials"] = initials(&name).into();
                entity["name"] = name.clone().into();
            }
            Some("agent") => entity["nodeId"] = node_id.into(),
            _ => {}
        }
    }
    serde_json::to_string(&seed).map_err(|e| e.to_string())
}

fn display_name() -> String {
    let git = std::process::Command::new("git")
        .args(["config", "--global", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    git.or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "You".into())
}

fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letters: String = if words.len() >= 2 {
        words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect()
    } else {
        name.chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect()
    };
    letters.to_uppercase()
}

/// Earlier builds filled a first launch with the design's demo workspace (and left `seed.json` next to
/// the store). Move that store aside once so the app starts clean; nothing is deleted.
fn retire_demo_store(dir: &Path) -> std::io::Result<()> {
    let marker = dir.join("seed.json");
    if !marker.exists() {
        return Ok(());
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = dir.join(format!("demo-backup-{secs}"));
    std::fs::create_dir_all(&backup)?;
    for name in ["astra.db", "astra.db-wal", "astra.db-shm", "seed.json"] {
        let from = dir.join(name);
        if from.exists() {
            std::fs::rename(&from, backup.join(name))?;
        }
    }
    Ok(())
}

async fn start_node(root: PathBuf) -> Result<(Node, NodeInfo), String> {
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    retire_demo_store(&root).map_err(|e| e.to_string())?;
    let settings = read_settings(&root);
    // One node id per install, whichever workspace it opens.
    let node_id = node_id(&root).map_err(|e| e.to_string())?;
    let workspace_id = settings
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws-local".into());
    let me = settings.me.clone().unwrap_or_else(|| "p-me".into());
    // Signed in from the app: that workspace has its own store. A relay set by hand in settings.json keeps
    // using the root store, as before.
    let data_dir = match &settings.account {
        Some(_) => workspace_dir(&root, &workspace_id)?,
        None => root.clone(),
    };
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    // Only read when the store is empty (a first launch).
    let starter_path = data_dir.join("starter.json");
    std::fs::write(&starter_path, starter_seed(&workspace_id, &me, &node_id)?)
        .map_err(|e| e.to_string())?;
    let relay = match (settings.relay_url, settings.relay_token) {
        (Some(url), Some(token)) => Some(RelayConfig { url, token }),
        _ => None,
    };
    let node = Node::open(NodeConfig {
        data_dir: data_dir.clone(),
        node_id: node_id.clone(),
        workspace_id: workspace_id.clone(),
        me: me.clone(),
        runtime: cfg!(not(any(target_os = "ios", target_os = "android"))),
        relay,
        seed_if_empty: Some(starter_path),
        name: None,
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok((
        node,
        NodeInfo {
            node_id,
            workspace_id,
            me,
            account: settings.account,
            github: settings.github,
        },
    ))
}

/// Forwards node events to the webview.
fn forward_events(app: AppHandle, node: Arc<Node>) {
    let mut rx = node.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(NodeEvent::Ops { ops, cursor }) => {
                    let _ = app.emit(
                        "astra://ops",
                        serde_json::json!({ "ops": ops, "cursor": cursor }),
                    );
                }
                Ok(NodeEvent::Presence(p)) => {
                    let _ = app.emit("astra://presence", p);
                }
                Ok(NodeEvent::Connectivity(c)) => {
                    let _ = app.emit("astra://connectivity", c);
                }
                // A slow webview missed events: it re-reads the snapshot on its next start; keep going.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// An app opened from Finder or the Dock gets launchd's minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), so
/// agent CLIs installed with Homebrew, npm or a Node version manager would not be found. Add the login
/// shell's PATH and the usual install folders; agents the runtime starts inherit it.
#[cfg(all(unix, not(any(target_os = "ios", target_os = "android"))))]
fn extend_path() {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut add = |d: PathBuf| {
        if !d.as_os_str().is_empty() && !dirs.contains(&d) {
            dirs.push(d);
        }
    };
    if let Some(path) = login_shell_path() {
        std::env::split_paths(&path).for_each(&mut add);
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".local/bin",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".cargo/bin",
        ] {
            add(home.join(rel));
        }
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
        add(PathBuf::from(d));
    }
    if let Ok(joined) = std::env::join_paths(dirs) {
        // SAFETY: called first thing in `run`, before Tauri or the async runtime start any threads.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

/// `$PATH` as the user's interactive login shell sets it, or `None` if that takes over 3 s.
#[cfg(all(unix, not(any(target_os = "ios", target_os = "android"))))]
fn login_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    const MARK: &str = "__ASTRA_PATH__";
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    // Profile scripts may print banners: keep only what sits between the markers.
    let path = out.split(MARK).nth(1)?.trim();
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(all(unix, not(any(target_os = "ios", target_os = "android"))))]
    extend_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(terminal::Terminals::default())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            let (node, info) = tauri::async_runtime::block_on(start_node(root.clone()))?;
            let node = Arc::new(node);
            forward_events(app.handle().clone(), node.clone());
            app.manage(AppState { node, info, root });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            node_info,
            github_device_start,
            github_device_poll,
            github_repos,
            github_clone,
            github_sign_out,
            sign_in,
            sign_out,
            snapshot,
            apply,
            set_presence,
            connectivity,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_close,
            files::files_list,
            files::files_read,
            files::files_write,
            files::files_base
        ])
        .run(tauri::generate_context!())
        .expect("error while running Astra");
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_rfc3339() {
        assert_eq!(super::rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::rfc3339(1_763_716_320), "2025-11-21T09:12:00Z");
    }

    #[test]
    fn sync_urls() {
        assert_eq!(
            super::sync_url("https://sync.example.com").as_deref(),
            Ok("wss://sync.example.com/v1/sync")
        );
        assert_eq!(
            super::sync_url("http://127.0.0.1:8787").as_deref(),
            Ok("ws://127.0.0.1:8787/v1/sync")
        );
        assert!(super::sync_url("sync.example.com").is_err());
        assert!(super::sync_url("https://").is_err());
    }

    #[test]
    fn signed_in_workspaces_get_their_own_store() {
        let root = std::path::Path::new("/data");
        assert_eq!(
            super::workspace_dir(root, "ws-mindify").expect("dir"),
            root.join("workspaces").join("ws-mindify")
        );
        assert!(super::workspace_dir(root, "../etc").is_err());
        assert!(super::workspace_dir(root, "").is_err());
    }

    #[test]
    fn initials_from_names() {
        assert_eq!(super::initials("Mark Chen"), "MC");
        assert_eq!(super::initials("next"), "NE");
        assert_eq!(super::initials("You"), "YO");
    }

    #[test]
    fn starter_has_no_demo_data() {
        let raw = super::starter_seed("ws-x", "p-x", "n-x").expect("starter");
        let seed: astra_protocol::Seed = serde_json::from_str(&raw).expect("parses as a seed");
        assert_eq!(seed.workspace_id, "ws-x");
        for e in &seed.entities {
            let v = serde_json::to_value(e).expect("json");
            match v["kind"].as_str() {
                Some("person") => assert_eq!(v["id"], "p-x"),
                Some("agent") => {
                    assert_eq!(v["nodeId"], "n-x");
                    assert_eq!(v["status"], "notConnected");
                }
                other => panic!("unexpected entity in starter: {other:?}"),
            }
        }
    }
}
