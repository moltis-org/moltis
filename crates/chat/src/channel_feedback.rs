//! Correlating delivered replies with the traces that produced them.
//!
//! Feedback attribution is recorded at send time because nothing else in the
//! system remembers which agent run wrote a given chat message, and by the
//! time someone reacts the turn is long finished.

use std::sync::Arc;

use crate::runtime::ChatRuntime;

/// Link a web-delivered assistant message to the trace that produced it.
///
/// The web is treated as one more channel so a thumb in the browser resolves
/// through the same correlation table as a Telegram reaction. The run id is
/// the message identity: unlike a message index it does not shift when older
/// history is compacted away.
pub(crate) async fn record_web_reply_trace(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    run_id: &str,
) {
    let Some(feedback) = state.feedback() else {
        return;
    };
    let Some(trace_id) = moltis_observability::recent_trace(session_key) else {
        return;
    };
    feedback
        .record_reply(
            moltis_channels::trace_link::WEB_CHANNEL,
            moltis_channels::trace_link::WEB_CHANNEL,
            session_key,
            std::slice::from_ref(&run_id.to_string()),
            &trace_id,
            Some(session_key),
        )
        .await;
}

/// Link the messages a reply produced to the trace that generated it.
///
/// Best-effort by design: a missing link costs feedback attribution for one
/// reply, and the user already has the message.
pub(crate) async fn record_reply_trace(
    state: &Arc<dyn ChatRuntime>,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    message_ids: &[String],
    session_key: &str,
) {
    if message_ids.is_empty() {
        return;
    }
    let Some(feedback) = state.feedback() else {
        return;
    };
    let Some(trace_id) = moltis_observability::recent_trace(session_key) else {
        return;
    };
    feedback
        .record_reply(
            target.channel_type.as_str(),
            &target.account_id,
            to,
            message_ids,
            &trace_id,
            Some(session_key),
        )
        .await;
}
