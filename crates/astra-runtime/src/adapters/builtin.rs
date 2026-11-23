//! The built-in Astra agent: astra-llm's tool loop with provider routing.

use std::path::Path;
use std::sync::Arc;

use astra_llm::{
    AgentLoop, AnthropicConfig, LoopEvent, LoopOutcome, Router, RouterConfig, RoutingPolicy,
    Toolset, Workspace,
};
use astra_protocol::{LogLine, ProviderPolicy};
use tokio::sync::mpsc;

use super::*;
use crate::policy::command_needs_approval;
use crate::secrets::names;

/// Runs the built-in agent.
pub struct BuiltinAdapter {
    pub policy: ProviderPolicy,
    pub model: Option<String>,
    /// Use this router configuration instead of one derived from secrets (tests).
    pub router_override: Option<RouterConfig>,
}

impl BuiltinAdapter {
    /// The router configuration for this agent.
    pub fn router_config(&self, secrets: &dyn SecretStore) -> RouterConfig {
        if let Some(rc) = &self.router_override {
            return rc.clone();
        }
        let policy = match self.policy {
            ProviderPolicy::Auto => RoutingPolicy::Auto,
            ProviderPolicy::Cloud => RoutingPolicy::Cloud,
            ProviderPolicy::Local => RoutingPolicy::Local,
        };
        let anthropic = secrets.get(names::ANTHROPIC_API_KEY).map(|key| {
            let mut c = AnthropicConfig::new(key);
            if let Some(m) = self.model.as_ref().filter(|m| m.starts_with("claude")) {
                c.model = m.clone();
            }
            c
        });
        let mut rc = RouterConfig::new(policy, anthropic);
        rc.local_model = self.model.clone().filter(|m| !m.starts_with("claude"));
        rc
    }
}

struct PolicyApprover {
    inner: Arc<dyn Approver>,
    policy: astra_protocol::CommandPolicy,
    worktree: std::path::PathBuf,
}

#[async_trait::async_trait]
impl astra_llm::CommandApprover for PolicyApprover {
    async fn approve(&self, command: &str, cwd: &Path) -> bool {
        if !command_needs_approval(self.policy, Some(cwd), &self.worktree) {
            return true;
        }
        self.inner
            .request(PermissionRequest {
                prompt: format!("Run `{command}`"),
                command: Some(command.to_string()),
                cwd: Some(cwd.to_path_buf()),
            })
            .await
    }
}

impl AgentAdapter for BuiltinAdapter {
    fn start(&self, ctx: AgentContext) -> Result<AgentSession, AdapterError> {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let workspace = Workspace::new(&ctx.worktree)
            .map_err(|e| AdapterError::Other(format!("worktree: {e}")))?;
        let router = Router::new(self.router_config(ctx.secrets.as_ref()));
        tokio::spawn(async move {
            let result: Result<(), String> = async {
                let provider = router.select().await.map_err(|e| e.to_string())?;
                ctx.logs
                    .push(format!("built-in agent using {}", provider.name()));
                let approver = Arc::new(PolicyApprover {
                    inner: ctx.approver.clone(),
                    policy: ctx.command_policy,
                    worktree: workspace.root().to_path_buf(),
                });
                let tools = Toolset::new(workspace, approver);
                let mut agent = AgentLoop::new(provider, tools, astra_llm::agent::DEFAULT_SYSTEM)
                    .map_err(|e| e.to_string())?;
                let mut prompt = Some(ctx.prompt.clone());
                loop {
                    let Some(p) = prompt.take() else {
                        match ctl_rx.recv().await {
                            Some(AgentControl::Message(m)) => prompt = Some(m),
                            Some(AgentControl::Cancel) | None => return Ok(()),
                            _ => {}
                        }
                        continue;
                    };
                    let (ltx, mut lrx) = mpsc::unbounded_channel::<LoopEvent>();
                    let fwd = ev_tx.clone();
                    let pump = tokio::spawn(async move {
                        while let Some(e) = lrx.recv().await {
                            match e {
                                LoopEvent::TextDelta(t) => {
                                    let _ = fwd.send(AgentEvent::Text(t));
                                }
                                LoopEvent::ToolFinished {
                                    log: Some((verb, target)),
                                    ..
                                } => {
                                    let _ = fwd.send(AgentEvent::Log(LogLine { verb, target }));
                                }
                                _ => {}
                            }
                        }
                    });
                    let mut queued = Vec::new();
                    let outcome = {
                        let run = agent.run(p, &ltx);
                        tokio::pin!(run);
                        loop {
                            tokio::select! {
                                r = &mut run => break Some(r),
                                c = ctl_rx.recv() => match c {
                                    Some(AgentControl::Message(m)) => queued.push(m),
                                    Some(AgentControl::Cancel) | None => break None,
                                    _ => {}
                                }
                            }
                        }
                    };
                    drop(ltx);
                    let _ = pump.await;
                    match outcome {
                        None => return Ok(()),
                        Some(Ok(LoopOutcome::Done { .. })) => {
                            let _ = ev_tx.send(AgentEvent::TurnDone);
                        }
                        Some(Ok(LoopOutcome::Refused { explanation })) => {
                            let _ = ev_tx.send(AgentEvent::Error(format!(
                                "the model declined: {}",
                                explanation.unwrap_or_else(|| "no details".into())
                            )));
                        }
                        Some(Ok(LoopOutcome::TurnLimit)) => {
                            let _ = ev_tx.send(AgentEvent::Error("turn limit reached".into()));
                        }
                        Some(Err(e)) => return Err(e.to_string()),
                    }
                    if !queued.is_empty() {
                        prompt = Some(queued.join("\n\n"));
                    }
                }
            }
            .await;
            if let Err(e) = result {
                let _ = ev_tx.send(AgentEvent::Error(e));
            }
            let _ = ev_tx.send(AgentEvent::Exited);
        });
        Ok(AgentSession {
            control: ctl_tx,
            events: ev_rx,
        })
    }
}
