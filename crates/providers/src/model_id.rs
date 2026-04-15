//! Model ID manipulation: namespacing, reasoning suffix parsing, raw ID extraction.

/// Separator between provider namespace and model ID.
pub(crate) const MODEL_ID_NAMESPACE_SEP: &str = "::";

/// Separator between a model ID and its reasoning effort suffix.
pub(crate) const REASONING_SUFFIX_SEP: char = '@';

/// Reasoning effort suffixes appended to model IDs.
pub(crate) const REASONING_SUFFIXES: &[(&str, moltis_agents::model::ReasoningEffort)] = &[
    ("reasoning-low", moltis_agents::model::ReasoningEffort::Low),
    (
        "reasoning-medium",
        moltis_agents::model::ReasoningEffort::Medium,
    ),
    (
        "reasoning-high",
        moltis_agents::model::ReasoningEffort::High,
    ),
];

#[must_use]
pub fn namespaced_model_id(provider: &str, model_id: &str) -> String {
    if model_id.contains(MODEL_ID_NAMESPACE_SEP) {
        return model_id.to_string();
    }
    format!("{provider}{MODEL_ID_NAMESPACE_SEP}{model_id}")
}

/// Split a model ID into (base_id, optional reasoning effort).
///
/// Examples:
/// - `"anthropic::claude-opus-4-5@reasoning-high"` → `("anthropic::claude-opus-4-5", Some(High))`
/// - `"gpt-4o"` → `("gpt-4o", None)`
#[must_use]
pub fn split_reasoning_suffix(
    model_id: &str,
) -> (&str, Option<moltis_agents::model::ReasoningEffort>) {
    if let Some((base, suffix)) = model_id.rsplit_once(REASONING_SUFFIX_SEP) {
        for &(tag, effort) in REASONING_SUFFIXES {
            if suffix == tag {
                return (base, Some(effort));
            }
        }
    }
    (model_id, None)
}

#[must_use]
pub fn raw_model_id(model_id: &str) -> &str {
    // Fast path: skip reasoning suffix parsing when no `@` is present.
    let base = if model_id.contains(REASONING_SUFFIX_SEP) {
        split_reasoning_suffix(model_id).0
    } else {
        model_id
    };
    base.rsplit_once(MODEL_ID_NAMESPACE_SEP)
        .map(|(_, raw)| raw)
        .unwrap_or(base)
}

#[must_use]
pub(crate) fn capability_model_id(model_id: &str) -> &str {
    let raw = raw_model_id(model_id).trim();
    raw.rsplit('/')
        .next()
        .filter(|id| !id.is_empty())
        .unwrap_or(raw)
}

pub(crate) fn configured_model_for_provider(model_id: &str) -> &str {
    raw_model_id(model_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn split_reasoning_suffix_parses_effort_levels() {
        use moltis_agents::model::ReasoningEffort;
        assert_eq!(
            split_reasoning_suffix("anthropic::claude-opus-4-5@reasoning-high"),
            ("anthropic::claude-opus-4-5", Some(ReasoningEffort::High))
        );
        assert_eq!(
            split_reasoning_suffix("o3@reasoning-low"),
            ("o3", Some(ReasoningEffort::Low))
        );
        assert_eq!(split_reasoning_suffix("gpt-4o"), ("gpt-4o", None));
        assert_eq!(
            split_reasoning_suffix("model@unknown-suffix"),
            ("model@unknown-suffix", None)
        );
    }

    #[test]
    fn raw_model_id_strips_reasoning_suffix() {
        assert_eq!(
            raw_model_id("anthropic::claude-opus-4-5@reasoning-high"),
            "claude-opus-4-5"
        );
        assert_eq!(raw_model_id("o3@reasoning-medium"), "o3");
        assert_eq!(raw_model_id("gpt-4o"), "gpt-4o");
    }
}
