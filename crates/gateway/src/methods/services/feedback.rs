//! `feedback.*` RPC methods backing the thumbs up/down control in the web UI.
//!
//! The web is treated as one more channel: a reply gets a link keyed on the
//! session and the message index, and a thumb resolves that link the same way
//! a Telegram reaction does.
//!
//! `feedback.status` is a read. `feedback.submit` writes a score and is
//! authorized as a write, with the scoring identity taken from the
//! authenticated operator rather than from the request.

use {
    moltis_channels::FeedbackOutcome,
    moltis_observability::FeedbackSignal,
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
                    "enabled": config.enabled
                        && config.feedback.enabled
                        && ctx.state.instrumentation.scores_available(),
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
                let signal = match signal {
                    Some("positive") => Some(FeedbackSignal::Positive),
                    Some("negative") => Some(FeedbackSignal::Negative),
                    // Clicking an active thumb clears it.
                    Some("clear") => None,
                    other => {
                        return Err(ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            format!(
                                "signal must be \"positive\", \"negative\" or \"clear\", got {other:?}"
                            ),
                        ));
                    },
                };

                // The score is attributed to the authenticated operator, never
                // to an identity named in the request: a caller-chosen `userId`
                // would let one client overwrite or retract another's vote,
                // since the score id is derived from (trace, name, user). A
                // stable value is required for that upsert to keep working
                // across reconnects, which also rules out the connection id.
                //
                // The web surface has exactly one identity because Moltis has
                // exactly one account: a single password, no user table, and
                // passkeys registered as "owner". `AuthIdentity` carries only a
                // method and scopes, so there is no per-person id to derive
                // here — everyone who authenticates *is* the owner, and one
                // constant states that honestly rather than manufacturing a
                // per-connection identity that would break the toggle.
                //
                // If Moltis ever grows real accounts, this is the line that has
                // to change, and the score id derivation will follow.
                let user_id = moltis_channels::trace_link::WEB_CHANNEL;

                let outcome = ctx
                    .state
                    .feedback
                    .submit_signal(
                        moltis_channels::trace_link::WEB_CHANNEL,
                        moltis_channels::trace_link::WEB_CHANNEL,
                        session_key,
                        message_id,
                        signal,
                        user_id,
                        Some("web feedback".to_string()),
                        ctx.state.instrumentation.scores_available(),
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
