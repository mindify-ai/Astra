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

/// The demo workspace shipped with the app so a first launch is not empty.
const SEED: &str = include_str!("../../../../fixtures/seed.json");

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

async fn start_node(data_dir: PathBuf) -> Result<(Node, NodeInfo), String> {
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let settings = read_settings(&data_dir);
    let node_id = node_id(&data_dir).map_err(|e| e.to_string())?;
    let seed_path = data_dir.join("seed.json");
    if !seed_path.exists() {
        std::fs::write(&seed_path, SEED).map_err(|e| e.to_string())?;
    }
    let workspace_id = settings
        .workspace_id
        .clone()
        .unwrap_or_else(|| "ws-mindify".into());
    let me = settings.me.clone().unwrap_or_else(|| "p-mark".into());
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
        seed_if_empty: Some(seed_path),
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
}
