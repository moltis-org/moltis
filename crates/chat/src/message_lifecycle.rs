//! Outbound assistant hooks, shared by streaming and tool-using turns.

use std::{collections::HashSet, sync::Arc};

use {
    moltis_common::hooks::{HookAction, HookEvent, HookPayload, HookRegistry},
    tokio::sync::RwLock,
};

use crate::{runtime::ChatRuntime, types::*};

/// Do not expose drafts or stream text that a later hook could rewrite or block.
pub(crate) fn buffers_text(hooks: Option<&HookRegistry>) -> bool {
    hooks.is_some_and(|hooks| hooks.has_handlers(HookEvent::MessageSending))
}

async fn rewrite(
    hooks: Option<&HookRegistry>,
    session_key: &str,
    content: String,
) -> Result<String, String> {
    let Some(hooks) = hooks else {
        return Ok(content);
    };
    match hooks
        .dispatch(&HookPayload::MessageSending {
            session_key: session_key.into(),
            content: content.clone(),
        })
        .await
    {
        Ok(HookAction::Block(reason)) => Err(reason),
        Ok(HookAction::ModifyPayload(payload)) => {
            if let Some(text) = payload.get("content").and_then(serde_json::Value::as_str) {
                return Ok(text.into());
            }
            tracing::warn!("MessageSending modification ignored: expected content string");
            Ok(content)
        },
        Ok(HookAction::Continue) => Ok(content),
        Err(error) => {
            tracing::warn!(%error, "MessageSending hook failed; proceeding fail-open");
            Ok(content)
        },
    }
}

pub(crate) async fn prepare(
    hooks: Option<&HookRegistry>,
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    run_id: &str,
    content: String,
    client_seq: Option<u64>,
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
) -> Option<String> {
    match rewrite(hooks, session_key, content).await {
        Ok(text) => Some(text),
        Err(reason) => {
            let message = format!("Response blocked by MessageSending hook: {reason}");
            state.set_run_error(run_id, message.clone()).await;
            let error = crate::chat_error::parse_chat_error(&message, None);
            crate::agent_loop::commit_terminal_and_finish_channel_stream(
                terminal_runs,
                run_id,
                None,
            )
            .await;
            crate::channels::deliver_channel_error(state, session_key, &error).await;
            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id, "sessionKey": session_key, "state": "error",
                    "error": error, "seq": client_seq,
                }),
                BroadcastOpts::default(),
            )
            .await;
            None
        },
    }
}

pub(crate) async fn sent(hooks: Option<&HookRegistry>, session_key: &str, content: String) {
    if let Some(hooks) = hooks
        && let Err(error) = hooks
            .dispatch(&HookPayload::MessageSent {
                session_key: session_key.into(),
                content,
            })
            .await
    {
        tracing::warn!(%error, "MessageSent hook failed");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {super::*, moltis_common::hooks::HookHandler, std::sync::Mutex};

    struct Handler {
        action: Mutex<Option<HookAction>>,
        fail: bool,
        seen: Arc<Mutex<Vec<HookPayload>>>,
    }

    #[async_trait::async_trait]
    impl HookHandler for Handler {
        fn name(&self) -> &str {
            "outbound-test"
        }

        fn events(&self) -> &[HookEvent] {
            &[
                HookEvent::AgentEnd,
                HookEvent::MessageSending,
                HookEvent::MessageSent,
            ]
        }

        async fn handle(
            &self,
            _: HookEvent,
            payload: &HookPayload,
        ) -> moltis_common::Result<HookAction> {
            self.seen.lock().unwrap().push(payload.clone());
            if self.fail {
                return Err(moltis_common::Error::message("hook failed"));
            }
            Ok(if matches!(payload, HookPayload::MessageSending { .. }) {
                self.action
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap_or(HookAction::Continue)
            } else {
                HookAction::Continue
            })
        }
    }

    pub(super) fn registry(
        action: HookAction,
        fail: bool,
    ) -> (HookRegistry, Arc<Mutex<Vec<HookPayload>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = HookRegistry::new();
        hooks.register(Arc::new(Handler {
            action: Mutex::new(Some(action)),
            fail,
            seen: Arc::clone(&seen),
        }));
        (hooks, seen)
    }

    #[tokio::test]
    async fn message_lifecycle_rewrite_then_sent_uses_published_content() {
        let (hooks, seen) = registry(
            HookAction::ModifyPayload(serde_json::json!({"content": "approved"})),
            false,
        );
        assert!(buffers_text(Some(&hooks)));
        let text = rewrite(Some(&hooks), "session", "original".into())
            .await
            .unwrap();
        assert_eq!(text, "approved");
        sent(Some(&hooks), "session", text).await;
        let seen = seen.lock().unwrap();
        assert!(matches!(&seen[..], [
            HookPayload::MessageSending { session_key, content },
            HookPayload::MessageSent { session_key: sent_session, content: sent_content }
        ] if session_key == "session" && content == "original" && sent_session == session_key && sent_content == "approved"));
    }

    #[tokio::test]
    async fn message_lifecycle_block_does_not_publish() {
        let (hooks, seen) = registry(HookAction::Block("policy".into()), false);
        assert_eq!(
            rewrite(Some(&hooks), "session", "original".into()).await,
            Err("policy".into())
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn message_lifecycle_fail_open_and_empty_rewrite() {
        assert!(!buffers_text(None));
        assert!(!buffers_text(Some(&HookRegistry::new())));
        assert_eq!(
            rewrite(None, "s", "original".into()).await.unwrap(),
            "original"
        );
        for (action, fail, expected) in [
            (HookAction::Continue, false, "original"),
            (
                HookAction::ModifyPayload(serde_json::json!({"content": 123})),
                false,
                "original",
            ),
            (
                HookAction::ModifyPayload(serde_json::json!({"content": ""})),
                false,
                "",
            ),
            (HookAction::Continue, true, "original"),
        ] {
            let (hooks, _) = registry(action, fail);
            assert_eq!(
                rewrite(Some(&hooks), "s", "original".into()).await.unwrap(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn message_lifecycle_dry_run_preserves_content() {
        let (hooks, _) = registry(HookAction::Block("policy".into()), false);
        let hooks = hooks.with_dry_run(true);
        assert!(buffers_text(Some(&hooks)));
        assert_eq!(
            rewrite(Some(&hooks), "s", "original".into()).await.unwrap(),
            "original"
        );
    }
}

#[cfg(test)]
#[path = "message_lifecycle_execution.rs"]
mod execution;
