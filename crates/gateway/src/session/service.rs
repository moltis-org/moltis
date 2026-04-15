use super::*;

fn default_channel_session_key(target: &moltis_channels::ChannelReplyTarget) -> String {
    match &target.thread_id {
        Some(thread_id) => format!(
            "{}:{}:{}:{}",
            target.channel_type, target.account_id, target.chat_id, thread_id
        ),
        None => format!(
            "{}:{}:{}",
            target.channel_type, target.account_id, target.chat_id
        ),
    }
}

async fn is_current_channel_session(
    metadata: &SqliteSessionMetadata,
    entry: &moltis_sessions::metadata::SessionEntry,
) -> bool {
    let Some(binding_json) = entry.channel_binding.as_deref() else {
        return false;
    };
    let Ok(target) = serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
    else {
        return false;
    };

    let active_key = metadata
        .get_active_session(
            target.channel_type.as_str(),
            &target.account_id,
            &target.chat_id,
            target.thread_id.as_deref(),
        )
        .await
        .unwrap_or_else(|| default_channel_session_key(&target));
    active_key == entry.key
}

async fn is_archivable_entry(
    metadata: &SqliteSessionMetadata,
    entry: &moltis_sessions::metadata::SessionEntry,
) -> bool {
    entry.key != "main" && !is_current_channel_session(metadata, entry).await
}

/// Live session service backed by JSONL store + SQLite metadata.
pub struct LiveSessionService {
    pub(super) store: Arc<SessionStore>,
    pub(super) metadata: Arc<SqliteSessionMetadata>,
    pub(super) agent_persona_store: Option<Arc<AgentPersonaStore>>,
    pub(super) tts_service: Option<Arc<dyn TtsService>>,
    pub(super) share_store: Option<Arc<ShareStore>>,
    pub(super) sandbox_router: Option<Arc<SandboxRouter>>,
    pub(super) project_store: Option<Arc<dyn ProjectStore>>,
    pub(super) hook_registry: Option<Arc<HookRegistry>>,
    pub(super) state_store: Option<Arc<SessionStateStore>>,
    pub(super) browser_service: Option<Arc<dyn crate::services::BrowserService>>,
    #[cfg(feature = "fs-tools")]
    pub(super) fs_state: Option<FsState>,
}

impl LiveSessionService {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self {
            store,
            metadata,
            agent_persona_store: None,
            tts_service: None,
            share_store: None,
            sandbox_router: None,
            project_store: None,
            hook_registry: None,
            state_store: None,
            browser_service: None,
            #[cfg(feature = "fs-tools")]
            fs_state: None,
        }
    }

    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    pub fn with_agent_persona_store(mut self, store: Arc<AgentPersonaStore>) -> Self {
        self.agent_persona_store = Some(store);
        self
    }

    pub fn with_tts_service(mut self, tts: Arc<dyn TtsService>) -> Self {
        self.tts_service = Some(tts);
        self
    }

    pub fn with_share_store(mut self, store: Arc<ShareStore>) -> Self {
        self.share_store = Some(store);
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

    pub fn with_browser_service(
        mut self,
        browser: Arc<dyn crate::services::BrowserService>,
    ) -> Self {
        self.browser_service = Some(browser);
        self
    }

    #[cfg(feature = "fs-tools")]
    pub fn with_fs_state(mut self, fs_state: FsState) -> Self {
        self.fs_state = Some(fs_state);
        self
    }

    pub(super) async fn default_agent_id(&self) -> String {
        if let Some(ref store) = self.agent_persona_store {
            return store
                .default_id()
                .await
                .unwrap_or_else(|_| "main".to_string());
        }
        "main".to_string()
    }

    pub(super) async fn resolve_agent_id_for_entry(
        &self,
        entry: &moltis_sessions::metadata::SessionEntry,
        patch_if_invalid: bool,
    ) -> String {
        let fallback = self.default_agent_id().await;
        let Some(agent_id) = entry
            .agent_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return fallback;
        };

        if agent_id == "main" {
            return "main".to_string();
        }

        if let Some(ref store) = self.agent_persona_store {
            match store.get(agent_id).await {
                Ok(Some(_)) => {
                    return agent_id.to_string();
                },
                Ok(None) => {
                    warn!(
                        session = %entry.key,
                        agent_id,
                        fallback = %fallback,
                        "session references unknown agent, falling back to default"
                    );
                },
                Err(error) => {
                    warn!(
                        session = %entry.key,
                        agent_id,
                        fallback = %fallback,
                        %error,
                        "failed to resolve session agent, falling back to default"
                    );
                },
            }
        } else {
            return agent_id.to_string();
        }

        if patch_if_invalid {
            let _ = self
                .metadata
                .set_agent_id(&entry.key, Some(&fallback))
                .await;
        }
        fallback
    }

    async fn ensure_entry_agent_id(
        &self,
        key: &str,
        inherit_from_key: Option<&str>,
    ) -> Option<moltis_sessions::metadata::SessionEntry> {
        let entry = self.metadata.get(key).await?;
        if entry
            .agent_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
        {
            let effective = self.resolve_agent_id_for_entry(&entry, true).await;
            if entry.agent_id.as_deref() == Some(effective.as_str()) {
                return Some(entry);
            }
            let mut updated = entry;
            updated.agent_id = Some(effective);
            return Some(updated);
        }

        let fallback = if let Some(parent_key) = inherit_from_key {
            if let Some(parent) = self.metadata.get(parent_key).await {
                self.resolve_agent_id_for_entry(&parent, false).await
            } else {
                self.default_agent_id().await
            }
        } else {
            self.default_agent_id().await
        };

        let _ = self.metadata.set_agent_id(key, Some(&fallback)).await;
        self.metadata.get(key).await
    }
}

#[async_trait]
impl SessionService for LiveSessionService {
    async fn voice_generate(&self, params: Value) -> ServiceResult {
        self.voice_generate_impl(params).await
    }

    async fn share_create(&self, params: Value) -> ServiceResult {
        self.share_create_impl(params).await
    }

    async fn share_list(&self, params: Value) -> ServiceResult {
        self.share_list_impl(params).await
    }

    async fn share_revoke(&self, params: Value) -> ServiceResult {
        self.share_revoke_impl(params).await
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        self.delete_impl(params).await
    }

    async fn search(&self, params: Value) -> ServiceResult {
        self.search_impl(params).await
    }

    async fn fork(&self, params: Value) -> ServiceResult {
        self.fork_impl(params).await
    }

    async fn branches(&self, params: Value) -> ServiceResult {
        self.branches_impl(params).await
    }

    async fn run_detail(&self, params: Value) -> ServiceResult {
        self.run_detail_impl(params).await
    }

    async fn clear_all(&self) -> ServiceResult {
        self.clear_all_impl().await
    }

    async fn mark_seen(&self, key: &str) {
        self.mark_seen_impl(key).await;
    }

    async fn list(&self) -> ServiceResult {
        let all = self.metadata.list().await;

        let mut entries: Vec<Value> = Vec::with_capacity(all.len());
        for mut e in all {
            let agent_id = self.resolve_agent_id_for_entry(&e, false).await;
            // Check if this session is the active one for its channel binding.
            let active_channel = is_current_channel_session(&self.metadata, &e).await;

            // Backfill preview for sessions that have messages but no preview yet.
            if e.preview.is_none()
                && e.message_count > 0
                && let Ok(history) = self.store.read(&e.key).await
            {
                let new_preview = extract_preview(&history);
                if let Some(ref preview) = new_preview {
                    self.metadata.set_preview(&e.key, Some(preview)).await;
                    e.preview = new_preview;
                }
            }

            let preview = e
                .preview
                .as_deref()
                .map(|p| truncate_preview(p, SESSION_PREVIEW_MAX_CHARS));

            entries.push(serde_json::json!({
                "id": e.id,
                "key": e.key,
                "label": e.label,
                "model": e.model,
                "createdAt": e.created_at,
                "updatedAt": e.updated_at,
                "messageCount": e.message_count,
                "lastSeenMessageCount": e.last_seen_message_count,
                "projectId": e.project_id,
                "sandbox_enabled": e.sandbox_enabled,
                "sandbox_image": e.sandbox_image,
                "worktree_branch": e.worktree_branch,
                "channelBinding": e.channel_binding,
                "activeChannel": active_channel,
                "parentSessionKey": e.parent_session_key,
                "forkPoint": e.fork_point,
                "mcpDisabled": e.mcp_disabled,
                "preview": preview,
                "archived": e.archived,
                "agent_id": agent_id,
                "agentId": agent_id,
                "node_id": e.node_id,
                "version": e.version,
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
            .map_err(ServiceError::message)?;
        Ok(serde_json::json!({ "messages": filter_ui_history(messages) }))
    }

    async fn resolve(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let include_history = params
            .get("include_history")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let inherit_from_key = params
            .get("inherit_agent_from")
            .and_then(|v| v.as_str())
            .filter(|value| !value.trim().is_empty());

        self.metadata
            .upsert(key, None)
            .await
            .map_err(ServiceError::message)?;
        let entry = self
            .ensure_entry_agent_id(key, inherit_from_key)
            .await
            .ok_or_else(|| format!("session '{key}' not found after resolve"))?;
        if !include_history {
            if entry.message_count == 0
                && let Some(ref hooks) = self.hook_registry
            {
                let channel = resolve_hook_channel_binding(key, Some(&entry));
                let payload = moltis_common::hooks::HookPayload::SessionStart {
                    session_key: key.to_string(),
                    channel,
                };
                if let Err(e) = hooks.dispatch(&payload).await {
                    warn!(session = %key, error = %e, "SessionStart hook failed");
                }
            }

            return Ok(serde_json::json!({
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
                    "agent_id": entry.agent_id,
                    "agentId": entry.agent_id,
                    "node_id": entry.node_id,
                    "version": entry.version,
                },
                "history": [],
                "historyTruncated": false,
                "historyDroppedCount": 0,
            }));
        }

        let raw_history = self.store.read(key).await.map_err(ServiceError::message)?;

        // Recompute preview from combined messages every time resolve runs,
        // so sessions get the latest multi-message preview algorithm.
        if !raw_history.is_empty() {
            let new_preview = extract_preview(&raw_history);
            if new_preview.as_deref() != entry.preview.as_deref() {
                self.metadata.set_preview(key, new_preview.as_deref()).await;
            }
        }

        // Dispatch SessionStart hook for newly created sessions (empty history).
        if raw_history.is_empty()
            && let Some(ref hooks) = self.hook_registry
        {
            let channel = resolve_hook_channel_binding(key, Some(&entry));
            let payload = moltis_common::hooks::HookPayload::SessionStart {
                session_key: key.to_string(),
                channel,
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %key, error = %e, "SessionStart hook failed");
            }
        }

        let (history, dropped_count) = trim_ui_history(filter_ui_history(raw_history));

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
                "agent_id": entry.agent_id,
                "agentId": entry.agent_id,
                "node_id": entry.node_id,
                "version": entry.version,
            },
            "history": history,
            "historyTruncated": dropped_count > 0,
            "historyDroppedCount": dropped_count,
        }))
    }

    async fn patch(&self, params: Value) -> ServiceResult {
        let p: PatchParams = parse_params(params)?;
        let key = &p.key;

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found"))?;
        if p.archived == Some(true) && !is_archivable_entry(&self.metadata, &entry).await {
            return Err(ServiceError::message(format!(
                "session '{key}' cannot be archived"
            )));
        }
        if p.label.is_some() {
            let _ = self.metadata.upsert(key, p.label).await;
        }
        if p.model.is_some() {
            self.metadata.set_model(key, p.model).await;
        }
        if let Some(archived) = p.archived {
            self.metadata.set_archived(key, archived).await;
        }
        if let Some(project_id_opt) = p.project_id {
            let project_id = project_id_opt.filter(|s| !s.is_empty());
            self.metadata.set_project_id(key, project_id).await;
        }
        if let Some(worktree_branch_opt) = p.worktree_branch {
            let worktree_branch = worktree_branch_opt.filter(|s| !s.is_empty());
            self.metadata
                .set_worktree_branch(key, worktree_branch)
                .await;
        }
        if let Some(sandbox_image_opt) = p.sandbox_image {
            let sandbox_image = sandbox_image_opt.filter(|s| !s.is_empty());
            self.metadata
                .set_sandbox_image(key, sandbox_image.clone())
                .await;
            if let Some(ref router) = self.sandbox_router {
                if let Some(ref img) = sandbox_image {
                    router.set_image_override(key, img.clone()).await;
                } else {
                    router.remove_image_override(key).await;
                }
            }
        }
        if let Some(mcp_disabled) = p.mcp_disabled {
            self.metadata.set_mcp_disabled(key, mcp_disabled).await;
        }
        if let Some(sandbox_enabled_opt) = p.sandbox_enabled {
            let old_sandbox = entry.sandbox_enabled;
            self.metadata
                .set_sandbox_enabled(key, sandbox_enabled_opt)
                .await;
            if let Some(ref router) = self.sandbox_router {
                if let Some(enabled) = sandbox_enabled_opt {
                    router.set_override(key, enabled).await;
                } else {
                    router.remove_override(key).await;
                }
            }
            // Notify the LLM when sandbox state actually changes.
            if old_sandbox != sandbox_enabled_opt {
                let notification = if sandbox_enabled_opt == Some(false) {
                    "Sandbox has been disabled for this session. The `exec` tool now runs \
                     commands directly on the host machine. Previous command outputs in this \
                     conversation may have come from a sandboxed Linux container with a \
                     different OS, filesystem, and environment."
                } else if sandbox_enabled_opt == Some(true) {
                    "Sandbox has been enabled for this session. The `exec` tool will now run \
                     commands inside a sandboxed container. The container has a different \
                     filesystem and environment than the host machine."
                } else {
                    "Sandbox override has been cleared for this session. The `exec` tool will \
                     use the global sandbox setting."
                };
                let msg = PersistedMessage::system(notification);
                if let Err(e) = self.store.append_typed(key, &msg).await {
                    warn!(session = key, error = %e, "failed to append sandbox state notification");
                }
            }
        }

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found after update"))?;
        Ok(serde_json::json!({
            "id": entry.id,
            "key": entry.key,
            "label": entry.label,
            "model": entry.model,
            "archived": entry.archived,
            "sandbox_enabled": entry.sandbox_enabled,
            "sandbox_image": entry.sandbox_image,
            "worktree_branch": entry.worktree_branch,
            "mcpDisabled": entry.mcp_disabled,
            "agent_id": entry.agent_id,
            "agentId": entry.agent_id,
            "node_id": entry.node_id,
            "version": entry.version,
        }))
    }

    async fn reset(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        self.store.clear(key).await.map_err(ServiceError::message)?;
        self.metadata.touch(key, 0).await;
        self.metadata.set_preview(key, None).await;

        Ok(serde_json::json!({}))
    }

    async fn compact(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}
