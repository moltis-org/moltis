//! NEAR AI Cloud model discovery helpers.

use {
    crate::{DiscoveredModel, ModelCapabilities},
    reqwest::StatusCode,
    serde::Deserialize,
    std::{collections::HashSet, sync::mpsc, time::Duration},
    thiserror::Error,
};

const MODEL_LIST_PATH: &str = "/model/list";

#[derive(Debug, Error)]
pub enum Error {
    #[error("NEAR AI model list request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("NEAR AI model list API at {endpoint} returned HTTP {status}")]
    HttpStatus {
        endpoint: String,
        status: StatusCode,
    },
    #[error("failed to parse NEAR AI model list JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("NEAR AI model list API returned no chat models")]
    NoChatModels,
    #[error("failed to create NEAR AI model discovery runtime: {0}")]
    Runtime(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct NearAiModelList {
    #[serde(default)]
    models: Vec<NearAiModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NearAiModel {
    model_id: String,
    #[serde(default)]
    metadata: NearAiModelMetadata,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NearAiModelMetadata {
    #[serde(default)]
    verifiable: bool,
    #[serde(default)]
    model_display_name: Option<String>,
    #[serde(default)]
    attestation_supported: bool,
    #[serde(default)]
    architecture: Option<NearAiArchitecture>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NearAiArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
}

fn normalize_base_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn has_version_suffix(base_url: &str) -> bool {
    let Some(last_segment) = base_url.rsplit('/').next() else {
        return false;
    };
    let Some(rest) = last_segment.strip_prefix('v') else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn model_list_endpoint(base_url: &str) -> String {
    let normalized = normalize_base_url(base_url);
    if normalized.ends_with(MODEL_LIST_PATH) {
        return normalized;
    }
    if normalized.ends_with("/v1") || has_version_suffix(&normalized) {
        return format!("{normalized}{MODEL_LIST_PATH}");
    }
    format!("{normalized}/v1{MODEL_LIST_PATH}")
}

fn has_modality(modalities: &[String], expected: &str) -> bool {
    modalities
        .iter()
        .any(|value| value.eq_ignore_ascii_case(expected))
}

fn is_tee_model(metadata: &NearAiModelMetadata) -> bool {
    metadata.verifiable || metadata.attestation_supported
}

fn is_known_non_chat_model(model_id: &str) -> bool {
    let lower_id = model_id.to_ascii_lowercase();
    lower_id == "openai/privacy-filter" || lower_id.contains("reranker")
}

fn catalog_capabilities_for(model_id: &str, metadata: &NearAiModelMetadata) -> ModelCapabilities {
    let mut capabilities = ModelCapabilities::infer(model_id);
    capabilities.tools = false;
    capabilities.reasoning = false;

    if !capabilities.text_generation {
        return ModelCapabilities {
            text_generation: false,
            vision: false,
            ..capabilities
        };
    }

    let Some(architecture) = metadata.architecture.as_ref() else {
        return capabilities;
    };

    capabilities.vision = has_modality(&architecture.input_modalities, "image");
    if architecture.input_modalities.is_empty() && architecture.output_modalities.is_empty() {
        capabilities.text_generation = false;
        return capabilities;
    }
    capabilities.text_generation = !has_modality(&architecture.input_modalities, "audio")
        && (architecture.output_modalities.is_empty()
            || has_modality(&architecture.output_modalities, "text"));

    capabilities
}

fn is_text_generation_model(model_id: &str, metadata: &NearAiModelMetadata) -> bool {
    !is_known_non_chat_model(model_id)
        && catalog_capabilities_for(model_id, metadata).text_generation
}

fn display_name_for(model: &NearAiModel) -> String {
    model
        .metadata
        .model_display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&model.model_id)
        .to_string()
}

fn parse_models_payload(payload: &str) -> Result<Vec<DiscoveredModel>, Error> {
    let parsed: NearAiModelList = serde_json::from_str(payload)?;
    let mut seen = HashSet::new();
    let mut models: Vec<DiscoveredModel> = parsed
        .models
        .into_iter()
        .filter(|model| is_text_generation_model(&model.model_id, &model.metadata))
        .filter(|model| seen.insert(model.model_id.clone()))
        .map(|model| {
            let recommended = is_tee_model(&model.metadata);
            DiscoveredModel::new(model.model_id.clone(), display_name_for(&model))
                .with_recommended(recommended)
                .with_capabilities(catalog_capabilities_for(&model.model_id, &model.metadata))
        })
        .collect();

    models.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });

    Ok(models)
}

/// Fetch available chat models from NEAR AI Cloud's public model catalog.
pub async fn fetch_models_from_api(base_url: String) -> Result<Vec<DiscoveredModel>, Error> {
    let client = crate::shared_http_client();
    let endpoint = model_list_endpoint(&base_url);
    let response = client
        .get(&endpoint)
        .timeout(Duration::from_secs(15))
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(Error::HttpStatus { endpoint, status });
    }
    let models = parse_models_payload(&body)?;
    if models.is_empty() {
        return Err(Error::NoChatModels);
    }
    Ok(models)
}

/// Spawn NEAR AI model discovery in a background thread and return immediately.
pub fn start_model_discovery(
    base_url: String,
) -> mpsc::Receiver<Result<Vec<DiscoveredModel>, Error>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(fetch_models_from_api(base_url))
        })();
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn nearai_endpoint_uses_model_list_path() {
        assert_eq!(
            model_list_endpoint("https://cloud-api.near.ai/v1"),
            "https://cloud-api.near.ai/v1/model/list"
        );
        assert_eq!(
            model_list_endpoint("https://cloud-api.near.ai"),
            "https://cloud-api.near.ai/v1/model/list"
        );
    }

    #[test]
    fn parses_and_filters_nearai_catalog() {
        let payload = r#"{
            "models": [
                {
                    "modelId": "zai-org/GLM-5.1-FP8",
                    "metadata": {
                        "verifiable": true,
                        "attestationSupported": true,
                        "modelDisplayName": "GLM 5.1",
                        "providerType": "vllm",
                        "architecture": {
                            "inputModalities": ["text"],
                            "outputModalities": ["text"]
                        }
                    }
                },
                {
                    "modelId": "Qwen/Qwen3-VL-30B-A3B-Instruct",
                    "metadata": {
                        "modelDisplayName": "Qwen3 VL",
                        "architecture": {
                            "inputModalities": ["text", "image"],
                            "outputModalities": ["text"]
                        }
                    }
                },
                {
                    "modelId": "Qwen/Qwen3-235B-A22B-Instruct-2507",
                    "metadata": {
                        "modelDisplayName": "Qwen3 vLLM",
                        "providerType": "vllm",
                        "architecture": {
                            "inputModalities": ["text"],
                            "outputModalities": ["text"]
                        }
                    }
                },
                {
                    "modelId": "Qwen/Qwen3-Embedding-0.6B",
                    "metadata": {
                        "architecture": {
                            "inputModalities": ["text"],
                            "outputModalities": ["embedding"]
                        }
                    }
                },
                {
                    "modelId": "openai/privacy-filter",
                    "metadata": {
                        "architecture": {
                            "inputModalities": ["text"],
                            "outputModalities": ["text"]
                        }
                    }
                }
            ]
        }"#;

        let models = parse_models_payload(payload).expect("payload parses");
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec![
            "zai-org/GLM-5.1-FP8",
            "Qwen/Qwen3-VL-30B-A3B-Instruct",
            "Qwen/Qwen3-235B-A22B-Instruct-2507"
        ]);
        assert!(models[0].recommended);
        let vllm_model = models
            .iter()
            .find(|model| model.id == "Qwen/Qwen3-235B-A22B-Instruct-2507")
            .expect("vLLM-hosted model is included");
        assert!(!vllm_model.recommended);
        assert!(
            models
                .iter()
                .all(|model| !model.capabilities.expect("capabilities").tools)
        );
        assert!(!models[0].capabilities.expect("capabilities").reasoning);
        assert!(models[1].capabilities.expect("capabilities").vision);
    }
}
