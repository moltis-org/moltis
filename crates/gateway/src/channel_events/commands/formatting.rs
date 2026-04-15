use std::collections::BTreeSet;

/// Collect the set of distinct `provider` values from a model list.
///
/// A `BTreeSet` makes the contract explicit: provider names are unique and
/// returned in deterministic order for the Telegram `/model` inline keyboard.
pub(in crate::channel_events) fn unique_providers(models: &[serde_json::Value]) -> Vec<String> {
    models
        .iter()
        .filter_map(|m| m.get("provider").and_then(|v| v.as_str()).map(String::from))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Format a numbered model list, optionally filtered by provider.
///
/// Each line is: `N. DisplayName [provider] *` (where `*` marks the current model).
/// Uses the global index (across all models) so the switch command works with
/// the same numbering regardless of filtering.
pub(in crate::channel_events) fn format_model_list(
    models: &[serde_json::Value],
    current_model: Option<&str>,
    provider_filter: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    for (i, m) in models.iter().enumerate() {
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("");
        let display = m.get("displayName").and_then(|v| v.as_str()).unwrap_or(id);
        if let Some(filter) = provider_filter
            && provider != filter
        {
            continue;
        }
        let marker = if current_model == Some(id) {
            " *"
        } else {
            ""
        };
        lines.push(format!("{}. {} [{}]{}", i + 1, display, provider, marker));
    }
    lines.join("\n")
}
