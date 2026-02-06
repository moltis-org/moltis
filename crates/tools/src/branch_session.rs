//! Agent tool for forking the current session into a new branch.

use std::sync::Arc;

use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
    serde_json::{Value, json},
};

/// Agent tool that forks the current session at a given message index.
pub struct BranchSessionTool {
    store: Arc<SessionStore>,
    metadata: Arc<SqliteSessionMetadata>,
}

impl BranchSessionTool {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self { store, metadata }
    }
}

#[async_trait]
impl AgentTool for BranchSessionTool {
    fn name(&self) -> &str {
        "branch_session"
    }

    fn description(&self) -> &str {
        "Fork the current session into a new branch at a given message index. \
         Messages up to fork_point are copied to the new session. \
         The new session inherits the parent's model and project."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["label"],
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Label for the new branched session"
                },
                "fork_point": {
                    "type": "integer",
                    "description": "Message index to fork at (0-based, exclusive). \
                                    Defaults to all messages."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let parent_key = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing session context"))?;

        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'label'"))?;

        let messages = self.store.read(parent_key).await?;
        let msg_count = messages.len();

        let fork_point = params
            .get("fork_point")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(msg_count);

        if fork_point > msg_count {
            anyhow::bail!("fork_point {fork_point} exceeds message count {msg_count}");
        }

        let new_key = format!("session:{}", uuid::Uuid::new_v4());
        let forked_messages: Vec<Value> = messages[..fork_point].to_vec();

        self.store
            .replace_history(&new_key, forked_messages)
            .await?;

        let entry = self
            .metadata
            .upsert(&new_key, Some(label.to_string()))
            .await
            .map_err(|e| anyhow::anyhow!("failed to create session: {e}"))?;

        self.metadata.touch(&new_key, fork_point as u32).await;

        // Inherit model and project from parent.
        if let Some(parent) = self.metadata.get(parent_key).await {
            if parent.model.is_some() {
                self.metadata.set_model(&new_key, parent.model).await;
            }
            if parent.project_id.is_some() {
                self.metadata
                    .set_project_id(&new_key, parent.project_id)
                    .await;
            }
        }

        // Set parent relationship.
        self.metadata
            .set_parent(
                &new_key,
                Some(parent_key.to_string()),
                Some(fork_point as u32),
            )
            .await;

        Ok(json!({
            "sessionKey": new_key,
            "id": entry.id,
            "label": label,
            "forkPoint": fork_point,
            "messageCount": fork_point,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (
        Arc<SessionStore>,
        Arc<SqliteSessionMetadata>,
        tempfile::TempDir,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));

        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        // Add the branch columns.
        sqlx::query("ALTER TABLE sessions ADD COLUMN parent_session_key TEXT")
            .execute(&pool)
            .await
            .ok();
        sqlx::query("ALTER TABLE sessions ADD COLUMN fork_point INTEGER")
            .execute(&pool)
            .await
            .ok();

        let metadata = Arc::new(SqliteSessionMetadata::new(pool));
        (store, metadata, tmp)
    }

    #[tokio::test]
    async fn test_branch_at_midpoint() {
        let (store, metadata, _tmp) = setup().await;
        let tool = BranchSessionTool::new(Arc::clone(&store), Arc::clone(&metadata));

        // Create parent session with 4 messages.
        let parent_key = "session:parent";
        metadata
            .upsert(parent_key, Some("Parent".into()))
            .await
            .unwrap();
        for i in 0..4 {
            store
                .append(
                    parent_key,
                    &json!({"role": "user", "content": format!("msg {i}")}),
                )
                .await
                .unwrap();
        }
        metadata.touch(parent_key, 4).await;
        metadata.set_model(parent_key, Some("gpt-4".into())).await;

        let result = tool
            .execute(json!({
                "label": "Branch at 2",
                "fork_point": 2,
                "_session_key": parent_key,
            }))
            .await
            .unwrap();

        let new_key = result["sessionKey"].as_str().unwrap();
        assert_eq!(result["forkPoint"], 2);
        assert_eq!(result["messageCount"], 2);

        // Verify the child has 2 messages.
        let child_msgs = store.read(new_key).await.unwrap();
        assert_eq!(child_msgs.len(), 2);

        // Parent still has 4 messages.
        let parent_msgs = store.read(parent_key).await.unwrap();
        assert_eq!(parent_msgs.len(), 4);

        // Child inherits model.
        let child_entry = metadata.get(new_key).await.unwrap();
        assert_eq!(child_entry.model.as_deref(), Some("gpt-4"));
    }

    #[tokio::test]
    async fn test_fork_point_beyond_count() {
        let (store, metadata, _tmp) = setup().await;
        let tool = BranchSessionTool::new(Arc::clone(&store), Arc::clone(&metadata));

        let parent_key = "session:parent2";
        metadata.upsert(parent_key, None).await.unwrap();
        store
            .append(parent_key, &json!({"role": "user", "content": "hi"}))
            .await
            .unwrap();

        let result = tool
            .execute(json!({
                "label": "Bad fork",
                "fork_point": 99,
                "_session_key": parent_key,
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fork_all_messages() {
        let (store, metadata, _tmp) = setup().await;
        let tool = BranchSessionTool::new(Arc::clone(&store), Arc::clone(&metadata));

        let parent_key = "session:parent3";
        metadata.upsert(parent_key, None).await.unwrap();
        for i in 0..3 {
            store
                .append(
                    parent_key,
                    &json!({"role": "user", "content": format!("msg {i}")}),
                )
                .await
                .unwrap();
        }

        // Default fork_point = all messages.
        let result = tool
            .execute(json!({
                "label": "Full fork",
                "_session_key": parent_key,
            }))
            .await
            .unwrap();

        let new_key = result["sessionKey"].as_str().unwrap();
        let child_msgs = store.read(new_key).await.unwrap();
        assert_eq!(child_msgs.len(), 3);
    }
}
