use serde_json::Value;

use super::*;

fn apply_public_context_ceiling(params: &mut Value) -> Result<(), String> {
    tool_policy::parse_request_tool_policy(params)?;
    tool_policy::apply_untrusted_request_context(params);
    params["channel"] = serde_json::json!({ "sender_id": "web" });
    Ok(())
}

impl LiveChatService {
    /// Apply the public-channel ceiling to web/API turns whose session can
    /// deliver its response to a channel. Native channel requests already
    /// carry a `channel` object and are authorized by the gateway.
    ///
    /// This applies to *every* non-native caller, including cron jobs, webhooks,
    /// and the `sessions_send` tool — a channel-bound session's history contains
    /// messages the bot did not author, so a turn over it cannot be trusted with
    /// tools or owner-private context no matter who scheduled it. The downgrade
    /// is logged because it is otherwise invisible: an affected cron job simply
    /// answers as if it had no tools. Clear the binding
    /// (`sessions.patch { channelBinding: null }`) to restore a session that was
    /// attached to a channel and no longer needs to be.
    pub(super) async fn apply_channel_bound_public_context(
        &self,
        params: &mut Value,
        session_key: &str,
    ) -> Result<bool, String> {
        if params
            .get("_native_channel_request")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Ok(false);
        }
        let Some(entry) = self.session_metadata.get(session_key).await else {
            return Ok(false);
        };
        let Some(binding_json) = entry.channel_binding.as_deref() else {
            return Ok(false);
        };

        apply_public_context_ceiling(params)?;
        tracing::info!(
            session = session_key,
            "session is bound to a channel; running this turn without tools or \
             private context. Clear the binding to restore full access."
        );

        match serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json) {
            Ok(target) => {
                params["channel"]["channel_type"] =
                    Value::String(target.channel_type.as_str().to_string());
                let active_session = self
                    .session_metadata
                    .get_active_session(
                        target.channel_type.as_str(),
                        &target.account_id,
                        &target.chat_id,
                        target.thread_id.as_deref(),
                    )
                    .await;
                let is_active = active_session.as_deref().map_or_else(
                    || target.default_session_key() == session_key,
                    |active| active == session_key,
                );
                if is_active {
                    params["_channel_reply_target"] =
                        serde_json::to_value(&target).map_err(|error| {
                            format!("failed to serialize channel reply target: {error}")
                        })?;
                } else if let Some(object) = params.as_object_mut() {
                    object.remove("_channel_reply_target");
                }
            },
            Err(error) => {
                if let Some(object) = params.as_object_mut() {
                    object.remove("_channel_reply_target");
                }
                tracing::warn!(
                    session = session_key,
                    %error,
                    "channel-bound request has an invalid reply target; keeping public context"
                );
            },
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::apply_public_context_ceiling;

    #[test]
    fn public_ceiling_overrides_caller_supplied_private_access() {
        let mut params = serde_json::json!({
            "_private_context": true,
            "_tool_policy": {"allow": ["*"]},
        });

        assert!(apply_public_context_ceiling(&mut params).is_ok());

        assert_eq!(params["_private_context"], false);
        assert_eq!(params["channel"]["sender_id"], "web");
        assert_eq!(params["_tool_audience"], "public");
        assert_eq!(params["_tool_policy"]["deny"], serde_json::json!(["*"]));
    }

    #[test]
    fn public_ceiling_cannot_be_widened_by_request_policy() {
        let mut params = serde_json::json!({
            "_tool_policy": {"allow": ["calc"]},
        });

        assert!(apply_public_context_ceiling(&mut params).is_ok());

        assert_eq!(params["_tool_audience"], "public");
        assert_eq!(params["_tool_policy"]["deny"], serde_json::json!(["*"]));

        let mut deny_all = serde_json::json!({
            "_tool_policy": {"deny": ["*"]},
        });
        assert!(apply_public_context_ceiling(&mut deny_all).is_ok());
        assert_eq!(deny_all["_tool_policy"]["deny"], serde_json::json!(["*"]));
    }
}
