//! Correlation between a delivered reply and the trace that produced it.
//!
//! A reaction arrives naming a message, not a turn. Attributing it to the
//! agent run that wrote that message needs a record made at send time, because
//! by the time someone reacts the turn is long finished and nothing else in the
//! system remembers which trace a given chat message came from.
//!
//! Deliberately holds identifiers only — no message body. The link table exists
//! to answer "which trace produced this message id", and storing the text again
//! would duplicate content the message log already owns.

use async_trait::async_trait;

use crate::Result;

/// Channel type recorded for replies delivered through the web UI.
pub const WEB_CHANNEL: &str = "web";

/// A delivered reply and the trace behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceLink {
    /// Channel the reply went out on, e.g. `telegram`. [`WEB_CHANNEL`] for the
    /// web UI.
    pub channel_type: String,
    /// Account that sent the reply.
    pub account_id: String,
    /// Conversation the reply landed in.
    pub chat_id: String,
    /// The reply's own id in the channel's namespace. This is what a reaction
    /// event names.
    pub message_id: String,
    /// Trace that produced the reply.
    pub trace_id: String,
    /// Session the turn belonged to.
    pub session_key: Option<String>,
    /// Unix seconds when the link was recorded.
    pub created_at: i64,
}

/// Store for reply/trace correlation.
#[async_trait]
pub trait TraceLinkStore: Send + Sync {
    /// Record a link, replacing any existing link for the same message.
    async fn link(&self, link: TraceLink) -> Result<()>;

    /// Find the trace behind a message, if it is still on record.
    async fn lookup(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<Option<TraceLink>>;

    /// Drop links recorded before `cutoff` (unix seconds), returning how many
    /// were removed. Without this the table grows for the life of the install.
    async fn prune(&self, cutoff: i64) -> Result<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_identifiers_only() {
        // Guards against a body field creeping in: the message log already owns
        // message content, and a second copy here would widen what a feedback
        // feature retains without anyone deciding to.
        let link = TraceLink {
            channel_type: "telegram".into(),
            account_id: "bot-1".into(),
            chat_id: "chat-1".into(),
            message_id: "42".into(),
            trace_id: "trace-1".into(),
            session_key: Some("agent:main:main".into()),
            created_at: 1_700_000_000,
        };

        let rendered = format!("{link:?}");
        assert!(rendered.contains("trace-1"));
        assert!(!rendered.contains("body"));
    }
}
