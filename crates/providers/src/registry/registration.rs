#![allow(unused_imports)]

use super::*;

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use {
    moltis_config::schema::{ProviderStreamTransport, ProvidersConfig},
    secrecy::ExposeSecret,
    tokio_stream::Stream,
};

use moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent};

#[cfg(feature = "provider-github-copilot")]
use crate::github_copilot;
#[cfg(feature = "provider-kimi-code")]
use crate::kimi_code;
#[cfg(feature = "local-llm")]
use crate::local_gguf;
#[cfg(feature = "local-llm")]
use crate::local_llm;
#[cfg(feature = "provider-openai-codex")]
use crate::openai_codex;
#[allow(unused_imports)]
use crate::{
    anthropic, async_openai_provider, config_helpers::*, discovered_model::*, genai_provider,
    model_capabilities::*, model_catalogs::*, model_id::*, ollama::*, openai,
};
impl ProviderRegistry {
    /// Register models from a [`RediscoveryResult`], skipping those already
    /// present. All I/O (model list fetches, Ollama probes) must be completed
    /// before calling this — it only does fast in-memory work.
    ///
    /// Returns the number of newly registered models.
    pub fn register_rediscovered_models(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        result: &RediscoveryResult,
    ) -> usize {
        let fetched = &result.models;
        let mut added = 0usize;

        // ── Anthropic builtin ─────────────────────────────────────────
        if fetched.contains_key("anthropic")
            && config.is_enabled("anthropic")
            && let Some(key) =
                resolve_api_key(config, "anthropic", "ANTHROPIC_API_KEY", env_overrides)
        {
            let base_url = config
                .get("anthropic")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "ANTHROPIC_BASE_URL"))
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            let alias = config.get("anthropic").and_then(|e| e.alias.clone());
            let provider_label = alias.clone().unwrap_or_else(|| "anthropic".into());
            let cache_retention = config
                .get("anthropic")
                .map(|e| e.cache_retention)
                .unwrap_or(moltis_config::CacheRetention::Short);
            let models = Self::desired_anthropic_models(config, fetched);

            added += self.replace_anthropic_catalog(
                models,
                &key,
                &base_url,
                &provider_label,
                alias,
                cache_retention,
            );
        }

        // ── OpenAI builtin ────────────────────────────────────────────
        if let Some(models) = fetched.get("openai")
            && config.is_enabled("openai")
            && let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY", env_overrides)
        {
            let base_url = config
                .get("openai")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "OPENAI_BASE_URL"))
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            let alias = config.get("openai").and_then(|e| e.alias.clone());
            let provider_label = alias.unwrap_or_else(|| "openai".into());
            let stream_transport = config
                .get("openai")
                .map(|entry| entry.stream_transport)
                .unwrap_or(ProviderStreamTransport::Sse);

            for model in models {
                if self.has_provider_model(&provider_label, &model.id) {
                    continue;
                }
                let provider = Arc::new(
                    openai::OpenAiProvider::new_with_name(
                        key.clone(),
                        model.id.clone(),
                        base_url.clone(),
                        provider_label.clone(),
                    )
                    .with_stream_transport(stream_transport),
                );
                self.register(
                    ModelInfo {
                        id: model.id.clone(),
                        provider: provider_label.clone(),
                        display_name: model.display_name.clone(),
                        created_at: model.created_at,
                        recommended: model.recommended,
                        capabilities: model
                            .capabilities
                            .unwrap_or_else(|| ModelCapabilities::infer(&model.id)),
                    },
                    provider,
                );
                added += 1;
            }
        }

        // ── OpenAI-compatible providers ───────────────────────────────
        for def in OPENAI_COMPAT_PROVIDERS {
            let Some(models) = fetched.get(def.config_name) else {
                continue;
            };
            if !config.is_enabled(def.config_name) {
                continue;
            }

            let key = resolve_api_key(config, def.config_name, def.env_key, env_overrides);
            let key = if !def.requires_api_key {
                key.or_else(|| Some(secrecy::Secret::new(def.config_name.into())))
            } else if def.config_name == "gemini" {
                key.or_else(|| env_value(env_overrides, "GOOGLE_API_KEY").map(secrecy::Secret::new))
            } else {
                key
            };
            let Some(key) = key else {
                continue;
            };

            let base_url = config
                .get(def.config_name)
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, def.env_base_url_key))
                .unwrap_or_else(|| def.default_base_url.into());
            let alias = config.get(def.config_name).and_then(|e| e.alias.clone());
            let provider_label = alias.unwrap_or_else(|| def.config_name.into());
            let stream_transport = config
                .get(def.config_name)
                .map(|entry| entry.stream_transport)
                .unwrap_or(ProviderStreamTransport::Sse);
            let cache_retention = config
                .get(def.config_name)
                .map(|e| e.cache_retention)
                .unwrap_or(moltis_config::CacheRetention::Short);
            let config_tool_mode = config
                .get(def.config_name)
                .map(|e| e.tool_mode)
                .unwrap_or_default();
            let is_ollama = def.config_name == "ollama";

            // Use pre-fetched Ollama `/api/show` probes (already collected
            // outside the registry lock by `fetch_discoverable_models`).
            let empty_probes = HashMap::new();
            let ollama_probes: &HashMap<String, OllamaShowResponse> = if is_ollama {
                &result.ollama_probes
            } else {
                &empty_probes
            };

            for model in models {
                if self.has_provider_model(&provider_label, &model.id) {
                    continue;
                }
                let effective_tool_mode = if is_ollama {
                    resolve_ollama_tool_mode(
                        config_tool_mode,
                        &model.id,
                        ollama_probes.get(&model.id),
                    )
                } else if !matches!(config_tool_mode, moltis_config::ToolMode::Auto) {
                    config_tool_mode
                } else {
                    moltis_config::ToolMode::Auto
                };

                let mut oai = openai::OpenAiProvider::new_with_name(
                    key.clone(),
                    model.id.clone(),
                    base_url.clone(),
                    provider_label.clone(),
                )
                .with_stream_transport(stream_transport)
                .with_cache_retention(cache_retention);

                if !matches!(effective_tool_mode, moltis_config::ToolMode::Auto) {
                    oai = oai.with_tool_mode(effective_tool_mode);
                }
                if let Some(strict) = config.get(def.config_name).and_then(|e| e.strict_tools) {
                    oai = oai.with_strict_tools(strict);
                }

                self.register(
                    ModelInfo {
                        id: model.id.clone(),
                        provider: provider_label.clone(),
                        display_name: model.display_name.clone(),
                        created_at: model.created_at,
                        recommended: model.recommended,
                        capabilities: model
                            .capabilities
                            .unwrap_or_else(|| ModelCapabilities::infer(&model.id)),
                    },
                    Arc::new(oai),
                );
                added += 1;
            }
        }

        // ── Custom providers ──────────────────────────────────────────
        for (name, entry) in &config.providers {
            if !name.starts_with("custom-") || !entry.enabled {
                continue;
            }
            let Some(models) = fetched.get(name.as_str()) else {
                continue;
            };
            let Some(api_key) = entry
                .api_key
                .as_ref()
                .filter(|k| !k.expose_secret().is_empty())
            else {
                continue;
            };
            let Some(base_url) = entry.base_url.as_ref().filter(|u| !u.trim().is_empty()) else {
                continue;
            };
            let custom_tool_mode = entry.tool_mode;

            for model in models {
                if self.has_provider_model(name, &model.id) {
                    continue;
                }
                let mut oai = openai::OpenAiProvider::new_with_name(
                    api_key.clone(),
                    model.id.clone(),
                    base_url.clone(),
                    name.clone(),
                )
                .with_stream_transport(entry.stream_transport);
                if !matches!(entry.wire_api, moltis_config::WireApi::ChatCompletions) {
                    oai = oai.with_wire_api(entry.wire_api);
                }
                if !matches!(custom_tool_mode, moltis_config::ToolMode::Auto) {
                    oai = oai.with_tool_mode(custom_tool_mode);
                }
                self.register(
                    ModelInfo {
                        id: model.id.clone(),
                        provider: name.clone(),
                        display_name: model.display_name.clone(),
                        created_at: model.created_at,
                        recommended: model.recommended,
                        capabilities: model
                            .capabilities
                            .unwrap_or_else(|| ModelCapabilities::infer(&model.id)),
                    },
                    Arc::new(oai),
                );
                added += 1;
            }
        }

        added
    }

    #[cfg(feature = "provider-genai")]
    pub(crate) fn register_genai_providers(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
    ) {
        use crate::genai_provider;

        // (env_key, provider_config_name, model_id, display_name)
        let genai_models: &[(&str, &str, &str, &str)] = &[
            (
                "ANTHROPIC_API_KEY",
                "anthropic",
                "claude-sonnet-4-20250514",
                "Claude Sonnet 4 (genai)",
            ),
            ("OPENAI_API_KEY", "openai", "gpt-4o", "GPT-4o (genai)"),
            (
                "GROQ_API_KEY",
                "groq",
                "llama-3.1-8b-instant",
                "Llama 3.1 8B (genai/groq)",
            ),
            ("XAI_API_KEY", "xai", "grok-3-mini", "Grok 3 Mini (genai)"),
        ];

        for &(env_key, provider_name, default_model_id, display_name) in genai_models {
            if !config.is_enabled(provider_name) {
                continue;
            }

            // Use config api_key or fall back to env var.
            let Some(resolved_key) = resolve_api_key(config, provider_name, env_key, env_overrides)
            else {
                continue;
            };

            let model_id = configured_models_for_provider(config, provider_name)
                .into_iter()
                .next()
                .unwrap_or_else(|| default_model_id.to_string());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get(provider_name).and_then(|e| e.alias.clone());
            let genai_provider_name = alias.unwrap_or_else(|| format!("genai/{provider_name}"));
            if self.has_model_any_provider(&model_id) {
                continue;
            }

            let provider = Arc::new(genai_provider::GenaiProvider::new(
                model_id.clone(),
                genai_provider_name.clone(),
                resolved_key,
            ));
            self.register(
                ModelInfo {
                    id: model_id.clone(),
                    provider: genai_provider_name,
                    display_name: display_name.into(),
                    created_at: None,
                    recommended: false,
                    capabilities: ModelCapabilities::infer(&model_id),
                },
                provider,
            );
        }
    }

    #[cfg(feature = "provider-async-openai")]
    pub(crate) fn register_async_openai_providers(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
    ) {
        use crate::async_openai_provider;

        if !config.is_enabled("openai") {
            return;
        }

        let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY", env_overrides) else {
            return;
        };

        let base_url = config
            .get("openai")
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, "OPENAI_BASE_URL"))
            .unwrap_or_else(|| "https://api.openai.com/v1".into());

        let model_id = configured_models_for_provider(config, "openai")
            .into_iter()
            .next()
            .unwrap_or_else(|| "gpt-4o".to_string());

        // Get alias if configured (for metrics differentiation).
        let alias = config.get("openai").and_then(|e| e.alias.clone());
        let provider_label = alias.clone().unwrap_or_else(|| "async-openai".into());
        if self.has_model_any_provider(&model_id) {
            return;
        }

        let provider = Arc::new(async_openai_provider::AsyncOpenAiProvider::with_alias(
            key,
            model_id.clone(),
            base_url,
            alias,
        ));
        self.register(
            ModelInfo {
                id: model_id.clone(),
                provider: provider_label,
                display_name: "GPT-4o (async-openai)".into(),
                created_at: None,
                recommended: false,
                capabilities: ModelCapabilities::infer(&model_id),
            },
            provider,
        );
    }

    #[cfg(feature = "provider-openai-codex")]
    pub(super) fn register_openai_codex_providers(
        &mut self,
        config: &ProvidersConfig,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) {
        use crate::openai_codex;
        let source = OpenAiCodexDiscovery;
        let catalog = if source.should_fetch_models(config) {
            // Use pre-fetched live models from parallel discovery.
            let fallback = openai_codex::default_model_catalog();
            match prefetched.get("openai-codex") {
                Some(live) => {
                    let merged = merge_discovered_with_fallback_catalog(live.clone(), fallback);
                    tracing::info!(
                        model_count = merged.len(),
                        "loaded openai-codex models catalog"
                    );
                    merged
                },
                None => fallback,
            }
        } else {
            Vec::new()
        };
        self.register_dynamic_source_models(&source, config, catalog);
    }

    pub fn refresh_openai_codex_models(&mut self, config: &ProvidersConfig) -> bool {
        #[cfg(feature = "provider-openai-codex")]
        {
            let source = OpenAiCodexDiscovery;
            self.refresh_dynamic_source_models(&source, config)
        }

        #[cfg(not(feature = "provider-openai-codex"))]
        {
            let _ = config;
            false
        }
    }

    #[cfg(feature = "provider-github-copilot")]
    pub(super) fn register_github_copilot_providers(
        &mut self,
        config: &ProvidersConfig,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) {
        let source = GitHubCopilotDiscovery;
        let catalog = if source.should_fetch_models(config) {
            // Use pre-fetched live models from parallel discovery.
            let fallback = crate::github_copilot::default_model_catalog();
            match prefetched.get("github-copilot") {
                Some(live) => {
                    let merged = merge_discovered_with_fallback_catalog(live.clone(), fallback);
                    tracing::debug!(
                        model_count = merged.len(),
                        "loaded github-copilot models catalog"
                    );
                    merged
                },
                None => fallback,
            }
        } else {
            Vec::new()
        };
        self.register_dynamic_source_models(&source, config, catalog);
    }

    pub fn refresh_github_copilot_models(&mut self, config: &ProvidersConfig) -> bool {
        #[cfg(feature = "provider-github-copilot")]
        {
            let source = GitHubCopilotDiscovery;
            self.refresh_dynamic_source_models(&source, config)
        }

        #[cfg(not(feature = "provider-github-copilot"))]
        {
            let _ = config;
            false
        }
    }

    pub fn refresh_dynamic_models(&mut self, config: &ProvidersConfig) -> Vec<(String, bool)> {
        #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
        {
            let mut results = Vec::new();
            for source in Self::dynamic_discovery_sources() {
                let refreshed = self.refresh_dynamic_source_models(source.as_ref(), config);
                results.push((source.provider_name().to_string(), refreshed));
            }
            results
        }

        #[cfg(not(any(feature = "provider-openai-codex", feature = "provider-github-copilot")))]
        {
            let _ = config;
            Vec::new()
        }
    }

    #[cfg(feature = "provider-kimi-code")]
    pub(super) fn register_kimi_code_providers(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
    ) {
        use crate::kimi_code;

        if !config.is_enabled("kimi-code") {
            return;
        }

        let api_key = resolve_api_key(config, "kimi-code", "KIMI_API_KEY", env_overrides);
        let has_oauth_tokens = kimi_code::has_stored_tokens();
        if api_key.is_none() && !has_oauth_tokens {
            return;
        }

        let base_url = config
            .get("kimi-code")
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, "KIMI_BASE_URL"))
            .unwrap_or_else(|| "https://api.kimi.com/coding/v1".into());

        let build_provider = |model_id: &str| -> Arc<dyn LlmProvider> {
            if let Some(api_key) = api_key.as_ref() {
                Arc::new(kimi_code::KimiCodeProvider::new_with_api_key(
                    api_key.clone(),
                    model_id.into(),
                    base_url.clone(),
                ))
            } else {
                Arc::new(kimi_code::KimiCodeProvider::new(model_id.into()))
            }
        };

        let preferred = configured_models_for_provider(config, "kimi-code");
        let discovered = if should_fetch_models(config, "kimi-code") {
            catalog_to_discovered(kimi_code::KIMI_CODE_MODELS, 1)
        } else {
            Vec::new()
        };
        let models = merge_preferred_and_discovered_models(preferred, discovered);
        for model in models {
            let caps = model
                .capabilities
                .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
            let (model_id, display_name, created_at, recommended) = (
                model.id,
                model.display_name,
                model.created_at,
                model.recommended,
            );
            if self.has_provider_model("kimi-code", &model_id) {
                continue;
            }
            let provider = build_provider(&model_id);
            self.register(
                ModelInfo {
                    id: model_id,
                    provider: "kimi-code".into(),
                    display_name,
                    created_at,
                    recommended,
                    capabilities: caps,
                },
                provider,
            );
        }
    }

    #[cfg(feature = "local-llm")]
    pub(super) fn register_local_gguf_providers(&mut self, config: &ProvidersConfig) {
        use std::path::PathBuf;

        use crate::{local_gguf, local_llm};

        if !config.is_enabled("local") {
            return;
        }

        // Collect all model IDs to register:
        // 1. From local_models (multi-model config from local-llm.json)
        // 2. From provider models in config (preferred pins)
        let mut model_ids: Vec<String> = config.local_models.clone();
        model_ids.extend(configured_models_for_provider(config, "local"));
        model_ids = normalize_unique_models(model_ids);

        if model_ids.is_empty() {
            tracing::info!(
                "local-llm enabled but no models configured. Add [providers.local] models = [\"...\"] to config."
            );
            return;
        }

        // Only probe local hardware/backends when at least one local model is
        // configured. On macOS this avoids loading Metal/MLX runtime state
        // during startup when local inference is not in use.
        local_gguf::log_system_info_and_suggestions();

        // Build config from provider entry for user overrides
        let entry = config.get("local");
        let user_model_path = entry
            .and_then(|e| e.base_url.as_deref()) // Reuse base_url for model_path
            .map(PathBuf::from);

        // Register each model
        for model_id in model_ids {
            if self.has_provider_model("local-llm", &model_id) {
                continue;
            }

            // Look up model in registries to get display name
            let display_name = if let Some(def) = local_llm::models::find_model(&model_id) {
                def.display_name.to_string()
            } else if let Some(def) = local_gguf::models::find_model(&model_id) {
                def.display_name.to_string()
            } else {
                format!("{} (local)", model_id)
            };

            // Use LocalLlmProvider which auto-detects backend based on model type
            let llm_config = local_llm::LocalLlmConfig {
                model_id: model_id.clone(),
                model_path: user_model_path.clone(),
                backend: None, // Auto-detect based on model type
                context_size: None,
                gpu_layers: 0,
                temperature: 0.7,
                cache_dir: local_llm::models::default_models_dir(),
            };

            tracing::info!(
                model = %model_id,
                display_name = %display_name,
                "local-llm model configured (will load on first use)"
            );

            // Use LocalLlmProvider which properly routes to GGUF or MLX backend
            let provider = Arc::new(local_llm::LocalLlmProvider::new(llm_config));
            self.register(
                ModelInfo {
                    id: model_id.clone(),
                    provider: "local-llm".into(),
                    display_name,
                    created_at: None,
                    recommended: false,
                    capabilities: ModelCapabilities::infer(&model_id),
                },
                provider,
            );
        }
    }

    pub(crate) fn register_builtin_providers(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) {
        // Anthropic — register all known Claude models when API key is available.
        if config.is_enabled("anthropic")
            && let Some(key) =
                resolve_api_key(config, "anthropic", "ANTHROPIC_API_KEY", env_overrides)
        {
            let base_url = config
                .get("anthropic")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "ANTHROPIC_BASE_URL"))
                .unwrap_or_else(|| "https://api.anthropic.com".into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get("anthropic").and_then(|e| e.alias.clone());
            let provider_label = alias.clone().unwrap_or_else(|| "anthropic".into());
            let cache_retention = config
                .get("anthropic")
                .map(|e| e.cache_retention)
                .unwrap_or(moltis_config::CacheRetention::Short);
            let models = Self::desired_anthropic_models(config, prefetched);
            self.register_anthropic_catalog(
                models,
                &key,
                &base_url,
                &provider_label,
                alias,
                cache_retention,
            );
        }

        // OpenAI — register all known OpenAI models when API key is available.
        if config.is_enabled("openai")
            && let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY", env_overrides)
        {
            let base_url = config
                .get("openai")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "OPENAI_BASE_URL"))
                .unwrap_or_else(|| "https://api.openai.com/v1".into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get("openai").and_then(|e| e.alias.clone());
            let provider_label = alias.clone().unwrap_or_else(|| "openai".into());
            let stream_transport = config
                .get("openai")
                .map(|entry| entry.stream_transport)
                .unwrap_or(ProviderStreamTransport::Sse);
            let preferred = configured_models_for_provider(config, "openai");
            let discovered = if should_fetch_models(config, "openai") {
                // Use pre-fetched live models from parallel discovery.
                let fallback = openai::default_model_catalog();
                match prefetched.get("openai") {
                    Some(live) => {
                        let merged = merge_discovered_with_fallback_catalog(live.clone(), fallback);
                        tracing::debug!(model_count = merged.len(), "loaded openai models catalog");
                        merged
                    },
                    None => fallback,
                }
            } else {
                Vec::new()
            };
            let models = merge_preferred_and_discovered_models(preferred, discovered);

            for model in models {
                let caps = model
                    .capabilities
                    .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
                let (model_id, display_name, created_at, recommended) = (
                    model.id,
                    model.display_name,
                    model.created_at,
                    model.recommended,
                );
                if self.has_provider_model(&provider_label, &model_id) {
                    continue;
                }
                let provider = Arc::new(
                    openai::OpenAiProvider::new_with_name(
                        key.clone(),
                        model_id.clone(),
                        base_url.clone(),
                        provider_label.clone(),
                    )
                    .with_stream_transport(stream_transport),
                );
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: provider_label.clone(),
                        display_name,
                        created_at,
                        recommended,
                        capabilities: caps,
                    },
                    provider,
                );
            }
        }
    }

    pub(crate) fn register_openai_compatible_providers(
        &mut self,
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) {
        for def in OPENAI_COMPAT_PROVIDERS {
            if !config.is_enabled(def.config_name) {
                continue;
            }

            let key = resolve_api_key(config, def.config_name, def.env_key, env_overrides);

            // Local providers don't require an API key — use a dummy value.
            // Gemini accepts both GEMINI_API_KEY and GOOGLE_API_KEY.
            let key = if !def.requires_api_key {
                key.or_else(|| Some(secrecy::Secret::new(def.config_name.into())))
            } else if def.config_name == "gemini" {
                key.or_else(|| env_value(env_overrides, "GOOGLE_API_KEY").map(secrecy::Secret::new))
            } else {
                key
            };

            let Some(key) = key else {
                continue;
            };

            let base_url = config
                .get(def.config_name)
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, def.env_base_url_key))
                .unwrap_or_else(|| def.default_base_url.into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get(def.config_name).and_then(|e| e.alias.clone());
            let provider_label = alias.unwrap_or_else(|| def.config_name.into());
            let cache_retention = config
                .get(def.config_name)
                .map(|e| e.cache_retention)
                .unwrap_or(moltis_config::CacheRetention::Short);
            let stream_transport = config
                .get(def.config_name)
                .map(|entry| entry.stream_transport)
                .unwrap_or(ProviderStreamTransport::Sse);
            let preferred = configured_models_for_provider(config, def.config_name);
            if def.local_only {
                let has_explicit_entry = config.get(def.config_name).is_some();
                let has_env_base_url = env_value(env_overrides, def.env_base_url_key).is_some();
                if !has_explicit_entry && !has_env_base_url && preferred.is_empty() {
                    continue;
                }
            }
            // Some providers need an explicit model before they can answer;
            // keep discovery off there when no model is configured.
            // OpenRouter supports `/models`, so we discover dynamically.
            let skip_discovery = def.models.is_empty()
                && preferred.is_empty()
                && !def.local_only
                && (def.config_name == "venice" || cfg!(test));
            // Respect `supports_model_discovery`: providers whose API lacks a
            // /models endpoint (e.g. MiniMax) skip live fetch unless the user
            // explicitly opted in via `fetch_models = true` in config.
            let user_opted_in = config
                .get(def.config_name)
                .is_some_and(|entry| entry.fetch_models);
            let try_fetch = def.supports_model_discovery || user_opted_in;
            let static_catalog =
                || -> Vec<DiscoveredModel> { catalog_to_discovered(def.models, 2) };
            let discovered =
                if !skip_discovery && try_fetch && should_fetch_models(config, def.config_name) {
                    // Use pre-fetched results from parallel discovery.
                    match prefetched.get(def.config_name) {
                        Some(models) => models.clone(),
                        None => static_catalog(),
                    }
                } else if !def.supports_model_discovery && !def.models.is_empty() {
                    // Provider has no /models endpoint — use the static catalog.
                    static_catalog()
                } else {
                    Vec::new()
                };
            let models = merge_preferred_and_discovered_models(preferred, discovered);

            // Resolve per-provider tool_mode from config (defaults to Auto).
            let config_tool_mode = config
                .get(def.config_name)
                .map(|e| e.tool_mode)
                .unwrap_or_default();

            // For Ollama, probe each model's family info to decide native vs text
            // tool calling. For non-Ollama, just pass through the config tool mode.
            let is_ollama = def.config_name == "ollama";

            // Batch-probe Ollama models for family metadata (best-effort, 3s timeout).
            let ollama_probes: HashMap<String, OllamaShowResponse> = if is_ollama {
                probe_ollama_models_batch(&base_url, &models)
            } else {
                HashMap::new()
            };

            for model in models {
                let caps = model
                    .capabilities
                    .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
                let (model_id, display_name, created_at, recommended) = (
                    model.id,
                    model.display_name,
                    model.created_at,
                    model.recommended,
                );
                if self.has_provider_model(&provider_label, &model_id) {
                    continue;
                }

                // Determine effective tool mode for this model.
                let effective_tool_mode = if is_ollama {
                    resolve_ollama_tool_mode(
                        config_tool_mode,
                        &model_id,
                        ollama_probes.get(&model_id),
                    )
                } else if !matches!(config_tool_mode, moltis_config::ToolMode::Auto) {
                    config_tool_mode
                } else {
                    // Non-Ollama providers: let OpenAiProvider use its default logic.
                    moltis_config::ToolMode::Auto
                };

                let mut oai = openai::OpenAiProvider::new_with_name(
                    key.clone(),
                    model_id.clone(),
                    base_url.clone(),
                    provider_label.clone(),
                )
                .with_stream_transport(stream_transport)
                .with_cache_retention(cache_retention);

                if !matches!(effective_tool_mode, moltis_config::ToolMode::Auto) {
                    oai = oai.with_tool_mode(effective_tool_mode);
                }
                if let Some(strict) = config.get(def.config_name).and_then(|e| e.strict_tools) {
                    oai = oai.with_strict_tools(strict);
                }

                let provider = Arc::new(oai);
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: provider_label.clone(),
                        display_name,
                        created_at,
                        recommended,
                        capabilities: caps,
                    },
                    provider,
                );
            }
        }
    }

    /// Register custom OpenAI-compatible providers (names starting with `custom-`).
    /// These are user-added endpoints that may support model discovery via `/v1/models`.
    pub(crate) fn register_custom_providers(
        &mut self,
        config: &ProvidersConfig,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) {
        for (name, entry) in &config.providers {
            if !name.starts_with("custom-") || !entry.enabled {
                continue;
            }

            let Some(api_key) = entry
                .api_key
                .as_ref()
                .filter(|k| !k.expose_secret().is_empty())
            else {
                continue;
            };

            let Some(base_url) = entry.base_url.as_ref().filter(|u| !u.trim().is_empty()) else {
                continue;
            };

            let preferred = configured_models_for_provider(config, name);

            // Use pre-fetched results from parallel discovery.
            let discovered = if should_fetch_models(config, name) {
                match prefetched.get(name.as_str()) {
                    Some(models) => models.clone(),
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };

            let models = merge_preferred_and_discovered_models(preferred, discovered);
            if models.is_empty() {
                tracing::debug!(
                    provider = %name,
                    "custom provider has no models — skipping registration"
                );
                continue;
            }

            let custom_tool_mode = entry.tool_mode;
            for model in models {
                let caps = model
                    .capabilities
                    .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
                let (model_id, display_name, created_at, recommended) = (
                    model.id,
                    model.display_name,
                    model.created_at,
                    model.recommended,
                );
                if self.has_provider_model(name, &model_id) {
                    continue;
                }
                let mut oai = openai::OpenAiProvider::new_with_name(
                    api_key.clone(),
                    model_id.clone(),
                    base_url.clone(),
                    name.clone(),
                )
                .with_stream_transport(entry.stream_transport)
                .with_cache_retention(entry.cache_retention);
                if !matches!(entry.wire_api, moltis_config::WireApi::ChatCompletions) {
                    oai = oai.with_wire_api(entry.wire_api);
                }
                if !matches!(custom_tool_mode, moltis_config::ToolMode::Auto) {
                    oai = oai.with_tool_mode(custom_tool_mode);
                }
                if let Some(strict) = entry.strict_tools {
                    oai = oai.with_strict_tools(strict);
                }
                let provider = Arc::new(oai);
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: name.clone(),
                        display_name,
                        created_at,
                        recommended,
                        capabilities: caps,
                    },
                    provider,
                );
            }

            tracing::info!(
                provider = %name,
                "registered custom OpenAI-compatible provider"
            );
        }
    }

    pub fn get(&self, model_id: &str) -> Option<Arc<dyn LlmProvider>> {
        let (base_id, reasoning) = split_reasoning_suffix(model_id);
        let provider = self
            .resolve_registry_model_id(base_id, None)
            .as_deref()
            .and_then(|id| self.providers.get(id))
            .cloned()?;
        if let Some(effort) = reasoning {
            let new_provider = Arc::clone(&provider).with_reasoning_effort(effort);
            if new_provider.is_none() {
                tracing::warn!(
                    model_id,
                    ?effort,
                    "provider does not support reasoning effort; ignoring suffix"
                );
            }
            Some(new_provider.unwrap_or(provider))
        } else {
            Some(provider)
        }
    }

    pub fn first(&self) -> Option<Arc<dyn LlmProvider>> {
        self.models
            .iter()
            .enumerate()
            .min_by_key(|(idx, m)| (subscription_preference_rank(&m.provider), *idx))
            .map(|(_, m)| m)
            .and_then(|m| self.providers.get(&m.id))
            .cloned()
    }

    /// Return the first provider that supports tool calling,
    /// falling back to the first provider overall.
    pub fn first_with_tools(&self) -> Option<Arc<dyn LlmProvider>> {
        self.models
            .iter()
            .enumerate()
            .filter_map(|(idx, m)| self.providers.get(&m.id).map(|p| (idx, m, p)))
            .filter(|(_, _, p)| p.supports_tools())
            .min_by_key(|(idx, m, _)| (subscription_preference_rank(&m.provider), *idx))
            .map(|(_, _, p)| Arc::clone(p))
            .or_else(|| self.first())
    }

    pub fn list_models(&self) -> &[ModelInfo] {
        &self.models
    }

    /// Return the base model list plus reasoning-effort variants for supported models.
    ///
    /// For each model that supports extended thinking, three additional entries
    /// are appended: `<id>@reasoning-low`, `<id>@reasoning-medium`, `<id>@reasoning-high`.
    /// These virtual IDs are resolved by `get()` back to the base provider with
    /// the corresponding reasoning effort applied.
    #[must_use]
    pub fn list_models_with_reasoning_variants(&self) -> Vec<ModelInfo> {
        let mut result = Vec::with_capacity(self.models.len() * 4);
        for m in &self.models {
            result.push(m.clone());
            if m.capabilities.reasoning {
                for &(suffix, _) in REASONING_SUFFIXES {
                    let label = suffix.strip_prefix("reasoning-").unwrap_or(suffix);
                    result.push(ModelInfo {
                        id: format!("{}{REASONING_SUFFIX_SEP}{suffix}", m.id),
                        provider: m.provider.clone(),
                        display_name: format!("{} ({label} reasoning)", m.display_name),
                        created_at: m.created_at,
                        recommended: false,
                        capabilities: ModelCapabilities {
                            reasoning: true,
                            ..m.capabilities
                        },
                    });
                }
            }
        }
        result
    }

    /// Return all registered providers in registration order.
    pub fn all_providers(&self) -> Vec<Arc<dyn LlmProvider>> {
        self.models
            .iter()
            .filter_map(|m| self.providers.get(&m.id).cloned())
            .collect()
    }

    /// Return providers for the given model IDs (in order), skipping unknown IDs.
    pub fn providers_for_models(&self, model_ids: &[String]) -> Vec<Arc<dyn LlmProvider>> {
        model_ids
            .iter()
            .filter_map(|id| {
                self.resolve_registry_model_id(id, None)
                    .as_deref()
                    .and_then(|rid| self.providers.get(rid))
                    .cloned()
            })
            .collect()
    }

    /// Return fallback providers ordered by affinity to the given primary:
    ///
    /// 1. Same model ID on a different provider backend (e.g. `gpt-4o` via openrouter)
    /// 2. Subscription providers (`openai-codex`, `github-copilot`)
    /// 3. Other models from the same provider (e.g. `claude-opus-4` when primary is `claude-sonnet-4`)
    /// 4. Models from other providers
    ///
    /// The primary itself is excluded from the result.
    pub fn fallback_providers_for(
        &self,
        primary_model_id: &str,
        primary_provider_name: &str,
    ) -> Vec<Arc<dyn LlmProvider>> {
        let primary_raw_model_id = raw_model_id(primary_model_id);
        let mut ranked: Vec<(u8, usize, usize, Arc<dyn LlmProvider>)> = Vec::new();

        for (idx, info) in self.models.iter().enumerate() {
            if info.id == primary_model_id && info.provider == primary_provider_name {
                continue; // skip the primary itself
            }
            let Some(p) = self.providers.get(&info.id).cloned() else {
                continue;
            };
            let provider_rank = subscription_preference_rank(&info.provider);
            let bucket = if raw_model_id(&info.id) == primary_raw_model_id {
                0
            } else if provider_rank == 0 {
                1
            } else if info.provider == primary_provider_name {
                2
            } else {
                3
            };
            ranked.push((bucket, provider_rank, idx, p));
        }

        ranked.sort_by_key(|(bucket, provider_rank, idx, _)| (*bucket, *provider_rank, *idx));
        ranked.into_iter().map(|(_, _, _, p)| p).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn provider_summary(&self) -> String {
        if self.providers.is_empty() {
            return "no LLM providers configured".into();
        }
        let provider_count = self
            .models
            .iter()
            .map(|m| m.provider.as_str())
            .collect::<HashSet<_>>()
            .len();
        let model_count = self.models.len();
        format!(
            "{} provider{}, {} model{}",
            provider_count,
            if provider_count == 1 {
                ""
            } else {
                "s"
            },
            model_count,
            if model_count == 1 {
                ""
            } else {
                "s"
            },
        )
    }
}
