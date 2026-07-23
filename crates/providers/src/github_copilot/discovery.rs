use std::collections::HashSet;

fn normalize_display_name(model_id: &str, display_name: Option<&str>) -> String {
    let normalized = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(model_id);
    if normalized == model_id {
        model_id.to_string()
    } else {
        normalized.to_string()
    }
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

fn u64_metadata_field(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    let obj = value.as_object()?;
    for key in keys {
        if let Some(value) = obj.get(*key).and_then(serde_json::Value::as_u64) {
            return Some(value);
        }
    }
    obj.values()
        .filter(|value| value.is_object())
        .find_map(|nested| u64_metadata_field(nested, keys))
}

fn copilot_context_window_metadata(entry: &serde_json::Value) -> Option<u32> {
    u64_metadata_field(entry, &[
        "context_window",
        "contextWindow",
        "context_length",
        "contextLength",
        "max_context_tokens",
        "maxContextTokens",
        "max_input_tokens",
        "maxInputTokens",
        "input_token_limit",
        "inputTokenLimit",
    ])
    .and_then(|value| u32::try_from(value).ok())
}

fn is_copilot_fast_mode(
    model_id: &str,
    display_name: Option<&str>,
    entry: &serde_json::Value,
) -> bool {
    let id = model_id.to_ascii_lowercase();
    if id.contains("fast-mode") || id.ends_with("-fast") || id.contains("-fast-") {
        return true;
    }
    if display_name
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| name.contains("fast mode") || name.contains("(fast)"))
    {
        return true;
    }
    entry
        .as_object()
        .and_then(|obj| obj.get("fast_mode").or_else(|| obj.get("fastMode")))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || entry
            .as_object()
            .and_then(|obj| obj.get("mode").or_else(|| obj.get("variant")))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("fast"))
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<super::super::DiscoveredModel> {
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
    let context_window = copilot_context_window_metadata(entry);

    let capabilities = context_window.map(|context_window| {
        let mut capabilities = super::super::ModelCapabilities::infer(model_id);
        capabilities.context_window = context_window;
        if is_copilot_fast_mode(model_id, display_name, entry)
            && capabilities.context_window > 200_000
        {
            capabilities.context_window = 200_000;
        }
        capabilities
    });

    let model = super::super::DiscoveredModel::new(
        model_id,
        normalize_display_name(model_id, display_name),
    )
    .with_created_at(created_at);

    Some(match capabilities {
        Some(capabilities) => model.with_capabilities(capabilities),
        None => model,
    })
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

pub(super) fn parse_models_payload(
    value: &serde_json::Value,
) -> Vec<super::super::DiscoveredModel> {
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

    models.sort_by(|a, b| match (a.created_at, b.created_at) {
        (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    models
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_model_entry_reads_context_window_metadata() {
        let entry = serde_json::json!({
            "id": "claude-opus-4.8",
            "name": "Claude Opus 4.8",
            "context_window": 1_000_000,
        });

        let Some(model) = super::parse_model_entry(&entry) else {
            panic!("model entry should parse");
        };
        let Some(capabilities) = model.capabilities else {
            panic!("capabilities should be populated");
        };
        assert_eq!(capabilities.context_window, 1_000_000);
    }

    #[test]
    fn parse_model_entry_reads_nested_context_window_metadata() {
        let entry = serde_json::json!({
            "id": "claude-opus-4.8",
            "name": "Claude Opus 4.8",
            "capabilities": {
                "limits": {
                    "maxInputTokens": 1_000_000,
                },
            },
        });

        let Some(model) = super::parse_model_entry(&entry) else {
            panic!("model entry should parse");
        };
        let Some(capabilities) = model.capabilities else {
            panic!("capabilities should be populated");
        };
        assert_eq!(capabilities.context_window, 1_000_000);
    }

    #[test]
    fn parse_model_entry_keeps_fast_mode_at_standard_claude_window() {
        let entry = serde_json::json!({
            "id": "claude-opus-4.8-fast",
            "name": "Claude Opus 4.8 (fast mode)",
            "context_window": 1_000_000,
        });

        let Some(model) = super::parse_model_entry(&entry) else {
            panic!("model entry should parse");
        };
        let Some(capabilities) = model.capabilities else {
            panic!("capabilities should be populated");
        };
        assert_eq!(capabilities.context_window, 200_000);
    }
}
