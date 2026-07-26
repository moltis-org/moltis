//! `feedback.*` RPC methods backing the thumbs up/down control in the web UI.
//!
//! The web is treated as one more channel: a reply gets a link keyed on the
//! session and the message index, and a thumb resolves that link the same way
//! a Telegram reaction does.

use {
    moltis_channels::FeedbackOutcome,
    moltis_protocol::{ErrorShape, error_codes},
};

use super::MethodRegistry;

/// Register the `feedback.*` namespace.
pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "feedback.status",
        Box::new(|ctx| {
            Box::pin(async move {
                let config = &ctx.state.config.instrumentation;
                Ok(serde_json::json!({
                    // The control is only useful when something is collecting
                    // the result, so the UI hides it unless both are on.
                    "enabled": config.enabled && config.feedback.enabled,
                    "instrumentation_active": ctx.state.instrumentation.status().active,
                    "retention_days": config.feedback.link_retention_days,
                }))
            })
        }),
    );

    reg.register(
        "feedback.submit",
        Box::new(|ctx| {
            Box::pin(async move {
                let session_key = ctx
                    .params
                    .get("sessionKey")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if session_key.is_empty() {
                    return Err(ErrorShape::new(
                        error_codes::INVALID_REQUEST,
                        "sessionKey is required",
                    ));
                }

                let message_id = ctx
                    .params
                    .get("messageId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if message_id.is_empty() {
                    return Err(ErrorShape::new(
                        error_codes::INVALID_REQUEST,
                        "messageId is required",
                    ));
                }

                // `signal` rather than a raw score so the wire format cannot
                // smuggle an arbitrary value into the metric.
                let signal = ctx.params.get("signal").and_then(|v| v.as_str());
                let (emoji, added) = match signal {
                    Some("positive") => ("\u{1f44d}", true),
                    Some("negative") => ("\u{1f44e}", true),
                    // Clicking an active thumb clears it.
                    Some("clear") => ("\u{1f44d}", false),
                    other => {
                        return Err(ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            format!(
                                "signal must be \"positive\", \"negative\" or \"clear\", got {other:?}"
                            ),
                        ));
                    },
                };

                let user_id = ctx
                    .params
                    .get("userId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("web");

                let outcome = ctx
                    .state
                    .feedback
                    .on_reaction(
                        moltis_channels::trace_link::WEB_CHANNEL,
                        moltis_channels::trace_link::WEB_CHANNEL,
                        session_key,
                        message_id,
                        emoji,
                        user_id,
                        added,
                        ctx.state.instrumentation.langfuse().as_ref(),
                    )
                    .await;

                Ok(serde_json::json!({
                    "ok": matches!(
                        outcome,
                        FeedbackOutcome::Recorded(_) | FeedbackOutcome::Retracted
                    ),
                    "outcome": match outcome {
                        FeedbackOutcome::Recorded(_) => "recorded",
                        FeedbackOutcome::Retracted => "retracted",
                        FeedbackOutcome::NotFeedback => "not_feedback",
                        // The turn is older than the retention window, or
                        // predates instrumentation being switched on.
                        FeedbackOutcome::UnknownMessage => "unknown_message",
                        FeedbackOutcome::Disabled => "disabled",
                    },
                }))
            })
        }),
    );
}
