//! Command adapter: spawn a command in the worktree, write the prompt to stdin, stream stdout lines as text.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::*;
use crate::process::{Supervised, pump_lines};

/// Runs `command args` once per prompt.
pub struct CommandAdapter {
    pub command: String,
    pub args: Vec<String>,
}

impl AgentAdapter for CommandAdapter {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError> {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (program, args) = crate::process::resolve_program(&self.command, &self.args)
            .map_err(|e| AdapterError::Spawn(self.command.clone(), e))?;
        let command = program.to_string_lossy().into_owned();
        // Spawn eagerly so a missing binary is reported to the caller.
        let first = Supervised::spawn(&command, &args, &ctx.worktree)
            .map_err(|e| AdapterError::Spawn(command.clone(), e.to_string()))?;
        tokio::spawn(async move {
            let mut next = Some(first);
            let mut prompt = ctx.prompt.clone();
            let mut queued: Option<String> = None;
            loop {
                let mut proc = match next.take() {
                    Some(p) => p,
                    None => match Supervised::spawn(&command, &args, &ctx.worktree) {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = ev_tx
                                .send(AgentEvent::Error(format!("failed to start {command}: {e}")));
                            break;
                        }
                    },
                };
                if let Some(mut stdin) = proc.child.stdin.take() {
                    let _ = stdin.write_all(prompt.as_bytes()).await;
                    let _ = stdin.shutdown().await;
                }
                if let Some(err) = proc.child.stderr.take() {
                    pump_lines(err, ctx.logs.clone(), "stderr: ");
                }
                let Some(out) = proc.child.stdout.take() else {
                    break;
                };
                let mut lines = BufReader::new(out).lines();
                let mut cancelled = false;
                loop {
                    tokio::select! {
                        line = lines.next_line() => match line {
                            Ok(Some(l)) => { ctx.logs.push(l.clone()); let _ = ev_tx.send(AgentEvent::Text(format!("{l}\n"))); }
                            _ => break,
                        },
                        ctl = ctl_rx.recv() => match ctl {
                            Some(AgentControl::Pause) => proc.pause(),
                            Some(AgentControl::Resume) => proc.resume(),
                            Some(AgentControl::Message(m)) => { ctx.logs.push(format!("queued message: {m}")); queued = Some(m); }
                            Some(AgentControl::Cancel) | None => { proc.kill(); cancelled = true; break; }
                        }
                    }
                }
                if cancelled {
                    break;
                }
                match proc.child.wait().await {
                    Ok(s) if s.success() => {
                        let _ = ev_tx.send(AgentEvent::TurnDone);
                    }
                    Ok(s) => {
                        let _ = ev_tx.send(AgentEvent::Error(format!("{command} exited with {s}")));
                    }
                    Err(e) => {
                        let _ = ev_tx.send(AgentEvent::Error(e.to_string()));
                    }
                }
                // Run again for a queued or new message, or end.
                if let Some(m) = queued.take() {
                    prompt = m;
                    continue;
                }
                match ctl_rx.recv().await {
                    Some(AgentControl::Message(m)) => prompt = m,
                    _ => break,
                }
            }
            let _ = ev_tx.send(AgentEvent::Exited);
        });
        Ok(AgentSession {
            control: ctl_tx,
            events: ev_rx,
        })
    }
}
