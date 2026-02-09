//! Agent tool for per-session persistent state.
//!
//! Provides a key-value store scoped to the current session and a namespace,
//! allowing skills and extensions to persist context across messages.

use std::sync::Arc;

use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_sessions::state_store::SessionStateStore,
    serde_json::{Value, json},
};

/// Agent tool exposing per-session key-value state operations.
pub struct SessionStateTool {
    store: Arc<SessionStateStore>,
}

impl SessionStateTool {
    pub fn new(store: Arc<SessionStateStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for SessionStateTool {
    fn name(&self) -> &str {
        "session_state"
    }

    fn description(&self) -> &str {
        "Persist and retrieve key-value state scoped to the current session and a namespace. \
         Use this to remember information across messages within a session. \
         Operations: get, set, delete, list, clear."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["operation", "namespace"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["get", "set", "delete", "list", "clear"],
                    "description": "The operation to perform"
                },
                "namespace": {
                    "type": "string",
                    "description": "Namespace to scope the state (e.g. skill name)"
                },
                "key": {
                    "type": "string",
                    "description": "The key (required for get, set, delete)"
                },
                "value": {
                    "type": "string",
                    "description": "The value to store (required for set)"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'operation' parameter"))?;

        let namespace = params
            .get("namespace")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'namespace' parameter"))?;

        let session_key = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing session context"))?;

        match operation {
            "get" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'get' requires 'key'"))?;
                let value = self.store.get(session_key, namespace, key).await?;
                Ok(json!({ "value": value }))
            },
            "set" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'set' requires 'key'"))?;
                let value = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'set' requires 'value'"))?;
                self.store.set(session_key, namespace, key, value).await?;
                Ok(json!({ "ok": true }))
            },
            "delete" => {
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'delete' requires 'key'"))?;
                let deleted = self.store.delete(session_key, namespace, key).await?;
                Ok(json!({ "deleted": deleted }))
            },
            "list" => {
                let entries = self.store.list(session_key, namespace).await?;
                let items: Vec<Value> = entries
                    .into_iter()
                    .map(|e| json!({ "key": e.key, "value": e.value }))
                    .collect();
                Ok(json!({ "entries": items }))
            },
            "clear" => {
                let count = self.store.delete_all(session_key, namespace).await?;
                Ok(json!({ "deleted": count }))
            },
            _ => anyhow::bail!("unknown operation: {operation}"),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS session_state (
                session_key TEXT NOT NULL,
                namespace   TEXT NOT NULL,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                updated_at  INTEGER NOT NULL,
                PRIMARY KEY (session_key, namespace, key)
            )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn make_tool(pool: sqlx::SqlitePool) -> SessionStateTool {
        SessionStateTool::new(Arc::new(SessionStateStore::new(pool)))
    }

    #[tokio::test]
    async fn test_set_and_get_via_tool() {
        let pool = test_pool().await;
        let tool = make_tool(pool);

        let result = tool
            .execute(json!({
                "operation": "set",
                "namespace": "my-skill",
                "key": "count",
                "value": "42",
                "_session_key": "session:1"
            }))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);

        let result = tool
            .execute(json!({
                "operation": "get",
                "namespace": "my-skill",
                "key": "count",
                "_session_key": "session:1"
            }))
            .await
            .unwrap();
        assert_eq!(result["value"], "42");
    }

    #[tokio::test]
    async fn test_list_via_tool() {
        let pool = test_pool().await;
        let tool = make_tool(pool);

        tool.execute(json!({
            "operation": "set",
            "namespace": "ns",
            "key": "a",
            "value": "1",
            "_session_key": "s1"
        }))
        .await
        .unwrap();

        tool.execute(json!({
            "operation": "set",
            "namespace": "ns",
            "key": "b",
            "value": "2",
            "_session_key": "s1"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "operation": "list",
                "namespace": "ns",
                "_session_key": "s1"
            }))
            .await
            .unwrap();

        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_clear_via_tool() {
        let pool = test_pool().await;
        let tool = make_tool(pool);

        tool.execute(json!({
            "operation": "set",
            "namespace": "ns",
            "key": "a",
            "value": "1",
            "_session_key": "s1"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "operation": "clear",
                "namespace": "ns",
                "_session_key": "s1"
            }))
            .await
            .unwrap();
        assert_eq!(result["deleted"], 1);
    }

    #[tokio::test]
    async fn test_missing_session_key() {
        let pool = test_pool().await;
        let tool = make_tool(pool);

        let result = tool
            .execute(json!({
                "operation": "get",
                "namespace": "ns",
                "key": "k"
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let pool = test_pool().await;
        let tool = make_tool(pool);

        let result = tool
            .execute(json!({
                "operation": "nope",
                "namespace": "ns",
                "_session_key": "s1"
            }))
            .await;
        assert!(result.is_err());
    }
}
