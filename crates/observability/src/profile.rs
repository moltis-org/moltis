//! Per-backend export profiles.
//!
//! Langfuse and an infrastructure APM want fundamentally different things from
//! the same agent run, so they must not receive the same payload:
//!
//! * **Langfuse** is an LLM-native product. It wants the whole conversation —
//!   prompts, completions, tool arguments and results — plus the observation
//!   taxonomy, token usage split by cache bucket, per-turn cost, managed-prompt
//!   versions, and session/user attribution. Content *is* the product.
//!
//! * **Datadog, Grafana Tempo, Honeycomb** are operational tools. They want
//!   latency, error rates and throughput. Shipping prompt bodies to them is
//!   actively harmful: it explodes span size and cardinality, most vendors bill
//!   per ingested span-byte or custom metric, and it copies user conversation
//!   content into a system that was never scoped to hold it. For these backends
//!   the useful signal is the *shape* of the run — how long, how many
//!   iterations, which model, did it error — not what was said.
//!
//! A profile is therefore attached to each sink, and the mapping layer consults
//! it before emitting anything.

use serde::{Deserialize, Serialize};

/// How much conversation content an exporter may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentCapture {
    /// Export inputs, outputs, tool arguments and tool results.
    Full,
    /// Export structural metadata only: sizes, counts, model, timings.
    #[default]
    MetadataOnly,
    /// Export neither content nor content-derived metadata.
    None,
}

impl ContentCapture {
    /// Whether payload bodies may be exported.
    #[must_use]
    pub const fn allows_bodies(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Which attribute vocabularies an exporter emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vocabulary {
    /// Langfuse's own `langfuse.*` mapping, plus `gen_ai.*`.
    Langfuse,
    /// OpenTelemetry GenAI semantic conventions only.
    GenAi,
}

/// What a given backend receives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportProfile {
    /// Content policy.
    pub content: ContentCapture,
    /// Attribute vocabulary.
    pub vocabulary: Vocabulary,
    /// Whether to attach end-user identity.
    ///
    /// High-cardinality in an APM's index and often a compliance question;
    /// essential in Langfuse, where per-user analysis is a core view.
    pub emit_user_id: bool,
    /// Whether to attach session identity.
    pub emit_session_id: bool,
    /// Whether to attach free-form tags.
    pub emit_tags: bool,
    /// Whether to attach token usage and cost.
    ///
    /// Langfuse prices these; an APM generally has no model price table, so
    /// the same numbers are better served from the Prometheus endpoint.
    pub emit_usage: bool,
    /// Ceiling on any single exported attribute value, in bytes.
    pub max_attribute_bytes: usize,
}

impl ExportProfile {
    /// Profile for Langfuse: everything, because the product is built on it.
    #[must_use]
    pub const fn langfuse() -> Self {
        Self {
            content: ContentCapture::Full,
            vocabulary: Vocabulary::Langfuse,
            emit_user_id: true,
            emit_session_id: true,
            emit_tags: true,
            emit_usage: true,
            max_attribute_bytes: 32_768,
        }
    }

    /// Profile for a generic OTel backend (Grafana Tempo, Honeycomb, an
    /// OpenTelemetry Collector): operational shape only, no conversation
    /// content, no per-user cardinality.
    #[must_use]
    pub const fn otel_generic() -> Self {
        Self {
            content: ContentCapture::MetadataOnly,
            vocabulary: Vocabulary::GenAi,
            emit_user_id: false,
            emit_session_id: false,
            emit_tags: true,
            emit_usage: true,
            max_attribute_bytes: 4_096,
        }
    }

    /// Profile for Datadog APM. As [`Self::otel_generic`], but tags are dropped
    /// too: Datadog indexes span tags and bills on custom metric cardinality,
    /// so an unbounded tag set is a billing surprise waiting to happen.
    #[must_use]
    pub const fn datadog() -> Self {
        Self {
            content: ContentCapture::MetadataOnly,
            vocabulary: Vocabulary::GenAi,
            emit_user_id: false,
            emit_session_id: false,
            emit_tags: false,
            emit_usage: true,
            max_attribute_bytes: 4_096,
        }
    }

    /// Whether `langfuse.*` attributes should be written.
    #[must_use]
    pub const fn emits_langfuse_attrs(&self) -> bool {
        matches!(self.vocabulary, Vocabulary::Langfuse)
    }

    /// Whether payload bodies may be written.
    #[must_use]
    pub const fn emits_bodies(&self) -> bool {
        self.content.allows_bodies()
    }

    /// Whether content-derived structural metadata (lengths, counts) may be
    /// written. Suppressed only under [`ContentCapture::None`].
    #[must_use]
    pub const fn emits_content_metadata(&self) -> bool {
        !matches!(self.content, ContentCapture::None)
    }
}

impl Default for ExportProfile {
    fn default() -> Self {
        Self::otel_generic()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn langfuse_profile_carries_conversation_content() {
        let profile = ExportProfile::langfuse();
        assert!(profile.emits_bodies());
        assert!(profile.emits_langfuse_attrs());
        assert!(profile.emit_user_id);
        assert!(profile.emit_usage);
    }

    #[test]
    fn generic_otel_profile_withholds_conversation_content() {
        // The whole point of the split: prompt bodies must not reach an APM.
        let profile = ExportProfile::otel_generic();
        assert!(!profile.emits_bodies());
        assert!(!profile.emits_langfuse_attrs());
        assert!(
            !profile.emit_user_id,
            "user id is high-cardinality in an APM"
        );
        assert!(
            !profile.emit_session_id,
            "channel session keys contain account and peer identifiers"
        );
        // Structural metadata is still useful for latency and error analysis.
        assert!(profile.emits_content_metadata());
    }

    #[test]
    fn datadog_profile_also_drops_tags() {
        let profile = ExportProfile::datadog();
        assert!(!profile.emits_bodies());
        assert!(!profile.emit_tags, "Datadog bills on tag cardinality");
        assert!(!profile.emit_session_id);
    }

    #[test]
    fn content_none_suppresses_even_derived_metadata() {
        let profile = ExportProfile {
            content: ContentCapture::None,
            ..ExportProfile::otel_generic()
        };
        assert!(!profile.emits_bodies());
        assert!(!profile.emits_content_metadata());
    }

    #[test]
    fn default_profile_is_the_conservative_one() {
        // Defaulting to Full would leak conversation content to any newly
        // configured backend that forgot to set a profile.
        assert_eq!(ExportProfile::default(), ExportProfile::otel_generic());
        assert!(!ExportProfile::default().emits_bodies());
    }

    #[test]
    fn langfuse_allows_larger_attributes_than_an_apm() {
        assert!(
            ExportProfile::langfuse().max_attribute_bytes
                > ExportProfile::otel_generic().max_attribute_bytes
        );
    }
}
