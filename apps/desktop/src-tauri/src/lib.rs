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

/// A first launch starts with only you and the catalog of agents you can connect: no demo data.
const STARTER: &str = include_str!("../../../../fixtures/starter.json");

struct AppState {
    node: Arc<Node>,
    info: NodeInfo,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeInfo {
    node_id: String,
    workspace_id: String,
    me: String,
}

/// Local settings (not synced). Keys never live here: they stay in the OS keychain.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    relay_url: Option<String>,
    relay_token: Option<String>,
    workspace_id: Option<String>,
    me: Option<String>,
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

async fn start_node(data_dir: PathBuf) -> Result<(Node, NodeInfo), String> {
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    retire_demo_store(&data_dir).map_err(|e| e.to_string())?;
    let settings = read_settings(&data_dir);
    let node_id = node_id(&data_dir).map_err(|e| e.to_string())?;
    let workspace_id = settings
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws-local".into());
    let me = settings.me.clone().unwrap_or_else(|| "p-me".into());
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
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok((
        node,
        NodeInfo {
            node_id,
            workspace_id,
            me,
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
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let (node, info) = tauri::async_runtime::block_on(start_node(data_dir))?;
            let node = Arc::new(node);
            forward_events(app.handle().clone(), node.clone());
            app.manage(AppState { node, info });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            node_info,
            snapshot,
            apply,
            set_presence,
            connectivity
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
