//! Channel acknowledgment-reaction signals.
//!
//! Small value types shared across the chat layer (which emits them) and the
//! gateway (which forwards them to a per-session reaction controller). Kept in
//! their own module so the plugin module stays focused.

/// Canonical acknowledgment-reaction emoji, as Unicode glyphs.
///
/// These are the platform-neutral names the reaction controller emits. Each
/// channel adapts them at its own boundary: Slack normalizes glyphs to
/// shortcodes (its Web API rejects raw glyphs), while Matrix uses the glyph
/// itself as the reaction key. Keeping the vocabulary here — rather than
/// hardcoding one platform's spelling in the controller — is what lets the
/// controller stay channel-agnostic.
pub mod ack_emoji {
    /// Message received, work starting.
    pub const RECEIVED: &str = "👀";
    /// Turn completed successfully.
    pub const SUCCESS: &str = "✅";
    /// Turn failed.
    pub const ERROR: &str = "❌";
    /// Turn is taking a while with no activity.
    pub const STALL: &str = "⏳";
    /// Phase: web search / fetch / browsing.
    pub const WEB: &str = "🌐";
    /// Phase: shell / process execution.
    pub const SHELL: &str = "💻";
    /// Phase: editing or writing files.
    pub const EDIT: &str = "✏️";
    /// Phase: deploying / releasing.
    pub const DEPLOY: &str = "🛫";
    /// Phase: building / compiling.
    pub const BUILD: &str = "🏗️";
    /// Phase: any other tool.
    pub const TOOL: &str = "🛠️";

    /// Every canonical emoji, for exhaustive per-channel mapping tests.
    pub const ALL: &[&str] = &[
        RECEIVED, SUCCESS, ERROR, STALL, WEB, SHELL, EDIT, DEPLOY, BUILD, TOOL,
    ];
}

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
