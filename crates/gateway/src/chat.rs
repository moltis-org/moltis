use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::{
        sync::{OwnedSemaphorePermit, RwLock, Semaphore},
        task::AbortHandle,
    },
    tokio_stream::StreamExt,
    tracing::{debug, info, warn},
};

use moltis_config::MessageQueueMode;

use {
    moltis_agents::{
        AgentRunError,
        model::StreamEvent,
        prompt::{build_system_prompt_minimal, build_system_prompt_with_session},
        providers::ProviderRegistry,
        runner::{RunnerEvent, run_agent_loop_streaming},
        tool_registry::ToolRegistry,
    },
    moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
    moltis_skills::discover::SkillDiscoverer,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    chat_error::parse_chat_error,
    services::{ChatService, ModelService, ServiceResult},
    state::GatewayState,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Disabled Models Store ────────────────────────────────────────────────────

/// Persistent store for disabled model IDs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisabledModelsStore {
    #[serde(default)]
    pub disabled: HashSet<String>,
}

impl DisabledModelsStore {
    fn config_path() -> Option<PathBuf> {
        moltis_config::config_dir().map(|d| d.join("disabled-models.json"))
    }

    /// Load disabled models from config file.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Save disabled models to config file.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Disable a model by ID.
    pub fn disable(&mut self, model_id: &str) -> bool {
        self.disabled.insert(model_id.to_string())
    }

    /// Enable a model by ID (remove from disabled set).
    pub fn enable(&mut self, model_id: &str) -> bool {
        self.disabled.remove(model_id)
    }

    /// Check if a model is disabled.
    pub fn is_disabled(&self, model_id: &str) -> bool {
        self.disabled.contains(model_id)
    }
}

// ── LiveModelService ────────────────────────────────────────────────────────

pub struct LiveModelService {
    providers: Arc<RwLock<ProviderRegistry>>,
    disabled: Arc<RwLock<DisabledModelsStore>>,
}

impl LiveModelService {
    pub fn new(providers: Arc<RwLock<ProviderRegistry>>) -> Self {
        Self {
            providers,
            disabled: Arc::new(RwLock::new(DisabledModelsStore::load())),
        }
    }
}

#[async_trait]
impl ModelService for LiveModelService {
    async fn list(&self) -> ServiceResult {
        let reg = self.providers.read().await;
        let disabled = self.disabled.read().await;
        let models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| !disabled.is_disabled(&m.id))
            .map(|m| {
                let supports_tools = reg.get(&m.id).is_some_and(|p| p.supports_tools());
                serde_json::json!({
                    "id": m.id,
                    "provider": m.provider,
                    "displayName": m.display_name,
                    "supportsTools": supports_tools,
                })
            })
            .collect();
        Ok(serde_json::json!(models))
    }

    async fn disable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "disabling model");

        let mut disabled = self.disabled.write().await;
        disabled.disable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "enabling model");

        let mut disabled = self.disabled.write().await;
        disabled.enable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }
}

// ── LiveChatService ─────────────────────────────────────────────────────────

/// A message that arrived while an agent run was already active on the session.
#[derive(Debug, Clone)]
struct QueuedMessage {
    params: Value,
}

pub struct LiveChatService {
    providers: Arc<RwLock<ProviderRegistry>>,
    state: Arc<GatewayState>,
    active_runs: Arc<RwLock<HashMap<String, AbortHandle>>>,
    tool_registry: Arc<RwLock<ToolRegistry>>,
    session_store: Arc<SessionStore>,
    session_metadata: Arc<SqliteSessionMetadata>,
    hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    /// Per-session semaphore ensuring only one agent run executes per session at a time.
    session_locks: Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
    /// Per-session message queue for messages arriving during an active run.
    message_queue: Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    /// Failover configuration for automatic model/provider failover.
    failover_config: moltis_config::schema::FailoverConfig,
}

impl LiveChatService {
    pub fn new(
        providers: Arc<RwLock<ProviderRegistry>>,
        state: Arc<GatewayState>,
        session_store: Arc<SessionStore>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            providers,
            state,
            active_runs: Arc::new(RwLock::new(HashMap::new())),
            tool_registry: Arc::new(RwLock::new(ToolRegistry::new())),
            session_store,
            session_metadata,
            hook_registry: None,
            session_locks: Arc::new(RwLock::new(HashMap::new())),
            message_queue: Arc::new(RwLock::new(HashMap::new())),
            failover_config: moltis_config::schema::FailoverConfig::default(),
        }
    }

    pub fn with_failover(mut self, config: moltis_config::schema::FailoverConfig) -> Self {
        self.failover_config = config;
        self
    }

    pub fn with_tools(mut self, registry: Arc<RwLock<ToolRegistry>>) -> Self {
        self.tool_registry = registry;
        self
    }

    pub fn with_hooks(mut self, registry: moltis_common::hooks::HookRegistry) -> Self {
        self.hook_registry = Some(Arc::new(registry));
        self
    }

    pub fn with_hooks_arc(mut self, registry: Arc<moltis_common::hooks::HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    fn has_tools_sync(&self) -> bool {
        // Best-effort check: try_read avoids blocking. If the lock is held,
        // assume tools are present (conservative — enables tool mode).
        self.tool_registry
            .try_read()
            .map(|r| !r.list_schemas().is_empty())
            .unwrap_or(true)
    }

    /// Return the per-session semaphore, creating one if absent.
    async fn session_semaphore(&self, key: &str) -> Arc<Semaphore> {
        // Fast path: read lock.
        {
            let locks = self.session_locks.read().await;
            if let Some(sem) = locks.get(key) {
                return Arc::clone(sem);
            }
        }
        // Slow path: write lock, insert.
        let mut locks = self.session_locks.write().await;
        Arc::clone(
            locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    /// Resolve a provider from session metadata, history, or first registered.
    async fn resolve_provider(
        &self,
        session_key: &str,
        history: &[serde_json::Value],
    ) -> Result<Arc<dyn moltis_agents::model::LlmProvider>, String> {
        let reg = self.providers.read().await;
        let session_model = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|e| e.model.clone());
        let history_model = history
            .iter()
            .rev()
            .find_map(|m| m.get("model").and_then(|v| v.as_str()).map(String::from));
        let model_id = session_model.or(history_model);

        model_id
            .and_then(|id| reg.get(&id))
            .or_else(|| reg.first())
            .ok_or_else(|| "no LLM providers configured".to_string())
    }

    /// Resolve the active session key for a connection.
    async fn session_key_for(&self, conn_id: Option<&str>) -> String {
        if let Some(cid) = conn_id {
            let sessions = self.state.active_sessions.read().await;
            if let Some(key) = sessions.get(cid) {
                return key.clone();
            }
        }
        "main".to_string()
    }
}

#[async_trait]
impl ChatService for LiveChatService {
    async fn send(&self, params: Value) -> ServiceResult {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'text' parameter".to_string())?
            .to_string();

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let explicit_model = params.get("model").and_then(|v| v.as_str());
        // Use streaming-only mode if explicitly requested or if no tools are registered.
        let explicit_stream_only = params
            .get("stream_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let stream_only = explicit_stream_only || !self.has_tools_sync();

        // Resolve session key: explicit override (used by cron callbacks) or connection-scoped lookup.
        let session_key = match params.get("_session_key").and_then(|v| v.as_str()) {
            Some(sk) => sk.to_string(),
            None => self.session_key_for(conn_id.as_deref()).await,
        };

        // Resolve model: explicit param → session metadata → first registered.
        let session_model = if explicit_model.is_none() {
            self.session_metadata
                .get(&session_key)
                .await
                .and_then(|e| e.model)
        } else {
            None
        };
        let model_id = explicit_model.or(session_model.as_deref());

        let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
            let reg = self.providers.read().await;
            let primary = if let Some(id) = model_id {
                reg.get(id).ok_or_else(|| {
                    let available: Vec<_> =
                        reg.list_models().iter().map(|m| m.id.clone()).collect();
                    format!("model '{}' not found. available: {:?}", id, available)
                })?
            } else if !stream_only {
                reg.first_with_tools()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            } else {
                reg.first()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            };

            if self.failover_config.enabled {
                let fallbacks = if self.failover_config.fallback_models.is_empty() {
                    // Auto-build: same model on other providers first, then same
                    // provider's other models, then everything else.
                    reg.fallback_providers_for(primary.id(), primary.name())
                } else {
                    reg.providers_for_models(&self.failover_config.fallback_models)
                };
                if fallbacks.is_empty() {
                    primary
                } else {
                    let mut chain = vec![primary];
                    chain.extend(fallbacks);
                    Arc::new(moltis_agents::provider_chain::ProviderChain::new(chain))
                }
            } else {
                primary
            }
        };

        // Resolve project context for this connection's active project.
        let project_context = {
            let project_id = if let Some(cid) = conn_id.as_deref() {
                let projects = self.state.active_projects.read().await;
                projects.get(cid).cloned()
            } else {
                None
            };
            // Also check session metadata for project binding.
            let project_id = if project_id.is_some() {
                project_id
            } else {
                self.session_metadata
                    .get(&session_key)
                    .await
                    .and_then(|e| e.project_id)
            };
            if let Some(pid) = project_id {
                match self
                    .state
                    .services
                    .project
                    .get(serde_json::json!({"id": pid}))
                    .await
                {
                    Ok(val) => {
                        if let Some(dir) = val.get("directory").and_then(|v| v.as_str()) {
                            match moltis_projects::context::load_context_files(
                                std::path::Path::new(dir),
                            ) {
                                Ok(files) => {
                                    let project: Option<moltis_projects::Project> =
                                        serde_json::from_value(val.clone()).ok();
                                    if let Some(p) = project {
                                        // Resolve worktree dir from session metadata.
                                        let worktree_dir = self
                                            .session_metadata
                                            .get(&session_key)
                                            .await
                                            .and_then(|e| e.worktree_branch)
                                            .and_then(|_| {
                                                let wt_path = std::path::Path::new(dir)
                                                    .join(".moltis-worktrees")
                                                    .join(&session_key);
                                                if wt_path.exists() {
                                                    Some(wt_path)
                                                } else {
                                                    None
                                                }
                                            });
                                        let ctx = moltis_projects::ProjectContext {
                                            project: p,
                                            context_files: files,
                                            worktree_dir,
                                        };
                                        Some(ctx.to_prompt_section())
                                    } else {
                                        None
                                    }
                                },
                                Err(e) => {
                                    warn!("failed to load project context: {e}");
                                    None
                                },
                            }
                        } else {
                            None
                        }
                    },
                    Err(_) => None,
                }
            } else {
                None
            }
        };

        // Dispatch MessageReceived hook (read-only).
        if let Some(ref hooks) = self.hook_registry {
            let channel = params
                .get("channel")
                .and_then(|v| v.as_str())
                .map(String::from);
            let payload = moltis_common::hooks::HookPayload::MessageReceived {
                session_key: session_key.clone(),
                content: text.clone(),
                channel,
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "MessageReceived hook failed");
            }
        }

        // Persist the user message (with optional channel metadata for UI display).
        let channel_meta = params.get("channel").cloned();
        let mut user_msg =
            serde_json::json!({"role": "user", "content": &text, "created_at": now_ms()});
        if let Some(ch) = &channel_meta {
            user_msg["channel"] = ch.clone();
        }
        if let Err(e) = self.session_store.append(&session_key, &user_msg).await {
            warn!("failed to persist user message: {e}");
        }

        // Load conversation history excluding the user message we just appended
        // (both run_streaming and run_agent_loop add the current user message themselves).
        let mut history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        // Pop the last message (the one we just appended).
        if !history.is_empty() {
            history.pop();
        }

        // Update metadata.
        let _ = self.session_metadata.upsert(&session_key, None).await;
        self.session_metadata
            .touch(&session_key, history.len() as u32)
            .await;

        // If this is a web UI message on a channel-bound session, echo the
        // user message to the channel and register a reply target so the LLM
        // response is also delivered there.
        let is_web_message = conn_id.is_some()
            && params.get("_session_key").is_none()
            && params.get("channel").is_none();

        if is_web_message
            && let Some(entry) = self.session_metadata.get(&session_key).await
            && let Some(ref binding_json) = entry.channel_binding
            && let Ok(target) =
                serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
        {
            // Only echo to channel if this is the active session for this chat.
            let is_active = self
                .session_metadata
                .get_active_session(&target.channel_type, &target.account_id, &target.chat_id)
                .await
                .map(|k| k == session_key)
                .unwrap_or(true);

            if is_active {
                // Push reply target so deliver_channel_replies sends the LLM response.
                self.state
                    .push_channel_reply(&session_key, target.clone())
                    .await;
            }
        }

        // Discover enabled skills/plugins for prompt injection.
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
        let discovered_skills = match discoverer.discover().await {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to discover skills: {e}");
                Vec::new()
            },
        };

        // Check if MCP tools are disabled for this session.
        let mcp_disabled = self
            .session_metadata
            .get(&session_key)
            .await
            .and_then(|e| e.mcp_disabled)
            .unwrap_or(false);

        let run_id = uuid::Uuid::new_v4().to_string();
        let state = Arc::clone(&self.state);
        let active_runs = Arc::clone(&self.active_runs);
        let run_id_clone = run_id.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let hook_registry = self.hook_registry.clone();

        // Log if tool mode is active but the provider doesn't support tools.
        // Note: We don't broadcast to the user here - they chose the model knowing
        // its limitations. The UI should show capabilities when selecting a model.
        if !stream_only && !provider.supports_tools() {
            debug!(
                provider = provider.name(),
                model = provider.id(),
                "selected provider does not support tool calling"
            );
        }

        info!(
            run_id = %run_id,
            user_message = %text,
            model = provider.id(),
            stream_only,
            session = %session_key,
            "chat.send"
        );

        // Capture user message index (0-based) so we can include assistant
        // message index in the "final" broadcast for client-side deduplication.
        let user_message_index = history.len(); // user msg is at this index in the JSONL

        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        let session_store = Arc::clone(&self.session_store);
        let session_metadata = Arc::clone(&self.session_metadata);
        let session_key_clone = session_key.clone();
        let accept_language = params
            .get("_accept_language")
            .and_then(|v| v.as_str())
            .map(String::from);
        // Compute session context stats for the system prompt.
        let session_stats = {
            let msg_count = history.len() + 1; // +1 for the current user message
            let total_input: u64 = history
                .iter()
                .filter_map(|m| m.get("inputTokens").and_then(|v| v.as_u64()))
                .sum();
            let total_output: u64 = history
                .iter()
                .filter_map(|m| m.get("outputTokens").and_then(|v| v.as_u64()))
                .sum();
            let total_tokens = total_input + total_output;

            format!(
                "Session \"{session_key}\": {msg_count} messages, {total_tokens} tokens used ({total_input} input / {total_output} output)."
            )
        };

        // Auto-compact: if conversation input tokens exceed 95% of context window, compact first.
        let context_window = provider.context_window() as u64;
        let total_input: u64 = history
            .iter()
            .filter_map(|m| m.get("inputTokens").and_then(|v| v.as_u64()))
            .sum();
        let compact_threshold = (context_window * 95) / 100;

        if total_input >= compact_threshold {
            let pre_compact_msg_count = history.len();
            let total_output: u64 = history
                .iter()
                .filter_map(|m| m.get("outputTokens").and_then(|v| v.as_u64()))
                .sum();
            let pre_compact_total = total_input + total_output;

            info!(
                session = %session_key,
                total_input,
                context_window,
                "auto-compact triggered (95% threshold reached)"
            );
            broadcast(
                &self.state,
                "chat",
                serde_json::json!({
                    "sessionKey": session_key,
                    "state": "auto_compact",
                    "phase": "start",
                    "messageCount": pre_compact_msg_count,
                    "totalTokens": pre_compact_total,
                    "inputTokens": total_input,
                    "outputTokens": total_output,
                    "contextWindow": context_window,
                }),
                BroadcastOpts::default(),
            )
            .await;

            let compact_params = serde_json::json!({ "_conn_id": conn_id });
            match self.compact(compact_params).await {
                Ok(_) => {
                    // Reload history after compaction.
                    history = self
                        .session_store
                        .read(&session_key)
                        .await
                        .unwrap_or_default();
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "done",
                            "messageCount": pre_compact_msg_count,
                            "totalTokens": pre_compact_total,
                            "contextWindow": context_window,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                Err(e) => {
                    warn!(session = %session_key, error = %e, "auto-compact failed, proceeding with full history");
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "error",
                            "error": e.to_string(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
            }
        }

        // Try to acquire the per-session semaphore.  If a run is already active,
        // queue the message according to the configured MessageQueueMode instead
        // of blocking the caller.
        let session_sem = self.session_semaphore(&session_key).await;
        let permit: OwnedSemaphorePermit = match session_sem.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                // Active run — enqueue and return immediately.
                let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                info!(
                    session = %session_key,
                    mode = ?queue_mode,
                    "queueing message (run active)"
                );
                self.message_queue
                    .write()
                    .await
                    .entry(session_key.clone())
                    .or_default()
                    .push(QueuedMessage {
                        params: params.clone(),
                    });
                broadcast(
                    &self.state,
                    "chat",
                    serde_json::json!({
                        "sessionKey": session_key,
                        "state": "queued",
                        "mode": format!("{queue_mode:?}").to_lowercase(),
                    }),
                    BroadcastOpts::default(),
                )
                .await;
                return Ok(serde_json::json!({
                    "queued": true,
                    "mode": format!("{queue_mode:?}").to_lowercase(),
                }));
            },
        };

        let agent_timeout_secs = moltis_config::discover_and_load().tools.agent_timeout_secs;

        let message_queue = Arc::clone(&self.message_queue);
        let state_for_drain = Arc::clone(&self.state);

        let handle = tokio::spawn(async move {
            let _permit = permit; // hold permit until task completes
            let ctx_ref = project_context.as_deref();
            let stats_ref = Some(session_stats.as_str());
            let agent_fut = async {
                if stream_only {
                    run_streaming(
                        &state,
                        &run_id_clone,
                        provider,
                        &text,
                        &provider_name,
                        &history,
                        &session_key_clone,
                        ctx_ref,
                        stats_ref,
                        user_message_index,
                        &discovered_skills,
                    )
                    .await
                } else {
                    run_with_tools(
                        &state,
                        &run_id_clone,
                        provider,
                        &tool_registry,
                        &text,
                        &provider_name,
                        &history,
                        &session_key_clone,
                        ctx_ref,
                        stats_ref,
                        user_message_index,
                        &discovered_skills,
                        hook_registry,
                        accept_language.clone(),
                        Some(&session_store),
                        mcp_disabled,
                    )
                    .await
                }
            };

            let assistant_text = if agent_timeout_secs > 0 {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(agent_timeout_secs),
                    agent_fut,
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        warn!(
                            run_id = %run_id_clone,
                            session = %session_key_clone,
                            timeout_secs = agent_timeout_secs,
                            "agent run timed out"
                        );
                        let error_obj = serde_json::json!({
                            "type": "timeout",
                            "message": format!(
                                "Agent run timed out after {agent_timeout_secs}s"
                            ),
                        });
                        broadcast(
                            &state,
                            "chat",
                            serde_json::json!({
                                "runId": run_id_clone,
                                "sessionKey": session_key_clone,
                                "state": "error",
                                "error": error_obj,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                        None
                    },
                }
            } else {
                agent_fut.await
            };

            // Persist assistant response.
            if let Some((response_text, input_tokens, output_tokens)) = assistant_text {
                let assistant_msg = serde_json::json!({"role": "assistant", "content": response_text, "model": model_id, "provider": provider_name, "inputTokens": input_tokens, "outputTokens": output_tokens, "created_at": now_ms()});
                if let Err(e) = session_store
                    .append(&session_key_clone, &assistant_msg)
                    .await
                {
                    warn!("failed to persist assistant message: {e}");
                }
                // Update metadata counts.
                if let Ok(count) = session_store.count(&session_key_clone).await {
                    session_metadata.touch(&session_key_clone, count).await;
                }
            }

            active_runs.write().await.remove(&run_id_clone);

            // Drain queued messages for this session.
            let queued = message_queue
                .write()
                .await
                .remove(&session_key_clone)
                .unwrap_or_default();
            if !queued.is_empty() {
                let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                let chat = state_for_drain.chat().await;
                match queue_mode {
                    MessageQueueMode::Followup => {
                        for msg in queued {
                            info!(session = %session_key_clone, "replaying queued message (followup)");
                            if let Err(e) = chat.send(msg.params).await {
                                warn!(session = %session_key_clone, error = %e, "failed to replay queued message");
                            }
                        }
                    },
                    MessageQueueMode::Collect => {
                        let combined: Vec<&str> = queued
                            .iter()
                            .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
                            .collect();
                        if !combined.is_empty() {
                            info!(
                                session = %session_key_clone,
                                count = combined.len(),
                                "replaying collected messages"
                            );
                            // Use the last queued message as the base params, override text.
                            let mut merged = queued.last().unwrap().params.clone();
                            merged["text"] = serde_json::json!(combined.join("\n\n"));
                            if let Err(e) = chat.send(merged).await {
                                warn!(session = %session_key_clone, error = %e, "failed to replay collected messages");
                            }
                        }
                    },
                }
            }
        });

        self.active_runs
            .write()
            .await
            .insert(run_id.clone(), handle.abort_handle());

        Ok(serde_json::json!({ "runId": run_id }))
    }

    async fn send_sync(&self, params: Value) -> ServiceResult {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'text' parameter".to_string())?
            .to_string();

        let explicit_model = params.get("model").and_then(|v| v.as_str());
        let stream_only = !self.has_tools_sync();

        // Resolve session key from explicit override.
        let session_key = match params.get("_session_key").and_then(|v| v.as_str()) {
            Some(sk) => sk.to_string(),
            None => "main".to_string(),
        };

        // Resolve provider.
        let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
            let reg = self.providers.read().await;
            if let Some(id) = explicit_model {
                reg.get(id)
                    .ok_or_else(|| format!("model '{id}' not found"))?
            } else if !stream_only {
                reg.first_with_tools()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            } else {
                reg.first()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            }
        };

        // Persist the user message.
        let user_msg =
            serde_json::json!({"role": "user", "content": &text, "created_at": now_ms()});
        if let Err(e) = self.session_store.append(&session_key, &user_msg).await {
            warn!("send_sync: failed to persist user message: {e}");
        }

        // Ensure this session appears in the sessions list.
        let _ = self.session_metadata.upsert(&session_key, None).await;
        self.session_metadata.touch(&session_key, 1).await;

        // Load conversation history (excluding the message we just appended).
        let mut history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        if !history.is_empty() {
            history.pop();
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let state = Arc::clone(&self.state);
        let tool_registry = Arc::clone(&self.tool_registry);
        let hook_registry = self.hook_registry.clone();
        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        let user_message_index = history.len();

        info!(
            run_id = %run_id,
            user_message = %text,
            model = %model_id,
            stream_only,
            session = %session_key,
            "chat.send_sync"
        );

        let result = if stream_only {
            run_streaming(
                &state,
                &run_id,
                provider,
                &text,
                &provider_name,
                &history,
                &session_key,
                None,
                None,
                user_message_index,
                &[],
            )
            .await
        } else {
            run_with_tools(
                &state,
                &run_id,
                provider,
                &tool_registry,
                &text,
                &provider_name,
                &history,
                &session_key,
                None,
                None,
                user_message_index,
                &[],
                hook_registry,
                None,
                Some(&self.session_store),
                false, // send_sync: MCP tools always enabled for API calls
            )
            .await
        };

        // Persist assistant response.
        if let Some((ref response_text, input_tokens, output_tokens)) = result {
            let assistant_msg = serde_json::json!({
                "role": "assistant",
                "content": response_text,
                "model": model_id,
                "provider": provider_name,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "created_at": now_ms(),
            });
            if let Err(e) = self
                .session_store
                .append(&session_key, &assistant_msg)
                .await
            {
                warn!("send_sync: failed to persist assistant message: {e}");
            }
            // Update metadata message count.
            if let Ok(count) = self.session_store.count(&session_key).await {
                self.session_metadata.touch(&session_key, count).await;
            }
        }

        match result {
            Some((response_text, input_tokens, output_tokens)) => Ok(serde_json::json!({
                "text": response_text,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
            })),
            None => {
                // Check the last broadcast for this run to get the actual error message.
                let error_msg = state
                    .last_run_error(&run_id)
                    .await
                    .unwrap_or_else(|| "agent run failed (check server logs)".to_string());

                // Persist the error in the session so it's visible in session history.
                let error_entry = serde_json::json!({
                    "role": "system",
                    "content": format!("[error] {error_msg}"),
                    "created_at": now_ms(),
                });
                let _ = self.session_store.append(&session_key, &error_entry).await;
                // Update metadata so the session shows in the UI.
                if let Ok(count) = self.session_store.count(&session_key).await {
                    self.session_metadata.touch(&session_key, count).await;
                }

                Err(error_msg)
            },
        }
    }

    async fn abort(&self, params: Value) -> ServiceResult {
        let run_id = params
            .get("runId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'runId'".to_string())?;

        if let Some(handle) = self.active_runs.write().await.remove(run_id) {
            handle.abort();
        }
        Ok(serde_json::json!({}))
    }

    async fn history(&self, params: Value) -> ServiceResult {
        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let session_key = self.session_key_for(conn_id.as_deref()).await;
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!(messages))
    }

    async fn inject(&self, _params: Value) -> ServiceResult {
        Err("inject not yet implemented".into())
    }

    async fn clear(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        self.session_store
            .clear(&session_key)
            .await
            .map_err(|e| e.to_string())?;

        // Reset metadata message count.
        self.session_metadata.touch(&session_key, 0).await;

        info!(session = %session_key, "chat.clear");
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn compact(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        let history = self
            .session_store
            .read(&session_key)
            .await
            .map_err(|e| e.to_string())?;

        if history.is_empty() {
            return Err("nothing to compact".into());
        }

        // Dispatch BeforeCompaction hook.
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::BeforeCompaction {
                session_key: session_key.clone(),
                message_count: history.len(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "BeforeCompaction hook failed");
            }
        }

        // Run silent memory turn before summarization — saves important memories to disk.
        // Write into the data directory (e.g. ~/.moltis/) so files don't end up in cwd.
        if let Some(ref mm) = self.state.memory_manager {
            let memory_dir = moltis_config::data_dir();
            if let Ok(provider) = self.resolve_provider(&session_key, &history).await {
                match moltis_agents::silent_turn::run_silent_memory_turn(
                    provider,
                    &history,
                    &memory_dir,
                )
                .await
                {
                    Ok(paths) => {
                        for path in &paths {
                            if let Err(e) = mm.sync_path(path).await {
                                warn!(path = %path.display(), error = %e, "compact: memory sync of written file failed");
                            }
                        }
                        if !paths.is_empty() {
                            info!(
                                files = paths.len(),
                                "compact: silent memory turn wrote files"
                            );
                        }
                    },
                    Err(e) => warn!(error = %e, "compact: silent memory turn failed"),
                }
            }
        }

        // Build a summary prompt from the conversation.
        let mut summary_messages: Vec<serde_json::Value> = Vec::new();
        summary_messages.push(serde_json::json!({
            "role": "system",
            "content": "You are a conversation summarizer. Summarize the following conversation into a concise form that preserves all key facts, decisions, and context. Output only the summary, no preamble."
        }));

        let mut conversation_text = String::new();
        for msg in &history {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            conversation_text.push_str(&format!("{role}: {content}\n\n"));
        }
        summary_messages.push(serde_json::json!({
            "role": "user",
            "content": conversation_text,
        }));

        // Use the session's model if available, otherwise fall back to the model
        // from the last assistant message, then to the first registered provider.
        let provider = self.resolve_provider(&session_key, &history).await?;

        info!(session = %session_key, messages = history.len(), "chat.compact: summarizing");

        let mut stream = provider.stream(summary_messages);
        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(delta) => summary.push_str(&delta),
                StreamEvent::Done(_) => break,
                StreamEvent::Error(e) => return Err(format!("compact summarization failed: {e}")),
                // Tool events not expected in summarization stream.
                StreamEvent::ToolCallStart { .. }
                | StreamEvent::ToolCallArgumentsDelta { .. }
                | StreamEvent::ToolCallComplete { .. } => {},
            }
        }

        if summary.is_empty() {
            return Err("compact produced empty summary".into());
        }

        // Replace history with a single assistant message containing the summary.
        let compacted = vec![serde_json::json!({
            "role": "assistant",
            "content": format!("[Conversation Summary]\n\n{summary}"),
            "created_at": now_ms(),
        })];

        self.session_store
            .replace_history(&session_key, compacted.clone())
            .await
            .map_err(|e| e.to_string())?;

        self.session_metadata.touch(&session_key, 1).await;

        // Save compaction summary to memory file and trigger sync.
        if let Some(ref mm) = self.state.memory_manager {
            let memory_dir = moltis_config::data_dir().join("memory");
            if let Err(e) = tokio::fs::create_dir_all(&memory_dir).await {
                warn!(error = %e, "compact: failed to create memory dir");
            } else {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let filename = format!("compaction-{}-{ts}.md", session_key);
                let path = memory_dir.join(&filename);
                let content = format!(
                    "# Compaction Summary\n\n- **Session**: {session_key}\n- **Timestamp**: {ts}\n\n{summary}"
                );
                if let Err(e) = tokio::fs::write(&path, &content).await {
                    warn!(error = %e, "compact: failed to write memory file");
                } else {
                    let mm = Arc::clone(mm);
                    tokio::spawn(async move {
                        if let Err(e) = mm.sync().await {
                            tracing::warn!("compact: memory sync failed: {e}");
                        }
                    });
                }
            }
        }

        // Dispatch AfterCompaction hook.
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::AfterCompaction {
                session_key: session_key.clone(),
                summary_len: summary.len(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "AfterCompaction hook failed");
            }
        }

        info!(session = %session_key, "chat.compact: done");
        Ok(serde_json::json!(compacted))
    }

    async fn context(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        // Session info
        let message_count = self.session_store.count(&session_key).await.unwrap_or(0);
        let session_entry = self.session_metadata.get(&session_key).await;
        let (provider_name, supports_tools) = {
            let reg = self.providers.read().await;
            let session_model = session_entry.as_ref().and_then(|e| e.model.as_deref());
            if let Some(id) = session_model {
                let p = reg.get(id);
                (
                    p.as_ref().map(|p| p.name().to_string()),
                    p.as_ref().map(|p| p.supports_tools()).unwrap_or(true),
                )
            } else {
                let p = reg.first();
                (
                    p.as_ref().map(|p| p.name().to_string()),
                    p.as_ref().map(|p| p.supports_tools()).unwrap_or(true),
                )
            }
        };
        let session_info = serde_json::json!({
            "key": session_key,
            "messageCount": message_count,
            "model": session_entry.as_ref().and_then(|e| e.model.as_deref()),
            "provider": provider_name,
            "label": session_entry.as_ref().and_then(|e| e.label.as_deref()),
            "projectId": session_entry.as_ref().and_then(|e| e.project_id.as_deref()),
        });

        // Project info & context files
        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let project_id = if let Some(cid) = conn_id.as_deref() {
            let projects = self.state.active_projects.read().await;
            projects.get(cid).cloned()
        } else {
            None
        };
        let project_id =
            project_id.or_else(|| session_entry.as_ref().and_then(|e| e.project_id.clone()));

        let project_info = if let Some(pid) = project_id {
            match self
                .state
                .services
                .project
                .get(serde_json::json!({"id": pid}))
                .await
            {
                Ok(val) => {
                    let dir = val.get("directory").and_then(|v| v.as_str());
                    let context_files = if let Some(d) = dir {
                        match moltis_projects::context::load_context_files(std::path::Path::new(d))
                        {
                            Ok(files) => files
                                .iter()
                                .map(|f| {
                                    serde_json::json!({
                                        "path": f.path.display().to_string(),
                                        "size": f.content.len(),
                                    })
                                })
                                .collect::<Vec<_>>(),
                            Err(_) => vec![],
                        }
                    } else {
                        vec![]
                    };
                    serde_json::json!({
                        "id": val.get("id"),
                        "label": val.get("label"),
                        "directory": dir,
                        "systemPrompt": val.get("system_prompt").or(val.get("systemPrompt")),
                        "contextFiles": context_files,
                    })
                },
                Err(_) => serde_json::json!(null),
            }
        } else {
            serde_json::json!(null)
        };

        // Tools (only include if the provider supports tool calling)
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|e| e.mcp_disabled)
            .unwrap_or(false);
        let tools: Vec<serde_json::Value> = if supports_tools {
            let registry_guard = self.tool_registry.read().await;
            let filtered;
            let effective_registry: &ToolRegistry = if mcp_disabled {
                filtered = registry_guard.clone_without_mcp();
                &filtered
            } else {
                &registry_guard
            };
            effective_registry
                .list_schemas()
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "description": s.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                })
                .collect()
        } else {
            vec![]
        };

        // Token usage from actual API-reported counts stored in messages.
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let total_input: u64 = messages
            .iter()
            .filter_map(|m| m.get("inputTokens").and_then(|v| v.as_u64()))
            .sum();
        let total_output: u64 = messages
            .iter()
            .filter_map(|m| m.get("outputTokens").and_then(|v| v.as_u64()))
            .sum();
        let total_tokens = total_input + total_output;

        // Context window from the session's provider
        let context_window = {
            let reg = self.providers.read().await;
            let session_model = session_entry.as_ref().and_then(|e| e.model.as_deref());
            if let Some(id) = session_model {
                reg.get(id).map(|p| p.context_window()).unwrap_or(200_000)
            } else {
                reg.first().map(|p| p.context_window()).unwrap_or(200_000)
            }
        };

        // Sandbox info
        let sandbox_info = if let Some(ref router) = self.state.sandbox_router {
            let is_sandboxed = router.is_sandboxed(&session_key).await;
            let config = router.config();
            let session_image = session_entry.as_ref().and_then(|e| e.sandbox_image.clone());
            let effective_image = match session_image {
                Some(img) if !img.is_empty() => img,
                _ => router.default_image().await,
            };
            let container_name = {
                let id = router.sandbox_id_for(&session_key);
                format!(
                    "{}-{}",
                    config
                        .container_prefix
                        .as_deref()
                        .unwrap_or("moltis-sandbox"),
                    id.key
                )
            };
            serde_json::json!({
                "enabled": is_sandboxed,
                "backend": router.backend_name(),
                "mode": config.mode,
                "scope": config.scope,
                "workspaceMount": config.workspace_mount,
                "image": effective_image,
                "containerName": container_name,
            })
        } else {
            serde_json::json!({
                "enabled": false,
                "backend": null,
            })
        };

        // Discover enabled skills/plugins (only if provider supports tools)
        let skills_list: Vec<serde_json::Value> = if supports_tools {
            let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
            let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
            match discoverer.discover().await {
                Ok(s) => s
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "name": s.name,
                            "description": s.description,
                            "source": s.source,
                        })
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        };

        // MCP servers (only if provider supports tools)
        let mcp_servers = if supports_tools {
            self.state
                .services
                .mcp
                .list()
                .await
                .unwrap_or(serde_json::json!([]))
        } else {
            serde_json::json!([])
        };

        Ok(serde_json::json!({
            "session": session_info,
            "project": project_info,
            "tools": tools,
            "skills": skills_list,
            "mcpServers": mcp_servers,
            "mcpDisabled": mcp_disabled,
            "sandbox": sandbox_info,
            "supportsTools": supports_tools,
            "tokenUsage": {
                "inputTokens": total_input,
                "outputTokens": total_output,
                "total": total_tokens,
                "contextWindow": context_window,
            },
        }))
    }
}

// ── Agent loop mode ─────────────────────────────────────────────────────────

async fn run_with_tools(
    state: &Arc<GatewayState>,
    run_id: &str,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    text: &str,
    provider_name: &str,
    history: &[serde_json::Value],
    session_key: &str,
    project_context: Option<&str>,
    session_context: Option<&str>,
    user_message_index: usize,
    skills: &[moltis_skills::types::SkillMetadata],
    hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    accept_language: Option<String>,
    session_store: Option<&Arc<SessionStore>>,
    mcp_disabled: bool,
) -> Option<(String, u32, u32)> {
    // Load identity and user profile from config so the LLM knows who it is.
    let config = moltis_config::discover_and_load();
    let mut identity = config.identity.clone();
    if let Some(file_identity) = moltis_config::load_identity() {
        if file_identity.name.is_some() {
            identity.name = file_identity.name;
        }
        if file_identity.emoji.is_some() {
            identity.emoji = file_identity.emoji;
        }
        if file_identity.creature.is_some() {
            identity.creature = file_identity.creature;
        }
        if file_identity.vibe.is_some() {
            identity.vibe = file_identity.vibe;
        }
    }
    let mut user = config.user.clone();
    if let Some(file_user) = moltis_config::load_user() {
        if file_user.name.is_some() {
            user.name = file_user.name;
        }
        if file_user.timezone.is_some() {
            user.timezone = file_user.timezone;
        }
    }
    let soul_text = moltis_config::load_soul();
    let tools_text = moltis_config::load_tools_md();

    let native_tools = provider.supports_tools();

    // Use a minimal prompt without tool schemas for providers that don't support tools.
    // This reduces context size and avoids confusing the LLM with unusable instructions.
    let system_prompt = if native_tools {
        let registry_guard = tool_registry.read().await;
        let owned_filtered;
        let tools_for_prompt: &ToolRegistry = if mcp_disabled {
            owned_filtered = registry_guard.clone_without_mcp();
            &owned_filtered
        } else {
            &registry_guard
        };
        let prompt = build_system_prompt_with_session(
            tools_for_prompt,
            native_tools,
            project_context,
            session_context,
            skills,
            Some(&identity),
            Some(&user),
            soul_text.as_deref(),
            tools_text.as_deref(),
        );
        drop(registry_guard);
        prompt
    } else {
        // Minimal prompt without tools for local LLMs
        build_system_prompt_minimal(
            project_context,
            session_context,
            Some(&identity),
            Some(&user),
            soul_text.as_deref(),
            tools_text.as_deref(),
        )
    };

    // Broadcast tool events to the UI as they happen.
    let state_for_events = Arc::clone(state);
    let run_id_for_events = run_id.to_string();
    let session_key_for_events = session_key.to_string();
    let on_event: Box<dyn Fn(RunnerEvent) + Send + Sync> = Box::new(move |event| {
        let state = Arc::clone(&state_for_events);
        let run_id = run_id_for_events.clone();
        let sk = session_key_for_events.clone();
        tokio::spawn(async move {
            let payload = match &event {
                RunnerEvent::Thinking => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "thinking",
                }),
                RunnerEvent::ThinkingDone => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "thinking_done",
                }),
                RunnerEvent::ToolCallStart {
                    id,
                    name,
                    arguments,
                } => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "tool_call_start",
                    "toolCallId": id,
                    "toolName": name,
                    "arguments": arguments,
                }),
                RunnerEvent::ToolCallEnd {
                    id,
                    name,
                    success,
                    error,
                    result,
                } => {
                    let mut payload = serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "tool_call_end",
                        "toolCallId": id,
                        "toolName": name,
                        "success": success,
                    });
                    if let Some(err) = error {
                        payload["error"] = serde_json::json!(parse_chat_error(err, None));
                    }
                    if let Some(res) = result {
                        // Cap output sent to the UI to avoid huge WS frames.
                        let mut capped = res.clone();
                        for field in &["stdout", "stderr"] {
                            if let Some(s) = capped.get(*field).and_then(|v| v.as_str())
                                && s.len() > 10_000
                            {
                                let truncated = format!(
                                    "{}\n\n... [truncated — {} bytes total]",
                                    &s[..10_000],
                                    s.len()
                                );
                                capped[*field] = serde_json::Value::String(truncated);
                            }
                        }
                        payload["result"] = capped;
                    }
                    payload
                },
                RunnerEvent::ThinkingText(text) => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "thinking_text",
                    "text": text,
                }),
                RunnerEvent::TextDelta(text) => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "delta",
                    "text": text,
                }),
                RunnerEvent::Iteration(n) => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "iteration",
                    "iteration": n,
                }),
                RunnerEvent::SubAgentStart { task, model, depth } => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "sub_agent_start",
                    "task": task,
                    "model": model,
                    "depth": depth,
                }),
                RunnerEvent::SubAgentEnd {
                    task,
                    model,
                    depth,
                    iterations,
                    tool_calls_made,
                } => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "sub_agent_end",
                    "task": task,
                    "model": model,
                    "depth": depth,
                    "iterations": iterations,
                    "toolCallsMade": tool_calls_made,
                }),
            };
            broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
        });
    });

    // Pass history (excluding the current user message, which run_agent_loop adds).
    let hist = if history.is_empty() {
        None
    } else {
        Some(history.to_vec())
    };

    // Inject session key and accept-language into tool call params so tools can
    // resolve per-session state and forward the user's locale to web requests.
    let mut tool_context = serde_json::json!({ "_session_key": session_key });
    if let Some(lang) = accept_language.as_deref() {
        tool_context["_accept_language"] = serde_json::json!(lang);
    }

    let provider_ref = provider.clone();
    let registry_guard = tool_registry.read().await;
    let owned_filtered_loop;
    let tools_for_loop: &ToolRegistry = if mcp_disabled {
        owned_filtered_loop = registry_guard.clone_without_mcp();
        &owned_filtered_loop
    } else {
        &registry_guard
    };
    let first_result = run_agent_loop_streaming(
        provider,
        tools_for_loop,
        &system_prompt,
        text,
        Some(&on_event),
        hist,
        Some(tool_context.clone()),
        hook_registry.clone(),
    )
    .await;

    // On context-window overflow, compact the session and retry once.
    let result = match first_result {
        Err(AgentRunError::ContextWindowExceeded(ref msg)) if session_store.is_some() => {
            let store = session_store.unwrap();
            info!(
                run_id,
                session = session_key,
                error = %msg,
                "context window exceeded — compacting and retrying"
            );

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "auto_compact",
                    "phase": "start",
                    "reason": "context_window_exceeded",
                }),
                BroadcastOpts::default(),
            )
            .await;

            // Inline compaction: summarize history, replace in store.
            match compact_session(store, session_key, &provider_ref).await {
                Ok(()) => {
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "done",
                            "reason": "context_window_exceeded",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    // Reload compacted history and retry.
                    let compacted_history = store.read(session_key).await.unwrap_or_default();
                    let retry_hist = if compacted_history.is_empty() {
                        None
                    } else {
                        Some(compacted_history)
                    };

                    run_agent_loop_streaming(
                        provider_ref.clone(),
                        tools_for_loop,
                        &system_prompt,
                        text,
                        Some(&on_event),
                        retry_hist,
                        Some(tool_context),
                        hook_registry,
                    )
                    .await
                },
                Err(e) => {
                    warn!(run_id, error = %e, "retry compaction failed");
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "error",
                            "error": e.to_string(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                    // Return the original error.
                    first_result
                },
            }
        },
        other => other,
    };

    match result {
        Ok(result) => {
            info!(
                run_id,
                iterations = result.iterations,
                tool_calls = result.tool_calls_made,
                response = %result.text,
                "agent run complete"
            );
            let assistant_message_index = user_message_index + 1;
            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "final",
                    "text": result.text,
                    "iterations": result.iterations,
                    "toolCallsMade": result.tool_calls_made,
                    "model": provider_ref.id(),
                    "provider": provider_name,
                    "inputTokens": result.usage.input_tokens,
                    "outputTokens": result.usage.output_tokens,
                    "messageIndex": assistant_message_index,
                }),
                BroadcastOpts::default(),
            )
            .await;
            // Send push notification when chat response completes
            #[cfg(feature = "push-notifications")]
            {
                tracing::info!("push: checking push notification (agent mode)");
                send_chat_push_notification(state, session_key, &result.text).await;
            }
            deliver_channel_replies(state, session_key, &result.text).await;
            Some((
                result.text,
                result.usage.input_tokens,
                result.usage.output_tokens,
            ))
        },
        Err(e) => {
            let error_str = e.to_string();
            warn!(run_id, error = %error_str, "agent run error");
            state.set_run_error(run_id, error_str.clone()).await;
            let error_obj = parse_chat_error(&error_str, Some(provider_name));
            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "error",
                    "error": error_obj,
                }),
                BroadcastOpts::default(),
            )
            .await;
            None
        },
    }
}

/// Compact a session's history by summarizing it with the given provider.
///
/// This is a standalone helper so `run_with_tools` can call it without
/// requiring `&self` on `LiveChatService`.
async fn compact_session(
    store: &Arc<SessionStore>,
    session_key: &str,
    provider: &Arc<dyn moltis_agents::model::LlmProvider>,
) -> Result<(), String> {
    let history = store.read(session_key).await.map_err(|e| e.to_string())?;
    if history.is_empty() {
        return Err("nothing to compact".into());
    }

    let mut summary_messages: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "system",
        "content": "You are a conversation summarizer. Summarize the following conversation into a concise form that preserves all key facts, decisions, and context. Output only the summary, no preamble."
    })];

    let mut conversation_text = String::new();
    for msg in &history {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
        conversation_text.push_str(&format!("{role}: {content}\n\n"));
    }
    summary_messages.push(serde_json::json!({
        "role": "user",
        "content": conversation_text,
    }));

    let mut stream = provider.stream(summary_messages);
    let mut summary = String::new();
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => summary.push_str(&delta),
            StreamEvent::Done(_) => break,
            StreamEvent::Error(e) => return Err(format!("compact summarization failed: {e}")),
            // Tool events not expected in summarization stream.
            StreamEvent::ToolCallStart { .. }
            | StreamEvent::ToolCallArgumentsDelta { .. }
            | StreamEvent::ToolCallComplete { .. } => {},
        }
    }

    if summary.is_empty() {
        return Err("compact produced empty summary".into());
    }

    let compacted = vec![serde_json::json!({
        "role": "assistant",
        "content": format!("[Conversation Summary]\n\n{summary}"),
        "created_at": now_ms(),
    })];

    store
        .replace_history(session_key, compacted)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ── Streaming mode (no tools) ───────────────────────────────────────────────

async fn run_streaming(
    state: &Arc<GatewayState>,
    run_id: &str,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    text: &str,
    provider_name: &str,
    history: &[serde_json::Value],
    session_key: &str,
    project_context: Option<&str>,
    session_context: Option<&str>,
    user_message_index: usize,
    skills: &[moltis_skills::types::SkillMetadata],
) -> Option<(String, u32, u32)> {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    // Prepend session + project context as system messages.
    if let Some(ctx) = session_context {
        messages.push(serde_json::json!({
            "role": "system",
            "content": format!("## Current Session\n\n{ctx}"),
        }));
    }
    if let Some(ctx) = project_context {
        messages.push(serde_json::json!({
            "role": "system",
            "content": ctx,
        }));
    }
    // Inject skills into the system prompt for streaming mode too.
    if !skills.is_empty() {
        let skills_block = moltis_skills::prompt_gen::generate_skills_prompt(skills);
        messages.push(serde_json::json!({
            "role": "system",
            "content": skills_block,
        }));
    }
    messages.extend_from_slice(history);
    messages.push(serde_json::json!({
        "role": "user",
        "content": text,
    }));

    let mut stream = provider.stream(messages);
    let mut accumulated = String::new();

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => {
                accumulated.push_str(&delta);
                broadcast(
                    state,
                    "chat",
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": session_key,
                        "state": "delta",
                        "text": delta,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
            },
            StreamEvent::Done(usage) => {
                debug!(
                    run_id,
                    input_tokens = usage.input_tokens,
                    output_tokens = usage.output_tokens,
                    "chat stream done"
                );
                let assistant_message_index = user_message_index + 1;
                broadcast(
                    state,
                    "chat",
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": session_key,
                        "state": "final",
                        "text": accumulated,
                        "model": provider.id(),
                        "provider": provider_name,
                        "inputTokens": usage.input_tokens,
                        "outputTokens": usage.output_tokens,
                        "messageIndex": assistant_message_index,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
                // Send push notification when chat response completes
                #[cfg(feature = "push-notifications")]
                {
                    tracing::info!("push: checking push notification");
                    send_chat_push_notification(state, session_key, &accumulated).await;
                }
                deliver_channel_replies(state, session_key, &accumulated).await;
                return Some((accumulated, usage.input_tokens, usage.output_tokens));
            },
            StreamEvent::Error(msg) => {
                warn!(run_id, error = %msg, "chat stream error");
                state.set_run_error(run_id, msg.clone()).await;
                let error_obj = parse_chat_error(&msg, Some(provider_name));
                broadcast(
                    state,
                    "chat",
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": session_key,
                        "state": "error",
                        "error": error_obj,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
                return None;
            },
            // Tool events not expected in stream-only mode.
            StreamEvent::ToolCallStart { .. }
            | StreamEvent::ToolCallArgumentsDelta { .. }
            | StreamEvent::ToolCallComplete { .. } => {},
        }
    }
    None
}

/// Send a push notification when a chat response completes.
/// Only sends if push notifications are configured and there are subscribers.
#[cfg(feature = "push-notifications")]
async fn send_chat_push_notification(state: &Arc<GatewayState>, session_key: &str, text: &str) {
    let push_service = match state.get_push_service().await {
        Some(svc) => svc,
        None => {
            tracing::info!("push notification skipped: service not configured");
            return;
        },
    };

    let sub_count = push_service.subscription_count().await;
    if sub_count == 0 {
        tracing::info!("push notification skipped: no subscribers");
        return;
    }

    tracing::info!(
        subscribers = sub_count,
        session = session_key,
        "sending push notification"
    );

    // Create a short summary of the response (first 100 chars)
    let summary = if text.len() > 100 {
        format!("{}…", &text[..100])
    } else {
        text.to_string()
    };

    // Build the notification
    let title = "Message received";
    let url = format!("/chat/{session_key}");

    match crate::push_routes::send_push_notification(
        &push_service,
        title,
        &summary,
        Some(&url),
        Some(session_key),
    )
    .await
    {
        Ok(sent) => {
            tracing::info!(sent, "push notification sent");
        },
        Err(e) => {
            tracing::warn!("failed to send push notification: {e}");
        },
    }
}

/// Drain any pending channel reply targets for a session and send the
/// response text back to each originating channel via outbound.
/// Each delivery runs in its own spawned task so slow network calls
/// don't block each other or the chat pipeline.
async fn deliver_channel_replies(state: &Arc<GatewayState>, session_key: &str, text: &str) {
    let targets = state.drain_channel_replies(session_key).await;
    if targets.is_empty() || text.is_empty() {
        return;
    }
    let outbound = match state.services.channel_outbound_arc() {
        Some(o) => o,
        None => return,
    };
    deliver_channel_replies_to_targets(outbound, targets, text).await;
}

async fn deliver_channel_replies_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    text: &str,
) {
    let text = text.to_string();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let text = text.clone();
        tasks.push(tokio::spawn(async move {
            match target.channel_type.as_str() {
                "telegram" => {
                    if let Err(e) = outbound
                        .send_text(&target.account_id, &target.chat_id, &text)
                        .await
                    {
                        warn!(
                            account_id = target.account_id,
                            chat_id = target.chat_id,
                            "failed to send channel reply: {e}"
                        );
                    }
                },
                other => {
                    warn!(channel_type = other, "unsupported channel type for reply");
                },
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel reply task join failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        anyhow::Result,
        moltis_common::types::ReplyPayload,
        std::{
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            time::{Duration, Instant},
        },
    };

    struct MockChannelOutbound {
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    #[async_trait]
    impl moltis_channels::plugin::ChannelOutbound for MockChannelOutbound {
        async fn send_text(&self, _account_id: &str, _to: &str, _text: &str) -> Result<()> {
            tokio::time::sleep(self.delay).await;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn deliver_channel_replies_waits_for_outbound_sends() {
        let calls = Arc::new(AtomicUsize::new(0));
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> =
            Arc::new(MockChannelOutbound {
                calls: Arc::clone(&calls),
                delay: Duration::from_millis(50),
            });
        let targets = vec![moltis_channels::ChannelReplyTarget {
            channel_type: "telegram".to_string(),
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
        }];

        let start = Instant::now();
        deliver_channel_replies_to_targets(outbound, targets, "hello").await;

        assert!(
            start.elapsed() >= Duration::from_millis(45),
            "delivery should wait for outbound send completion"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deliver_channel_replies_ignores_unsupported_channels() {
        let calls = Arc::new(AtomicUsize::new(0));
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> =
            Arc::new(MockChannelOutbound {
                calls: Arc::clone(&calls),
                delay: Duration::from_millis(1),
            });
        let targets = vec![moltis_channels::ChannelReplyTarget {
            channel_type: "discord".to_string(),
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
        }];

        deliver_channel_replies_to_targets(outbound, targets, "hello").await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// Build a bare session_locks map for testing the semaphore logic
    /// without constructing a full LiveChatService.
    fn make_session_locks() -> Arc<RwLock<HashMap<String, Arc<Semaphore>>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    async fn get_or_create_semaphore(
        locks: &Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
        key: &str,
    ) -> Arc<Semaphore> {
        {
            let map = locks.read().await;
            if let Some(sem) = map.get(key) {
                return Arc::clone(sem);
            }
        }
        let mut map = locks.write().await;
        Arc::clone(
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    #[tokio::test]
    async fn same_session_runs_are_serialized() {
        let locks = make_session_locks();
        let sem = get_or_create_semaphore(&locks, "s1").await;

        // Acquire the permit — simulates a running task.
        let permit = sem.clone().acquire_owned().await.unwrap();

        // A second acquire should not resolve while the first is held.
        let sem2 = sem.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _p = sem2.acquire_owned().await.unwrap();
            let _ = tx.send(());
        });

        // Give the second task a chance to run — it should be blocked.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            rx.try_recv().is_err(),
            "second run should be blocked while first holds permit"
        );

        // Release first permit.
        drop(permit);

        // Now the second task should complete.
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn different_sessions_run_in_parallel() {
        let locks = make_session_locks();
        let sem_a = get_or_create_semaphore(&locks, "a").await;
        let sem_b = get_or_create_semaphore(&locks, "b").await;

        let _pa = sem_a.clone().acquire_owned().await.unwrap();
        // Session "b" should still be acquirable.
        let _pb = sem_b.clone().acquire_owned().await.unwrap();
    }

    #[tokio::test]
    async fn abort_releases_permit() {
        let locks = make_session_locks();
        let sem = get_or_create_semaphore(&locks, "s").await;

        let sem2 = sem.clone();
        let task = tokio::spawn(async move {
            let _p = sem2.acquire_owned().await.unwrap();
            // Simulate long-running work.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        // Give the task time to acquire the permit.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Abort the task — this drops the permit.
        task.abort();
        let _ = task.await;

        // The semaphore should now be acquirable.
        let _p = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            sem.clone().acquire_owned(),
        )
        .await
        .expect("permit should be available after abort")
        .unwrap();
    }

    #[tokio::test]
    async fn agent_timeout_cancels_slow_future() {
        use std::time::Duration;

        let timeout_secs: u64 = 1;
        let slow_fut = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Some(("done".to_string(), 0u32, 0u32))
        };

        let result: Option<(String, u32, u32)> =
            tokio::time::timeout(Duration::from_secs(timeout_secs), slow_fut)
                .await
                .unwrap_or_default();

        assert!(
            result.is_none(),
            "slow future should have been cancelled by timeout"
        );
    }

    #[tokio::test]
    async fn agent_timeout_zero_means_no_timeout() {
        use std::time::Duration;

        let timeout_secs: u64 = 0;
        let fast_fut = async { Some(("ok".to_string(), 10u32, 5u32)) };

        let result = if timeout_secs > 0 {
            tokio::time::timeout(Duration::from_secs(timeout_secs), fast_fut)
                .await
                .unwrap_or_default()
        } else {
            fast_fut.await
        };

        assert_eq!(result, Some(("ok".to_string(), 10, 5)));
    }

    // ── Message queue tests ──────────────────────────────────────────────

    fn make_message_queue() -> Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    #[tokio::test]
    async fn queue_enqueue_and_drain() {
        let queue = make_message_queue();
        let key = "sess1";

        // Enqueue two messages.
        {
            let mut q = queue.write().await;
            q.entry(key.to_string()).or_default().push(QueuedMessage {
                params: serde_json::json!({"text": "hello"}),
            });
            q.entry(key.to_string()).or_default().push(QueuedMessage {
                params: serde_json::json!({"text": "world"}),
            });
        }

        // Drain.
        let drained = queue.write().await.remove(key).unwrap_or_default();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].params["text"], "hello");
        assert_eq!(drained[1].params["text"], "world");

        // Queue should be empty after drain.
        assert!(queue.read().await.get(key).is_none());
    }

    #[tokio::test]
    async fn queue_collect_concatenates_texts() {
        let msgs = [
            QueuedMessage {
                params: serde_json::json!({"text": "first", "model": "gpt-4"}),
            },
            QueuedMessage {
                params: serde_json::json!({"text": "second"}),
            },
            QueuedMessage {
                params: serde_json::json!({"text": "third", "_conn_id": "c1"}),
            },
        ];

        let combined: Vec<&str> = msgs
            .iter()
            .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
            .collect();
        let joined = combined.join("\n\n");
        assert_eq!(joined, "first\n\nsecond\n\nthird");
    }

    #[tokio::test]
    async fn try_acquire_returns_err_when_held() {
        let sem = Arc::new(Semaphore::new(1));
        let _permit = sem.clone().try_acquire_owned().unwrap();

        // Second try_acquire should fail.
        assert!(sem.clone().try_acquire_owned().is_err());
    }

    #[tokio::test]
    async fn try_acquire_succeeds_when_free() {
        let sem = Arc::new(Semaphore::new(1));
        assert!(sem.clone().try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn queue_drain_empty_is_noop() {
        let queue = make_message_queue();
        let drained = queue
            .write()
            .await
            .remove("nonexistent")
            .unwrap_or_default();
        assert!(drained.is_empty());
    }

    #[test]
    fn message_queue_mode_default_is_followup() {
        let mode = MessageQueueMode::default();
        assert_eq!(mode, MessageQueueMode::Followup);
    }

    #[test]
    fn message_queue_mode_deserializes_from_toml() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct Wrapper {
            mode: MessageQueueMode,
        }

        let followup: Wrapper = toml::from_str(r#"mode = "followup""#).unwrap();
        assert_eq!(followup.mode, MessageQueueMode::Followup);

        let collect: Wrapper = toml::from_str(r#"mode = "collect""#).unwrap();
        assert_eq!(collect.mode, MessageQueueMode::Collect);
    }
}
