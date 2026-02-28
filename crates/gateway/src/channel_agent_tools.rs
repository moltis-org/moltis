use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::sync::Arc,
};

use crate::services::ChannelService;

/// Agent tool that sends proactive outbound messages to configured channels.
pub struct SendMessageTool {
    channel_service: Arc<dyn ChannelService>,
}

impl SendMessageTool {
    pub fn new(channel_service: Arc<dyn ChannelService>) -> Self {
        Self { channel_service }
    }
}

fn first_non_empty_string(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn bool_or_default(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(default)
}

#[async_trait]
impl AgentTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a proactive message to any configured channel account/chat (Telegram, Discord, Teams, WhatsApp). Use this for alerts, reminders, and scheduled outreach."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["account_id", "to", "text"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Channel account identifier (for example a Telegram bot account id). Alias: channel."
                },
                "to": {
                    "type": "string",
                    "description": "Destination recipient/chat id in the target channel. Aliases: chat_id, chatId, peer_id, peerId."
                },
                "text": {
                    "type": "string",
                    "description": "Message text to send. Alias: message."
                },
                "type": {
                    "type": "string",
                    "enum": ["telegram", "discord", "msteams", "whatsapp"],
                    "description": "Optional channel type hint when account ids may overlap across channel types."
                },
                "reply_to": {
                    "type": "string",
                    "description": "Optional platform message id to thread the outbound reply. Aliases: replyTo, message_id, messageId."
                },
                "silent": {
                    "type": "boolean",
                    "description": "Send without notification when supported by the channel.",
                    "default": false
                },
                "html": {
                    "type": "boolean",
                    "description": "Treat text as pre-formatted HTML when supported by the channel.",
                    "default": false
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = first_non_empty_string(&params, &["account_id", "accountId", "channel"])
            .ok_or_else(|| {
            anyhow::anyhow!("missing required field `account_id` (alias: `channel`)")
        })?;
        let to = first_non_empty_string(&params, &["to", "chat_id", "chatId", "peer_id", "peerId"])
            .ok_or_else(|| {
                anyhow::anyhow!("missing required field `to` (aliases: `chat_id`, `peer_id`)")
            })?;
        let text = first_non_empty_string(&params, &["text", "message"])
            .ok_or_else(|| anyhow::anyhow!("missing required field `text` (alias: `message`)"))?;
        let reply_to =
            first_non_empty_string(&params, &["reply_to", "replyTo", "message_id", "messageId"]);
        let channel_type =
            first_non_empty_string(&params, &["type", "channel_type", "channelType"]);
        let silent = bool_or_default(&params, "silent", false);
        let html = bool_or_default(&params, "html", false)
            || bool_or_default(&params, "as_html", false)
            || bool_or_default(&params, "asHtml", false);

        if silent && html {
            return Err(anyhow::anyhow!(
                "invalid send options: `silent` and `html` cannot both be true"
            ));
        }

        let mut payload = json!({
            "account_id": account_id,
            "to": to,
            "text": text,
            "silent": silent,
            "html": html,
        });
        if let Some(reply_to) = reply_to {
            payload["reply_to"] = Value::String(reply_to);
        }
        if let Some(channel_type) = channel_type {
            payload["type"] = Value::String(channel_type);
        }

        self.channel_service
            .send(payload)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*, crate::services::ServiceResult, async_trait::async_trait, serde_json::json,
        tokio::sync::Mutex,
    };

    struct RecordingChannelService {
        sent: Mutex<Option<Value>>,
    }

    impl RecordingChannelService {
        fn new() -> Self {
            Self {
                sent: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl ChannelService for RecordingChannelService {
        async fn status(&self) -> ServiceResult {
            Ok(json!({}))
        }

        async fn logout(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn send(&self, params: Value) -> ServiceResult {
            *self.sent.lock().await = Some(params.clone());
            Ok(json!({ "ok": true, "echo": params }))
        }

        async fn add(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn remove(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn update(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn senders_list(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn sender_approve(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }

        async fn sender_deny(&self, _params: Value) -> ServiceResult {
            Ok(json!({}))
        }
    }

    #[tokio::test]
    async fn send_message_tool_maps_aliases_and_forwards_payload() {
        let service = Arc::new(RecordingChannelService::new());
        let tool = SendMessageTool::new(service.clone() as Arc<dyn ChannelService>);

        let out = tool
            .execute(json!({
                "channel": "bot-alpha",
                "chatId": "12345",
                "message": "ping",
                "type": "telegram",
                "replyTo": "42",
                "silent": true
            }))
            .await
            .expect("send_message execute");

        assert_eq!(out.get("ok").and_then(Value::as_bool), Some(true));
        let sent = service.sent.lock().await.clone().expect("captured payload");
        assert_eq!(
            sent,
            json!({
                "account_id": "bot-alpha",
                "to": "12345",
                "text": "ping",
                "type": "telegram",
                "reply_to": "42",
                "silent": true,
                "html": false
            })
        );
    }

    #[tokio::test]
    async fn send_message_tool_requires_text() {
        let tool = SendMessageTool::new(Arc::new(RecordingChannelService::new()));
        let err = tool
            .execute(json!({
                "account_id": "bot-alpha",
                "to": "12345"
            }))
            .await
            .expect_err("expected validation error");
        assert!(err.to_string().contains("missing required field `text`"));
    }
}
