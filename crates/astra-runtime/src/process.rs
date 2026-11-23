//! Child process supervision: process groups, kill on drop, pause/resume, log ring buffers.

use std::collections::VecDeque;
use std::path::Path;
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
        LogRing { inner: Arc::new(Mutex::new(VecDeque::with_capacity(cap.min(1024)))), cap }
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
        self.inner.lock().map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }
}

/// Copy lines from a reader into a ring (with a prefix).
pub fn pump_lines<R: AsyncRead + Unpin + Send + 'static>(reader: R, ring: LogRing, prefix: &'static str) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            ring.push(format!("{prefix}{line}"));
        }
    });
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
        let Some(pgid) = self.pgid.and_then(|p| i32::try_from(p).ok()) else { return };
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
