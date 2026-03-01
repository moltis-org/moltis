//! Session communication tools for listing, inspecting, and messaging sessions.
//!
//! These tools expose cross-session coordination primitives:
//! - `sessions_list`: list sessions with optional filtering
//! - `sessions_history`: read paginated history from a session
//! - `sessions_send`: send a message to another session (async or sync)

use std::sync::Arc;

use {async_trait::async_trait, futures::future::BoxFuture, serde_json::Value};

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
};

use crate::{
    Error,
    params::{bool_param, owned_str_param, require_str, str_param, u64_param},
};

/// Request payload for cross-session message delivery.
#[derive(Debug, Clone)]
pub struct SendToSessionRequest {
    pub key: String,
    pub message: String,
    pub wait_for_reply: bool,
    pub model: Option<String>,
}

/// Callback used by `sessions_send`.
pub type SendToSessionFn =
    Arc<dyn Fn(SendToSessionRequest) -> BoxFuture<'static, crate::Result<Value>> + Send + Sync>;

/// Tool for listing known sessions.
pub struct SessionsListTool {
    metadata: Arc<SqliteSessionMetadata>,
}

impl SessionsListTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self { metadata }
    }
}

/// Tool for reading history from a target session.
pub struct SessionsHistoryTool {
    store: Arc<SessionStore>,
    metadata: Arc<SqliteSessionMetadata>,
}

impl SessionsHistoryTool {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self { store, metadata }
    }
}

/// Tool for sending a message to another session.
pub struct SessionsSendTool {
    metadata: Arc<SqliteSessionMetadata>,
    send_fn: SendToSessionFn,
}

impl SessionsSendTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>, send_fn: SendToSessionFn) -> Self {
        Self { metadata, send_fn }
    }
}

#[async_trait]
impl AgentTool for SessionsListTool {
    fn name(&self) -> &str {
        "sessions_list"
    }

    fn description(&self) -> &str {
        "List available sessions with metadata. Supports optional text filtering and limit."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Optional substring to match against session key or label."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum sessions returned (default: 20, max: 100)."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let filter = str_param(&params, "filter").map(|v| v.to_lowercase());
        let limit = u64_param(&params, "limit", 20).min(100) as usize;

        let mut sessions: Vec<Value> = self
            .metadata
            .list()
            .await
            .into_iter()
            .filter(|entry| {
                filter.as_ref().is_none_or(|needle| {
                    let key_match = entry.key.to_lowercase().contains(needle);
                    let label_match = entry
                        .label
                        .as_ref()
                        .map(|label| label.to_lowercase().contains(needle))
                        .unwrap_or(false);
                    key_match || label_match
                })
            })
            .take(limit)
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "key": entry.key,
                    "label": entry.label,
                    "model": entry.model,
                    "messageCount": entry.message_count,
                    "createdAt": entry.created_at,
                    "updatedAt": entry.updated_at,
                    "projectId": entry.project_id,
                    "agentId": entry.agent_id,
                    "version": entry.version,
                })
            })
            .collect();
        let count = sessions.len();
        sessions.shrink_to_fit();

        Ok(serde_json::json!({
            "sessions": sessions,
            "count": count,
        }))
    }
}

#[async_trait]
impl AgentTool for SessionsHistoryTool {
    fn name(&self) -> &str {
        "sessions_history"
    }

    fn description(&self) -> &str {
        "Read paginated message history from another session."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Session key to read."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum messages to return (default: 20, max: 100)."
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip this many newest messages (default: 0)."
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let key = require_str(&params, "key")?;
        let limit = u64_param(&params, "limit", 20).min(100) as usize;
        let offset = u64_param(&params, "offset", 0) as usize;

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| Error::message(format!("session not found: {key}")))?;
        let all_messages = self
            .store
            .read(key)
            .await
            .map_err(|e| Error::message(format!("failed to read session '{key}': {e}")))?;
        let total = all_messages.len();

        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(limit);
        let messages: Vec<Value> = all_messages[start..end].to_vec();

        Ok(serde_json::json!({
            "key": key,
            "label": entry.label,
            "messages": messages,
            "totalMessages": total,
            "offset": offset,
            "count": end.saturating_sub(start),
            "hasMore": start > 0,
        }))
    }
}

#[async_trait]
impl AgentTool for SessionsSendTool {
    fn name(&self) -> &str {
        "sessions_send"
    }

    fn description(&self) -> &str {
        "Send a message to another session. Optionally wait for the target session's reply."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Session key to send to."
                },
                "message": {
                    "type": "string",
                    "description": "Message text to send."
                },
                "wait_for_reply": {
                    "type": "boolean",
                    "description": "Wait for a synchronous response from the target session."
                },
                "context": {
                    "type": "string",
                    "description": "Optional sender context prepended to the message."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override for the target session turn."
                }
            },
            "required": ["key", "message"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let key = require_str(&params, "key")?.to_string();
        let message = require_str(&params, "message")?.to_string();
        let wait_for_reply = bool_param(&params, "wait_for_reply", false)
            || bool_param(&params, "waitForReply", false);
        let context = owned_str_param(&params, &["context"]);
        let model = owned_str_param(&params, &["model"]);

        let entry = self
            .metadata
            .get(&key)
            .await
            .ok_or_else(|| Error::message(format!("session not found: {key}")))?;

        let message = if let Some(ctx) = context {
            format!("[From: {ctx}]\n\n{message}")
        } else {
            message
        };

        let result = (self.send_fn)(SendToSessionRequest {
            key: key.clone(),
            message,
            wait_for_reply,
            model,
        })
        .await?;

        Ok(serde_json::json!({
            "key": key,
            "label": entry.label,
            "sent": true,
            "waitForReply": wait_for_reply,
            "result": result,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    async fn test_pool() -> TestResult<sqlx::SqlitePool> {
        let pool = sqlx::SqlitePool::connect(":memory:").await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await?;
        SqliteSessionMetadata::init(&pool).await?;
        Ok(pool)
    }

    #[tokio::test]
    async fn sessions_list_filters_and_limits() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata.upsert("main", Some("Main".to_string())).await?;
        metadata
            .upsert("session:alpha", Some("Alpha".to_string()))
            .await?;
        metadata
            .upsert("session:beta", Some("Beta".to_string()))
            .await?;

        let tool = SessionsListTool::new(metadata);
        let result = tool
            .execute(serde_json::json!({
                "filter": "alp",
                "limit": 5
            }))
            .await?;

        assert_eq!(result["count"], 1);
        let sessions = result
            .get("sessions")
            .and_then(Value::as_array)
            .ok_or_else(|| std::io::Error::other("missing sessions array"))?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["key"], "session:alpha");
        Ok(())
    }

    #[tokio::test]
    async fn sessions_history_reads_paginated_messages() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata
            .upsert("session:history", Some("History".to_string()))
            .await?;

        let tmp = tempfile::tempdir()?;
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
        store
            .append(
                "session:history",
                &serde_json::json!({
                    "role": "user",
                    "content": "one"
                }),
            )
            .await?;
        store
            .append(
                "session:history",
                &serde_json::json!({
                    "role": "assistant",
                    "content": "two"
                }),
            )
            .await?;
        store
            .append(
                "session:history",
                &serde_json::json!({
                    "role": "user",
                    "content": "three"
                }),
            )
            .await?;

        let tool = SessionsHistoryTool::new(store, metadata);
        let result = tool
            .execute(serde_json::json!({
                "key": "session:history",
                "limit": 2
            }))
            .await?;

        assert_eq!(result["totalMessages"], 3);
        assert_eq!(result["count"], 2);
        let messages = result
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| std::io::Error::other("missing messages array"))?;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "two");
        assert_eq!(messages[1]["content"], "three");
        Ok(())
    }

    #[tokio::test]
    async fn sessions_history_rejects_missing_session() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        let tmp = tempfile::tempdir()?;
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
        let tool = SessionsHistoryTool::new(store, metadata);

        let result = tool
            .execute(serde_json::json!({
                "key": "session:missing"
            }))
            .await;
        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected missing session error"))?;
        assert!(err.to_string().contains("session not found"));
        Ok(())
    }

    #[tokio::test]
    async fn sessions_send_calls_callback_and_wraps_context() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata
            .upsert("session:target", Some("Target".to_string()))
            .await?;

        let called = Arc::new(AtomicBool::new(false));
        let called_ref = Arc::clone(&called);
        let send_fn: SendToSessionFn = Arc::new(move |req| {
            let called_ref = Arc::clone(&called_ref);
            Box::pin(async move {
                called_ref.store(true, Ordering::SeqCst);
                assert_eq!(req.key, "session:target");
                assert!(req.message.starts_with("[From: coordinator]"));
                assert!(req.wait_for_reply);
                Ok(serde_json::json!({
                    "text": "ok",
                    "inputTokens": 1,
                    "outputTokens": 1
                }))
            })
        });
        let tool = SessionsSendTool::new(metadata, send_fn);

        let result = tool
            .execute(serde_json::json!({
                "key": "session:target",
                "message": "Do work",
                "context": "coordinator",
                "wait_for_reply": true
            }))
            .await?;

        assert_eq!(result["sent"], true);
        assert_eq!(result["waitForReply"], true);
        assert_eq!(result["result"]["text"], "ok");
        assert!(called.load(Ordering::SeqCst));
        Ok(())
    }

    #[tokio::test]
    async fn sessions_send_rejects_missing_target() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        let send_fn: SendToSessionFn = Arc::new(move |_req| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "ok": true
                }))
            })
        });
        let tool = SessionsSendTool::new(metadata, send_fn);

        let result = tool
            .execute(serde_json::json!({
                "key": "session:missing",
                "message": "hello"
            }))
            .await;
        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected missing target error"))?;
        assert!(err.to_string().contains("session not found"));
        Ok(())
    }
}
