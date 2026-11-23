//! Provider routing: `auto` (cloud when keyed and reachable, else first local server), `cloud`, `local`.

use std::sync::Arc;
use std::time::Duration;

use crate::anthropic::{AnthropicConfig, AnthropicProvider};
use crate::openai::{LOCAL_CANDIDATES, OpenAiCompatProvider, detect_servers};
use crate::types::Provider;
use crate::{LlmError, Result};

/// Routing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutingPolicy {
    #[default]
    Auto,
    Cloud,
    Local,
}

/// Router configuration.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub policy: RoutingPolicy,
    /// Cloud provider config; `None` when no API key is available.
    pub anthropic: Option<AnthropicConfig>,
    /// Local candidates `(kind, base_url with /v1)`.
    pub local_candidates: Vec<(String, String)>,
    /// Preferred local model (else the server's first model).
    pub local_model: Option<String>,
    pub probe_timeout: Duration,
}

impl RouterConfig {
    /// Defaults with the well-known local ports.
    pub fn new(policy: RoutingPolicy, anthropic: Option<AnthropicConfig>) -> Self {
        RouterConfig {
            policy,
            anthropic,
            local_candidates: LOCAL_CANDIDATES
                .iter()
                .map(|(k, b)| (k.to_string(), b.to_string()))
                .collect(),
            local_model: None,
            probe_timeout: Duration::from_millis(400),
        }
    }
}

/// Picks a provider according to the policy.
#[derive(Debug, Clone)]
pub struct Router {
    config: RouterConfig,
}

impl Router {
    /// A router.
    pub fn new(config: RouterConfig) -> Self {
        Router { config }
    }

    /// Whether the cloud API answers at all (any HTTP status counts as reachable).
    pub async fn cloud_reachable(&self) -> bool {
        let Some(cfg) = &self.config.anthropic else {
            return false;
        };
        let Ok(http) = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
        else {
            return false;
        };
        http.get(format!("{}/v1/models", cfg.base_url.trim_end_matches('/')))
            .send()
            .await
            .is_ok()
    }

    async fn local(&self) -> Result<Arc<dyn Provider>> {
        let servers =
            detect_servers(&self.config.local_candidates, self.config.probe_timeout).await;
        for s in servers {
            let model = match &self.config.local_model {
                Some(m) => m.clone(),
                None => match s.models.first() {
                    Some(m) => m.clone(),
                    None => continue,
                },
            };
            return Ok(Arc::new(OpenAiCompatProvider::new(
                s.kind, s.base_url, model,
            )?));
        }
        Err(LlmError::NoProvider(
            "no local model server detected (Ollama, LM Studio, llama.cpp)".into(),
        ))
    }

    fn cloud(&self) -> Result<Arc<dyn Provider>> {
        match &self.config.anthropic {
            Some(cfg) => Ok(Arc::new(AnthropicProvider::new(cfg.clone())?)),
            None => Err(LlmError::NoProvider(
                "no Anthropic API key configured".into(),
            )),
        }
    }

    /// Select a provider.
    pub async fn select(&self) -> Result<Arc<dyn Provider>> {
        match self.config.policy {
            RoutingPolicy::Cloud => self.cloud(),
            RoutingPolicy::Local => self.local().await,
            RoutingPolicy::Auto => {
                if self.config.anthropic.is_some() && self.cloud_reachable().await {
                    self.cloud()
                } else {
                    self.local().await
                }
            }
        }
    }
}
