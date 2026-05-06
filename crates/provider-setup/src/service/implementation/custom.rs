//! Custom provider management — `add_custom` implementation.

use {serde_json::Value, tracing::info};

use moltis_service_traits::{ServiceError, ServiceResult};

use {
    super::{LiveProviderSetupService, support::ProviderSetupTiming},
    crate::{
        config_helpers::set_provider_enabled_in_config,
        custom_providers::{
            base_url_to_display_name, derive_provider_name_from_url,
            existing_custom_provider_for_base_url, make_unique_provider_name,
        },
    },
};

impl LiveProviderSetupService {
    pub(super) async fn add_custom_inner(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start("providers.add_custom", None);

        let base_url = params
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "missing 'baseUrl' parameter".to_string())?;

        let api_key = params
            .get("apiKey")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "missing 'apiKey' parameter".to_string())?;

        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());

        let base_name = derive_provider_name_from_url(base_url)
            .ok_or_else(|| "could not parse endpoint URL".to_string())?;

        let existing = self.key_store.load_all_configs();
        let provider_name = existing_custom_provider_for_base_url(base_url, &existing)
            .unwrap_or_else(|| make_unique_provider_name(&base_name, &existing));
        let reused_existing_provider = existing.contains_key(&provider_name);
        let display_name = base_url_to_display_name(base_url);

        let models = model.map(|m| vec![m.to_string()]);

        self.key_store
            .save_config_with_display_name(
                &provider_name,
                Some(api_key.to_string()),
                Some(base_url.to_string()),
                models,
                Some(display_name.clone()),
            )
            .map_err(ServiceError::message)?;

        set_provider_enabled_in_config(&provider_name, true)?;
        self.set_provider_enabled_in_memory(&provider_name, true);

        // Rebuild synchronously so the just-added custom provider is immediately
        // available for model probing in the same UI flow.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = %provider_name,
            display_name = %display_name,
            reused = reused_existing_provider,
            provider_summary = %provider_summary,
            models = model_count,
            "saved custom OpenAI-compatible provider and rebuilt provider registry"
        );

        Ok(serde_json::json!({
            "ok": true,
            "providerName": provider_name,
            "displayName": display_name,
        }))
    }
}
