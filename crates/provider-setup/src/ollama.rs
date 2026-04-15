//! Ollama-specific URL normalization, model discovery, and model matching.

use serde_json::Value;

pub(crate) const OLLAMA_DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub(crate) fn normalize_ollama_openai_base_url(base_url: Option<&str>) -> String {
    let base = base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(OLLAMA_DEFAULT_BASE_URL);
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

pub(crate) fn normalize_ollama_api_base_url(base_url: Option<&str>) -> String {
    let openai_base = normalize_ollama_openai_base_url(base_url);
    openai_base
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or(openai_base.as_str())
        .to_string()
}

pub(crate) fn normalize_ollama_model_id(model: &str) -> &str {
    model.strip_prefix("ollama::").unwrap_or(model)
}

pub(crate) fn ollama_model_matches(installed_model: &str, requested_model: &str) -> bool {
    installed_model == requested_model
        || installed_model.starts_with(&format!("{requested_model}:"))
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsModel {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagsModel>,
}

pub(crate) async fn discover_ollama_models(base_url: &str) -> crate::error::Result<Vec<String>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|source| {
            crate::error::Error::external("failed to query Ollama model discovery endpoint", source)
        })?;

    if !response.status().is_success() {
        return Err(crate::error::Error::message(format!(
            "Ollama model discovery failed at {url} (HTTP {}).",
            response.status(),
        )));
    }

    let payload: OllamaTagsResponse = response.json().await.map_err(|source| {
        crate::error::Error::external("invalid JSON from Ollama model discovery endpoint", source)
    })?;

    let mut models: Vec<String> = payload
        .models
        .into_iter()
        .map(|m| m.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

pub(crate) fn ollama_models_payload(models: &[String]) -> Vec<Value> {
    models
        .iter()
        .map(|model| {
            serde_json::json!({
                "id": format!("ollama::{model}"),
                "displayName": model,
                "provider": "ollama",
                "supportsTools": true,
            })
        })
        .collect()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ollama_openai_base_url_appends_v1() {
        assert_eq!(
            normalize_ollama_openai_base_url(Some("http://localhost:11434")),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_ollama_openai_base_url(Some("http://localhost:11434/v1")),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn normalize_ollama_api_base_url_strips_v1() {
        assert_eq!(
            normalize_ollama_api_base_url(Some("http://localhost:11434/v1")),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_api_base_url(Some("http://localhost:11434")),
            "http://localhost:11434"
        );
    }

    #[test]
    fn ollama_model_matches_accepts_tag_suffix() {
        assert!(ollama_model_matches("llama3.2:latest", "llama3.2"));
        assert!(ollama_model_matches("qwen2.5:7b", "qwen2.5:7b"));
        assert!(!ollama_model_matches("llama3.2:latest", "qwen2.5"));
    }
}
