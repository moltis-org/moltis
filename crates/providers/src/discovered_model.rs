//! Discovered model types and merge helpers for model list composition.

use std::collections::{HashMap, HashSet};

use crate::model_capabilities::ModelCapabilities;

/// A model discovered from a provider API (e.g. `/v1/models`).
///
/// Replaces bare `(String, String)` tuples so that optional metadata
/// such as `created_at` can travel alongside the id/display_name pair.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub id: String,
    pub display_name: String,
    /// Unix timestamp from the API (e.g. OpenAI `created` field).
    /// Used to sort models newest-first. `None` for static catalog entries.
    pub created_at: Option<i64>,
    /// Flagged by the provider as a recommended/flagship model.
    /// Used to surface the most relevant models in the UI.
    pub recommended: bool,
    pub capabilities: Option<ModelCapabilities>,
}

impl DiscoveredModel {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            created_at: None,
            recommended: false,
            capabilities: None,
        }
    }

    pub fn with_created_at(mut self, created_at: Option<i64>) -> Self {
        self.created_at = created_at;
        self
    }

    pub fn with_recommended(mut self, recommended: bool) -> Self {
        self.recommended = recommended;
        self
    }

    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = Some(capabilities);
        self
    }
}

/// Convert a static model catalog into `DiscoveredModel` entries, marking
/// the first `recommended_count` as recommended.
pub fn catalog_to_discovered(
    catalog: &[(&str, &str)],
    recommended_count: usize,
) -> Vec<DiscoveredModel> {
    catalog
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            DiscoveredModel::new(*id, *name).with_recommended(i < recommended_count)
        })
        .collect()
}

pub(crate) fn merge_preferred_and_discovered_models(
    preferred: Vec<String>,
    discovered: Vec<DiscoveredModel>,
) -> Vec<DiscoveredModel> {
    let discovered_by_id: HashMap<String, &DiscoveredModel> =
        discovered.iter().map(|m| (m.id.clone(), m)).collect();
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for model_id in preferred {
        if !seen.insert(model_id.clone()) {
            continue;
        }
        let model = if let Some(d) = discovered_by_id.get(&model_id) {
            DiscoveredModel {
                id: model_id,
                display_name: d.display_name.clone(),
                created_at: d.created_at,
                recommended: d.recommended,
                capabilities: d.capabilities,
            }
        } else {
            DiscoveredModel::new(model_id.clone(), model_id)
        };
        merged.push(model);
    }

    for model in discovered {
        if !seen.insert(model.id.clone()) {
            continue;
        }
        merged.push(model);
    }

    merged
}

pub(crate) fn merge_discovered_with_fallback_catalog(
    discovered: Vec<DiscoveredModel>,
    fallback: Vec<DiscoveredModel>,
) -> Vec<DiscoveredModel> {
    if discovered.is_empty() {
        return fallback;
    }

    let fallback_by_id: HashMap<String, DiscoveredModel> =
        fallback.into_iter().map(|m| (m.id.clone(), m)).collect();
    discovered
        .into_iter()
        .map(|m| {
            let display_name = if m.display_name.trim().is_empty() {
                fallback_by_id
                    .get(&m.id)
                    .map(|fb| fb.display_name.clone())
                    .unwrap_or_else(|| m.id.clone())
            } else {
                m.display_name
            };
            DiscoveredModel {
                id: m.id,
                display_name,
                created_at: m.created_at,
                recommended: m.recommended,
                capabilities: m.capabilities,
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn merge_discovered_with_fallback_keeps_discovered_when_non_empty() {
        let merged = merge_discovered_with_fallback_catalog(
            vec![
                DiscoveredModel::new("live-a", "Live A"),
                DiscoveredModel::new("live-b", "Live B"),
            ],
            vec![
                DiscoveredModel::new("live-a", "Fallback A"),
                DiscoveredModel::new("fallback-only", "Fallback Only"),
            ],
        );

        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["live-a", "live-b"]);
    }

    #[test]
    fn merge_discovered_with_fallback_uses_fallback_when_discovered_empty() {
        let merged = merge_discovered_with_fallback_catalog(Vec::new(), vec![
            DiscoveredModel::new("fallback-a", "Fallback A"),
            DiscoveredModel::new("fallback-b", "Fallback B"),
        ]);

        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["fallback-a", "fallback-b"]);
    }
}
