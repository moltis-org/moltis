//! Channel acknowledgment-reaction signals.
//!
//! Small value types shared across the chat layer (which emits them) and the
//! gateway (which forwards them to a per-session reaction controller). Kept in
//! their own module so the plugin module stays focused.

/// Terminal outcome of a channel-dispatched agent turn, used to finalize
/// acknowledgment reactions (✅ / ❌ / none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAckOutcome {
    /// The reply was delivered successfully.
    Success,
    /// The turn errored (timeout, provider failure, etc.).
    Failure,
    /// The turn was cancelled/aborted — leave no terminal marker.
    Cancelled,
}

/// A mid-turn activity signal emitted by the agent run, used to drive channel
/// acknowledgment reactions (phase emojis) and, later, live status text.
///
/// Kept intentionally small — richer phases are derived from the tool name at
/// the controller.
#[derive(Debug, Clone)]
pub enum ChannelActivity {
    /// The model is reasoning/planning before or between tool calls.
    Thinking,
    /// A tool call started; carries the tool name for phase classification.
    Tool(String),
    /// The turn finished with the given terminal outcome.
    Finished(ChannelAckOutcome),
}
