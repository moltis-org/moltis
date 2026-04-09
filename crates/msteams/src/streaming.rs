//! Edit-in-place streaming for Teams messages.
//!
//! Uses the Bot Framework activity update API to progressively edit a message
//! as LLM tokens arrive, providing real-time streaming UX similar to
//! ChatGPT/Claude web interfaces.
//!
//! Protocol:
//! 1. Post initial message with accumulated text + "..."
//! 2. PATCH the same activity at throttled intervals with more text
//! 3. Final PATCH with complete text (no "..." suffix)

use std::time::{Duration, Instant};

use tracing::debug;

/// Minimum characters before posting the initial message.
pub const MIN_INITIAL_CHARS: usize = 20;

/// Default throttle between edits (Teams recommends 1.5-2s).
pub const DEFAULT_EDIT_THROTTLE: Duration = Duration::from_millis(1500);

/// Streaming suffix appended during in-progress streaming.
const STREAMING_SUFFIX: &str = " ...";

/// Manages the state of an edit-in-place streaming session.
pub struct StreamSession {
    accumulated: String,
    activity_id: Option<String>,
    last_edit: Option<Instant>,
    throttle: Duration,
    finalized: bool,
    initial_post_failed: bool,
}

impl StreamSession {
    pub fn new(throttle: Duration) -> Self {
        Self {
            accumulated: String::new(),
            activity_id: None,
            last_edit: None,
            throttle,
            finalized: false,
            initial_post_failed: false,
        }
    }

    /// Append a text delta to the accumulated text.
    pub fn push_delta(&mut self, delta: &str) {
        self.accumulated.push_str(delta);
    }

    /// Whether the session has enough text to post the initial message.
    pub fn ready_for_initial_post(&self) -> bool {
        self.activity_id.is_none()
            && !self.initial_post_failed
            && self.accumulated.len() >= MIN_INITIAL_CHARS
    }

    /// Mark that the initial post attempt failed, preventing retries on every delta.
    pub fn mark_initial_post_failed(&mut self) {
        self.initial_post_failed = true;
    }

    /// Whether enough time has passed to send an edit update.
    pub fn ready_for_edit(&self) -> bool {
        self.activity_id.is_some()
            && !self.finalized
            && self.last_edit.is_none_or(|t| t.elapsed() >= self.throttle)
    }

    /// Set the activity ID from the initial post response.
    pub fn set_activity_id(&mut self, id: String) {
        self.activity_id = Some(id);
        self.last_edit = Some(Instant::now());
    }

    /// Get the activity ID (for updates).
    pub fn activity_id(&self) -> Option<&str> {
        self.activity_id.as_deref()
    }

    /// Get accumulated text with the streaming suffix.
    pub fn text_with_suffix(&self) -> String {
        let mut text = self.accumulated.clone();
        text.push_str(STREAMING_SUFFIX);
        text
    }

    /// Get the final accumulated text (no suffix).
    pub fn final_text(&self) -> &str {
        &self.accumulated
    }

    /// Whether the stream has any accumulated text.
    pub fn has_text(&self) -> bool {
        !self.accumulated.is_empty()
    }

    /// Mark this stream session as finalized.
    pub fn finalize(&mut self) {
        self.finalized = true;
    }

    /// Record that an edit was sent.
    pub fn mark_edited(&mut self) {
        self.last_edit = Some(Instant::now());
    }

    /// Whether the session is finalized.
    pub fn is_finalized(&self) -> bool {
        self.finalized
    }
}

/// Build the initial streaming message activity.
pub fn build_initial_activity(text: &str, reply_to: Option<&str>) -> serde_json::Value {
    let mut activity = serde_json::json!({
        "type": "message",
        "text": text,
    });
    if let Some(reply_id) = reply_to {
        activity["replyToId"] = serde_json::Value::String(reply_id.to_string());
    }
    activity
}

/// Build an activity update payload for editing an existing message.
pub fn build_update_activity(activity_id: &str, text: &str) -> serde_json::Value {
    debug!(activity_id, text_len = text.len(), "building stream update");
    serde_json::json!({
        "type": "message",
        "id": activity_id,
        "text": text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lifecycle() {
        let mut session = StreamSession::new(Duration::from_millis(100));

        // Initially not ready.
        assert!(!session.ready_for_initial_post());
        assert!(!session.ready_for_edit());
        assert!(!session.has_text());

        // Push some text.
        session.push_delta("Hello, this is a test message");
        assert!(session.ready_for_initial_post());
        assert!(session.has_text());

        // Set activity ID (initial post done).
        session.set_activity_id("act-123".into());
        assert!(!session.ready_for_initial_post());
        assert_eq!(session.activity_id(), Some("act-123"));

        // Not ready for edit yet (throttle).
        assert!(!session.ready_for_edit());

        // After throttle period.
        session.last_edit = Some(Instant::now() - Duration::from_millis(200));
        assert!(session.ready_for_edit());

        // Text with suffix.
        let text = session.text_with_suffix();
        assert!(text.ends_with(" ..."));

        // Finalize.
        session.finalize();
        assert!(session.is_finalized());
        assert!(!session.ready_for_edit());
        assert!(!session.final_text().ends_with(" ..."));
    }

    #[test]
    fn initial_activity_with_reply() {
        let activity = build_initial_activity("hello", Some("msg-1"));
        assert_eq!(activity["type"], "message");
        assert_eq!(activity["text"], "hello");
        assert_eq!(activity["replyToId"], "msg-1");
    }

    #[test]
    fn initial_activity_no_reply() {
        let activity = build_initial_activity("hello", None);
        assert!(activity.get("replyToId").is_none());
    }

    #[test]
    fn update_activity_format() {
        let activity = build_update_activity("act-456", "updated text");
        assert_eq!(activity["type"], "message");
        assert_eq!(activity["id"], "act-456");
        assert_eq!(activity["text"], "updated text");
    }

    #[test]
    fn min_chars_threshold() {
        let mut session = StreamSession::new(Duration::from_secs(1));
        session.push_delta("short");
        assert!(!session.ready_for_initial_post());

        session.push_delta(" but now it is long enough");
        assert!(session.ready_for_initial_post());
    }

    #[test]
    fn initial_post_failed_prevents_retry() {
        let mut session = StreamSession::new(Duration::from_secs(1));
        session.push_delta("Hello, this is enough text for initial post");
        assert!(session.ready_for_initial_post());

        // Simulate a failed initial post.
        session.mark_initial_post_failed();
        assert!(!session.ready_for_initial_post());

        // Additional deltas should not trigger another initial post attempt.
        session.push_delta(" more text arriving");
        assert!(!session.ready_for_initial_post());
    }
}
