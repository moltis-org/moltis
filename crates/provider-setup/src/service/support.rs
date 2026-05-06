use {
    serde_json::{Map, Value},
    tracing::info,
};

pub(crate) fn progress_payload(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

pub(crate) struct ProviderSetupTiming {
    operation: &'static str,
    provider: String,
    started: std::time::Instant,
}

impl ProviderSetupTiming {
    pub(crate) fn start(operation: &'static str, provider: Option<&str>) -> Self {
        let provider_name = provider.unwrap_or("<missing>").to_string();
        info!(
            operation,
            provider = %provider_name,
            "provider setup operation started"
        );
        Self {
            operation,
            provider: provider_name,
            started: std::time::Instant::now(),
        }
    }
}

impl Drop for ProviderSetupTiming {
    fn drop(&mut self) {
        info!(
            operation = self.operation,
            provider = %self.provider,
            elapsed_ms = self.started.elapsed().as_millis(),
            "provider setup operation finished"
        );
    }
}

pub type ErrorParser = fn(&str, Option<&str>) -> Value;

pub(crate) fn default_error_parser(raw: &str, _provider: Option<&str>) -> Value {
    serde_json::json!({ "type": "unknown", "detail": raw })
}

#[derive(Clone)]
pub(crate) struct PendingOAuthFlow {
    pub(crate) provider_name: String,
    pub(crate) oauth_config: moltis_oauth::OAuthConfig,
    pub(crate) verifier: String,
}
