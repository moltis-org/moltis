//! Ollama-specific model discovery, probing, and tool mode resolution.

use std::collections::HashMap;

use crate::DiscoveredModel;

pub(crate) fn normalize_ollama_api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagEntry {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsPayload {
    #[serde(default)]
    models: Vec<OllamaTagEntry>,
}

pub(crate) async fn discover_ollama_models_from_api(
    base_url: String,
) -> anyhow::Result<Vec<DiscoveredModel>> {
    let api_base = normalize_ollama_api_base_url(&base_url);
    let endpoint = format!("{}/api/tags", api_base.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?
        .get(&endpoint)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("ollama model discovery failed HTTP {status}");
    }

    let payload: OllamaTagsPayload = response.json().await?;
    let mut models: Vec<DiscoveredModel> = payload
        .models
        .into_iter()
        .map(|entry| entry.name.trim().to_string())
        .filter(|model| !model.is_empty())
        .map(|model| DiscoveredModel::new(model.clone(), model))
        .collect();
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    Ok(models)
}

/// Spawn Ollama model discovery in a background thread and return the receiver
/// immediately, without blocking. Call `.recv()` later to collect the result.
pub(crate) fn start_ollama_discovery(
    base_url: &str,
) -> std::sync::mpsc::Receiver<anyhow::Result<Vec<DiscoveredModel>>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let base_url = base_url.to_string();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(discover_ollama_models_from_api(base_url)));
        let _ = tx.send(result);
    });
    rx
}

// ── Ollama model info probing ────────────────────────────────────────────────

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct OllamaShowResponse {
    #[serde(default)]
    pub(crate) details: OllamaModelDetails,
    /// Ollama >= 0.5.x returns a list of model capabilities (e.g. `["tools"]`).
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct OllamaModelDetails {
    family: Option<String>,
    #[serde(default)]
    families: Option<Vec<String>>,
}

/// Model families known to support native OpenAI-style tool calling in Ollama.
const OLLAMA_NATIVE_TOOL_FAMILIES: &[&str] = &[
    "llama3.1",
    "llama3.2",
    "llama3.3",
    "llama4",
    "qwen2.5",
    "qwen3",
    "mistral",
    "mixtral",
    "command-r",
    "firefunction",
    "hermes",
];

/// Determine whether an Ollama model supports native tool calling based on its
/// model name and family metadata from `/api/show`.
fn ollama_model_supports_native_tools(model_name: &str, details: &OllamaModelDetails) -> bool {
    let name_lower = model_name.to_ascii_lowercase();

    // Check all family strings from the model details.
    let families_iter = details
        .family
        .iter()
        .chain(details.families.iter().flatten());
    for family in families_iter {
        let fam_lower = family.to_ascii_lowercase();
        if OLLAMA_NATIVE_TOOL_FAMILIES
            .iter()
            .any(|known| fam_lower.contains(known))
        {
            return true;
        }
    }

    // Heuristic: check model name for known families.
    OLLAMA_NATIVE_TOOL_FAMILIES
        .iter()
        .any(|known| name_lower.contains(known))
}

/// Check if Ollama's `capabilities` list indicates native tool support.
///
/// Returns `Some(true)` if `"tools"` is present, `Some(false)` if capabilities
/// exist but don't include tools, and `None` if the list is empty (pre-0.5.x
/// Ollama versions that don't report capabilities).
fn ollama_capabilities_support_tools(capabilities: &[String]) -> Option<bool> {
    if capabilities.is_empty() {
        return None;
    }
    Some(capabilities.iter().any(|c| c == "tools"))
}

/// Probe the Ollama `/api/show` endpoint for a specific model to get its family
/// and details. Returns `Ok(response)` on success, error on timeout/failure.
async fn probe_ollama_model_info(
    base_url: &str,
    model_name: &str,
) -> anyhow::Result<OllamaShowResponse> {
    let api_base = normalize_ollama_api_base_url(base_url);
    let endpoint = format!("{}/api/show", api_base.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?
        .post(&endpoint)
        .json(&serde_json::json!({ "name": model_name }))
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("ollama /api/show for {model_name} failed HTTP {status}");
    }
    Ok(response.json().await?)
}

/// Resolve the effective tool mode for an Ollama model.
///
/// - If the user configured an explicit `tool_mode`, use that.
/// - Otherwise, check the model's `capabilities` list from Ollama (>= 0.5.x).
/// - Fall back to the hardcoded family whitelist only when capabilities are
///   unavailable (pre-0.5.x Ollama).
pub(crate) fn resolve_ollama_tool_mode(
    config_tool_mode: moltis_config::ToolMode,
    model_name: &str,
    probe_result: Option<&OllamaShowResponse>,
) -> moltis_config::ToolMode {
    use moltis_config::ToolMode;

    match config_tool_mode {
        ToolMode::Native | ToolMode::Text | ToolMode::Off => config_tool_mode,
        ToolMode::Auto => {
            // Prefer Ollama's own capabilities field when available.
            if let Some(resp) = probe_result
                && let Some(supports) = ollama_capabilities_support_tools(&resp.capabilities)
            {
                return if supports {
                    ToolMode::Native
                } else {
                    ToolMode::Text
                };
            }
            // Fallback: family whitelist (pre-0.5.x Ollama without capabilities).
            let details = probe_result
                .map(|r| &r.details)
                .cloned()
                .unwrap_or_default();
            if ollama_model_supports_native_tools(model_name, &details) {
                ToolMode::Native
            } else {
                ToolMode::Text
            }
        },
    }
}

/// Batch-probe Ollama `/api/show` for a list of models.
/// Runs probes in a dedicated thread with its own tokio runtime (same pattern
/// as `discover_ollama_models`). Returns a map from model ID to show response;
/// failures are silently dropped.
pub(crate) fn probe_ollama_models_batch(
    base_url: &str,
    models: &[DiscoveredModel],
) -> HashMap<String, OllamaShowResponse> {
    if models.is_empty() {
        return HashMap::new();
    }
    let base_url = base_url.to_string();
    let model_ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map(|rt| {
                rt.block_on(async {
                    let futs: Vec<_> = model_ids
                        .iter()
                        .map(|id| {
                            let base = base_url.clone();
                            let model_id = id.clone();
                            async move {
                                let resp = probe_ollama_model_info(&base, &model_id).await;
                                (model_id, resp)
                            }
                        })
                        .collect();
                    futures::future::join_all(futs).await
                })
            });
        let _ = tx.send(result);
    });

    match rx.recv() {
        Ok(Ok(results)) => results
            .into_iter()
            .filter_map(|(id, r)| r.ok().map(|resp| (id, resp)))
            .collect(),
        _ => HashMap::new(),
    }
}

/// Async variant of [`probe_ollama_models_batch`] that runs directly on the
/// current tokio runtime. Suitable for callers already in an async context
/// (e.g. runtime rediscovery in `detect_supported`).
pub(crate) async fn probe_ollama_models_batch_async(
    base_url: &str,
    models: &[DiscoveredModel],
) -> HashMap<String, OllamaShowResponse> {
    if models.is_empty() {
        return HashMap::new();
    }
    let futs: Vec<_> = models
        .iter()
        .map(|m| {
            let base = base_url.to_string();
            let model_id = m.id.clone();
            async move {
                let resp = probe_ollama_model_info(&base, &model_id).await;
                (model_id, resp)
            }
        })
        .collect();
    futures::future::join_all(futs)
        .await
        .into_iter()
        .filter_map(|(id, r)| r.ok().map(|resp| (id, resp)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn ollama_native_tools_known_families() {
        let details = OllamaModelDetails {
            family: Some("llama".into()),
            families: Some(vec!["llama3.1".into()]),
        };
        assert!(ollama_model_supports_native_tools("llama3.1:8b", &details));
    }

    #[test]
    fn ollama_native_tools_qwen_family() {
        let details = OllamaModelDetails {
            family: Some("qwen2.5".into()),
            families: None,
        };
        assert!(ollama_model_supports_native_tools("qwen2.5:7b", &details));
    }

    #[test]
    fn ollama_native_tools_unknown_family() {
        let details = OllamaModelDetails {
            family: Some("phi3".into()),
            families: None,
        };
        // "phi3" is not in the native tool families list, and model name
        // doesn't match either.
        assert!(!ollama_model_supports_native_tools("phi3:mini", &details));
    }

    #[test]
    fn ollama_native_tools_name_heuristic() {
        // Even without details, model name matching should work.
        let details = OllamaModelDetails::default();
        assert!(ollama_model_supports_native_tools(
            "llama3.3:70b-instruct",
            &details
        ));
        assert!(!ollama_model_supports_native_tools(
            "codellama:13b",
            &details
        ));
    }

    #[test]
    fn resolve_ollama_tool_mode_explicit_override() {
        use moltis_config::ToolMode;
        // Explicit modes are passed through regardless of probe result.
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Native, "anything", None),
            ToolMode::Native
        );
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Text, "anything", None),
            ToolMode::Text
        );
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Off, "anything", None),
            ToolMode::Off
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_auto_with_probe() {
        use moltis_config::ToolMode;
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("llama3.1".into()),
                families: None,
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "llama3.1:8b", Some(&show_resp)),
            ToolMode::Native
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_auto_unknown_model() {
        use moltis_config::ToolMode;
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("starcoder2".into()),
                families: None,
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "starcoder2:3b", Some(&show_resp)),
            ToolMode::Text
        );
    }

    // ── Ollama capabilities-based tool detection ──────────────────────

    #[test]
    fn ollama_capabilities_with_tools() {
        let caps = vec!["completion".into(), "tools".into()];
        assert_eq!(ollama_capabilities_support_tools(&caps), Some(true));
    }

    #[test]
    fn ollama_capabilities_without_tools() {
        let caps = vec!["completion".into(), "vision".into()];
        assert_eq!(ollama_capabilities_support_tools(&caps), Some(false));
    }

    #[test]
    fn ollama_capabilities_empty_returns_none() {
        let caps: Vec<String> = vec![];
        assert_eq!(ollama_capabilities_support_tools(&caps), None);
    }

    #[test]
    fn resolve_ollama_tool_mode_capabilities_override_family() {
        use moltis_config::ToolMode;
        // Model is NOT in the family whitelist but Ollama reports "tools" capability.
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("minimax".into()),
                families: None,
            },
            capabilities: vec!["completion".into(), "tools".into()],
        };
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "MiniMax-M2.5:latest", Some(&show_resp)),
            ToolMode::Native
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_capabilities_no_tools() {
        use moltis_config::ToolMode;
        // Model has capabilities but "tools" is not among them.
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("llama3.1".into()),
                families: None,
            },
            capabilities: vec!["completion".into()],
        };
        // Even though family matches, capabilities say no tools.
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "llama3.1:8b", Some(&show_resp)),
            ToolMode::Text
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_empty_capabilities_falls_back_to_family() {
        use moltis_config::ToolMode;
        // Empty capabilities (pre-0.5.x Ollama) — falls back to family whitelist.
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("llama3.1".into()),
                families: None,
            },
            capabilities: vec![],
        };
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "llama3.1:8b", Some(&show_resp)),
            ToolMode::Native
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_no_probe_result_falls_back_to_name_heuristic() {
        use moltis_config::ToolMode;
        // No probe result at all — falls back to model name matching.
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "llama3.1:8b", None),
            ToolMode::Native
        );
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Auto, "unknown-model:latest", None),
            ToolMode::Text
        );
    }

    #[test]
    fn resolve_ollama_tool_mode_explicit_overrides_capabilities() {
        use moltis_config::ToolMode;
        // Even with capabilities saying "tools", explicit Text override wins.
        let show_resp = OllamaShowResponse {
            details: OllamaModelDetails {
                family: Some("minimax".into()),
                families: None,
            },
            capabilities: vec!["tools".into()],
        };
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Text, "MiniMax-M2.5:latest", Some(&show_resp)),
            ToolMode::Text
        );
        assert_eq!(
            resolve_ollama_tool_mode(ToolMode::Off, "MiniMax-M2.5:latest", Some(&show_resp)),
            ToolMode::Off
        );
    }

    /// Verify OllamaShowResponse deserializes from Ollama >= 0.5.x JSON with capabilities.
    #[test]
    fn ollama_show_response_deserializes_with_capabilities() {
        let json = r#"{
            "details": {"family": "minimax", "families": null},
            "capabilities": ["completion", "tools"]
        }"#;
        let resp: OllamaShowResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.details.family.as_deref(), Some("minimax"));
        assert_eq!(resp.capabilities, vec!["completion", "tools"]);
    }

    /// Verify OllamaShowResponse deserializes from old Ollama without capabilities field.
    #[test]
    fn ollama_show_response_deserializes_without_capabilities() {
        let json = r#"{"details": {"family": "llama3.1", "families": ["llama3.1"]}}"#;
        let resp: OllamaShowResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.details.family.as_deref(), Some("llama3.1"));
        assert!(
            resp.capabilities.is_empty(),
            "missing field should default to empty vec"
        );
    }

    /// Capabilities with only "tools" (single item).
    #[test]
    fn ollama_capabilities_single_tools_entry() {
        let caps = vec!["tools".into()];
        assert_eq!(ollama_capabilities_support_tools(&caps), Some(true));
    }
}
