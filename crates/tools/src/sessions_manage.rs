//! Session management tools for creating and deleting chat sessions.
//!
//! These tools expose explicit session lifecycle operations to the agent:
//! - `sessions_create`: create (or resolve) a session key
//! - `sessions_delete`: delete a session and its history

use std::sync::Arc;

use {async_trait::async_trait, futures::future::BoxFuture, serde_json::Value};

use {moltis_agents::tool_registry::AgentTool, moltis_sessions::metadata::SqliteSessionMetadata};

use crate::{
    Error,
    params::{bool_param, owned_str_param, require_str, str_param},
};

/// Request payload for session creation.
#[derive(Debug, Clone)]
pub struct CreateSessionRequest {
    pub key: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub project_id: Option<String>,
    pub inherit_agent_from: Option<String>,
}

/// Callback used by `sessions_create`.
pub type CreateSessionFn =
    Arc<dyn Fn(CreateSessionRequest) -> BoxFuture<'static, crate::Result<Value>> + Send + Sync>;

/// Request payload for session deletion.
#[derive(Debug, Clone)]
pub struct DeleteSessionRequest {
    pub key: String,
    pub force: bool,
}

/// Callback used by `sessions_delete`.
pub type DeleteSessionFn =
    Arc<dyn Fn(DeleteSessionRequest) -> BoxFuture<'static, crate::Result<Value>> + Send + Sync>;

/// Tool for creating sessions.
pub struct SessionsCreateTool {
    metadata: Arc<SqliteSessionMetadata>,
    create_fn: CreateSessionFn,
}

impl SessionsCreateTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>, create_fn: CreateSessionFn) -> Self {
        Self {
            metadata,
            create_fn,
        }
    }
}

/// Tool for deleting sessions.
pub struct SessionsDeleteTool {
    metadata: Arc<SqliteSessionMetadata>,
    delete_fn: DeleteSessionFn,
}

impl SessionsDeleteTool {
    pub fn new(metadata: Arc<SqliteSessionMetadata>, delete_fn: DeleteSessionFn) -> Self {
        Self {
            metadata,
            delete_fn,
        }
    }
}

#[async_trait]
impl AgentTool for SessionsCreateTool {
    fn name(&self) -> &str {
        "sessions_create"
    }

    fn description(&self) -> &str {
        "Create a new chat session or resolve an existing one. \
         Optionally set label/model/project and inherit agent persona from another session."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Session key to create. If omitted, a new key is generated."
                },
                "label": {
                    "type": "string",
                    "description": "Optional session label."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override for the session."
                },
                "project_id": {
                    "type": "string",
                    "description": "Optional project ID to associate with the session."
                },
                "inherit_agent_from": {
                    "type": "string",
                    "description": "Optional source session key to inherit agent persona from."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let key = str_param(&params, "key")
            .map(String::from)
            .unwrap_or_else(|| format!("session:{}", uuid::Uuid::new_v4()));

        let label = owned_str_param(&params, &["label"]);
        let model = owned_str_param(&params, &["model"]);
        let project_id = owned_str_param(&params, &["project_id", "projectId"]);
        let inherit_agent_from =
            owned_str_param(&params, &["inherit_agent_from", "inheritAgentFrom"]);

        let created = self.metadata.get(&key).await.is_none();

        let req = CreateSessionRequest {
            key: key.clone(),
            label,
            model,
            project_id,
            inherit_agent_from,
        };
        let result = (self.create_fn)(req).await?;

        Ok(serde_json::json!({
            "key": key,
            "created": created,
            "result": result,
        }))
    }
}

#[async_trait]
impl AgentTool for SessionsDeleteTool {
    fn name(&self) -> &str {
        "sessions_delete"
    }

    fn description(&self) -> &str {
        "Delete a chat session and its history by key. \
         Deleting the main session is not allowed."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Session key to delete."
                },
                "force": {
                    "type": "boolean",
                    "description": "Force deletion for sessions with worktree checks (default: false)."
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let key = require_str(&params, "key")?;
        let force = bool_param(&params, "force", false);

        if key == "main" {
            return Err(Error::message("cannot delete the main session").into());
        }

        if self.metadata.get(key).await.is_none() {
            return Err(Error::message(format!("session not found: {key}")).into());
        }

        let req = DeleteSessionRequest {
            key: key.to_string(),
            force,
        };
        let result = (self.delete_fn)(req).await?;

        Ok(serde_json::json!({
            "key": key,
            "deleted": true,
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
    async fn sessions_create_generates_key_when_missing() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        let called = Arc::new(AtomicBool::new(false));
        let called_ref = Arc::clone(&called);

        let create_fn: CreateSessionFn = Arc::new(move |req| {
            let called_ref = Arc::clone(&called_ref);
            Box::pin(async move {
                called_ref.store(true, Ordering::SeqCst);
                Ok(serde_json::json!({
                    "entry": { "key": req.key }
                }))
            })
        });

        let tool = SessionsCreateTool::new(metadata, create_fn);

        let result = tool
            .execute(serde_json::json!({
                "label": "Worker session"
            }))
            .await?;

        let key = result
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| std::io::Error::other("missing key in create response"))?;
        assert!(key.starts_with("session:"));
        assert_eq!(result["created"], true);
        assert!(called.load(Ordering::SeqCst));
        Ok(())
    }

    #[tokio::test]
    async fn sessions_create_marks_existing_session_as_not_created() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata
            .upsert("session:existing", Some("Existing".to_string()))
            .await?;

        let create_fn: CreateSessionFn = Arc::new(move |req| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "entry": { "key": req.key }
                }))
            })
        });

        let tool = SessionsCreateTool::new(Arc::clone(&metadata), create_fn);
        let result = tool
            .execute(serde_json::json!({
                "key": "session:existing"
            }))
            .await?;

        assert_eq!(result["created"], false);
        assert_eq!(result["key"], "session:existing");
        Ok(())
    }

    #[tokio::test]
    async fn sessions_delete_deletes_existing_session() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata
            .upsert("session:to-delete", Some("Delete me".to_string()))
            .await?;

        let called = Arc::new(AtomicBool::new(false));
        let called_ref = Arc::clone(&called);
        let delete_fn: DeleteSessionFn = Arc::new(move |req| {
            let called_ref = Arc::clone(&called_ref);
            Box::pin(async move {
                assert_eq!(req.key, "session:to-delete");
                assert!(req.force);
                called_ref.store(true, Ordering::SeqCst);
                Ok(serde_json::json!({ "ok": true }))
            })
        });

        let tool = SessionsDeleteTool::new(metadata, delete_fn);
        let result = tool
            .execute(serde_json::json!({
                "key": "session:to-delete",
                "force": true
            }))
            .await?;

        assert_eq!(result["deleted"], true);
        assert!(called.load(Ordering::SeqCst));
        Ok(())
    }

    #[tokio::test]
    async fn sessions_delete_rejects_missing_session() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        let delete_fn: DeleteSessionFn =
            Arc::new(move |_req| Box::pin(async move { Ok(serde_json::json!({ "ok": true })) }));

        let tool = SessionsDeleteTool::new(metadata, delete_fn);
        let result = tool
            .execute(serde_json::json!({
                "key": "session:missing"
            }))
            .await;

        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected missing-session delete to fail"))?;
        assert!(err.to_string().contains("session not found"));
        Ok(())
    }

    #[tokio::test]
    async fn sessions_delete_rejects_main_session() -> TestResult<()> {
        let metadata = Arc::new(SqliteSessionMetadata::new(test_pool().await?));
        metadata.upsert("main", Some("Main".to_string())).await?;

        let delete_fn: DeleteSessionFn =
            Arc::new(move |_req| Box::pin(async move { Ok(serde_json::json!({ "ok": true })) }));

        let tool = SessionsDeleteTool::new(metadata, delete_fn);
        let result = tool
            .execute(serde_json::json!({
                "key": "main"
            }))
            .await;

        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected main-session delete to fail"))?;
        assert!(err.to_string().contains("cannot delete the main session"));
        Ok(())
    }
}
