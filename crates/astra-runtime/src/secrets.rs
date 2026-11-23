//! Secrets (agent API keys) stay on the runner node: OS keychain or memory. Never synced, never logged.

use std::collections::HashMap;
use std::sync::Mutex;

/// Well-known secret names.
pub mod names {
    /// Anthropic API key used by the built-in agent.
    pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";
}

/// A secret store.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// In-memory secrets (tests, mobile).
#[derive(Default)]
pub struct MemorySecrets {
    map: Mutex<HashMap<String, String>>,
}

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Option<String> {
        self.map.lock().ok()?.get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.map.lock().map_err(|_| "poisoned".to_string())?.insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.map.lock().map_err(|_| "poisoned".to_string())?.remove(key);
        Ok(())
    }
}

/// Falls back from a primary store to environment variables (`ASTRA_SECRET_<KEY>` or, for the Anthropic
/// key, `ANTHROPIC_API_KEY`).
pub struct WithEnvFallback<S>(pub S);

impl<S: SecretStore> SecretStore for WithEnvFallback<S> {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).or_else(|| {
            let env = format!("ASTRA_SECRET_{}", key.to_ascii_uppercase());
            std::env::var(env)
                .ok()
                .or_else(|| (key == names::ANTHROPIC_API_KEY).then(|| std::env::var("ANTHROPIC_API_KEY").ok()).flatten())
                .filter(|v| !v.is_empty())
        })
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key)
    }
}

/// OS keychain via the `keyring` crate (feature `keyring`).
#[cfg(feature = "keyring")]
pub struct KeyringSecrets {
    service: String,
}

#[cfg(feature = "keyring")]
impl KeyringSecrets {
    /// Secrets under keychain service `service` (e.g. `"ai.mindify.astra"`).
    pub fn new(service: impl Into<String>) -> Self {
        KeyringSecrets { service: service.into() }
    }
}

#[cfg(feature = "keyring")]
impl SecretStore for KeyringSecrets {
    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(&self.service, key).ok()?.get_password().ok()
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(&self.service, key)
            .and_then(|e| e.set_password(value))
            .map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        keyring::Entry::new(&self.service, key)
            .and_then(|e| e.delete_credential())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_and_fallback() {
        let s = WithEnvFallback(MemorySecrets::new());
        assert_eq!(s.get("nope_never_set"), None);
        s.set("k", "v").expect("set");
        assert_eq!(s.get("k").as_deref(), Some("v"));
        s.delete("k").expect("delete");
        assert_eq!(s.get("k"), None);
    }
}
