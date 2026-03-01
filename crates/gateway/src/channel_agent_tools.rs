use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::sync::Arc,
};

use crate::services::ChannelService;

/// Agent tool that sends proactive outbound messages to configured channels.
///
/// Validation and alias resolution are handled by the underlying
/// [`ChannelService::send`] implementation; this tool only provides the
/// LLM-facing schema and forwards the parameters.
pub struct SendMessageTool {
    channel_service: Arc<dyn ChannelService>,
}

impl SendMessageTool {
    pub fn new(channel_service: Arc<dyn ChannelService>) -> Self {
        Self { channel_service }
    }
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
        self.channel_service
            .send(params)
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
    async fn send_message_tool_forwards_params_to_channel_service() {
        let service = Arc::new(RecordingChannelService::new());
        let tool = SendMessageTool::new(service.clone() as Arc<dyn ChannelService>);

        let input = json!({
            "account_id": "bot-alpha",
            "to": "12345",
            "text": "ping",
            "type": "telegram",
            "reply_to": "42",
            "silent": true
        });
        let out = tool
            .execute(input.clone())
            .await
            .expect("send_message execute");

        assert_eq!(out.get("ok").and_then(Value::as_bool), Some(true));
        let sent = service.sent.lock().await.clone().expect("captured payload");
        assert_eq!(sent, input);
    }

    #[tokio::test]
    async fn send_message_tool_propagates_service_errors() {
        use crate::services::ServiceError;

        struct FailingChannelService;

        #[async_trait]
        impl ChannelService for FailingChannelService {
            async fn status(&self) -> ServiceResult {
                Ok(json!({}))
            }

            async fn logout(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn send(&self, _: Value) -> ServiceResult {
                Err(ServiceError::message("missing 'text' (or alias 'message')"))
            }

            async fn add(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn remove(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn update(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn senders_list(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn sender_approve(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }

            async fn sender_deny(&self, _: Value) -> ServiceResult {
                Ok(json!({}))
            }
        }

        let tool = SendMessageTool::new(Arc::new(FailingChannelService));
        let err = tool
            .execute(json!({
                "account_id": "bot-alpha",
                "to": "12345"
            }))
            .await
            .expect_err("expected validation error");
        assert!(err.to_string().contains("missing"));
    }
}
