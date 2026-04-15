use super::*;

impl LiveSessionService {
    pub(super) async fn delete_impl(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        if key == "main" {
            return Err("cannot delete the main session".into());
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
                    "worktree has uncommitted changes; use force: true to delete anyway".into(),
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

        self.store.clear(key).await.map_err(ServiceError::message)?;

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

        #[cfg(feature = "fs-tools")]
        if let Some(ref fs_state) = self.fs_state {
            let mut guard = fs_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.remove_session(key);
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

        Ok(serde_json::json!({ "ok": true }))
    }

    pub(super) async fn fork_impl(&self, params: Value) -> ServiceResult {
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
            .map_err(ServiceError::message)?;
        let msg_count = messages.len();

        let fork_point = params
            .get("forkPoint")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(msg_count);

        if fork_point > msg_count {
            return Err(format!("forkPoint {fork_point} exceeds message count {msg_count}").into());
        }

        let new_key = format!("session:{}", uuid::Uuid::new_v4());
        let forked_messages: Vec<Value> = messages[..fork_point].to_vec();

        self.store
            .replace_history(&new_key, forked_messages)
            .await
            .map_err(ServiceError::message)?;

        let _entry = self
            .metadata
            .upsert(&new_key, label)
            .await
            .map_err(ServiceError::message)?;

        self.metadata.touch(&new_key, fork_point as u32).await;

        // Inherit model, project, mcp_disabled, and agent_id from parent.
        if let Some(parent) = self.metadata.get(parent_key).await {
            let parent_agent = self.resolve_agent_id_for_entry(&parent, false).await;
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
            let _ = self
                .metadata
                .set_agent_id(&new_key, Some(&parent_agent))
                .await;
            if parent.node_id.is_some() {
                let _ = self
                    .metadata
                    .set_node_id(&new_key, parent.node_id.as_deref())
                    .await;
            }
        } else {
            let default_agent = self.default_agent_id().await;
            let _ = self
                .metadata
                .set_agent_id(&new_key, Some(&default_agent))
                .await;
        }

        // Set parent relationship.
        self.metadata
            .set_parent(
                &new_key,
                Some(parent_key.to_string()),
                Some(fork_point as u32),
            )
            .await;

        // Re-fetch after all mutations to get the final version.
        let final_entry = self
            .metadata
            .get(&new_key)
            .await
            .ok_or_else(|| format!("forked session '{new_key}' not found after creation"))?;
        Ok(serde_json::json!({
            "sessionKey": new_key,
            "id": final_entry.id,
            "label": final_entry.label,
            "forkPoint": fork_point,
            "messageCount": fork_point,
            "agent_id": final_entry.agent_id,
            "agentId": final_entry.agent_id,
            "node_id": final_entry.node_id,
            "version": final_entry.version,
        }))
    }

    pub(super) async fn branches_impl(&self, params: Value) -> ServiceResult {
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

    pub(super) async fn search_impl(&self, params: Value) -> ServiceResult {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if query.is_empty() {
            return Ok(serde_json::json!([]));
        }

        let max = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let include_archived = params
            .get("includeArchived")
            .and_then(|v| v.as_bool())
            .or_else(|| params.get("include_archived").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let search_limit = if include_archived {
            max
        } else {
            max.saturating_mul(10).min(200)
        };

        let results = self
            .store
            .search(query, search_limit)
            .await
            .map_err(ServiceError::message)?;

        let enriched: Vec<Value> = {
            let mut out = Vec::with_capacity(results.len());
            for r in results {
                let (label, archived) = match self.metadata.get(&r.session_key).await {
                    Some(entry) => (entry.label, entry.archived),
                    None => (None, false),
                };
                if archived && !include_archived {
                    continue;
                }
                out.push(serde_json::json!({
                    "sessionKey": r.session_key,
                    "snippet": r.snippet,
                    "role": r.role,
                    "messageIndex": r.message_index,
                    "label": label,
                    "archived": archived,
                }));
                if out.len() >= max {
                    break;
                }
            }
            out
        };

        Ok(serde_json::json!(enriched))
    }

    pub(super) async fn mark_seen_impl(&self, key: &str) {
        self.metadata.mark_seen(key).await;
    }

    pub(super) async fn clear_all_impl(&self) -> ServiceResult {
        let all = self.metadata.list().await;
        let mut deleted = 0u32;

        for entry in &all {
            // Keep main, channel-bound (telegram etc.), and cron sessions.
            if entry.key == "main"
                || entry.channel_binding.is_some()
                || entry.key.starts_with("telegram:")
                || entry.key.starts_with("msteams:")
                || entry.key.starts_with("cron:")
            {
                continue;
            }

            // Reuse delete logic via params.
            let params = serde_json::json!({ "key": entry.key, "force": true });
            if let Err(e) = self.delete_impl(params).await {
                warn!(session = %entry.key, error = %e, "clear_all: failed to delete session");
                continue;
            }
            deleted += 1;
        }

        // Close all browser containers since all user sessions are being cleared.
        if let Some(ref browser) = self.browser_service {
            info!("closing all browser sessions after clear_all");
            browser.close_all().await;
        }

        Ok(serde_json::json!({ "deleted": deleted }))
    }

    pub(super) async fn run_detail_impl(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'sessionKey' parameter".to_string())?;
        let run_id = params
            .get("runId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'runId' parameter".to_string())?;

        let messages = self
            .store
            .read_by_run_id(session_key, run_id)
            .await
            .map_err(|e| e.to_string())?;

        // Build summary counts.
        let mut user_messages = 0u32;
        let mut tool_calls = 0u32;
        let mut assistant_messages = 0u32;

        for msg in &messages {
            match msg.get("role").and_then(|v| v.as_str()) {
                Some("user") => user_messages += 1,
                Some("assistant") => assistant_messages += 1,
                Some("tool_result") => tool_calls += 1,
                _ => {},
            }
        }

        Ok(serde_json::json!({
            "runId": run_id,
            "messages": messages,
            "summary": {
                "userMessages": user_messages,
                "toolCalls": tool_calls,
                "assistantMessages": assistant_messages,
            }
        }))
    }
}
