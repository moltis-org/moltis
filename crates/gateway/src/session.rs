use std::sync::Arc;

use {async_trait::async_trait, serde_json::Value, tracing::warn};

use {
    moltis_common::hooks::HookRegistry,
    moltis_projects::ProjectStore,
    moltis_sessions::{
        metadata::SqliteSessionMetadata, state_store::SessionStateStore, store::SessionStore,
    },
    moltis_tools::sandbox::SandboxRouter,
};

use crate::services::{ServiceResult, SessionService};

/// Filter out empty assistant messages from history before sending to the UI.
///
/// Empty assistant messages are persisted in the session JSONL for LLM history
/// coherence (so the model sees a complete user→assistant turn), but they
/// should not be shown in the web UI or sent to channels.
fn filter_ui_history(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .filter(|msg| {
            if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                return true;
            }
            // Keep assistant messages that have non-empty content.
            msg.get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        })
        .collect()
}

/// Live session service backed by JSONL store + SQLite metadata.
pub struct LiveSessionService {
    store: Arc<SessionStore>,
    metadata: Arc<SqliteSessionMetadata>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    project_store: Option<Arc<dyn ProjectStore>>,
    hook_registry: Option<Arc<HookRegistry>>,
    state_store: Option<Arc<SessionStateStore>>,
}

impl LiveSessionService {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self {
            store,
            metadata,
            sandbox_router: None,
            project_store: None,
            hook_registry: None,
            state_store: None,
        }
    }

    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    pub fn with_project_store(mut self, store: Arc<dyn ProjectStore>) -> Self {
        self.project_store = Some(store);
        self
    }

    pub fn with_hooks(mut self, registry: Arc<HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    pub fn with_state_store(mut self, store: Arc<SessionStateStore>) -> Self {
        self.state_store = Some(store);
        self
    }
}

#[async_trait]
impl SessionService for LiveSessionService {
    async fn list(&self) -> ServiceResult {
        let all = self.metadata.list().await;

        let mut entries: Vec<Value> = Vec::with_capacity(all.len());
        for e in all {
            // Check if this session is the active one for its channel binding.
            let active_channel = if let Some(ref binding_json) = e.channel_binding {
                if let Ok(target) =
                    serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
                {
                    self.metadata
                        .get_active_session(
                            target.channel_type.as_str(),
                            &target.account_id,
                            &target.chat_id,
                        )
                        .await
                        .map(|k| k == e.key)
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            entries.push(serde_json::json!({
                "id": e.id,
                "key": e.key,
                "label": e.label,
                "model": e.model,
                "createdAt": e.created_at,
                "updatedAt": e.updated_at,
                "messageCount": e.message_count,
                "projectId": e.project_id,
                "sandbox_enabled": e.sandbox_enabled,
                "sandbox_image": e.sandbox_image,
                "worktree_branch": e.worktree_branch,
                "channelBinding": e.channel_binding,
                "activeChannel": active_channel,
                "parentSessionKey": e.parent_session_key,
                "forkPoint": e.fork_point,
                "mcpDisabled": e.mcp_disabled,
            }));
        }
        Ok(serde_json::json!(entries))
    }

    async fn preview(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let messages = self
            .store
            .read_last_n(key, limit)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "messages": filter_ui_history(messages) }))
    }

    async fn resolve(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let entry = self
            .metadata
            .upsert(key, None)
            .await
            .map_err(|e| e.to_string())?;
        let history = self.store.read(key).await.map_err(|e| e.to_string())?;

        // Dispatch SessionStart hook for newly created sessions (empty history).
        if history.is_empty()
            && let Some(ref hooks) = self.hook_registry
        {
            let payload = moltis_common::hooks::HookPayload::SessionStart {
                session_key: key.to_string(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %key, error = %e, "SessionStart hook failed");
            }
        }

        Ok(serde_json::json!({
            "entry": {
                "id": entry.id,
                "key": entry.key,
                "label": entry.label,
                "model": entry.model,
                "createdAt": entry.created_at,
                "updatedAt": entry.updated_at,
                "messageCount": entry.message_count,
                "projectId": entry.project_id,
                "archived": entry.archived,
                "sandbox_enabled": entry.sandbox_enabled,
                "sandbox_image": entry.sandbox_image,
                "worktree_branch": entry.worktree_branch,
                "mcpDisabled": entry.mcp_disabled,
            },
            "history": filter_ui_history(history),
        }))
    }

    async fn patch(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .map(String::from);
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found"))?;
        if label.is_some() {
            if entry.channel_binding.is_some() {
                return Err("cannot rename a channel-bound session".to_string());
            }
            let _ = self.metadata.upsert(key, label).await;
        }
        if model.is_some() {
            self.metadata.set_model(key, model).await;
        }
        if params.get("project_id").is_some() {
            let project_id = params
                .get("project_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata.set_project_id(key, project_id).await;
        }
        // Update worktree_branch if provided.
        if params.get("worktree_branch").is_some() {
            let worktree_branch = params
                .get("worktree_branch")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata
                .set_worktree_branch(key, worktree_branch)
                .await;
        }

        // Update sandbox_image if provided.
        if params.get("sandbox_image").is_some() {
            let sandbox_image = params
                .get("sandbox_image")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata
                .set_sandbox_image(key, sandbox_image.clone())
                .await;
            // Push image override to sandbox router.
            if let Some(ref router) = self.sandbox_router {
                if let Some(ref img) = sandbox_image {
                    router.set_image_override(key, img.clone()).await;
                } else {
                    router.remove_image_override(key).await;
                }
            }
        }

        // Update mcp_disabled if provided.
        if params.get("mcp_disabled").is_some() {
            let mcp_disabled = params.get("mcp_disabled").and_then(|v| v.as_bool());
            self.metadata.set_mcp_disabled(key, mcp_disabled).await;
        }

        // Update sandbox_enabled if provided.
        if params.get("sandbox_enabled").is_some() {
            let sandbox_enabled = params.get("sandbox_enabled").and_then(|v| v.as_bool());
            self.metadata
                .set_sandbox_enabled(key, sandbox_enabled)
                .await;
            // Push override to sandbox router.
            if let Some(ref router) = self.sandbox_router {
                if let Some(enabled) = sandbox_enabled {
                    router.set_override(key, enabled).await;
                } else {
                    router.remove_override(key).await;
                }
            }
        }

        let entry = self.metadata.get(key).await.unwrap();
        Ok(serde_json::json!({
            "id": entry.id,
            "key": entry.key,
            "label": entry.label,
            "model": entry.model,
            "sandbox_enabled": entry.sandbox_enabled,
            "sandbox_image": entry.sandbox_image,
            "worktree_branch": entry.worktree_branch,
            "mcpDisabled": entry.mcp_disabled,
        }))
    }

    async fn reset(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        self.store.clear(key).await.map_err(|e| e.to_string())?;
        self.metadata.touch(key, 0).await;

        Ok(serde_json::json!({}))
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        if key == "main" {
            return Err("cannot delete the main session".to_string());
        }

        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check for worktree cleanup before deleting metadata.
        if let Some(entry) = self.metadata.get(key).await
            && entry.worktree_branch.is_some()
            && let Some(ref project_id) = entry.project_id
            && let Some(ref project_store) = self.project_store
            && let Ok(Some(project)) = project_store.get(project_id).await
        {
            let project_dir = &project.directory;
            let wt_dir = project_dir.join(".moltis-worktrees").join(key);

            // Safety checks unless force is set.
            if !force
                && wt_dir.exists()
                && let Ok(true) =
                    moltis_projects::WorktreeManager::has_uncommitted_changes(&wt_dir).await
            {
                return Err(
                    "worktree has uncommitted changes; use force: true to delete anyway"
                        .to_string(),
                );
            }

            // Run teardown command if configured.
            if let Some(ref cmd) = project.teardown_command
                && wt_dir.exists()
                && let Err(e) =
                    moltis_projects::WorktreeManager::run_teardown(&wt_dir, cmd, project_dir, key)
                        .await
            {
                tracing::warn!("worktree teardown failed: {e}");
            }

            if let Err(e) = moltis_projects::WorktreeManager::cleanup(project_dir, key).await {
                tracing::warn!("worktree cleanup failed: {e}");
            }
        }

        self.store.clear(key).await.map_err(|e| e.to_string())?;

        // Clean up sandbox resources for this session.
        if let Some(ref router) = self.sandbox_router
            && let Err(e) = router.cleanup_session(key).await
        {
            tracing::warn!("sandbox cleanup for session {key}: {e}");
        }

        // Cascade-delete session state.
        if let Some(ref state_store) = self.state_store
            && let Err(e) = state_store.delete_session(key).await
        {
            tracing::warn!("session state cleanup for {key}: {e}");
        }

        self.metadata.remove(key).await;

        // Dispatch SessionEnd hook (read-only).
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::SessionEnd {
                session_key: key.to_string(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %key, error = %e, "SessionEnd hook failed");
            }
        }

        Ok(serde_json::json!({}))
    }

    async fn compact(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn fork(&self, params: Value) -> ServiceResult {
        let parent_key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .map(String::from);

        let messages = self
            .store
            .read(parent_key)
            .await
            .map_err(|e| e.to_string())?;
        let msg_count = messages.len();

        let fork_point = params
            .get("forkPoint")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(msg_count);

        if fork_point > msg_count {
            return Err(format!(
                "forkPoint {fork_point} exceeds message count {msg_count}"
            ));
        }

        let new_key = format!("session:{}", uuid::Uuid::new_v4());
        let forked_messages: Vec<Value> = messages[..fork_point].to_vec();

        self.store
            .replace_history(&new_key, forked_messages)
            .await
            .map_err(|e| e.to_string())?;

        let entry = self
            .metadata
            .upsert(&new_key, label)
            .await
            .map_err(|e| e.to_string())?;

        self.metadata.touch(&new_key, fork_point as u32).await;

        // Inherit model, project, and mcp_disabled from parent.
        if let Some(parent) = self.metadata.get(parent_key).await {
            if parent.model.is_some() {
                self.metadata.set_model(&new_key, parent.model).await;
            }
            if parent.project_id.is_some() {
                self.metadata
                    .set_project_id(&new_key, parent.project_id)
                    .await;
            }
            if parent.mcp_disabled.is_some() {
                self.metadata
                    .set_mcp_disabled(&new_key, parent.mcp_disabled)
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

        Ok(serde_json::json!({
            "sessionKey": new_key,
            "id": entry.id,
            "label": entry.label,
            "forkPoint": fork_point,
            "messageCount": fork_point,
        }))
    }

    async fn branches(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let children = self.metadata.list_children(key).await;
        let items: Vec<Value> = children
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "key": e.key,
                    "label": e.label,
                    "forkPoint": e.fork_point,
                    "messageCount": e.message_count,
                    "createdAt": e.created_at,
                })
            })
            .collect();
        Ok(serde_json::json!(items))
    }

    async fn search(&self, params: Value) -> ServiceResult {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if query.is_empty() {
            return Ok(serde_json::json!([]));
        }

        let max = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let results = self
            .store
            .search(query, max)
            .await
            .map_err(|e| e.to_string())?;

        let enriched: Vec<Value> = {
            let mut out = Vec::with_capacity(results.len());
            for r in results {
                let label = self
                    .metadata
                    .get(&r.session_key)
                    .await
                    .and_then(|e| e.label);
                out.push(serde_json::json!({
                    "sessionKey": r.session_key,
                    "snippet": r.snippet,
                    "role": r.role,
                    "messageIndex": r.message_index,
                    "label": label,
                }));
            }
            out
        };

        Ok(serde_json::json!(enriched))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_ui_history_removes_empty_assistant_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi there"}),
            serde_json::json!({"role": "user", "content": "run ls"}),
            // Empty assistant after tool use — should be filtered
            serde_json::json!({"role": "assistant", "content": ""}),
            serde_json::json!({"role": "user", "content": "run pwd"}),
            serde_json::json!({"role": "assistant", "content": "here is the output"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 5);
        // The empty assistant message at index 3 should be gone.
        assert_eq!(filtered[2]["role"], "user");
        assert_eq!(filtered[2]["content"], "run ls");
        assert_eq!(filtered[3]["role"], "user");
        assert_eq!(filtered[3]["content"], "run pwd");
    }

    #[test]
    fn filter_ui_history_removes_whitespace_only_assistant() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "   \n  "}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["role"], "user");
    }

    #[test]
    fn filter_ui_history_keeps_non_empty_assistant() {
        let messages = vec![
            serde_json::json!({"role": "assistant", "content": "real response"}),
            serde_json::json!({"role": "assistant", "content": ".", "model": "gpt-4o"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_ui_history_keeps_non_assistant_roles() {
        let messages = vec![
            serde_json::json!({"role": "system", "content": ""}),
            serde_json::json!({"role": "tool", "tool_call_id": "x", "content": ""}),
            serde_json::json!({"role": "user", "content": ""}),
        ];
        // Non-assistant roles pass through even if content is empty.
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 3);
    }
}
