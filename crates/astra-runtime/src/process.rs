//! Child process supervision: process groups, kill on drop, pause/resume, log ring buffers.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

/// A bounded buffer of recent log lines.
#[derive(Clone, Debug)]
pub struct LogRing {
    inner: Arc<Mutex<VecDeque<String>>>,
    cap: usize,
}

impl LogRing {
    /// A ring holding at most `cap` lines.
    pub fn new(cap: usize) -> Self {
        LogRing {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(cap.min(1024)))),
            cap,
        }
    }

    /// Append a line.
    pub fn push(&self, line: impl Into<String>) {
        if let Ok(mut q) = self.inner.lock() {
            if q.len() == self.cap {
                q.pop_front();
            }
            q.push_back(line.into());
        }
    }

    /// Current lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Copy lines from a reader into a ring (with a prefix).
pub fn pump_lines<R: AsyncRead + Unpin + Send + 'static>(
    reader: R,
    ring: LogRing,
    prefix: &'static str,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            ring.push(format!("{prefix}{line}"));
        }
    });
}

/// Agent Client Protocol bridges Astra knows by command name, and the npm package that provides each.
/// `claude-code-acp` is the bridge's old name, kept so agents set up with it still resolve.
pub const KNOWN_BRIDGES: &[(&str, &str)] = &[
    ("claude-agent-acp", "@agentclientprotocol/claude-agent-acp"),
    ("claude-code-acp", "@agentclientprotocol/claude-agent-acp"),
    ("codex-acp", "@agentclientprotocol/codex-acp"),
];

/// Finds the program to run for an agent's `command args`. A known ACP bridge that isn't installed runs
/// through `npx -y <package>` instead, so connecting Claude Code or Codex needs only Node.js.
pub fn resolve_program(command: &str, args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    resolve_with(command, args, |c| which::which(c).ok())
}

fn resolve_with(
    command: &str,
    args: &[String],
    find: impl Fn(&str) -> Option<PathBuf>,
) -> Result<(PathBuf, Vec<String>), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("no command configured".into());
    }
    if let Some(path) = find(command) {
        return Ok((path, args.to_vec()));
    }
    let Some((_, package)) = KNOWN_BRIDGES.iter().find(|(name, _)| *name == command) else {
        return Err(format!("`{command}` was not found on PATH"));
    };
    let Some(npx) = find("npx") else {
        return Err(format!(
            "`{command}` was not found on PATH. Install Node.js, or run `npm install -g {package}`"
        ));
    };
    let mut argv = vec!["-y".to_string(), (*package).to_string()];
    argv.extend(args.iter().cloned());
    Ok((npx, argv))
}

/// A child in its own process group; the whole group is killed when this is dropped.
#[derive(Debug)]
pub struct Supervised {
    pub child: Child,
    pgid: Option<u32>,
}

impl Supervised {
    /// Spawn `program args` in `cwd` with piped stdio, in a new process group.
    pub fn spawn(program: &str, args: &[String], cwd: &Path) -> std::io::Result<Supervised> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn()?;
        let pgid = child.id();
        Ok(Supervised { child, pgid })
    }

    /// Stop (SIGSTOP) the process group.
    pub fn pause(&self) {
        self.signal(Sig::Stop);
    }

    /// Continue (SIGCONT) the process group.
    pub fn resume(&self) {
        self.signal(Sig::Cont);
    }

    /// Kill the process group.
    pub fn kill(&mut self) {
        self.signal(Sig::Cont);
        self.signal(Sig::Kill);
        let _ = self.child.start_kill();
    }

    #[cfg(unix)]
    fn signal(&self, sig: Sig) {
        use nix::sys::signal::{Signal, killpg};
        use nix::unistd::Pid;
        let Some(pgid) = self.pgid.and_then(|p| i32::try_from(p).ok()) else {
            return;
        };
        let s = match sig {
            Sig::Stop => Signal::SIGSTOP,
            Sig::Cont => Signal::SIGCONT,
            Sig::Kill => Signal::SIGKILL,
        };
        let _ = killpg(Pid::from_raw(pgid), s);
    }

    #[cfg(not(unix))]
    fn signal(&self, _sig: Sig) {}
}

#[derive(Clone, Copy)]
enum Sig {
    Stop,
    Cont,
    Kill,
}

impl Drop for Supervised {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    fn found(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |c| {
            names
                .contains(&c)
                .then(|| PathBuf::from(format!("/bin/{c}")))
        }
    }

    #[test]
    fn installed_command_runs_directly() {
        let (p, a) =
            resolve_with("codex-acp", &["--x".into()], found(&["codex-acp", "npx"])).unwrap();
        assert_eq!(p, PathBuf::from("/bin/codex-acp"));
        assert_eq!(a, vec!["--x".to_string()]);
    }

    #[test]
    fn missing_bridge_runs_through_npx() {
        for (cmd, pkg) in [
            ("claude-code-acp", "@agentclientprotocol/claude-agent-acp"),
            ("claude-agent-acp", "@agentclientprotocol/claude-agent-acp"),
            ("codex-acp", "@agentclientprotocol/codex-acp"),
        ] {
            let (p, a) = resolve_with(cmd, &[], found(&["npx"])).unwrap();
            assert_eq!(p, PathBuf::from("/bin/npx"));
            assert_eq!(a, vec!["-y".to_string(), pkg.to_string()]);
        }
    }

    #[test]
    fn missing_commands_explain_what_to_install() {
        let e = resolve_with("codex-acp", &[], found(&[])).unwrap_err();
        assert!(
            e.contains("npm install -g @agentclientprotocol/codex-acp"),
            "{e}"
        );
        let e = resolve_with("my-agent", &[], found(&["npx"])).unwrap_err();
        assert_eq!(e, "`my-agent` was not found on PATH");
        assert!(resolve_with(" ", &[], found(&["npx"])).is_err());
    }
}
