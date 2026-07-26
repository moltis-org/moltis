//! Local model pricing.
//!
//! Cost is computed here rather than left to the backend so it exists whether
//! or not anything is configured, and so the numbers on the Metrics page do not
//! depend on a third party being reachable.
//!
//! Prices are USD per million tokens and go stale whenever a provider changes
//! them, which is the accepted cost of not needing a backend to see spend. A
//! model with no entry reports no cost rather than guessing: a wrong number
//! that looks authoritative is worse than a visible gap.

use std::collections::BTreeMap;

use crate::model::TokenUsage;

/// USD per million tokens for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    /// Fresh input tokens.
    pub input: f64,
    /// Generated tokens.
    pub output: f64,
    /// Tokens read from the prompt cache, usually far cheaper than fresh input.
    pub cache_read: f64,
    /// Tokens written to the prompt cache, usually dearer than fresh input.
    pub cache_write: f64,
}

impl ModelPrice {
    /// A price with no cache tiers, for providers that do not bill them
    /// separately.
    const fn flat(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            cache_read: input,
            cache_write: input,
        }
    }

    /// A price with explicit cache tiers.
    const fn cached(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Self {
        Self {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    /// Cost in USD for `usage`, broken down by token bucket.
    ///
    /// Cache buckets are priced separately rather than folded into input:
    /// a cache read can be an order of magnitude cheaper, and summing them
    /// would overstate spend on exactly the workloads caching is meant to help.
    #[must_use]
    pub fn cost_details(&self, usage: &TokenUsage) -> BTreeMap<String, f64> {
        const PER: f64 = 1_000_000.0;
        let mut details = BTreeMap::new();
        let mut put = |key: &str, tokens: u32, rate: f64| {
            if tokens > 0 && rate > 0.0 {
                details.insert(key.to_string(), (tokens as f64) * rate / PER);
            }
        };
        put("input", usage.input, self.input);
        put("output", usage.output, self.output);
        put("cache_read", usage.cache_read, self.cache_read);
        put("cache_write", usage.cache_write, self.cache_write);

        let total: f64 = details.values().sum();
        if total > 0.0 {
            details.insert("total".to_string(), total);
        }
        details
    }
}

/// Built-in prices, keyed by a normalized model id prefix.
///
/// Matched by longest prefix so a dated release like
/// `claude-opus-4-5-20260101` picks up its family's price without an entry per
/// snapshot.
const PRICES: &[(&str, ModelPrice)] = &[
    // Anthropic
    ("claude-opus-4", ModelPrice::cached(15.0, 75.0, 1.5, 18.75)),
    ("claude-sonnet-4", ModelPrice::cached(3.0, 15.0, 0.3, 3.75)),
    ("claude-haiku-4", ModelPrice::cached(1.0, 5.0, 0.1, 1.25)),
    ("claude-3-5-haiku", ModelPrice::cached(0.8, 4.0, 0.08, 1.0)),
    (
        "claude-3-5-sonnet",
        ModelPrice::cached(3.0, 15.0, 0.3, 3.75),
    ),
    ("claude-3-opus", ModelPrice::cached(15.0, 75.0, 1.5, 18.75)),
    // OpenAI
    ("gpt-4o-mini", ModelPrice::cached(0.15, 0.6, 0.075, 0.15)),
    ("gpt-4o", ModelPrice::cached(2.5, 10.0, 1.25, 2.5)),
    ("gpt-4.1-mini", ModelPrice::cached(0.4, 1.6, 0.1, 0.4)),
    ("gpt-4.1", ModelPrice::cached(2.0, 8.0, 0.5, 2.0)),
    ("o3-mini", ModelPrice::flat(1.1, 4.4)),
    ("o3", ModelPrice::flat(2.0, 8.0)),
    // Google
    ("gemini-2.5-pro", ModelPrice::flat(1.25, 10.0)),
    ("gemini-2.5-flash", ModelPrice::flat(0.3, 2.5)),
    ("gemini-2.0-flash", ModelPrice::flat(0.1, 0.4)),
    // Meta / open weights, typical hosted rates
    ("llama-3.3-70b", ModelPrice::flat(0.6, 0.6)),
    ("llama-3.1-8b", ModelPrice::flat(0.05, 0.08)),
    ("mistral-large", ModelPrice::flat(2.0, 6.0)),
    ("deepseek-chat", ModelPrice::flat(0.27, 1.1)),
    ("deepseek-reasoner", ModelPrice::flat(0.55, 2.19)),
];

/// Look up a price for `model`.
///
/// Provider prefixes (`anthropic/claude-opus-4`) are stripped first so a model
/// routed through OpenRouter prices the same as one called directly.
#[must_use]
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let normalized = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    PRICES
        .iter()
        .filter(|(prefix, _)| normalized.starts_with(prefix))
        // Longest prefix wins, so `gpt-4o-mini` does not match `gpt-4o`.
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, price)| *price)
}

/// Cost breakdown for `usage` on `model`, empty when the model is unpriced.
#[must_use]
pub fn cost_details(model: &str, usage: &TokenUsage) -> BTreeMap<String, f64> {
    price_for(model).map_or_else(BTreeMap::new, |price| price.cost_details(usage))
}

/// Total USD for `usage` on `model`, or `None` when the model is unpriced.
#[must_use]
pub fn total_cost(model: &str, usage: &TokenUsage) -> Option<f64> {
    cost_details(model, usage).get("total").copied()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32, cache_read: u32, cache_write: u32) -> TokenUsage {
        TokenUsage::from_provider_totals(input, output, cache_read, cache_write)
    }

    #[test]
    fn a_known_model_prices_input_and_output() {
        let details = cost_details("claude-opus-4", &usage(1_000_000, 1_000_000, 0, 0));

        assert!((details["input"] - 15.0).abs() < 1e-9);
        assert!((details["output"] - 75.0).abs() < 1e-9);
        assert!((details["total"] - 90.0).abs() < 1e-9);
    }

    #[test]
    fn cache_reads_are_not_priced_as_fresh_input() {
        // Folding cache into input would overstate spend by 10x on exactly the
        // workloads caching exists to make cheaper.
        let details = cost_details("claude-opus-4", &usage(0, 0, 1_000_000, 0));

        assert!((details["cache_read"] - 1.5).abs() < 1e-9);
        assert!(!details.contains_key("input"));
    }

    #[test]
    fn cache_writes_are_priced_above_fresh_input() {
        let details = cost_details("claude-opus-4", &usage(0, 0, 0, 1_000_000));
        assert!(details["cache_write"] > 15.0);
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        // `gpt-4o-mini` must not be priced as `gpt-4o`.
        let mini = price_for("gpt-4o-mini").expect("priced");
        let full = price_for("gpt-4o").expect("priced");

        assert!((mini.input - 0.15).abs() < 1e-9);
        assert!((full.input - 2.5).abs() < 1e-9);
    }

    #[test]
    fn dated_snapshots_inherit_the_family_price() {
        let dated = price_for("claude-opus-4-5-20260101").expect("priced");
        let family = price_for("claude-opus-4").expect("priced");

        assert_eq!(dated, family);
    }

    #[test]
    fn a_provider_prefix_is_stripped() {
        // The same model routed through OpenRouter must price identically.
        assert_eq!(
            price_for("anthropic/claude-opus-4"),
            price_for("claude-opus-4")
        );
        assert_eq!(price_for("openai/gpt-4o"), price_for("gpt-4o"));
    }

    #[test]
    fn model_ids_are_matched_case_insensitively() {
        assert_eq!(price_for("Claude-Opus-4"), price_for("claude-opus-4"));
    }

    #[test]
    fn an_unknown_model_reports_no_cost_rather_than_guessing() {
        // A confident wrong number is worse than a visible gap.
        assert!(price_for("some-private-finetune").is_none());
        assert!(cost_details("some-private-finetune", &usage(1000, 1000, 0, 0)).is_empty());
        assert_eq!(
            total_cost("some-private-finetune", &usage(1000, 1000, 0, 0)),
            None
        );
    }

    #[test]
    fn an_empty_model_id_is_unpriced() {
        assert!(price_for("").is_none());
        assert!(price_for("   ").is_none());
    }

    #[test]
    fn zero_usage_produces_no_entries() {
        assert!(cost_details("claude-opus-4", &usage(0, 0, 0, 0)).is_empty());
    }

    #[test]
    fn the_total_is_the_sum_of_the_buckets() {
        let details = cost_details("claude-opus-4", &usage(1_000, 2_000, 3_000, 4_000));
        let summed: f64 = details
            .iter()
            .filter(|(k, _)| k.as_str() != "total")
            .map(|(_, v)| v)
            .sum();

        assert!((details["total"] - summed).abs() < 1e-12);
    }

    #[test]
    fn flat_priced_models_charge_cache_reads_as_input() {
        // Providers that do not bill a cache tier must not silently price
        // cached tokens at zero.
        let details = cost_details("gemini-2.5-pro", &usage(0, 0, 1_000_000, 0));
        assert!((details["cache_read"] - 1.25).abs() < 1e-9);
    }
}
