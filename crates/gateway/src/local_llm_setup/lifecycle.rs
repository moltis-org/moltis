//! Model lifecycle manager — idle-timeout unloading and manual load/unload.

use std::{collections::HashMap, sync::Arc};

use {
    serde::Serialize,
    time::OffsetDateTime,
    tokio::sync::RwLock,
    tracing::{debug, info},
};

use moltis_providers::local_llm::{self, backend::loaded_llama_model_bytes};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

/// Per-model lifecycle state exposed to the web UI.
#[derive(Debug, Clone, Serialize)]
pub struct ModelState {
    pub model_id: String,
    pub is_loaded: bool,
    pub memory_bytes: u64,
    pub last_activity: i64,
    pub idle_timeout_secs: Option<u64>,
}

/// Manages idle-timeout unloading and manual load/unload for local LLM models.
pub struct ModelLifecycleManager {
    providers: RwLock<HashMap<String, Arc<local_llm::LocalLlmProvider>>>,
    timeouts: RwLock<HashMap<String, Option<u64>>>,
    state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
}

impl Default for ModelLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelLifecycleManager {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            timeouts: RwLock::new(HashMap::new()),
            state: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// Set the gateway state for broadcasting lifecycle events.
    pub fn set_state(&self, state: Arc<GatewayState>) {
        let _ = self.state.set(state);
    }

    /// Register a model provider with its effective timeout.
    pub async fn register(
        &self,
        model_id: String,
        provider: Arc<local_llm::LocalLlmProvider>,
        timeout: Option<u64>,
    ) {
        self.providers
            .write()
            .await
            .insert(model_id.clone(), provider);
        self.timeouts.write().await.insert(model_id, timeout);
    }

    /// Remove a model from lifecycle management.
    pub async fn unregister(&self, model_id: &str) {
        self.providers.write().await.remove(model_id);
        self.timeouts.write().await.remove(model_id);
    }

    /// Remove all models (used before re-populating after registry rebuild).
    pub async fn clear(&self) {
        self.providers.write().await.clear();
        self.timeouts.write().await.clear();
    }

    /// Update the idle timeout for a model.
    pub async fn set_timeout(&self, model_id: &str, timeout: Option<u64>) {
        self.timeouts
            .write()
            .await
            .insert(model_id.to_string(), timeout);
    }

    /// Manually load a model (calls `ensure_loaded`), broadcasting events.
    pub async fn load_model(&self, model_id: &str) -> anyhow::Result<()> {
        let provider = {
            let providers = self.providers.read().await;
            Arc::clone(
                providers
                    .get(model_id)
                    .ok_or_else(|| anyhow::anyhow!("unknown model: {model_id}"))?,
            )
        };

        if provider.is_loaded().await {
            return Ok(());
        }

        self.broadcast_lifecycle(model_id, "loading", 0, loaded_llama_model_bytes(), "manual")
            .await;

        provider.ensure_loaded().await?;

        let model_bytes = provider.model_size_bytes().await;
        self.broadcast_lifecycle(
            model_id,
            "loaded",
            model_bytes,
            loaded_llama_model_bytes(),
            "manual",
        )
        .await;

        info!(model = model_id, bytes = model_bytes, "model loaded");
        Ok(())
    }

    /// Manually unload a model, broadcasting events.
    pub async fn unload_model(&self, model_id: &str) -> anyhow::Result<()> {
        let provider = {
            let providers = self.providers.read().await;
            Arc::clone(
                providers
                    .get(model_id)
                    .ok_or_else(|| anyhow::anyhow!("unknown model: {model_id}"))?,
            )
        };

        let model_bytes = provider.model_size_bytes().await;

        self.broadcast_lifecycle(
            model_id,
            "unloading",
            model_bytes,
            loaded_llama_model_bytes(),
            "manual",
        )
        .await;

        let was_loaded = provider.unload().await;

        if was_loaded {
            self.broadcast_lifecycle(
                model_id,
                "unloaded",
                model_bytes,
                loaded_llama_model_bytes(),
                "manual",
            )
            .await;
            info!(
                model = model_id,
                freed_bytes = model_bytes,
                "model unloaded"
            );
        }

        Ok(())
    }

    /// Return current state for all registered models.
    pub async fn model_states(&self) -> Vec<ModelState> {
        // Snapshot Arcs and timeouts so we don't hold locks across awaits.
        let snapshot: Vec<(String, Arc<local_llm::LocalLlmProvider>, Option<u64>)> = {
            let providers = self.providers.read().await;
            let timeouts = self.timeouts.read().await;
            providers
                .iter()
                .map(|(id, prov)| {
                    (
                        id.clone(),
                        Arc::clone(prov),
                        timeouts.get(id).copied().flatten(),
                    )
                })
                .collect()
        };

        let mut states = Vec::with_capacity(snapshot.len());
        for (model_id, provider, timeout) in &snapshot {
            states.push(ModelState {
                model_id: model_id.clone(),
                is_loaded: provider.is_loaded().await,
                memory_bytes: provider.model_size_bytes().await,
                last_activity: provider.last_activity_secs(),
                idle_timeout_secs: *timeout,
            });
        }

        states
    }

    /// Spawn the background idle-check timer. Returns the `JoinHandle`.
    pub fn spawn_idle_checker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                mgr.check_idle_models().await;
            }
        })
    }

    /// One pass of the idle checker.
    async fn check_idle_models(&self) {
        let now = OffsetDateTime::now_utc().unix_timestamp();

        // Snapshot provider Arcs and timeouts so we don't hold locks across awaits.
        let snapshot: Vec<(String, Arc<local_llm::LocalLlmProvider>, u64)> = {
            let providers = self.providers.read().await;
            let timeouts = self.timeouts.read().await;
            providers
                .iter()
                .filter_map(|(id, prov)| {
                    let timeout = timeouts.get(id).copied().flatten()?;
                    if timeout == 0 {
                        return None; // 0 = never unload
                    }
                    Some((id.clone(), Arc::clone(prov), timeout))
                })
                .collect()
        };

        for (model_id, provider, timeout) in &snapshot {
            if !provider.is_loaded().await {
                continue;
            }

            let last = provider.last_activity_secs();
            if last == 0 {
                continue; // never used yet
            }

            let idle_secs = now.saturating_sub(last) as u64;
            if idle_secs <= *timeout {
                continue;
            }

            debug!(
                model = model_id.as_str(),
                idle_secs, timeout, "model idle timeout reached, unloading"
            );

            let model_bytes = provider.model_size_bytes().await;
            self.broadcast_lifecycle(
                model_id,
                "unloading",
                model_bytes,
                loaded_llama_model_bytes(),
                "idle",
            )
            .await;

            let was_loaded = provider.unload().await;
            if was_loaded {
                self.broadcast_lifecycle(
                    model_id,
                    "unloaded",
                    model_bytes,
                    loaded_llama_model_bytes(),
                    "idle",
                )
                .await;
                info!(
                    model = model_id.as_str(),
                    freed_bytes = model_bytes,
                    idle_secs,
                    "model auto-unloaded after idle timeout"
                );
            }
        }
    }

    async fn broadcast_lifecycle(
        &self,
        model_id: &str,
        state_name: &str,
        model_size_bytes: u64,
        total_loaded_bytes: u64,
        reason: &str,
    ) {
        let Some(state) = self.state.get() else {
            return;
        };
        broadcast(
            state,
            "local-llm.lifecycle",
            serde_json::json!({
                "modelId": model_id,
                "state": state_name,
                "modelSizeBytes": model_size_bytes,
                "totalLoadedBytes": total_loaded_bytes,
                "reason": reason,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_model_states_empty() {
        let mgr = ModelLifecycleManager::new();
        let states = mgr.model_states().await;
        assert!(states.is_empty());
    }

    #[tokio::test]
    async fn test_register_and_unregister() {
        let mgr = ModelLifecycleManager::new();
        let config = local_llm::LocalLlmConfig {
            model_id: "test-model".into(),
            ..Default::default()
        };
        let provider = Arc::new(local_llm::LocalLlmProvider::new(config));

        mgr.register("test-model".into(), provider, Some(300)).await;
        assert_eq!(mgr.model_states().await.len(), 1);

        mgr.unregister("test-model").await;
        assert!(mgr.model_states().await.is_empty());
    }

    #[tokio::test]
    async fn test_model_state_fields() {
        let mgr = ModelLifecycleManager::new();
        let config = local_llm::LocalLlmConfig {
            model_id: "test-model".into(),
            ..Default::default()
        };
        let provider = Arc::new(local_llm::LocalLlmProvider::new(config));

        mgr.register("test-model".into(), provider, Some(600)).await;

        let states = mgr.model_states().await;
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].model_id, "test-model");
        assert!(!states[0].is_loaded);
        assert_eq!(states[0].memory_bytes, 0);
        assert_eq!(states[0].last_activity, 0);
        assert_eq!(states[0].idle_timeout_secs, Some(600));
    }

    #[tokio::test]
    async fn test_set_timeout() {
        let mgr = ModelLifecycleManager::new();
        let config = local_llm::LocalLlmConfig {
            model_id: "test-model".into(),
            ..Default::default()
        };
        let provider = Arc::new(local_llm::LocalLlmProvider::new(config));

        mgr.register("test-model".into(), provider, None).await;
        assert_eq!(mgr.model_states().await[0].idle_timeout_secs, None);

        mgr.set_timeout("test-model", Some(120)).await;
        assert_eq!(mgr.model_states().await[0].idle_timeout_secs, Some(120));
    }

    #[tokio::test]
    async fn test_unload_unknown_model() {
        let mgr = ModelLifecycleManager::new();
        let result = mgr.unload_model("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_unknown_model() {
        let mgr = ModelLifecycleManager::new();
        let result = mgr.load_model("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_check_idle_models_no_timeout() {
        // Models with no timeout should not be unloaded
        let mgr = ModelLifecycleManager::new();
        let config = local_llm::LocalLlmConfig {
            model_id: "test".into(),
            ..Default::default()
        };
        let provider = Arc::new(local_llm::LocalLlmProvider::new(config));
        mgr.register("test".into(), provider, None).await;
        // Should not panic
        mgr.check_idle_models().await;
    }

    #[tokio::test]
    async fn test_check_idle_models_zero_timeout() {
        // Models with timeout=0 should never be unloaded
        let mgr = ModelLifecycleManager::new();
        let config = local_llm::LocalLlmConfig {
            model_id: "test".into(),
            ..Default::default()
        };
        let provider = Arc::new(local_llm::LocalLlmProvider::new(config));
        mgr.register("test".into(), provider, Some(0)).await;
        mgr.check_idle_models().await;
    }
}
