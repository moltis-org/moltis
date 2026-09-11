//! Hook helpers for terminal assistant-message delivery.

use {
    moltis_agents::runner::{AgentRunError, AgentRunResult},
    moltis_common::hooks::{HookRegistry, MessageSendingOutcome, dispatch_message_sending},
};

pub(crate) async fn apply_message_sending_to_agent_result(
    registry: Option<&HookRegistry>,
    session_key: &str,
    result: Result<AgentRunResult, AgentRunError>,
) -> Result<AgentRunResult, AgentRunError> {
    let mut result = result?;
    result.text = apply_message_sending_to_text(registry, session_key, &result.text)
        .await
        .map_err(|reason| {
            AgentRunError::Other(anyhow::anyhow!("blocked by MessageSending hook: {reason}"))
        })?;
    Ok(result)
}

pub(crate) async fn apply_message_sending_to_text(
    registry: Option<&HookRegistry>,
    session_key: &str,
    content: &str,
) -> Result<String, String> {
    match dispatch_message_sending(registry, session_key, content).await {
        MessageSendingOutcome::Send(content) => Ok(content),
        MessageSendingOutcome::Block(reason) => Err(reason),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use {
        async_trait::async_trait,
        moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload},
    };

    use super::*;

    struct MessageHook {
        action: &'static str,
    }

    #[async_trait]
    impl HookHandler for MessageHook {
        fn name(&self) -> &str {
            "message-hook"
        }

        fn events(&self) -> &[HookEvent] {
            static EVENTS: [HookEvent; 1] = [HookEvent::MessageSending];
            &EVENTS
        }

        async fn handle(
            &self,
            _event: HookEvent,
            _payload: &HookPayload,
        ) -> moltis_common::Result<HookAction> {
            Ok(match self.action {
                "modify" => {
                    HookAction::ModifyPayload(serde_json::json!({"content": "rewritten response"}))
                },
                "block" => HookAction::Block("response denied".into()),
                _ => HookAction::Continue,
            })
        }
    }

    #[tokio::test]
    async fn message_sending_applies_content_replacement() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(MessageHook { action: "modify" }));

        let text = apply_message_sending_to_text(Some(&registry), "main", "original").await;

        assert_eq!(text, Ok("rewritten response".to_string()));
    }

    #[tokio::test]
    async fn message_sending_honors_block() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(MessageHook { action: "block" }));

        let error = apply_message_sending_to_text(Some(&registry), "main", "original").await;

        assert_eq!(error, Err("response denied".to_string()));
    }
}
