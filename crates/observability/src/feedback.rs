//! End-user feedback and its mapping onto scores.
//!
//! A reaction on a chat message is the cheapest quality signal there is: no
//! form, no rating widget, just a thumb. This module turns one into a
//! [`ScoreRecord`] without knowing anything about the channel it came from —
//! Telegram sends a raw emoji, Slack sends a shortcode, Discord sends either,
//! and all three normalize to the same signal here.

use std::collections::BTreeSet;

use crate::model::{ScoreRecord, ScoreValue, TraceId};

/// Score name used for end-user reaction feedback.
///
/// Fixed rather than configurable: it is the join key for every feedback
/// dashboard, and letting it drift per deployment makes those dashboards
/// silently stop matching.
pub const USER_FEEDBACK_SCORE: &str = "user-feedback";

/// Namespace for deriving deterministic feedback score ids.
const FEEDBACK_NAMESPACE: uuid::Uuid = uuid::uuid!("6f9f4d3a-1b2c-4e5f-8a90-0b1c2d3e4f50");

/// A feedback signal expressed by an end user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackSignal {
    /// The answer was good.
    Positive,
    /// The answer was bad.
    Negative,
}

impl FeedbackSignal {
    /// Boolean value carried to the backend.
    #[must_use]
    pub const fn score_value(self) -> bool {
        match self {
            Self::Positive => true,
            Self::Negative => false,
        }
    }
}

/// Reaction tokens that count as feedback.
///
/// Configurable because reaction conventions are team-specific: some treat 🎉
/// as praise, others use it for releases only. The defaults stay deliberately
/// narrow — a broad default vocabulary turns every celebratory emoji into a
/// quality score and quietly poisons the metric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackVocabulary {
    positive: BTreeSet<String>,
    negative: BTreeSet<String>,
}

impl Default for FeedbackVocabulary {
    fn default() -> Self {
        Self::new(DEFAULT_POSITIVE, DEFAULT_NEGATIVE)
    }
}

/// Unambiguous approval, in both raw-emoji and shortcode form.
const DEFAULT_POSITIVE: &[&str] = &[
    "\u{1f44d}", // thumbs up
    "+1",
    "thumbsup",
    "\u{2764}", // red heart
    "heart",
    "\u{1f4af}", // hundred points
    "100",
    "\u{2705}", // check mark button
    "white_check_mark",
];

/// Unambiguous disapproval.
const DEFAULT_NEGATIVE: &[&str] = &[
    "\u{1f44e}", // thumbs down
    "-1",
    "thumbsdown",
    "\u{274c}", // cross mark
    "x",
    "\u{1f621}", // enraged face
    "rage",
    "\u{1f4a9}", // pile of poo
    "poop",
];

impl FeedbackVocabulary {
    /// Build a vocabulary from explicit token lists.
    #[must_use]
    pub fn new<S: AsRef<str>>(positive: &[S], negative: &[S]) -> Self {
        Self {
            positive: positive.iter().map(|s| normalize(s.as_ref())).collect(),
            negative: negative.iter().map(|s| normalize(s.as_ref())).collect(),
        }
    }

    /// Build from configured lists, falling back to the defaults when a list is
    /// empty.
    ///
    /// Per-list rather than all-or-nothing so an operator can extend the
    /// positive side without having to restate the negative one.
    #[must_use]
    pub fn from_config(positive: &[String], negative: &[String]) -> Self {
        let default = Self::default();
        Self {
            positive: if positive.is_empty() {
                default.positive
            } else {
                positive.iter().map(|s| normalize(s)).collect()
            },
            negative: if negative.is_empty() {
                default.negative
            } else {
                negative.iter().map(|s| normalize(s)).collect()
            },
        }
    }

    /// Classify a raw reaction token, or `None` when it carries no signal.
    #[must_use]
    pub fn classify(&self, raw: &str) -> Option<FeedbackSignal> {
        let token = normalize(raw);
        if token.is_empty() {
            return None;
        }
        if self.positive.contains(&token) {
            return Some(FeedbackSignal::Positive);
        }
        if self.negative.contains(&token) {
            return Some(FeedbackSignal::Negative);
        }
        None
    }
}

/// Reduce a reaction token to a comparable form.
///
/// Handles the three shapes the channels actually send: a bare emoji
/// (Telegram, Discord), a shortcode (Slack), and a shortcode wrapped in colons
/// or carrying a skin-tone suffix. Skin tone and presentation selectors are
/// stripped so `👍🏾` and `👍` are the same signal — a preference about how an
/// emoji renders is not a different opinion about the answer.
fn normalize(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(':');
    // Slack encodes skin tone as a `::skin-tone-4` suffix on the shortcode.
    let base = trimmed.split("::").next().unwrap_or(trimmed);

    base.chars()
        .filter(|c| !is_modifier(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `c` only affects rendering rather than meaning.
const fn is_modifier(c: char) -> bool {
    matches!(
        c,
        // Variation selectors: text vs emoji presentation.
        '\u{fe0e}' | '\u{fe0f}'
        // Fitzpatrick skin tone modifiers.
        | '\u{1f3fb}'..='\u{1f3ff}'
    )
}

/// Build a score for a feedback signal against a trace.
///
/// The score id is derived from the trace, the score name and the reacting
/// user, so it is stable across submissions. Langfuse upserts on score id,
/// which makes this idempotent in exactly the way reactions need: a user who
/// switches from 👍 to 👎 overwrites their own score instead of adding a
/// second one, and a duplicate webhook delivery is a no-op rather than
/// double-counting.
#[must_use]
pub fn feedback_score(
    trace_id: &TraceId,
    signal: FeedbackSignal,
    user_id: Option<&str>,
    comment: Option<String>,
    environment: Option<String>,
) -> ScoreRecord {
    ScoreRecord {
        id: feedback_score_id(trace_id, user_id),
        trace_id: trace_id.clone(),
        observation_id: None,
        name: USER_FEEDBACK_SCORE.to_string(),
        value: ScoreValue::Boolean(signal.score_value()),
        comment,
        environment,
    }
}

/// Deterministic score id for one user's feedback on one trace.
#[must_use]
pub fn feedback_score_id(trace_id: &TraceId, user_id: Option<&str>) -> String {
    let key = format!(
        "{}|{USER_FEEDBACK_SCORE}|{}",
        trace_id.0,
        user_id.unwrap_or("anonymous")
    );
    uuid::Uuid::new_v5(&FEEDBACK_NAMESPACE, key.as_bytes()).to_string()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn vocab() -> FeedbackVocabulary {
        FeedbackVocabulary::default()
    }

    #[test]
    fn a_bare_thumb_classifies_in_both_directions() {
        assert_eq!(
            vocab().classify("\u{1f44d}"),
            Some(FeedbackSignal::Positive)
        );
        assert_eq!(
            vocab().classify("\u{1f44e}"),
            Some(FeedbackSignal::Negative)
        );
    }

    #[test]
    fn slack_shortcodes_classify_the_same_as_the_emoji() {
        // Slack sends `+1`, Telegram sends the emoji; both are one thumb up.
        assert_eq!(vocab().classify("+1"), Some(FeedbackSignal::Positive));
        assert_eq!(vocab().classify("thumbsup"), Some(FeedbackSignal::Positive));
        assert_eq!(vocab().classify("-1"), Some(FeedbackSignal::Negative));
        assert_eq!(
            vocab().classify("thumbsdown"),
            Some(FeedbackSignal::Negative)
        );
    }

    #[test]
    fn shortcodes_wrapped_in_colons_are_accepted() {
        assert_eq!(vocab().classify(":+1:"), Some(FeedbackSignal::Positive));
    }

    #[test]
    fn skin_tone_is_not_a_different_opinion() {
        // A rendering preference must not read as a different signal, or as no
        // signal at all.
        assert_eq!(
            vocab().classify("\u{1f44d}\u{1f3ff}"),
            Some(FeedbackSignal::Positive)
        );
        assert_eq!(
            vocab().classify("+1::skin-tone-4"),
            Some(FeedbackSignal::Positive)
        );
    }

    #[test]
    fn variation_selectors_do_not_defeat_matching() {
        // Telegram sends the heart with an emoji presentation selector.
        assert_eq!(
            vocab().classify("\u{2764}\u{fe0f}"),
            Some(FeedbackSignal::Positive)
        );
    }

    #[test]
    fn unrelated_reactions_carry_no_signal() {
        // A default vocabulary that swallowed every emoji would turn party
        // reactions into quality data.
        assert_eq!(vocab().classify("\u{1f389}"), None); // tada
        assert_eq!(vocab().classify("eyes"), None);
        assert_eq!(vocab().classify(""), None);
        assert_eq!(vocab().classify("   "), None);
    }

    #[test]
    fn case_is_ignored_for_shortcodes() {
        assert_eq!(vocab().classify("ThumbsUp"), Some(FeedbackSignal::Positive));
    }

    #[test]
    fn configured_lists_replace_only_the_side_they_set() {
        let custom = FeedbackVocabulary::from_config(&["\u{1f389}".to_string()], &[]);

        assert_eq!(custom.classify("\u{1f389}"), Some(FeedbackSignal::Positive));
        // The positive override must not silently wipe the negative defaults.
        assert_eq!(custom.classify("\u{1f44e}"), Some(FeedbackSignal::Negative));
        // ...but it does replace the positive defaults it overrode.
        assert_eq!(custom.classify("\u{1f44d}"), None);
    }

    #[test]
    fn empty_config_falls_back_to_the_defaults() {
        let empty = FeedbackVocabulary::from_config(&[], &[]);
        assert_eq!(empty, FeedbackVocabulary::default());
    }

    #[test]
    fn signals_map_to_booleans() {
        assert!(FeedbackSignal::Positive.score_value());
        assert!(!FeedbackSignal::Negative.score_value());
    }

    #[test]
    fn the_score_id_is_stable_for_the_same_user_and_trace() {
        let trace = TraceId("trace-1".into());
        let first = feedback_score(
            &trace,
            FeedbackSignal::Positive,
            Some("telegram:42"),
            None,
            None,
        );
        let second = feedback_score(
            &trace,
            FeedbackSignal::Negative,
            Some("telegram:42"),
            None,
            None,
        );

        // Langfuse upserts on id, so a user changing their mind must overwrite
        // rather than add a second vote.
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn different_users_score_the_same_trace_independently() {
        let trace = TraceId("trace-1".into());
        let alice = feedback_score(
            &trace,
            FeedbackSignal::Positive,
            Some("slack:alice"),
            None,
            None,
        );
        let bob = feedback_score(
            &trace,
            FeedbackSignal::Positive,
            Some("slack:bob"),
            None,
            None,
        );

        assert_ne!(alice.id, bob.id, "one user's vote overwrote another's");
    }

    #[test]
    fn the_same_user_scores_different_traces_independently() {
        let user = Some("telegram:42");
        let first = feedback_score_id(&TraceId("trace-1".into()), user);
        let second = feedback_score_id(&TraceId("trace-2".into()), user);

        assert_ne!(first, second);
    }

    #[test]
    fn a_score_carries_the_canonical_name_and_environment() {
        let score = feedback_score(
            &TraceId("trace-1".into()),
            FeedbackSignal::Positive,
            Some("web:1"),
            Some("great answer".into()),
            Some("production".into()),
        );

        assert_eq!(score.name, USER_FEEDBACK_SCORE);
        assert_eq!(score.value, ScoreValue::Boolean(true));
        assert_eq!(score.comment.as_deref(), Some("great answer"));
        assert_eq!(score.environment.as_deref(), Some("production"));
    }
}
