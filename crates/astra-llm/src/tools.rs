//! Workspace tools of the built-in agent, confined to one worktree root.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::types::ToolSpec;

/// Maximum bytes returned by `read_file`.
pub const MAX_READ: usize = 256 * 1024;
/// Maximum bytes of command output returned.
pub const MAX_OUTPUT: usize = 64 * 1024;

/// A directory the agent may touch. Every path is resolved inside it.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// A workspace rooted at `root` (must exist).
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Workspace {
            root: root.as_ref().canonicalize()?,
        })
    }

    /// The canonical root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `path` (relative to the root, or absolute inside it). Rejects anything escaping the root,
    /// including through symlinks.
    pub fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let p = Path::new(path);
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        let mut normal = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => {
                    if !normal.pop() {
                        return Err(format!("path escapes the workspace: {path}"));
                    }
                }
                Component::CurDir => {}
                other => normal.push(other),
            }
        }
        if !normal.starts_with(&self.root) {
            return Err(format!("path escapes the workspace: {path}"));
        }
        // Follow symlinks of the deepest existing ancestor.
        let mut existing = normal.as_path();
        while !existing.exists() {
            existing = match existing.parent() {
                Some(p) => p,
                None => break,
            };
        }
        let real = existing
            .canonicalize()
            .map_err(|e| format!("{path}: {e}"))?;
        if !real.starts_with(&self.root) {
            return Err(format!("path escapes the workspace: {path}"));
        }
        Ok(normal)
    }

    /// Path relative to the root, for display.
    pub fn display(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    }
}

/// Asked before `run_command` executes.
#[async_trait::async_trait]
pub trait CommandApprover: Send + Sync {
    /// Return true to run `command` in `cwd`.
    async fn approve(&self, command: &str, cwd: &Path) -> bool;
}

/// Approves everything (tests).
pub struct AllowAllCommands;

#[async_trait::async_trait]
impl CommandApprover for AllowAllCommands {
    async fn approve(&self, _command: &str, _cwd: &Path) -> bool {
        true
    }
}

/// Denies everything.
pub struct DenyAllCommands;

#[async_trait::async_trait]
impl CommandApprover for DenyAllCommands {
    async fn approve(&self, _command: &str, _cwd: &Path) -> bool {
        false
    }
}

/// Result of a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    /// Activity line `(verb, target)` for the thread.
    pub log: Option<(String, String)>,
}

impl ToolOutcome {
    fn ok(content: impl Into<String>, verb: &str, target: impl Into<String>) -> Self {
        ToolOutcome {
            content: content.into(),
            is_error: false,
            log: Some((verb.into(), target.into())),
        }
    }
    fn err(content: impl Into<String>) -> Self {
        ToolOutcome {
            content: content.into(),
            is_error: true,
            log: None,
        }
    }
}

/// The tool set.
#[derive(Clone)]
pub struct Toolset {
    pub workspace: Workspace,
    pub approver: Arc<dyn CommandApprover>,
    pub command_timeout: Duration,
}

fn schema(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

impl Toolset {
    /// Tools over `workspace`, asking `approver` before commands.
    pub fn new(workspace: Workspace, approver: Arc<dyn CommandApprover>) -> Self {
        Toolset {
            workspace,
            approver,
            command_timeout: Duration::from_secs(300),
        }
    }

    /// Tool definitions, in a fixed order (stable for prompt caching).
    pub fn specs() -> Vec<ToolSpec> {
        let s = |t: &str| json!({"type": "string", "description": t});
        vec![
            ToolSpec {
                name: "read_file".into(),
                description: "Read a UTF-8 text file in the workspace.".into(),
                input_schema: schema(json!({"path": s("Path relative to the workspace root.")}), &["path"]),
            },
            ToolSpec {
                name: "write_file".into(),
                description: "Create or overwrite a file in the workspace with the given content.".into(),
                input_schema: schema(
                    json!({"path": s("Path relative to the workspace root."), "content": s("Full file content.")}),
                    &["path", "content"],
                ),
            },
            ToolSpec {
                name: "edit_file".into(),
                description: "Replace one exact occurrence of old_string with new_string in a file.".into(),
                input_schema: schema(
                    json!({
                        "path": s("Path relative to the workspace root."),
                        "old_string": s("Exact text to replace; must occur exactly once."),
                        "new_string": s("Replacement text.")
                    }),
                    &["path", "old_string", "new_string"],
                ),
            },
            ToolSpec {
                name: "list_dir".into(),
                description: "List the entries of a directory in the workspace.".into(),
                input_schema: schema(json!({"path": s("Directory relative to the workspace root; default '.'.")}), &[]),
            },
            ToolSpec {
                name: "search".into(),
                description: "Search files under a directory for a regular expression; returns path:line: text."
                    .into(),
                input_schema: schema(
                    json!({"pattern": s("Rust regex syntax."), "path": s("Directory to search; default '.'.")}),
                    &["pattern"],
                ),
            },
            ToolSpec {
                name: "run_command".into(),
                description: "Run a shell command in the workspace (may require the user's approval).".into(),
                input_schema: schema(json!({"command": s("Shell command line.")}), &["command"]),
            },
        ]
    }

    /// Execute a validated tool call.
    pub async fn execute(&self, name: &str, input: &Value) -> ToolOutcome {
        let str_arg = |k: &str| input.get(k).and_then(Value::as_str);
        match name {
            "read_file" => {
                let Some(path) = str_arg("path") else {
                    return ToolOutcome::err("missing path");
                };
                match self.workspace.resolve(path) {
                    Err(e) => ToolOutcome::err(e),
                    Ok(p) => match tokio::fs::read(&p).await {
                        Ok(bytes) => {
                            let mut text = String::from_utf8_lossy(&bytes).into_owned();
                            if text.len() > MAX_READ {
                                text.truncate(floor_char(&text, MAX_READ));
                                text.push_str("\n[truncated]");
                            }
                            ToolOutcome::ok(text, "read", self.workspace.display(&p))
                        }
                        Err(e) => ToolOutcome::err(format!("{path}: {e}")),
                    },
                }
            }
            "write_file" => {
                let (Some(path), Some(content)) = (str_arg("path"), str_arg("content")) else {
                    return ToolOutcome::err("missing path or content");
                };
                match self.workspace.resolve(path) {
                    Err(e) => ToolOutcome::err(e),
                    Ok(p) => {
                        if let Some(dir) = p.parent()
                            && let Err(e) = tokio::fs::create_dir_all(dir).await
                        {
                            return ToolOutcome::err(format!("{path}: {e}"));
                        }
                        match tokio::fs::write(&p, content).await {
                            Ok(()) => ToolOutcome::ok(
                                format!("wrote {} bytes", content.len()),
                                "edit",
                                self.workspace.display(&p),
                            ),
                            Err(e) => ToolOutcome::err(format!("{path}: {e}")),
                        }
                    }
                }
            }
            "edit_file" => {
                let (Some(path), Some(old), Some(new)) = (
                    str_arg("path"),
                    str_arg("old_string"),
                    str_arg("new_string"),
                ) else {
                    return ToolOutcome::err("missing path, old_string or new_string");
                };
                let p = match self.workspace.resolve(path) {
                    Ok(p) => p,
                    Err(e) => return ToolOutcome::err(e),
                };
                let text = match tokio::fs::read_to_string(&p).await {
                    Ok(t) => t,
                    Err(e) => return ToolOutcome::err(format!("{path}: {e}")),
                };
                match text.matches(old).count() {
                    0 => ToolOutcome::err(format!("old_string not found in {path}")),
                    1 => match tokio::fs::write(&p, text.replacen(old, new, 1)).await {
                        Ok(()) => ToolOutcome::ok("edited", "edit", self.workspace.display(&p)),
                        Err(e) => ToolOutcome::err(format!("{path}: {e}")),
                    },
                    n => ToolOutcome::err(format!(
                        "old_string occurs {n} times in {path}; make it unique"
                    )),
                }
            }
            "list_dir" => {
                let path = str_arg("path").unwrap_or(".");
                let p = match self.workspace.resolve(path) {
                    Ok(p) => p,
                    Err(e) => return ToolOutcome::err(e),
                };
                let mut rd = match tokio::fs::read_dir(&p).await {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::err(format!("{path}: {e}")),
                };
                let mut names = Vec::new();
                while let Ok(Some(e)) = rd.next_entry().await {
                    let mut n = e.file_name().to_string_lossy().into_owned();
                    if e.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                        n.push('/');
                    }
                    names.push(n);
                }
                names.sort();
                ToolOutcome::ok(names.join("\n"), "read", self.workspace.display(&p))
            }
            "search" => {
                let Some(pattern) = str_arg("pattern") else {
                    return ToolOutcome::err("missing pattern");
                };
                let re = match regex::Regex::new(pattern) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::err(format!("bad regex: {e}")),
                };
                let path = str_arg("path").unwrap_or(".");
                let p = match self.workspace.resolve(path) {
                    Ok(p) => p,
                    Err(e) => return ToolOutcome::err(e),
                };
                let ws = self.workspace.clone();
                let hits = tokio::task::spawn_blocking(move || search(&ws, &p, &re))
                    .await
                    .unwrap_or_default();
                ToolOutcome::ok(
                    if hits.is_empty() {
                        "no matches".into()
                    } else {
                        hits.join("\n")
                    },
                    "search",
                    pattern,
                )
            }
            "run_command" => {
                let Some(command) = str_arg("command") else {
                    return ToolOutcome::err("missing command");
                };
                let cwd = self.workspace.root().to_path_buf();
                if !self.approver.approve(command, &cwd).await {
                    return ToolOutcome {
                        content: "The user denied running this command.".into(),
                        is_error: true,
                        log: None,
                    };
                }
                let mut cmd = tokio::process::Command::new("sh");
                cmd.arg("-c")
                    .arg(command)
                    .current_dir(&cwd)
                    .kill_on_drop(true)
                    .stdin(std::process::Stdio::null());
                match tokio::time::timeout(self.command_timeout, cmd.output()).await {
                    Err(_) => ToolOutcome::err(format!(
                        "command timed out after {:?}",
                        self.command_timeout
                    )),
                    Ok(Err(e)) => ToolOutcome::err(format!("failed to start: {e}")),
                    Ok(Ok(out)) => {
                        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                        text.push_str(&String::from_utf8_lossy(&out.stderr));
                        if text.len() > MAX_OUTPUT {
                            text.truncate(floor_char(&text, MAX_OUTPUT));
                            text.push_str("\n[truncated]");
                        }
                        let code = out.status.code().unwrap_or(-1);
                        ToolOutcome {
                            content: format!("exit code {code}\n{text}"),
                            is_error: !out.status.success(),
                            log: Some(("run".into(), command.into())),
                        }
                    }
                }
            }
            other => ToolOutcome::err(format!("unknown tool {other}")),
        }
    }
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn search(ws: &Workspace, dir: &Path, re: &regex::Regex) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let name = e.file_name();
            if name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                let Ok(text) = std::fs::read_to_string(&p) else {
                    continue;
                };
                for (i, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        out.push(format!("{}:{}: {}", ws.display(&p), i + 1, line.trim_end()));
                        if out.len() >= 200 {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn confines_paths_and_edits() {
        let dir = tempfile::tempdir().expect("tmp");
        let ws = Workspace::new(dir.path()).expect("ws");
        assert!(ws.resolve("../x").is_err());
        assert!(ws.resolve("/etc/passwd").is_err());
        assert!(ws.resolve("a/../../x").is_err());
        assert!(ws.resolve("a/b/../c.txt").is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.path().join("link")).expect("symlink");
            assert!(ws.resolve("link/passwd").is_err());
        }
        let tools = Toolset::new(ws, Arc::new(DenyAllCommands));
        let w = tools
            .execute(
                "write_file",
                &json!({"path": "src/a.txt", "content": "hello world"}),
            )
            .await;
        assert!(!w.is_error, "{w:?}");
        let e = tools
            .execute(
                "edit_file",
                &json!({"path": "src/a.txt", "old_string": "world", "new_string": "astra"}),
            )
            .await;
        assert!(!e.is_error);
        let r = tools
            .execute("read_file", &json!({"path": "src/a.txt"}))
            .await;
        assert_eq!(r.content, "hello astra");
        let s = tools.execute("search", &json!({"pattern": "ast.a"})).await;
        assert!(s.content.contains("src/a.txt:1: hello astra"), "{s:?}");
        let l = tools.execute("list_dir", &json!({})).await;
        assert!(l.content.contains("src/"));
        let c = tools
            .execute("run_command", &json!({"command": "touch x"}))
            .await;
        assert!(c.is_error && !dir.path().join("x").exists());
        let bad = tools
            .execute("write_file", &json!({"path": "../escape", "content": ""}))
            .await;
        assert!(bad.is_error);
    }

    #[tokio::test]
    async fn runs_approved_commands() {
        let dir = tempfile::tempdir().expect("tmp");
        let tools = Toolset::new(
            Workspace::new(dir.path()).expect("ws"),
            Arc::new(AllowAllCommands),
        );
        let c = tools
            .execute("run_command", &json!({"command": "echo hi > f && cat f"}))
            .await;
        assert!(!c.is_error, "{c:?}");
        assert!(c.content.contains("hi"));
        assert_eq!(c.log, Some(("run".into(), "echo hi > f && cat f".into())));
    }
}
