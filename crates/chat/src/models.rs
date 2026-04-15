//! Model service, probe infrastructure, and disabled model store.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::sync::{Mutex, OnceCell, OwnedSemaphorePermit, RwLock, Semaphore},
    tokio_util::sync::CancellationToken,
    tracing::{debug, info, warn},
};

use {
    moltis_providers::{ProviderRegistry, model_id::raw_model_id},
    moltis_service_traits::{ModelService, ServiceError, ServiceResult},
};

use crate::{
    chat_error::parse_chat_error,
    runtime::ChatRuntime,
    types::{
        BroadcastOpts, broadcast, normalize_model_key, now_ms, probe_max_parallel_per_provider,
        provider_filter_from_params, provider_matches_filter, push_provider_model,
        subscription_provider_rank, suggest_model_ids,
    },
};

// ── Probe infrastructure ────────────────────────────────────────────────────

pub(crate) const PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS: u64 = 1_000;
pub(crate) const PROBE_RATE_LIMIT_MAX_BACKOFF_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy)]
struct ProbeRateLimitState {
    backoff_ms: u64,
    until: Instant,
}

#[derive(Debug, Default)]
struct ProbeRateLimiter {
    by_provider: Mutex<HashMap<String, ProbeRateLimitState>>,
}

impl ProbeRateLimiter {
    async fn remaining_backoff(&self, provider: &str) -> Option<Duration> {
        let map = self.by_provider.lock().await;
        map.get(provider).and_then(|state| {
            let now = Instant::now();
            (state.until > now).then_some(state.until - now)
        })
    }

    async fn mark_rate_limited(&self, provider: &str) -> Duration {
        let mut map = self.by_provider.lock().await;
        let next_backoff_ms =
            next_probe_rate_limit_backoff_ms(map.get(provider).map(|s| s.backoff_ms));
        let delay = Duration::from_millis(next_backoff_ms);
        let state = ProbeRateLimitState {
            backoff_ms: next_backoff_ms,
            until: Instant::now() + delay,
        };
        let _ = map.insert(provider.to_string(), state);
        delay
    }

    async fn clear(&self, provider: &str) {
        let mut map = self.by_provider.lock().await;
        let _ = map.remove(provider);
    }
}

pub(crate) fn next_probe_rate_limit_backoff_ms(previous_ms: Option<u64>) -> u64 {
    previous_ms
        .map(|ms| ms.saturating_mul(2))
        .unwrap_or(PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS)
        .clamp(
            PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS,
            PROBE_RATE_LIMIT_MAX_BACKOFF_MS,
        )
}

pub(crate) fn is_probe_rate_limited_error(error_obj: &Value, error_text: &str) -> bool {
    if error_obj.get("type").and_then(|v| v.as_str()) == Some("rate_limit_exceeded") {
        return true;
    }

    let lower = error_text.to_ascii_lowercase();
    lower.contains("status=429")
        || lower.contains("http 429")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("quota exceeded")
}

#[derive(Debug)]
struct ProbeProviderLimiter {
    permits_per_provider: usize,
    by_provider: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl ProbeProviderLimiter {
    fn new(permits_per_provider: usize) -> Self {
        Self {
            permits_per_provider,
            by_provider: Mutex::new(HashMap::new()),
        }
    }

    async fn acquire(
        &self,
        provider: &str,
    ) -> Result<OwnedSemaphorePermit, tokio::sync::AcquireError> {
        let provider_sem = {
            let mut map = self.by_provider.lock().await;
            Arc::clone(
                map.entry(provider.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(self.permits_per_provider))),
            )
        };

        provider_sem.acquire_owned().await
    }
}

#[derive(Debug)]
enum ProbeStatus {
    Supported,
    Unsupported { detail: String, provider: String },
    Error { message: String },
}

#[derive(Debug)]
struct ProbeOutcome {
    model_id: String,
    display_name: String,
    provider_name: String,
    status: ProbeStatus,
}

/// Run a single model probe: acquire concurrency permits, respect rate-limit
/// backoff, send a "ping" completion, and classify the result.
async fn run_single_probe(
    model_id: String,
    display_name: String,
    provider_name: String,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    limiter: Arc<Semaphore>,
    provider_limiter: Arc<ProbeProviderLimiter>,
    rate_limiter: Arc<ProbeRateLimiter>,
) -> ProbeOutcome {
    let _permit = match limiter.acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => {
            return ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Error {
                    message: "probe limiter closed".to_string(),
                },
            };
        },
    };
    let _provider_permit = match provider_limiter.acquire(&provider_name).await {
        Ok(permit) => permit,
        Err(_) => {
            return ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Error {
                    message: "provider probe limiter closed".to_string(),
                },
            };
        },
    };

    if let Some(wait_for) = rate_limiter.remaining_backoff(&provider_name).await {
        debug!(
            provider = %provider_name,
            model = %model_id,
            wait_ms = wait_for.as_millis() as u64,
            "skipping model probe while provider is in rate-limit backoff"
        );
        return ProbeOutcome {
            model_id,
            display_name,
            provider_name,
            status: ProbeStatus::Error {
                message: format!(
                    "probe skipped due provider backoff ({}ms remaining)",
                    wait_for.as_millis()
                ),
            },
        };
    }

    let completion = tokio::time::timeout(Duration::from_secs(20), provider.probe()).await;

    match completion {
        Ok(Ok(_)) => {
            rate_limiter.clear(&provider_name).await;
            ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Supported,
            }
        },
        Ok(Err(err)) => {
            let error_text = err.to_string();
            let error_obj = parse_chat_error(&error_text, Some(provider_name.as_str()));
            if is_probe_rate_limited_error(&error_obj, &error_text) {
                let backoff = rate_limiter.mark_rate_limited(&provider_name).await;
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Too many requests while probing model support");
                warn!(
                    provider = %provider_name,
                    model = %model_id,
                    backoff_ms = backoff.as_millis() as u64,
                    "model probe rate limited, applying provider backoff"
                );
                return ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Error {
                        message: format!("{detail} (probe backoff {}ms)", backoff.as_millis()),
                    },
                };
            }

            rate_limiter.clear(&provider_name).await;
            let is_unsupported =
                error_obj.get("type").and_then(|v| v.as_str()) == Some("unsupported_model");

            if is_unsupported {
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Model is not supported for this account/provider")
                    .to_string();
                let parsed_provider = error_obj
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(provider_name.as_str())
                    .to_string();
                ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Unsupported {
                        detail,
                        provider: parsed_provider,
                    },
                }
            } else {
                ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Error {
                        message: error_text,
                    },
                }
            }
        },
        Err(_) => ProbeOutcome {
            model_id,
            display_name,
            provider_name,
            status: ProbeStatus::Error {
                message: "probe timeout after 20s".to_string(),
            },
        },
    }
}

// ── Disabled Models Store ────────────────────────────────────────────────────

/// Persistent store for disabled model IDs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisabledModelsStore {
    #[serde(default)]
    pub disabled: HashSet<String>,
    #[serde(default)]
    pub unsupported: HashMap<String, UnsupportedModelInfo>,
}

/// Metadata for a model that failed at runtime due to provider support/account limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedModelInfo {
    pub detail: String,
    pub provider: Option<String>,
    pub updated_at_ms: u64,
}

impl DisabledModelsStore {
    fn config_path() -> Option<PathBuf> {
        moltis_config::config_dir().map(|d| d.join("disabled-models.json"))
    }

    /// Load disabled models from config file.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Save disabled models to config file.
    pub fn save(&self) -> crate::error::Result<()> {
        let path = Self::config_path().ok_or(crate::error::Error::NoConfigDirectory)?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Disable a model by ID.
    pub fn disable(&mut self, model_id: &str) -> bool {
        self.disabled.insert(model_id.to_string())
    }

    /// Enable a model by ID (remove from disabled set).
    pub fn enable(&mut self, model_id: &str) -> bool {
        self.disabled.remove(model_id)
    }

    /// Check if a model is disabled.
    pub fn is_disabled(&self, model_id: &str) -> bool {
        self.disabled.contains(model_id)
    }

    /// Mark a model as unsupported with a human-readable reason.
    pub fn mark_unsupported(
        &mut self,
        model_id: &str,
        detail: &str,
        provider: Option<&str>,
    ) -> bool {
        let next = UnsupportedModelInfo {
            detail: detail.to_string(),
            provider: provider.map(ToString::to_string),
            updated_at_ms: now_ms(),
        };
        let should_update = self
            .unsupported
            .get(model_id)
            .map(|existing| existing.detail != next.detail || existing.provider != next.provider)
            .unwrap_or(true);

        if should_update {
            self.unsupported.insert(model_id.to_string(), next);
            true
        } else {
            false
        }
    }

    /// Clear unsupported status when a model succeeds again.
    pub fn clear_unsupported(&mut self, model_id: &str) -> bool {
        self.unsupported.remove(model_id).is_some()
    }

    /// Get unsupported metadata for a model.
    pub fn unsupported_info(&self, model_id: &str) -> Option<&UnsupportedModelInfo> {
        self.unsupported.get(model_id)
    }
}

// ── LiveModelService ────────────────────────────────────────────────────────

pub struct LiveModelService {
    providers: Arc<RwLock<ProviderRegistry>>,
    disabled: Arc<RwLock<DisabledModelsStore>>,
    state: Arc<OnceCell<Arc<dyn ChatRuntime>>>,
    detect_gate: Arc<Semaphore>,
    /// Token used to cancel an in-flight `detect_supported` run.
    detect_cancel: Arc<RwLock<Option<CancellationToken>>>,
    priority_models: Arc<RwLock<Vec<String>>>,
    show_legacy_models: bool,
    /// Provider config for runtime model rediscovery.
    providers_config: moltis_config::schema::ProvidersConfig,
    /// Environment variable overrides for runtime model rediscovery.
    /// Shared so the gateway can update it after loading UI-stored keys.
    env_overrides: Arc<RwLock<HashMap<String, String>>>,
}

impl LiveModelService {
    pub fn new(
        providers: Arc<RwLock<ProviderRegistry>>,
        disabled: Arc<RwLock<DisabledModelsStore>>,
        priority_models: Vec<String>,
    ) -> Self {
        Self {
            providers,
            disabled,
            state: Arc::new(OnceCell::new()),
            detect_gate: Arc::new(Semaphore::new(1)),
            detect_cancel: Arc::new(RwLock::new(None)),
            priority_models: Arc::new(RwLock::new(priority_models)),
            show_legacy_models: false,
            providers_config: moltis_config::schema::ProvidersConfig::default(),
            env_overrides: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_show_legacy_models(mut self, show: bool) -> Self {
        self.show_legacy_models = show;
        self
    }

    /// Set the provider config and initial env overrides used for runtime
    /// model rediscovery when "Detect All Models" is triggered.
    pub fn with_discovery_config(
        mut self,
        providers_config: moltis_config::schema::ProvidersConfig,
        env_overrides: HashMap<String, String>,
    ) -> Self {
        self.providers_config = providers_config;
        self.env_overrides = Arc::new(RwLock::new(env_overrides));
        self
    }

    /// Shared handle to the env overrides. Pass this to code that needs to
    /// update the overrides after construction (e.g. when runtime UI-stored
    /// API keys are loaded from the credential store).
    pub fn env_overrides_handle(&self) -> Arc<RwLock<HashMap<String, String>>> {
        Arc::clone(&self.env_overrides)
    }

    /// Shared handle to the priority models list. Pass this to services
    /// that need to update model ordering at runtime (e.g. `save_model`).
    pub fn priority_models_handle(&self) -> Arc<RwLock<Vec<String>>> {
        Arc::clone(&self.priority_models)
    }

    fn build_priority_order(models: &[String]) -> HashMap<String, usize> {
        let mut order = HashMap::new();
        for (idx, model) in models.iter().enumerate() {
            let key = normalize_model_key(model);
            if !key.is_empty() {
                let _ = order.entry(key).or_insert(idx);
            }
        }
        order
    }

    fn priority_rank(order: &HashMap<String, usize>, model: &moltis_providers::ModelInfo) -> usize {
        let full = normalize_model_key(&model.id);
        if let Some(rank) = order.get(&full) {
            return *rank;
        }
        let raw = normalize_model_key(raw_model_id(&model.id));
        if let Some(rank) = order.get(&raw) {
            return *rank;
        }
        let display = normalize_model_key(&model.display_name);
        if let Some(rank) = order.get(&display) {
            return *rank;
        }
        usize::MAX
    }

    fn prioritize_models<'a>(
        order: &HashMap<String, usize>,
        models: impl Iterator<Item = &'a moltis_providers::ModelInfo>,
    ) -> Vec<&'a moltis_providers::ModelInfo> {
        let mut ordered: Vec<(usize, &'a moltis_providers::ModelInfo)> =
            models.enumerate().collect();
        ordered.sort_by(|(idx_a, a), (idx_b, b)| {
            let rank_a = Self::priority_rank(order, a);
            let rank_b = Self::priority_rank(order, b);
            // Preferred (rank != MAX) first, then non-preferred
            let bucket_a = if rank_a == usize::MAX {
                1u8
            } else {
                0
            };
            let bucket_b = if rank_b == usize::MAX {
                1u8
            } else {
                0
            };
            bucket_a
                .cmp(&bucket_b)
                .then_with(|| {
                    if bucket_a == 0 {
                        rank_a.cmp(&rank_b)
                    } else {
                        Ordering::Equal
                    }
                })
                .then_with(|| {
                    a.display_name
                        .to_lowercase()
                        .cmp(&b.display_name.to_lowercase())
                })
                .then_with(|| {
                    subscription_provider_rank(&a.provider)
                        .cmp(&subscription_provider_rank(&b.provider))
                })
                .then_with(|| idx_a.cmp(idx_b))
        });
        ordered.into_iter().map(|(_, model)| model).collect()
    }

    async fn priority_order(&self) -> HashMap<String, usize> {
        let list = self.priority_models.read().await;
        Self::build_priority_order(&list)
    }

    /// Set the gateway state reference for broadcasting model updates.
    pub fn set_state(&self, state: Arc<dyn ChatRuntime>) {
        let _ = self.state.set(state);
    }

    async fn broadcast_model_visibility_update(&self, model_id: &str, disabled: bool) {
        if let Some(state) = self.state.get() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "modelId": model_id,
                    "disabled": disabled,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }
    }
}

#[async_trait]
impl ModelService for LiveModelService {
    async fn list(&self) -> ServiceResult {
        let reg = self.providers.read().await;
        let disabled = self.disabled.read().await;
        let order = self.priority_order().await;
        let all_models = reg.list_models_with_reasoning_variants();

        // Hide models older than 1 year from the chat selector unless the
        // user opted in via `providers.show_legacy_models`.  Preferred models
        // and models without a timestamp are never hidden.
        let legacy_cutoff: Option<i64> = if self.show_legacy_models {
            None
        } else {
            let one_year_ago = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
                - 365 * 24 * 60 * 60;
            Some(one_year_ago)
        };

        let prioritized = Self::prioritize_models(
            &order,
            all_models
                .iter()
                .filter(|m| moltis_providers::is_chat_capable_model(&m.id))
                .filter(|m| !disabled.is_disabled(&m.id))
                .filter(|m| disabled.unsupported_info(&m.id).is_none()),
        );
        debug!(model_count = prioritized.len(), "models.list response");
        let models: Vec<_> = prioritized
            .iter()
            .copied()
            .filter(|m| {
                let preferred = Self::priority_rank(&order, m) != usize::MAX;
                if preferred {
                    return true;
                }
                match (legacy_cutoff, m.created_at) {
                    (Some(cutoff), Some(ts)) => ts >= cutoff,
                    _ => true, // no cutoff or no timestamp -> keep
                }
            })
            .map(|m| {
                let preferred = Self::priority_rank(&order, m) != usize::MAX;
                serde_json::json!({
                    "id": m.id,
                    "provider": m.provider,
                    "displayName": m.display_name,
                    "supportsTools": m.capabilities.tools,
                    "supportsVision": m.capabilities.vision,
                    "supportsReasoning": m.capabilities.reasoning,
                    "preferred": preferred,
                    "recommended": m.recommended,
                    "createdAt": m.created_at,
                    "unsupported": false,
                    "unsupportedReason": Value::Null,
                    "unsupportedProvider": Value::Null,
                    "unsupportedUpdatedAt": Value::Null,
                })
            })
            .collect();
        Ok(serde_json::json!(models))
    }

    async fn list_all(&self) -> ServiceResult {
        let reg = self.providers.read().await;
        let disabled = self.disabled.read().await;
        let order = self.priority_order().await;
        let all_models = reg.list_models_with_reasoning_variants();
        let prioritized = Self::prioritize_models(
            &order,
            all_models
                .iter()
                .filter(|m| moltis_providers::is_chat_capable_model(&m.id)),
        );
        info!(model_count = prioritized.len(), "models.list_all response");
        let models: Vec<_> = prioritized
            .iter()
            .copied()
            .map(|m| {
                let preferred = Self::priority_rank(&order, m) != usize::MAX;
                let unsupported = disabled.unsupported_info(&m.id);
                serde_json::json!({
                    "id": m.id,
                    "provider": m.provider,
                    "displayName": m.display_name,
                    "supportsTools": m.capabilities.tools,
                    "supportsVision": m.capabilities.vision,
                    "supportsReasoning": m.capabilities.reasoning,
                    "preferred": preferred,
                    "recommended": m.recommended,
                    "createdAt": m.created_at,
                    "disabled": disabled.is_disabled(&m.id),
                    "unsupported": unsupported.is_some(),
                    "unsupportedReason": unsupported.map(|u| u.detail.clone()),
                    "unsupportedProvider": unsupported.and_then(|u| u.provider.clone()),
                    "unsupportedUpdatedAt": unsupported.map(|u| u.updated_at_ms),
                })
            })
            .collect();
        Ok(serde_json::json!(models))
    }

    async fn disable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "disabling model");

        let mut disabled = self.disabled.write().await;
        disabled.disable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;
        drop(disabled);

        self.broadcast_model_visibility_update(model_id, true).await;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "enabling model");

        let mut disabled = self.disabled.write().await;
        disabled.enable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;
        drop(disabled);

        self.broadcast_model_visibility_update(model_id, false)
            .await;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }

    async fn detect_supported(&self, params: Value) -> ServiceResult {
        let background = params
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("manual")
            .to_string();
        let max_parallel = params
            .get("maxParallel")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 32) as usize)
            .unwrap_or(8);
        let max_parallel_per_provider = probe_max_parallel_per_provider(&params);
        let provider_filter = provider_filter_from_params(&params);

        let _run_permit: OwnedSemaphorePermit = if background {
            match Arc::clone(&self.detect_gate).try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return Ok(serde_json::json!({
                        "ok": true,
                        "background": true,
                        "reason": reason,
                        "skipped": true,
                        "message": "model probe already running",
                    }));
                },
            }
        } else {
            Arc::clone(&self.detect_gate)
                .acquire_owned()
                .await
                .map_err(|_| ServiceError::message("model probe gate closed"))?
        };

        // Install a cancellation token for this run so cancel_detect() can stop it.
        let cancel_token = CancellationToken::new();
        {
            let mut guard = self.detect_cancel.write().await;
            *guard = Some(cancel_token.clone());
        }

        let state = self.state.get().cloned();

        // Phase 0: re-discover models from provider APIs so that newly
        // added models (e.g. a model loaded into llama.cpp after startup)
        // are found before probing.
        {
            let env_snapshot = self.env_overrides.read().await.clone();
            let result = moltis_providers::fetch_discoverable_models(
                &self.providers_config,
                &env_snapshot,
                provider_filter.as_deref(),
            )
            .await;
            if !result.is_empty() {
                let mut reg = self.providers.write().await;
                let new_count = reg.register_rediscovered_models(
                    &self.providers_config,
                    &env_snapshot,
                    &result,
                );
                if new_count > 0 {
                    tracing::info!(
                        new_models = new_count,
                        "rediscovery registered new models before probe"
                    );
                }
            }
        }

        // Phase 1: notify clients to refresh and show the full current model list first.
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": "catalog",
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        let checks = {
            let reg = self.providers.read().await;
            let disabled = self.disabled.read().await;
            reg.list_models()
                .iter()
                .filter(|m| !disabled.is_disabled(&m.id))
                .filter(|m| provider_matches_filter(&m.provider, provider_filter.as_deref()))
                .filter_map(|m| {
                    reg.get(&m.id).map(|provider| {
                        (
                            m.id.clone(),
                            m.display_name.clone(),
                            provider.name().to_string(),
                            provider,
                        )
                    })
                })
                .collect::<Vec<_>>()
        };

        let total = checks.len();
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": "start",
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                    "maxParallelPerProvider": max_parallel_per_provider,
                    "total": total,
                    "checked": 0,
                    "supported": 0,
                    "unsupported": 0,
                    "errors": 0,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        let limiter = Arc::new(Semaphore::new(max_parallel));
        let provider_limiter = Arc::new(ProbeProviderLimiter::new(max_parallel_per_provider));
        let rate_limiter = Arc::new(ProbeRateLimiter::default());
        let mut tasks = tokio::task::JoinSet::new();
        for (model_id, display_name, provider_name, provider) in checks {
            let limiter = Arc::clone(&limiter);
            let provider_limiter = Arc::clone(&provider_limiter);
            let rate_limiter = Arc::clone(&rate_limiter);
            tasks.spawn(run_single_probe(
                model_id,
                display_name,
                provider_name,
                provider,
                limiter,
                provider_limiter,
                rate_limiter,
            ));
        }

        let mut results = Vec::with_capacity(total);
        let mut checked = 0usize;
        let mut supported = 0usize;
        let mut unsupported = 0usize;
        let mut flagged = 0usize;
        let mut cleared = 0usize;
        let mut errors = 0usize;
        let mut supported_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut unsupported_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut errors_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();

        let mut cancelled = false;
        loop {
            let joined = tokio::select! {
                biased;
                () = cancel_token.cancelled() => {
                    tasks.abort_all();
                    cancelled = true;
                    break;
                }
                next = tasks.join_next() => match next {
                    Some(joined) => joined,
                    None => break,
                },
            };
            checked += 1;
            let outcome = match joined {
                Ok(outcome) => outcome,
                Err(err) => {
                    errors += 1;
                    results.push(serde_json::json!({
                        "modelId": "",
                        "displayName": "",
                        "provider": "",
                        "status": "error",
                        "error": format!("probe task failed: {err}"),
                    }));
                    if let Some(state) = state.as_ref() {
                        broadcast(
                            state,
                            "models.updated",
                            serde_json::json!({
                                "phase": "progress",
                                "background": background,
                                "reason": reason,
                                "provider": provider_filter.as_deref(),
                                "total": total,
                                "checked": checked,
                                "supported": supported,
                                "unsupported": unsupported,
                                "errors": errors,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                    }
                    continue;
                },
            };

            match outcome.status {
                ProbeStatus::Supported => {
                    supported += 1;
                    push_provider_model(
                        &mut supported_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    let mut changed = false;
                    {
                        let mut store = self.disabled.write().await;
                        if store.clear_unsupported(&outcome.model_id) {
                            changed = true;
                            if let Err(err) = store.save() {
                                warn!(
                                    model = %outcome.model_id,
                                    error = %err,
                                    "failed to persist unsupported model clear"
                                );
                            }
                        }
                    }
                    if changed {
                        cleared += 1;
                        if let Some(state) = state.as_ref() {
                            broadcast(
                                state,
                                "models.updated",
                                serde_json::json!({
                                    "modelId": outcome.model_id,
                                    "unsupported": false,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                        }
                    }

                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "supported",
                    }));
                },
                ProbeStatus::Unsupported { detail, provider } => {
                    unsupported += 1;
                    push_provider_model(
                        &mut unsupported_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    let mut changed = false;
                    let mut updated_at_ms = now_ms();
                    {
                        let mut store = self.disabled.write().await;
                        if store.mark_unsupported(&outcome.model_id, &detail, Some(&provider)) {
                            changed = true;
                            if let Some(info) = store.unsupported_info(&outcome.model_id) {
                                updated_at_ms = info.updated_at_ms;
                            }
                            if let Err(save_err) = store.save() {
                                warn!(
                                    model = %outcome.model_id,
                                    provider = provider,
                                    error = %save_err,
                                    "failed to persist unsupported model flag"
                                );
                            }
                        }
                    }
                    if changed {
                        flagged += 1;
                        if let Some(state) = state.as_ref() {
                            broadcast(
                                state,
                                "models.updated",
                                serde_json::json!({
                                    "modelId": outcome.model_id,
                                    "unsupported": true,
                                    "unsupportedReason": detail,
                                    "unsupportedProvider": provider,
                                    "unsupportedUpdatedAt": updated_at_ms,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                        }
                    }

                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "unsupported",
                        "error": detail,
                    }));
                },
                ProbeStatus::Error { message } => {
                    errors += 1;
                    push_provider_model(
                        &mut errors_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "error",
                        "error": message,
                    }));
                },
            }

            if let Some(state) = state.as_ref() {
                broadcast(
                    state,
                    "models.updated",
                    serde_json::json!({
                        "phase": "progress",
                        "background": background,
                        "reason": reason,
                        "provider": provider_filter.as_deref(),
                        "total": total,
                        "checked": checked,
                        "supported": supported,
                        "unsupported": unsupported,
                        "errors": errors,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
            }
        }

        // Clear the cancellation token now that the loop has exited.
        {
            let mut guard = self.detect_cancel.write().await;
            *guard = None;
        }

        let phase = if cancelled {
            "cancelled"
        } else {
            "complete"
        };
        let summary = serde_json::json!({
            "ok": true,
            "cancelled": cancelled,
            "probeWord": "ping",
            "background": background,
            "reason": reason,
            "provider": provider_filter.as_deref(),
            "maxParallel": max_parallel,
            "maxParallelPerProvider": max_parallel_per_provider,
            "total": total,
            "checked": checked,
            "supported": supported,
            "unsupported": unsupported,
            "flagged": flagged,
            "cleared": cleared,
            "errors": errors,
            "supportedByProvider": supported_by_provider,
            "unsupportedByProvider": unsupported_by_provider,
            "errorsByProvider": errors_by_provider,
            "results": results,
        });

        // Final refresh event to ensure clients are in sync after the full pass.
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": phase,
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                    "summary": summary,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        Ok(summary)
    }

    async fn cancel_detect(&self) -> ServiceResult {
        let token = self.detect_cancel.read().await.clone();
        let cancelled = if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        };
        Ok(serde_json::json!({ "ok": true, "cancelled": cancelled }))
    }

    async fn test(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        let provider = {
            let reg = self.providers.read().await;
            if let Some(provider) = reg.get(model_id) {
                provider
            } else {
                let available_model_ids: Vec<String> =
                    reg.list_models().iter().map(|m| m.id.clone()).collect();
                let suggestions = suggest_model_ids(model_id, &available_model_ids, 5);
                warn!(
                    model_id,
                    available_model_count = available_model_ids.len(),
                    available_model_ids = ?available_model_ids,
                    suggested_model_ids = ?suggestions,
                    "models.test received unknown model id"
                );
                let suggestion_hint = if suggestions.is_empty() {
                    String::new()
                } else {
                    format!(". did you mean: {}", suggestions.join(", "))
                };
                return Err(format!("unknown model: {model_id}{suggestion_hint}").into());
            }
        };
        let started = Instant::now();
        info!(model_id, provider = provider.name(), "model probe started");

        match provider.probe().await {
            Ok(()) => {
                info!(
                    model_id,
                    provider = provider.name(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "model probe succeeded"
                );
                Ok(serde_json::json!({
                    "ok": true,
                    "modelId": model_id,
                }))
            },
            Err(err) => {
                let error_text = err.to_string();
                let error_obj = parse_chat_error(&error_text, Some(provider.name()));
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&error_text)
                    .to_string();

                warn!(
                    model_id,
                    provider = provider.name(),
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %detail,
                    "model probe failed"
                );
                Err(detail.into())
            },
        }
    }
}
