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

mod deterministic;
mod recency_preserving;
mod runner;
mod shared;

#[cfg(feature = "llm-compaction")]
mod llm_replace;
#[cfg(feature = "llm-compaction")]
mod structured;

#[cfg(test)]
mod test_support;

pub(crate) use runner::{
    CompactionOutcome, CompactionRunError, SETTINGS_HINT, extract_summary_body, run_compaction,
};
