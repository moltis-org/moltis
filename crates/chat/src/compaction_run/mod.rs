//! Compaction strategy dispatcher.
//!
//! Routes a session history through the [`CompactionMode`] selected in
//! `chat.compaction`. Each strategy lives in its own submodule and owns
//! its own tests; shared boundary / pruning / tool-pair helpers live in
//! [`shared`].
//!
//! Submodules:
//! - [`deterministic`] — zero-LLM replace-all extraction.
//! - [`recency_preserving`] — zero-LLM head + middle-marker + tail.
//! - [`structured`] — LLM head + structured-summary + tail (feature-gated).
//! - [`llm_replace`] — LLM streaming-summary replace-all (feature-gated).
//! - [`shared`] — boundary computation, pruning, tool-pair repair,
//!   message builders.
//! - `test_support` — `#[cfg(test)]`-only stub provider for the LLM modes.
//!
//! See `docs/src/compaction.md` for the full mode comparison and trade-off
//! guidance, and the rustdoc on [`moltis_config::CompactionMode`] for
//! per-variant semantics.

use {
    moltis_agents::model::LlmProvider,
    moltis_config::{CompactionConfig, CompactionMode},
    serde_json::Value,
    thiserror::Error,
};

mod deterministic;
mod recency_preserving;
mod shared;

#[cfg(feature = "llm-compaction")]
mod llm_replace;
#[cfg(feature = "llm-compaction")]
mod structured;

#[cfg(test)]
mod test_support;

/// Errors surfaced by [`run_compaction`].
///
/// Several variants are gated on the `llm-compaction` cargo feature;
/// when the feature is off the LLM-backed strategies aren't compiled in,
/// so their dedicated error variants become dead code.
#[derive(Debug, Error)]
pub(crate) enum CompactionRunError {
    /// History was empty — nothing to compact.
    #[error("nothing to compact")]
    EmptyHistory,
    /// The strategy produced no summary text.
    #[error("compact produced empty summary")]
    EmptySummary,
    /// A mode that requires an LLM provider was selected but none was passed.
    #[cfg(feature = "llm-compaction")]
    #[error("compaction mode '{mode}' requires an LLM provider to be available for the session")]
    ProviderRequired { mode: &'static str },
    /// The user selected a mode that requires a cargo feature that isn't enabled.
    #[cfg(not(feature = "llm-compaction"))]
    #[error("compaction mode '{mode}' requires the 'llm-compaction' cargo feature to be enabled")]
    FeatureDisabled { mode: &'static str },
    /// The LLM streaming summary call failed.
    #[cfg(feature = "llm-compaction")]
    #[error("compact summarization failed: {0}")]
    LlmFailed(String),
    /// `recency_preserving` couldn't make meaningful progress: the history
    /// is already smaller than `protect_head + protect_tail_min + 1`, and
    /// there was no bulky tool-result content to prune. The caller should
    /// fall back to a different mode (e.g. `deterministic`) rather than
    /// loop.
    #[error(
        "history has {messages} messages — too small for recency_preserving with \
         protect_head={head} and protect_tail_min={tail}; no tool-result pruning \
         was possible either. Try chat.compaction.mode = \"deterministic\" for \
         tiny sessions."
    )]
    TooSmallToCompact {
        messages: usize,
        head: usize,
        tail: usize,
    },
}

/// Outcome of a successful compaction run.
///
/// Surfaces the effective mode and any token usage so callers can
/// broadcast human-readable metadata to the chat UI ("Compacted via
/// Structured mode, used 1,234 tokens"). `effective_mode` may differ
/// from the user-selected `config.mode` when the `Structured` strategy
/// falls back to `RecencyPreserving` on LLM failure.
#[derive(Debug, Clone)]
pub(crate) struct CompactionOutcome {
    /// Replacement history to write back to the session store.
    pub history: Vec<Value>,
    /// The strategy that actually ran (post-fallback).
    pub effective_mode: CompactionMode,
    /// Input tokens consumed by the summary LLM call, if any. Zero for
    /// the no-LLM strategies and for LLM strategies that fell back.
    pub input_tokens: u32,
    /// Output tokens produced by the summary LLM call, if any. Zero for
    /// the no-LLM strategies and for LLM strategies that fell back.
    pub output_tokens: u32,
}

impl CompactionOutcome {
    /// Total tokens consumed by the compaction call (sum of input + output).
    #[must_use]
    pub(crate) fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Build a JSON metadata fragment suitable for splicing into a
    /// broadcast event payload alongside `state` / `phase`.
    ///
    /// When `include_settings_hint` is true, the [`SETTINGS_HINT`]
    /// text is added under `settingsHint` so UI consumers don't have to
    /// know the wording themselves. Callers pass
    /// `config.show_settings_hint` here to honour the user's preference
    /// for hiding the repetitive footer.
    #[must_use]
    pub(crate) fn broadcast_metadata(&self, include_settings_hint: bool) -> Value {
        let mut fragment = serde_json::json!({
            "mode": compaction_mode_key(self.effective_mode),
            "compactionInputTokens": self.input_tokens,
            "compactionOutputTokens": self.output_tokens,
            "compactionTotalTokens": self.total_tokens(),
        });
        if include_settings_hint && let Some(obj) = fragment.as_object_mut() {
            obj.insert(
                "settingsHint".to_string(),
                Value::String(SETTINGS_HINT.to_string()),
            );
        }
        fragment
    }
}

/// Short, UI-friendly snake_case name for a compaction mode. Matches the
/// config-file serialisation so consumers can display the exact value a
/// user would paste into `chat.compaction.mode`.
#[must_use]
pub(crate) fn compaction_mode_key(mode: CompactionMode) -> &'static str {
    match mode {
        CompactionMode::Deterministic => "deterministic",
        CompactionMode::RecencyPreserving => "recency_preserving",
        CompactionMode::Structured => "structured",
        CompactionMode::LlmReplace => "llm_replace",
    }
}

/// Human-readable hint pointing users at the configuration surface so
/// they can change the compaction strategy.
///
/// Kept as a single constant so UI, broadcast events, and any future
/// doc links share the same wording. The reference to
/// `chat.compaction.mode` matches the TOML key so users can paste it
/// straight into `moltis.toml`.
pub(crate) const SETTINGS_HINT: &str = "Change chat.compaction.mode in moltis.toml (or the web UI settings panel) \
     to pick a different compaction strategy. See \
     https://docs.moltis.org/compaction for a comparison of the four modes.";

/// Best-effort extraction of a human-readable summary body from a
/// compacted history, for use in memory-file snapshots and hook payloads.
///
/// Walks the compacted messages in **reverse** looking for a message whose
/// text content begins with either `[Conversation Summary]` (produced by
/// the `deterministic`, `structured`, and `llm_replace` modes) or
/// `[Conversation Compacted]` (produced by the `recency_preserving`
/// middle marker), and returns the stripped body.
///
/// Reverse iteration matters for iterative `structured` re-compaction:
/// the prior compaction's `[Conversation Summary]` message is preserved
/// verbatim inside the head, and the freshly-generated summary is
/// spliced in at `head_end`. A naïve left-to-right scan would return
/// the stale prior body, so the memory-file snapshot and
/// `AfterCompaction.summary_len` hook payload would describe the old
/// compaction instead of the new one. For the other modes (and for
/// first-time structured) there is only one summary-shaped message, so
/// the reverse walk has no effect.
///
/// Returns an empty string when no summary-shaped message is found,
/// which is fine for hook `summary_len` reporting and falls through
/// gracefully.
#[must_use]
pub(crate) fn extract_summary_body(compacted: &[Value]) -> String {
    compacted
        .iter()
        .rev()
        .filter_map(|msg| msg.get("content").and_then(Value::as_str))
        .find_map(|content| {
            content
                .strip_prefix("[Conversation Summary]\n\n")
                .or_else(|| content.strip_prefix("[Conversation Compacted]\n\n"))
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Run the compaction strategy selected by `config` against `history`.
///
/// Returns a [`CompactionOutcome`] containing the replacement history
/// plus the effective mode and any LLM token usage. Call sites are
/// responsible for writing `outcome.history` back to the session store
/// and for splicing `outcome.broadcast_metadata()` into any
/// `auto_compact` / `compact` broadcast events so users can see what
/// ran and how to change it.
///
/// `provider` is only consulted by LLM-backed modes; pass `None` when
/// no provider has been resolved for the session. LLM modes return
/// [`CompactionRunError::ProviderRequired`] when called without one (or
/// [`CompactionRunError::FeatureDisabled`] when the `llm-compaction`
/// cargo feature is off).
pub(crate) async fn run_compaction(
    history: &[Value],
    config: &CompactionConfig,
    provider: Option<&dyn LlmProvider>,
) -> Result<CompactionOutcome, CompactionRunError> {
    if history.is_empty() {
        return Err(CompactionRunError::EmptyHistory);
    }

    match config.mode {
        CompactionMode::Deterministic => deterministic::run(history),
        CompactionMode::RecencyPreserving => {
            // `RecencyPreserving` doesn't call the LLM, but it still
            // needs a context window to size the tail-budget token cut.
            // Pull it from the provider if available; otherwise fall
            // back to the 200 K default (matching Claude-3.5-sonnet /
            // the LlmProvider trait default). On small-context models
            // the 200 K fallback would give an oversized tail budget
            // that covers the entire history and causes
            // `TooSmallToCompact`, so log a WARN so operators can see
            // the degraded behaviour in the logs and decide whether to
            // configure a provider or switch to `deterministic` for
            // the session.
            let context_window = if let Some(p) = provider {
                p.context_window()
            } else {
                tracing::warn!(
                    "chat.compact: recency_preserving has no resolved provider, \
                     falling back to a 200 K context-window estimate for the tail \
                     budget. On smaller-context models this may produce an \
                     oversized tail that skips effective compaction — configure \
                     a session provider or set chat.compaction.mode = \"deterministic\""
                );
                200_000
            };
            recency_preserving::run(history, config, context_window)
        },
        CompactionMode::Structured => {
            #[cfg(feature = "llm-compaction")]
            {
                let provider =
                    provider.ok_or(CompactionRunError::ProviderRequired { mode: "structured" })?;
                let context_window = provider.context_window();
                structured::run(history, config, context_window, provider).await
            }
            #[cfg(not(feature = "llm-compaction"))]
            {
                let _ = (config, provider);
                Err(CompactionRunError::FeatureDisabled { mode: "structured" })
            }
        },
        CompactionMode::LlmReplace => {
            #[cfg(feature = "llm-compaction")]
            {
                let provider = provider.ok_or(CompactionRunError::ProviderRequired {
                    mode: "llm_replace",
                })?;
                llm_replace::run(history, config, provider).await
            }
            #[cfg(not(feature = "llm-compaction"))]
            {
                let _ = (config, provider);
                Err(CompactionRunError::FeatureDisabled {
                    mode: "llm_replace",
                })
            }
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, serde_json::json};

    /// Minimal fixture used by the feature-off tests that exercise the
    /// dispatcher's `FeatureDisabled` branches without going through a
    /// strategy implementation.
    #[cfg(not(feature = "llm-compaction"))]
    fn sample_history() -> Vec<Value> {
        vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "hi there"}),
        ]
    }

    // ── Dispatcher ────────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_history_returns_empty_history_error() {
        let config = CompactionConfig::default();
        let err = run_compaction(&[], &config, None).await.unwrap_err();
        assert!(matches!(err, CompactionRunError::EmptyHistory));
    }

    #[cfg(not(feature = "llm-compaction"))]
    #[tokio::test]
    async fn llm_replace_mode_returns_feature_disabled_when_feature_off() {
        let config = CompactionConfig {
            mode: CompactionMode::LlmReplace,
            ..Default::default()
        };
        let err = run_compaction(&sample_history(), &config, None)
            .await
            .unwrap_err();
        match err {
            CompactionRunError::FeatureDisabled { mode } => assert_eq!(mode, "llm_replace"),
            other => panic!("expected FeatureDisabled, got {other:?}"),
        }
    }

    #[cfg(not(feature = "llm-compaction"))]
    #[tokio::test]
    async fn structured_mode_returns_feature_disabled_when_feature_off() {
        let config = CompactionConfig {
            mode: CompactionMode::Structured,
            ..Default::default()
        };
        let err = run_compaction(&sample_history(), &config, None)
            .await
            .unwrap_err();
        match err {
            CompactionRunError::FeatureDisabled { mode } => assert_eq!(mode, "structured"),
            other => panic!("expected FeatureDisabled, got {other:?}"),
        }
    }

    // ── CompactionOutcome broadcast metadata ──────────────────────────

    #[test]
    fn broadcast_metadata_exposes_mode_and_tokens() {
        let outcome = CompactionOutcome {
            history: vec![json!({"role": "user", "content": "ok"})],
            effective_mode: CompactionMode::Structured,
            input_tokens: 1_234,
            output_tokens: 567,
        };
        let meta = outcome.broadcast_metadata(true);
        assert_eq!(meta["mode"], "structured");
        assert_eq!(meta["compactionInputTokens"], 1_234);
        assert_eq!(meta["compactionOutputTokens"], 567);
        assert_eq!(meta["compactionTotalTokens"], 1_801);
        assert!(
            !meta["settingsHint"]
                .as_str()
                .expect("settingsHint is string")
                .is_empty()
        );
    }

    #[test]
    fn broadcast_metadata_omits_settings_hint_when_disabled() {
        let outcome = CompactionOutcome {
            history: vec![json!({"role": "user", "content": "ok"})],
            effective_mode: CompactionMode::Deterministic,
            input_tokens: 0,
            output_tokens: 0,
        };
        let meta = outcome.broadcast_metadata(false);
        assert_eq!(meta["mode"], "deterministic");
        assert_eq!(meta["compactionTotalTokens"], 0);
        assert!(
            meta.get("settingsHint").is_none(),
            "settingsHint must be absent when include_settings_hint=false, got: {meta}"
        );
    }

    #[test]
    fn compaction_mode_key_matches_toml_serialization() {
        // The keys must match exactly what users would paste into
        // `chat.compaction.mode` in moltis.toml, so UI consumers can
        // display them and offer a "copy to clipboard" action.
        assert_eq!(
            compaction_mode_key(CompactionMode::Deterministic),
            "deterministic"
        );
        assert_eq!(
            compaction_mode_key(CompactionMode::RecencyPreserving),
            "recency_preserving"
        );
        assert_eq!(
            compaction_mode_key(CompactionMode::Structured),
            "structured"
        );
        assert_eq!(
            compaction_mode_key(CompactionMode::LlmReplace),
            "llm_replace"
        );
    }

    // ── extract_summary_body (memory-file / hook helper) ──────────────

    #[test]
    fn extract_summary_body_finds_conversation_summary_prefix() {
        // deterministic / structured / llm_replace shape: single summary
        // message at index 0.
        let compacted = vec![json!({
            "role": "user",
            "content": "[Conversation Summary]\n\nBody text here",
        })];
        assert_eq!(extract_summary_body(&compacted), "Body text here");
    }

    #[test]
    fn extract_summary_body_finds_conversation_compacted_marker() {
        // recency_preserving shape: head verbatim, then middle marker,
        // then tail verbatim. The marker is NOT at index 0.
        let compacted = vec![
            json!({"role": "user", "content": "first user"}),
            json!({"role": "assistant", "content": "first reply"}),
            json!({
                "role": "user",
                "content": "[Conversation Compacted]\n\n6 earlier messages were elided …",
            }),
            json!({"role": "user", "content": "recent user"}),
            json!({"role": "assistant", "content": "recent reply"}),
        ];
        let body = extract_summary_body(&compacted);
        assert!(
            body.starts_with("6 earlier messages were elided"),
            "got: {body}"
        );
    }

    #[test]
    fn extract_summary_body_returns_empty_when_no_summary_shaped_message_present() {
        // Pathological: history with no summary-shaped message. Helper
        // should return "" rather than picking up unrelated content.
        let compacted = vec![
            json!({"role": "user", "content": "just a regular user turn"}),
            json!({"role": "assistant", "content": "just a regular reply"}),
        ];
        assert_eq!(extract_summary_body(&compacted), "");
    }

    #[test]
    fn extract_summary_body_picks_newest_summary_in_iterative_structured_recompaction() {
        // Iterative `structured` re-compaction shape: the prior
        // compaction's `[Conversation Summary]` sits in the retained
        // head at index 0, and the freshly-generated summary is spliced
        // in at `head_end`. The memory-file snapshot and
        // AfterCompaction hook need the NEWEST summary body, not the
        // stale one.
        let compacted = vec![
            json!({
                "role": "user",
                "content": "[Conversation Summary]\n\nOld summary body from a previous compaction",
            }),
            json!({"role": "assistant", "content": "ok got it"}),
            json!({
                "role": "user",
                "content": "[Conversation Summary]\n\nFresh summary body from this compaction",
            }),
            json!({"role": "user", "content": "recent user turn"}),
            json!({"role": "assistant", "content": "recent reply"}),
        ];
        assert_eq!(
            extract_summary_body(&compacted),
            "Fresh summary body from this compaction"
        );
    }
}
