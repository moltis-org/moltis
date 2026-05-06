//! Key validation — `validate_key` implementation.

use {secrecy::Secret, serde_json::Value, tracing::info};

use moltis_service_traits::ServiceResult;

use {
    super::{
        LiveProviderSetupService,
        support::{ProviderSetupTiming, progress_payload},
    },
    crate::{
        config_helpers::normalize_provider_name,
        custom_providers::{is_custom_provider, validation_provider_name_for_endpoint},
        key_store::parse_models_param,
        known_providers::{AuthType, KnownProvider, known_providers},
        ollama::{
            discover_ollama_models, normalize_ollama_api_base_url, normalize_ollama_model_id,
            normalize_ollama_openai_base_url, ollama_model_matches, ollama_models_payload,
        },
    },
};

impl LiveProviderSetupService {
    pub(super) async fn validate_key_inner(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let preferred_models = parse_models_param(&params);
        let request_id = params
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string);
        let saved_config = self.key_store.load_config(provider_name);
        let saved_base_url = saved_config
            .as_ref()
            .and_then(|config| config.base_url.as_deref())
            .filter(|url| !url.trim().is_empty());
        let effective_base_url = base_url
            .filter(|url| !url.trim().is_empty())
            .or(saved_base_url);

        // Custom providers bypass known_providers() validation.
        let is_custom = is_custom_provider(provider_name);
        let provider_info = if is_custom {
            None
        } else {
            let known = known_providers();
            let info = known
                .iter()
                .find(|p| p.name == provider_name)
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;
            // API key is required for api-key providers unless the provider
            // marks the key as optional (Ollama, LM Studio).
            if info.auth_type == AuthType::ApiKey && !info.key_optional && api_key.is_none() {
                return Err("missing 'apiKey' parameter".into());
            }
            Some(KnownProvider {
                name: info.name,
                display_name: info.display_name,
                auth_type: info.auth_type,
                env_key: info.env_key,
                default_base_url: info.default_base_url,
                requires_model: info.requires_model,
                key_optional: info.key_optional,
                local_only: info.local_only,
            })
        };

        if is_custom && api_key.is_none() {
            return Err("missing 'apiKey' parameter".into());
        }
        if is_custom && effective_base_url.is_none() {
            return Err("missing 'baseUrl' parameter".into());
        }

        let selected_model = preferred_models.first().map(String::as_str);
        let validation_provider_name = validation_provider_name_for_endpoint(
            provider_name,
            provider_info.as_ref().and_then(|p| p.default_base_url),
            effective_base_url,
        );
        let _timing =
            ProviderSetupTiming::start("providers.validate_key", Some(&validation_provider_name));
        self.emit_validation_progress(
            &validation_provider_name,
            request_id.as_deref(),
            "start",
            progress_payload(serde_json::json!({
                "message": "Starting provider validation.",
            })),
        )
        .await;

        // Ollama supports native model discovery through /api/tags.
        if provider_name == "ollama" {
            return self
                .validate_ollama(
                    &validation_provider_name,
                    request_id.as_deref(),
                    effective_base_url.or(provider_info.as_ref().and_then(|p| p.default_base_url)),
                    selected_model,
                )
                .await;
        }

        // Custom OpenAI-compatible providers: discover models via /v1/models
        // when no model is specified.
        if is_custom && selected_model.is_none() {
            return self
                .validate_custom_discover(
                    provider_name,
                    &validation_provider_name,
                    request_id.as_deref(),
                    api_key.unwrap_or_default(),
                    effective_base_url.unwrap_or_default(),
                )
                .await;
        }

        let normalized_base_url = if provider_name == "ollama" {
            effective_base_url.map(|url| normalize_ollama_openai_base_url(Some(url)))
        } else {
            effective_base_url.map(String::from)
        };

        // Build a temporary ProvidersConfig with just this provider.
        let mut temp_config = moltis_config::schema::ProvidersConfig::default();
        temp_config.providers.insert(
            validation_provider_name.clone(),
            moltis_config::schema::ProviderEntry {
                enabled: true,
                api_key: api_key.map(|k| Secret::new(k.to_string())),
                base_url: normalized_base_url,
                models: preferred_models,
                ..Default::default()
            },
        );

        // Build a temporary registry from the temp config.
        let temp_registry = self.build_registry(&temp_config);

        // Filter models for this provider.
        let models: Vec<_> = temp_registry
            .list_models()
            .iter()
            .filter(|m| {
                normalize_provider_name(&m.provider)
                    == normalize_provider_name(&validation_provider_name)
            })
            .cloned()
            .collect();

        if models.is_empty() {
            let error =
                "No models available for this provider. Check your credentials and try again.";
            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "error",
                progress_payload(serde_json::json!({
                    "message": error,
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": false,
                "error": error,
            }));
        }

        info!(
            provider = %validation_provider_name,
            model_count = models.len(),
            "provider validation discovered candidate models"
        );

        let model_list: Vec<Value> = models
            .iter()
            .filter(|m| moltis_providers::is_chat_capable_model(&m.id))
            .map(|m| {
                let supports_tools = temp_registry.get(&m.id).is_some_and(|p| p.supports_tools());
                serde_json::json!({
                    "id": m.id,
                    "displayName": m.display_name,
                    "provider": m.provider,
                    "supportsTools": supports_tools,
                })
            })
            .collect();

        self.emit_validation_progress(
            &validation_provider_name,
            request_id.as_deref(),
            "complete",
            progress_payload(serde_json::json!({
                "message": "Validation complete.",
                "modelCount": model_list.len(),
            })),
        )
        .await;
        Ok(serde_json::json!({
            "valid": true,
            "models": model_list,
        }))
    }

    /// Validate Ollama provider using native model discovery.
    async fn validate_ollama(
        &self,
        validation_provider_name: &str,
        request_id: Option<&str>,
        base_url: Option<&str>,
        selected_model: Option<&str>,
    ) -> ServiceResult {
        let ollama_api_base = normalize_ollama_api_base_url(base_url);
        let discovered_models = match discover_ollama_models(&ollama_api_base).await {
            Ok(models) => models,
            Err(error) => {
                let error = error.to_string();
                self.emit_validation_progress(
                    validation_provider_name,
                    request_id,
                    "error",
                    progress_payload(serde_json::json!({
                        "message": error.clone(),
                    })),
                )
                .await;
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": error,
                }));
            },
        };

        if discovered_models.is_empty() {
            let error = "No Ollama models found. Install one first with `ollama pull <model>`.";
            self.emit_validation_progress(
                validation_provider_name,
                request_id,
                "error",
                progress_payload(serde_json::json!({
                    "message": error,
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": false,
                "error": error,
            }));
        }

        let models_payload = if let Some(requested_model) = selected_model {
            let requested_model = normalize_ollama_model_id(requested_model.trim());
            let installed = discovered_models
                .iter()
                .any(|installed_model| ollama_model_matches(installed_model, requested_model));
            if !installed {
                let error = format!(
                    "Model '{requested_model}' is not installed in Ollama. Install it with `ollama pull {requested_model}`."
                );
                self.emit_validation_progress(
                    validation_provider_name,
                    request_id,
                    "error",
                    progress_payload(serde_json::json!({
                        "message": error.clone(),
                    })),
                )
                .await;
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": error,
                }));
            }
            discovered_models
                .iter()
                .map(|installed_model| {
                    let response_model = if ollama_model_matches(installed_model, requested_model) {
                        requested_model
                    } else {
                        installed_model.as_str()
                    };
                    serde_json::json!({
                        "id": format!("ollama::{response_model}"),
                        "displayName": response_model,
                        "provider": "ollama",
                        "supportsTools": true,
                    })
                })
                .collect()
        } else {
            ollama_models_payload(&discovered_models)
        };

        self.emit_validation_progress(
            validation_provider_name,
            request_id,
            "complete",
            progress_payload(serde_json::json!({
                "message": "Discovered installed Ollama models.",
                "modelCount": discovered_models.len(),
            })),
        )
        .await;
        Ok(serde_json::json!({
            "valid": true,
            "models": models_payload,
        }))
    }

    /// Discover models from a custom OpenAI-compatible endpoint.
    async fn validate_custom_discover(
        &self,
        provider_name: &str,
        validation_provider_name: &str,
        request_id: Option<&str>,
        api_key: &str,
        base_url: &str,
    ) -> ServiceResult {
        match moltis_providers::openai::fetch_models_from_api(
            Secret::new(api_key.to_string()),
            base_url.to_string(),
        )
        .await
        {
            Ok(discovered) => {
                let model_list: Vec<Value> = discovered
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": format!("{provider_name}::{}", m.id),
                            "displayName": &m.display_name,
                            "provider": provider_name,
                        })
                    })
                    .collect();
                self.emit_validation_progress(
                    validation_provider_name,
                    request_id,
                    "complete",
                    progress_payload(serde_json::json!({
                        "message": "Discovered models from endpoint.",
                        "modelCount": model_list.len(),
                    })),
                )
                .await;
                Ok(serde_json::json!({
                    "valid": true,
                    "models": model_list,
                }))
            },
            Err(err) => {
                let error = format!("Failed to discover models from endpoint: {err}");
                self.emit_validation_progress(
                    validation_provider_name,
                    request_id,
                    "error",
                    progress_payload(serde_json::json!({
                        "message": error.clone(),
                    })),
                )
                .await;
                Ok(serde_json::json!({
                    "valid": false,
                    "error": error,
                }))
            },
        }
    }
}
