//! Credential management — `save_key`, `remove_key`, `save_model`,
//! `save_models` implementations.

use {
    serde_json::Value,
    tracing::{info, warn},
};

use moltis_service_traits::{ServiceError, ServiceResult};

use {
    super::{LiveProviderSetupService, support::ProviderSetupTiming},
    crate::{
        config_helpers::set_provider_enabled_in_config,
        custom_providers::is_custom_provider,
        key_store::parse_models_param,
        known_providers::{AuthType, known_providers},
        ollama::normalize_ollama_openai_base_url,
    },
};

impl LiveProviderSetupService {
    pub(super) async fn save_key_inner(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start(
            "providers.save_key",
            params.get("provider").and_then(Value::as_str),
        );
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        // API key is optional for some providers (e.g., Ollama)
        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let models = parse_models_param(&params);

        // Custom providers bypass known_providers() validation.
        let is_custom = is_custom_provider(provider_name);
        if !is_custom {
            // Validate provider name - allow both api-key and local providers
            let known = known_providers();
            let provider = known
                .iter()
                .find(|p| {
                    p.name == provider_name
                        && (p.auth_type == AuthType::ApiKey || p.auth_type == AuthType::Local)
                })
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

            // API key is required for api-key providers unless the provider
            // marks the key as optional (Ollama, LM Studio).
            if provider.auth_type == AuthType::ApiKey && !provider.key_optional && api_key.is_none()
            {
                return Err("missing 'apiKey' parameter".into());
            }
        } else if api_key.is_none() {
            return Err("missing 'apiKey' parameter".into());
        }

        let normalized_base_url = if provider_name == "ollama" {
            base_url.map(|url| normalize_ollama_openai_base_url(Some(url)))
        } else {
            base_url.map(String::from)
        };

        let key_store_path = self.key_store.path();
        info!(
            provider = provider_name,
            has_api_key = api_key.is_some(),
            has_base_url = normalized_base_url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty()),
            models = models.len(),
            key_store_path = %key_store_path.display(),
            "saving provider config"
        );

        // Persist full config to disk
        if let Err(error) = self.key_store.save_config(
            provider_name,
            api_key.map(String::from),
            normalized_base_url,
            (!models.is_empty()).then_some(models),
        ) {
            warn!(
                provider = provider_name,
                key_store_path = %key_store_path.display(),
                error = %error,
                "failed to persist provider config"
            );
            return Err(ServiceError::message(error));
        }
        set_provider_enabled_in_config(provider_name, true)?;
        self.set_provider_enabled_in_memory(provider_name, true);

        // Rebuild the provider registry with saved keys merged into config.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = provider_name,
            provider_summary = %provider_summary,
            models = model_count,
            "saved provider config to disk and rebuilt provider registry"
        );

        Ok(serde_json::json!({ "ok": true }))
    }

    pub(super) async fn remove_key_inner(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        if is_custom_provider(provider_name) {
            // Custom provider: remove key store entry + disable.
            self.key_store
                .remove(provider_name)
                .map_err(ServiceError::message)?;
            set_provider_enabled_in_config(provider_name, false)?;
            self.set_provider_enabled_in_memory(provider_name, false);
        } else {
            let providers = known_providers();
            let known = providers
                .iter()
                .find(|p| p.name == provider_name)
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

            // Remove persisted API key
            if known.auth_type == AuthType::ApiKey {
                self.key_store
                    .remove(provider_name)
                    .map_err(ServiceError::message)?;
            }

            // Remove OAuth tokens
            if known.auth_type == AuthType::Oauth || provider_name == "kimi-code" {
                let _ = self.token_store.delete(provider_name);
            }

            // Persist explicit disable so auto-detected/global credentials do not
            // immediately re-enable the provider on next rebuild.
            set_provider_enabled_in_config(provider_name, false)?;
            self.set_provider_enabled_in_memory(provider_name, false);

            // Remove local-llm config
            #[cfg(feature = "local-llm")]
            if known.auth_type == AuthType::Local
                && provider_name == "local-llm"
                && let Some(config_dir) = moltis_config::config_dir()
            {
                let config_path = config_dir.join("local-llm.json");
                let _ = std::fs::remove_file(config_path);
            }
        }

        // Rebuild the provider registry without the removed provider.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = provider_name,
            "removed provider credentials and rebuilt registry"
        );

        Ok(serde_json::json!({ "ok": true }))
    }

    pub(super) async fn save_model_inner(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'model' parameter".to_string())?;

        // Validate provider exists (known or custom).
        if !is_custom_provider(provider_name) {
            let known = known_providers();
            if !known.iter().any(|p| p.name == provider_name) {
                return Err(format!("unknown provider: {provider_name}").into());
            }
        }

        // Prepend chosen model to existing saved models so it appears first.
        let mut models = vec![model.to_string()];
        if let Some(existing) = self.key_store.load_config(provider_name) {
            models.extend(existing.models);
        }

        self.key_store
            .save_config(provider_name, None, None, Some(models))
            .map_err(ServiceError::message)?;

        // Update the cross-provider priority list.
        if let Some(ref priority) = self.priority_models {
            let mut list = priority.write().await;
            let normalized = model.to_string();
            list.retain(|m| m != &normalized);
            list.insert(0, normalized);
        }

        info!(
            provider = provider_name,
            model, "saved model preference and queued async registry rebuild"
        );
        self.queue_registry_rebuild(provider_name, "save_model");
        Ok(serde_json::json!({ "ok": true }))
    }

    pub(super) async fn save_models_inner(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start(
            "providers.save_models",
            params.get("provider").and_then(Value::as_str),
        );
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let models: Vec<String> = params
            .get("models")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "missing 'models' array parameter".to_string())?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        // Validate provider exists (known or custom).
        if !is_custom_provider(provider_name) {
            let known = known_providers();
            if !known.iter().any(|p| p.name == provider_name) {
                return Err(format!("unknown provider: {provider_name}").into());
            }
        }

        self.key_store
            .save_config(provider_name, None, None, Some(models.clone()))
            .map_err(ServiceError::message)?;

        // Update the cross-provider priority list.
        if let Some(ref priority) = self.priority_models {
            let mut list = priority.write().await;
            for m in models.iter().rev() {
                list.retain(|existing| existing != m);
                list.insert(0, m.clone());
            }
        }

        info!(
            provider = provider_name,
            count = models.len(),
            models = ?models,
            "saved model preferences and queued async registry rebuild"
        );
        self.queue_registry_rebuild(provider_name, "save_models");
        Ok(serde_json::json!({ "ok": true }))
    }
}
