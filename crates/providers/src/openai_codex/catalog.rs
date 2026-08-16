use std::{collections::HashSet, sync::mpsc, time::Duration};

use {
    moltis_oauth::TokenStore,
    secrecy::{ExposeSecret, Secret},
    tracing::{debug, info, warn},
};

use super::OpenAiCodexProvider;

const CODEX_MODELS_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/models";
/// Report a client version that satisfies the Codex API's
/// `minimal_client_version` filter so all available models are returned.
/// Using the crate's own version (0.x) caused the API to hide newer models
/// that require >= 0.98.0. See <https://github.com/moltis-org/moltis/issues/354>.
///
/// **DO NOT** change this to `env!("CARGO_PKG_VERSION")` — the crate version
/// is unrelated to the Codex client version and will break model discovery.
pub(super) const CODEX_MODELS_CLIENT_VERSION: &str = "1.0.0";

pub(super) const DEFAULT_CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-5.6-sol", "GPT-5.6 Sol"),
    ("gpt-5.6-terra", "GPT-5.6 Terra"),
    ("gpt-5.6-luna", "GPT-5.6 Luna"),
    ("gpt-5.4", "GPT-5.4"),
    ("gpt-5.3-codex-spark", "GPT-5.3 Codex Spark"),
    ("gpt-5.3-codex", "GPT-5.3 Codex"),
    ("gpt-5.2-codex", "GPT-5.2 Codex"),
    ("gpt-5.2", "GPT-5.2"),
    ("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
    ("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini"),
];

/// Parse tokens from Codex CLI auth.json content.
pub(super) fn parse_codex_cli_tokens(data: &str) -> Option<moltis_oauth::OAuthTokens> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;
    let tokens = json.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str()?.to_string();
    let id_token = tokens
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(moltis_oauth::OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        id_token: id_token.map(Secret::new),
        account_id,
        expires_at: None,
    })
}

/// Try to load tokens from the Codex CLI file at `~/.codex/auth.json`.
pub(super) fn load_codex_cli_tokens() -> Option<moltis_oauth::OAuthTokens> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home)
        .join(".codex")
        .join("auth.json");
    let data = std::fs::read_to_string(path).ok()?;
    parse_codex_cli_tokens(&data)
}

pub fn has_stored_tokens() -> bool {
    TokenStore::new().load("openai-codex").is_some() || load_codex_cli_tokens().is_some()
}

pub fn default_model_catalog() -> Vec<crate::DiscoveredModel> {
    crate::catalog_to_discovered(DEFAULT_CODEX_MODELS, 3)
        .into_iter()
        .map(|model| {
            let mut capabilities = crate::ModelCapabilities::infer(&model.id);
            if let Some(context_window) =
                crate::model_capabilities::context_window_fallback_for_model(
                    crate::model_capabilities::ContextWindowFallbackScope::OpenAiCodex,
                    &model.id,
                )
            {
                capabilities.context_window = context_window;
            }
            model.with_capabilities(capabilities)
        })
        .collect()
}

fn formatted_model_name(model_id: &str) -> String {
    let mut out = Vec::new();
    for part in model_id.split('-') {
        let item = match part {
            "gpt" => "GPT".to_string(),
            "codex" => "Codex".to_string(),
            "mini" => "Mini".to_string(),
            "max" => "Max".to_string(),
            other => {
                if other.is_empty() {
                    continue;
                }
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => {
                        let mut chunk = String::new();
                        chunk.push(first.to_ascii_uppercase());
                        chunk.push_str(chars.as_str());
                        chunk
                    },
                    None => continue,
                }
            },
        };
        out.push(item);
    }
    if out.is_empty() {
        model_id.to_string()
    } else {
        out.join(" ")
    }
}

fn normalize_display_name(model_id: &str, display_name: Option<&str>) -> String {
    let normalized = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(model_id);
    if normalized == model_id {
        return formatted_model_name(model_id);
    }
    normalized.to_string()
}

fn is_likely_model_id(model_id: &str) -> bool {
    if model_id.is_empty() || model_id.len() > 120 {
        return false;
    }
    if model_id.chars().any(char::is_whitespace) {
        return false;
    }
    model_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<crate::DiscoveredModel> {
    let obj = entry.as_object()?;
    let model_id = obj
        .get("id")
        .or_else(|| obj.get("slug"))
        .or_else(|| obj.get("model"))
        .and_then(serde_json::Value::as_str)?;

    if !is_likely_model_id(model_id) {
        return None;
    }

    let display_name = obj
        .get("display_name")
        .or_else(|| obj.get("displayName"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("title"))
        .and_then(serde_json::Value::as_str);

    let created_at = obj.get("created").and_then(serde_json::Value::as_i64);

    Some(
        crate::DiscoveredModel::new(model_id, normalize_display_name(model_id, display_name))
            .with_created_at(created_at),
    )
}

fn collect_candidate_arrays<'a>(
    value: &'a serde_json::Value,
    out: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Array(items) => out.extend(items),
        serde_json::Value::Object(map) => {
            for key in ["models", "data", "items", "results", "available"] {
                if let Some(nested) = map.get(key) {
                    collect_candidate_arrays(nested, out);
                }
            }
        },
        _ => {},
    }
}

pub(super) fn parse_models_payload(value: &serde_json::Value) -> Vec<crate::DiscoveredModel> {
    let mut candidates = Vec::new();
    collect_candidate_arrays(value, &mut candidates);

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for entry in candidates {
        if let Some(model) = parse_model_entry(entry)
            && seen.insert(model.id.clone())
        {
            models.push(model);
        }
    }

    // Sort by created_at descending (newest first). Models without a
    // timestamp are placed after those with one, preserving relative order.
    models.sort_by(|a, b| match (a.created_at, b.created_at) {
        (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    models
}

async fn fetch_models_from_api(
    access_token: String,
    account_id: String,
) -> anyhow::Result<Vec<crate::DiscoveredModel>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let url = format!("{CODEX_MODELS_ENDPOINT}?client_version={CODEX_MODELS_CLIENT_VERSION}");
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("originator", "pi")
        .header("accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("codex models API error HTTP {status}");
    }
    let payload: serde_json::Value = serde_json::from_str(&body)?;
    let models = parse_models_payload(&payload);
    if models.is_empty() {
        anyhow::bail!("codex models API returned no models");
    }
    Ok(models)
}

/// Spawn model discovery in a background thread and return the receiver
/// immediately, without blocking. Returns `None` if tokens are not configured.
pub fn start_model_discovery() -> Option<mpsc::Receiver<anyhow::Result<Vec<crate::DiscoveredModel>>>>
{
    let (access_token, account_id) = load_access_token_and_account_id().ok()?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(fetch_models_from_api(access_token, account_id)));
        let _ = tx.send(result);
    });
    Some(rx)
}

fn fetch_models_blocking(
    access_token: String,
    account_id: String,
) -> anyhow::Result<Vec<crate::DiscoveredModel>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(fetch_models_from_api(access_token, account_id)));
        let _ = tx.send(result);
    });
    rx.recv()
        .map_err(|err| anyhow::anyhow!("codex model discovery worker failed: {err}"))?
}

fn load_access_token_and_account_id() -> anyhow::Result<(String, String)> {
    let tokens = TokenStore::new()
        .load("openai-codex")
        .or_else(load_codex_cli_tokens)
        .ok_or_else(|| {
            debug!("openai-codex tokens not found in token store or codex CLI auth");
            anyhow::anyhow!("openai-codex tokens not found")
        })?;

    let access_token = tokens.access_token.expose_secret().clone();
    let account_id = OpenAiCodexProvider::resolve_account_id(&tokens)?;
    Ok((access_token, account_id))
}

pub fn live_models() -> anyhow::Result<Vec<crate::DiscoveredModel>> {
    let (access_token, account_id) = load_access_token_and_account_id()?;
    let models = fetch_models_blocking(access_token, account_id)?;
    info!(
        model_count = models.len(),
        "loaded openai-codex live models"
    );
    Ok(models)
}

pub fn available_models() -> Vec<crate::DiscoveredModel> {
    let fallback = default_model_catalog();
    let discovered = match live_models() {
        Ok(models) => models,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("tokens not found") || msg.contains("not logged in") {
                debug!(error = %err, "openai-codex not configured, using fallback catalog");
            } else {
                warn!(error = %err, "failed to fetch openai-codex models, using fallback catalog");
            }
            return fallback;
        },
    };

    let merged = crate::merge_discovered_with_fallback_catalog(discovered, fallback);

    info!(
        model_count = merged.len(),
        "loaded openai-codex models catalog"
    );
    merged
}
