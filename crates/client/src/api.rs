//! Typed wrappers for the RPC methods native clients use.

use std::future::Future;

use {
    moltis_protocol::{
        client_api::{
            AgentIdentity, ApprovalDecision, ChatContext, ChatSendParams,
            ExecApprovalResolveParams, ModelEntry, SessionEntry, SessionKeyParams, methods,
        },
        message::PersistedMessage,
    },
    serde::{Serialize, de::DeserializeOwned},
    serde_json::Value,
};

use crate::{connection::Connection, error::Result};

/// `{}` params for methods that take none.
#[derive(Serialize)]
struct NoParams {}

/// Anything that can perform an RPC call; typed helpers come for free.
pub trait Rpc: Sync {
    fn call(&self, method: &str, params: Value) -> impl Future<Output = Result<Value>> + Send;

    fn call_typed<P, R>(&self, method: &str, params: &P) -> impl Future<Output = Result<R>> + Send
    where
        P: Serialize + Sync + ?Sized,
        R: DeserializeOwned,
    {
        async move {
            let payload = self.call(method, serde_json::to_value(params)?).await?;
            Ok(serde_json::from_value(payload)?)
        }
    }

    fn sessions_list(&self) -> impl Future<Output = Result<Vec<SessionEntry>>> + Send {
        self.call_typed(methods::SESSIONS_LIST, &NoParams {})
    }

    /// Messages of a session, oldest first. Entries of a shape this client
    /// does not know are skipped rather than failing the whole history.
    fn chat_history(
        &self,
        session_key: &str,
    ) -> impl Future<Output = Result<Vec<PersistedMessage>>> + Send {
        let params = SessionKeyParams {
            session_key: session_key.into(),
        };
        async move {
            let raw: Vec<Value> = self.call_typed(methods::CHAT_HISTORY, &params).await?;
            Ok(raw
                .into_iter()
                .filter_map(|value| serde_json::from_value(value).ok())
                .collect())
        }
    }

    /// Start a turn; streaming arrives as `chat` events.
    fn chat_send(&self, params: &ChatSendParams) -> impl Future<Output = Result<Value>> + Send {
        async move {
            self.call(methods::CHAT_SEND, serde_json::to_value(params)?)
                .await
        }
    }

    fn chat_abort(&self, session_key: &str) -> impl Future<Output = Result<Value>> + Send {
        let params = SessionKeyParams {
            session_key: session_key.into(),
        };
        async move {
            self.call(methods::CHAT_ABORT, serde_json::to_value(params)?)
                .await
        }
    }

    fn models_list(&self) -> impl Future<Output = Result<Vec<ModelEntry>>> + Send {
        self.call_typed(methods::MODELS_LIST, &NoParams {})
    }

    /// Token usage and context window of a session.
    fn chat_context(&self, session_key: &str) -> impl Future<Output = Result<ChatContext>> + Send {
        let params = SessionKeyParams {
            session_key: session_key.into(),
        };
        async move { self.call_typed(methods::CHAT_CONTEXT, &params).await }
    }

    /// The agent's identity and the owner's name.
    fn agent_identity(&self) -> impl Future<Output = Result<AgentIdentity>> + Send {
        self.call_typed(methods::AGENT_IDENTITY_GET, &NoParams {})
    }

    fn resolve_approval(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> impl Future<Output = Result<Value>> + Send {
        let params = ExecApprovalResolveParams {
            request_id: request_id.into(),
            decision,
        };
        async move {
            self.call(
                methods::EXEC_APPROVAL_RESOLVE,
                serde_json::to_value(params)?,
            )
            .await
        }
    }
}

impl Rpc for Connection {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        Self::call(self, method, params).await
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use {super::*, serde_json::json};

    /// Records calls and answers from a fixed table.
    struct Fake {
        calls: Mutex<Vec<(String, Value)>>,
        reply: Value,
    }

    impl Rpc for Fake {
        async fn call(&self, method: &str, params: Value) -> Result<Value> {
            self.calls.lock().unwrap().push((method.into(), params));
            Ok(self.reply.clone())
        }
    }

    fn fake(reply: Value) -> Fake {
        Fake {
            calls: Mutex::new(Vec::new()),
            reply,
        }
    }

    #[tokio::test]
    async fn history_skips_unknown_shapes() {
        let rpc = fake(json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello", "model": "m"},
            {"role": "mystery", "content": 1},
        ]));
        let messages = rpc.chat_history("main").await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            rpc.calls.lock().unwrap()[0],
            ("chat.history".to_string(), json!({"sessionKey": "main"}))
        );
    }

    #[tokio::test]
    async fn send_abort_and_approvals_use_wire_names() {
        let rpc = fake(json!({"ok": true}));
        rpc.chat_send(&ChatSendParams {
            text: "yo".into(),
            session_key: Some("s".into()),
            model: Some("m".into()),
            seq: None,
        })
        .await
        .unwrap();
        rpc.chat_abort("s").await.unwrap();
        rpc.resolve_approval("r1", ApprovalDecision::Approved)
            .await
            .unwrap();
        let calls = rpc.calls.lock().unwrap().clone();
        assert_eq!(
            calls[0],
            (
                "chat.send".into(),
                json!({"text": "yo", "sessionKey": "s", "model": "m"})
            )
        );
        assert_eq!(calls[1], ("chat.abort".into(), json!({"sessionKey": "s"})));
        assert_eq!(
            calls[2],
            (
                "exec.approval.resolve".into(),
                json!({"requestId": "r1", "decision": "approved"})
            )
        );
    }

    #[tokio::test]
    async fn lists_parse() {
        let sessions = fake(json!([{"key": "main", "messageCount": 2}]))
            .sessions_list()
            .await
            .unwrap();
        assert_eq!(sessions[0].key, "main");
        let models = fake(json!([{"id": "a", "provider": "p"}]))
            .models_list()
            .await
            .unwrap();
        assert_eq!(models[0].name(), "a");
    }

    #[tokio::test]
    async fn context_and_identity_use_wire_names() {
        let rpc = fake(
            json!({"tokenUsage": {"currentTotal": 10, "contextWindow": 100}, "user_name": "Ada"}),
        );
        let context = rpc.chat_context("s").await.unwrap();
        assert_eq!(context.token_usage.fraction(), Some(0.1));
        let identity = rpc.agent_identity().await.unwrap();
        assert_eq!(identity.user_name.as_deref(), Some("Ada"));
        let calls = rpc.calls.lock().unwrap().clone();
        assert_eq!(
            calls[0],
            ("chat.context".into(), json!({"sessionKey": "s"}))
        );
        assert_eq!(calls[1], ("agent.identity.get".into(), json!({})));
    }
}
