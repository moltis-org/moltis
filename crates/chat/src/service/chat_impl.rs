//! `ChatService` trait implementation for `LiveChatService`.

mod send;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

use {
    moltis_agents::{
        ChatMessage, UserContent,
        model::values_to_chat_messages,
        prompt::{
            build_system_prompt_minimal_runtime_details,
            build_system_prompt_with_session_runtime_details,
        },
    },
    moltis_config::ToolMode,
    moltis_service_traits::{ChatService, ServiceError, ServiceResult},
    moltis_sessions::{ContentBlock, MessageContent, PersistedMessage},
    moltis_tools::policy::{PolicyContext, ToolPolicy},
};

use crate::{
    agent_loop::effective_tool_mode,
    channels::notify_channels_of_compaction,
    compaction_run,
    memory_tools::AgentScopedMemoryWriter,
    message::{
        infer_reply_medium, user_audio_path_from_params, user_documents_for_persistence,
        user_documents_from_params,
    },
    prompt::{
        apply_request_runtime_context, apply_runtime_tool_filters, build_policy_context,
        build_prompt_runtime_context, clear_prompt_memory_snapshot, discover_skills_if_enabled,
        load_prompt_persona_for_agent, load_prompt_persona_for_session,
        prompt_build_limits_from_config, resolve_prompt_agent_id, resolve_prompt_mode_context,
    },
    run_with_tools::run_with_tools,
    service::build_persisted_assistant_message,
    streaming::run_streaming,
    types::*,
};

use super::*;

#[async_trait]
impl ChatService for LiveChatService {
    async fn send(&self, params: Value) -> ServiceResult {
        self.send_impl(params).await
    }

    async fn send_sync(&self, params: Value) -> ServiceResult {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'text' parameter".to_string())?
            .to_string();
        let desired_reply_medium = infer_reply_medium(&params, &text);
        let requested_agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let request_tool_policy = params
            .get("_tool_policy")
            .cloned()
            .map(serde_json::from_value::<ToolPolicy>)
            .transpose()
            .map_err(|e| format!("invalid '_tool_policy' parameter: {e}"))?;

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

        let user_audio = user_audio_path_from_params(&params, &session_key);
        let user_documents =
            user_documents_from_params(&params, &session_key, self.session_store.as_ref());
        // Persist the user message.
        let user_msg = PersistedMessage::User {
            content: MessageContent::Text(text.clone()),
            created_at: Some(now_ms()),
            audio: user_audio,
            documents: user_documents
                .as_deref()
                .and_then(user_documents_for_persistence),
            channel: None,
            seq: None,
            run_id: None,
        };
        if let Err(e) = self
            .session_store
            .append(&session_key, &user_msg.to_value())
            .await
        {
            warn!("send_sync: failed to persist user message: {e}");
        }

        // Ensure this session appears in the sessions list.
        let _ = self.session_metadata.upsert(&session_key, None).await;
        if let Some(agent_id) = requested_agent_id.as_deref()
            && let Err(error) = self
                .session_metadata
                .set_agent_id(&session_key, Some(agent_id))
                .await
        {
            warn!(
                session = %session_key,
                agent_id,
                error = %error,
                "send_sync: failed to assign requested agent to session"
            );
        }
        self.session_metadata.touch(&session_key, 1).await;

        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        runtime_context.mode = resolve_prompt_mode_context(&persona.config, session_entry.as_ref());
        apply_request_runtime_context(&mut runtime_context.host, &params);

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
        let tool_registry = if let Some(policy) = request_tool_policy.as_ref() {
            let registry_guard = self.tool_registry.read().await;
            Arc::new(RwLock::new(
                registry_guard.clone_allowed_by(|name| policy.is_allowed(name)),
            ))
        } else {
            Arc::clone(&self.tool_registry)
        };
        let hook_registry = self.hook_registry.clone();
        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        let model_store = Arc::clone(&self.model_store);
        let user_message_index = history.len();

        info!(
            run_id = %run_id,
            user_message = %text,
            model = %model_id,
            stream_only,
            session = %session_key,
            reply_medium = ?desired_reply_medium,
            "chat.send_sync"
        );

        if desired_reply_medium == ReplyMedium::Voice {
            broadcast(
                &state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "voice_pending",
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        // send_sync is text-only (used by API calls and channels).
        let user_content = UserContent::text(&text);
        let active_event_forwarders = Arc::new(RwLock::new(HashMap::new()));
        let terminal_runs = Arc::new(RwLock::new(HashSet::new()));
        let result = if stream_only {
            run_streaming(
                persona,
                &state,
                &model_store,
                &run_id,
                provider,
                &model_id,
                &user_content,
                &provider_name,
                &history,
                &session_key,
                &session_agent_id,
                desired_reply_medium,
                None,
                user_message_index,
                &[],
                Some(&runtime_context),
                None, // send_sync: no sender name
                Some(&self.session_store),
                None, // send_sync: no client seq
                None, // send_sync: no partial assistant tracking
                &terminal_runs,
            )
            .await
        } else {
            run_with_tools(
                persona,
                &state,
                &model_store,
                &run_id,
                provider,
                &model_id,
                &tool_registry,
                &user_content,
                &provider_name,
                &history,
                &session_key,
                &session_agent_id,
                desired_reply_medium,
                None,
                Some(&runtime_context),
                user_message_index,
                &[],
                hook_registry,
                None,
                None, // send_sync: no conn_id
                Some(&self.session_store),
                false, // send_sync: MCP tools always enabled for API calls
                None,  // send_sync: no client seq
                None,  // send_sync: no thinking text tracking
                None,  // send_sync: no tool call tracking
                None,  // send_sync: no partial assistant tracking
                &active_event_forwarders,
                &terminal_runs,
                None, // send_sync: no sender name
            )
            .await
        };

        // Persist assistant response (even empty ones — needed for LLM history coherence).
        if let Some(ref assistant_output) = result {
            let assistant_msg = build_persisted_assistant_message(
                assistant_output.clone(),
                Some(model_id.clone()),
                Some(provider_name.clone()),
                None,
                Some(run_id.clone()),
            );
            if let Err(e) = self
                .session_store
                .append(&session_key, &assistant_msg.to_value())
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
            Some(assistant_output) => Ok(serde_json::json!({
                "text": assistant_output.text,
                "inputTokens": assistant_output.input_tokens,
                "outputTokens": assistant_output.output_tokens,
                "cacheReadTokens": assistant_output.cache_read_tokens,
                "cacheWriteTokens": assistant_output.cache_write_tokens,
                "durationMs": assistant_output.duration_ms,
                "requestInputTokens": assistant_output.request_input_tokens,
                "requestOutputTokens": assistant_output.request_output_tokens,
                "requestCacheReadTokens": assistant_output.request_cache_read_tokens,
                "requestCacheWriteTokens": assistant_output.request_cache_write_tokens,
            })),
            None => {
                // Check the last broadcast for this run to get the actual error message.
                let error_msg = state
                    .last_run_error(&run_id)
                    .await
                    .unwrap_or_else(|| "agent run failed (check server logs)".to_string());

                // Persist the error in the session so it's visible in session history.
                let error_entry = PersistedMessage::system(format!("[error] {error_msg}"));
                let _ = self
                    .session_store
                    .append(&session_key, &error_entry.to_value())
                    .await;
                // Update metadata so the session shows in the UI.
                if let Ok(count) = self.session_store.count(&session_key).await {
                    self.session_metadata.touch(&session_key, count).await;
                }

                Err(error_msg.into())
            },
        }
    }

    async fn abort(&self, params: Value) -> ServiceResult {
        let run_id = params.get("runId").and_then(|v| v.as_str());
        let session_key = params.get("sessionKey").and_then(|v| v.as_str());
        if run_id.is_none() && session_key.is_none() {
            return Err("missing 'runId' or 'sessionKey'".into());
        }

        let resolved_session_key =
            Self::resolve_session_key_for_run(&self.active_runs_by_session, run_id, session_key)
                .await;

        let (resolved_run_id, aborted) = Self::abort_run_handle(
            &self.active_runs,
            &self.active_runs_by_session,
            &self.terminal_runs,
            run_id,
            session_key,
        )
        .await;
        info!(
            requested_run_id = ?run_id,
            session_key = ?session_key,
            resolved_run_id = ?resolved_run_id,
            aborted,
            "chat.abort"
        );

        if aborted && let Some(key) = resolved_session_key.as_deref() {
            let _ = Self::wait_for_event_forwarder(&self.active_event_forwarders, key).await;
            let partial = self.persist_partial_assistant_on_abort(key).await;
            self.active_thinking_text.write().await.remove(key);
            self.active_tool_calls.write().await.remove(key);
            self.active_reply_medium.write().await.remove(key);
            let mut payload = serde_json::json!({
                "state": "aborted",
                "runId": resolved_run_id,
                "sessionKey": key,
            });
            if let Some((partial_message, message_index)) = partial {
                payload["partialMessage"] = partial_message;
                if let Some(index) = message_index {
                    payload["messageIndex"] = serde_json::json!(index);
                }
            }
            broadcast(&self.state, "chat", payload, BroadcastOpts::default()).await;
        }

        Ok(serde_json::json!({
            "aborted": aborted,
            "runId": resolved_run_id,
            "sessionKey": resolved_session_key,
        }))
    }

    async fn cancel_queued(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'sessionKey'".to_string())?;

        let removed = self
            .message_queue
            .write()
            .await
            .remove(session_key)
            .unwrap_or_default();
        let count = removed.len();
        info!(session = %session_key, count, "cancel_queued: cleared message queue");

        broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "sessionKey": session_key,
                "state": "queue_cleared",
                "count": count,
            }),
            BroadcastOpts::default(),
        )
        .await;

        Ok(serde_json::json!({ "cleared": count }))
    }

    async fn history(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .map_err(ServiceError::message)?;
        // Filter out empty assistant messages — they are kept in storage for LLM
        // history coherence but should not be shown in the UI.
        let visible: Vec<Value> = messages
            .into_iter()
            .filter(assistant_message_is_visible)
            .collect();
        Ok(serde_json::json!(visible))
    }

    async fn inject(&self, _params: Value) -> ServiceResult {
        Err("inject not yet implemented".into())
    }

    async fn clear(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;

        self.session_store
            .clear(&session_key)
            .await
            .map_err(ServiceError::message)?;

        // Reset client sequence tracking for this session. A cleared chat starts
        // a fresh sequence from the web UI.
        {
            let mut seq_map = self.last_client_seq.write().await;
            seq_map.remove(&session_key);
        }

        // Reset metadata message count and preview.
        self.session_metadata.touch(&session_key, 0).await;
        self.session_metadata.set_preview(&session_key, None).await;

        // Notify all WebSocket clients so the web UI clears the session
        // even when /clear is issued from a channel (e.g. Telegram).
        broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "sessionKey": session_key,
                "state": "session_cleared",
            }),
            BroadcastOpts::default(),
        )
        .await;

        info!(session = %session_key, "chat.clear");
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn compact(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;
        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());

        let history = self
            .session_store
            .read(&session_key)
            .await
            .map_err(ServiceError::message)?;

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
        // The manager implements MemoryWriter directly (with path validation, size limits,
        // and automatic re-indexing), so no manual sync_path is needed after the turn.
        if let Some(mm) = self.state.memory_manager()
            && let Ok(provider) = self.resolve_provider(&session_key, &history).await
        {
            let write_mode = moltis_config::discover_and_load().memory.agent_write_mode;
            if !memory_write_mode_allows_save(write_mode) {
                debug!(
                    "compact: agent-authored memory writes disabled, skipping silent memory turn"
                );
            } else {
                let chat_history_for_memory = values_to_chat_messages(&history);
                let writer: Arc<dyn moltis_agents::memory_writer::MemoryWriter> =
                    Arc::new(AgentScopedMemoryWriter::new(
                        Arc::clone(mm),
                        session_agent_id.clone(),
                        write_mode,
                    ));
                match moltis_agents::silent_turn::run_silent_memory_turn(
                    provider,
                    &chat_history_for_memory,
                    writer,
                )
                .await
                {
                    Ok(paths) => {
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

        // Resolve the session persona so we can pick up the compaction config
        // and provide a provider to LLM-backed compaction modes. Agent-scoped
        // config falls back through `load_prompt_persona_for_agent`'s default
        // path, so this is safe even when the session has no custom preset.
        let persona = load_prompt_persona_for_agent(&session_agent_id);
        let compaction_config = &persona.config.chat.compaction;

        // LLM-backed modes need a resolved provider. Deterministic mode
        // ignores it, so resolution failures are only fatal for the other
        // modes — and `run_compaction` returns a clear ProviderRequired
        // error in that case.
        let provider_arc = self.resolve_provider(&session_key, &history).await.ok();

        let outcome =
            compaction_run::run_compaction(&history, compaction_config, provider_arc.as_deref())
                .await
                .map_err(|e| ServiceError::message(e.to_string()))?;

        let compacted = outcome.history.clone();

        // Keep a plain-text copy of the summary so the memory-file snapshot
        // below can still record what we compacted to. The helper walks the
        // compacted history because recency_preserving / structured modes
        // splice head and tail messages around the summary — it isn't
        // necessarily compacted[0].
        let summary_for_memory = compaction_run::extract_summary_body(&compacted);

        info!(
            session = %session_key,
            requested_mode = ?compaction_config.mode,
            effective_mode = ?outcome.effective_mode,
            input_tokens = outcome.input_tokens,
            output_tokens = outcome.output_tokens,
            messages = history.len(),
            "chat.compact: strategy dispatched"
        );

        // Enforce summary budget discipline: max 1,200 chars, 24 lines,
        // 160 chars/line.  Mutate the compacted history in place so the
        // compressed text is what gets persisted and broadcast.
        let compacted = compress_summary_in_history(compacted);

        // Replace the session history BEFORE broadcasting or notifying
        // channels. If we did it the other way around, a concurrent
        // `send()` RPC that landed between the broadcast and the store
        // update would see the stale history and the client UI would
        // already believe compaction had finished — a narrow but real
        // race window flagged by Greptile on commit 0714de07.
        self.session_store
            .replace_history(&session_key, compacted.clone())
            .await
            .map_err(ServiceError::message)?;

        self.session_metadata.touch(&session_key, 1).await;

        // Broadcast a chat.compact-scoped "done" event so UI consumers see
        // the effective mode and token usage even when compaction is
        // triggered manually via the RPC (the auto-compact path broadcasts
        // separately around `send()`). The settings hint is included only
        // when the user hasn't opted out via chat.compaction.show_settings_hint.
        //
        // Include `totalTokens` / `contextWindow` on this payload so the
        // web UI's compact card can render a full "Before compact"
        // section even when this event fires first in `send()`'s
        // pre-emptive auto-compact path. Without these fields the card
        // was rendering without the "Total tokens" and "Context usage"
        // rows on that path.
        let show_hint = compaction_config.show_settings_hint;
        let pre_compact_total_tokens: u32 = history
            .iter()
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .map(|text| u32::try_from(estimate_text_tokens(text)).unwrap_or(u32::MAX))
            .sum();
        let context_window = provider_arc.as_deref().map(|p| p.context_window());
        let mut compact_payload = serde_json::json!({
            "sessionKey": session_key,
            "state": "compact",
            "phase": "done",
            "messageCount": history.len(),
            "totalTokens": pre_compact_total_tokens,
        });
        if let Some(window) = context_window
            && let Some(obj) = compact_payload.as_object_mut()
        {
            obj.insert("contextWindow".to_string(), serde_json::json!(window));
        }
        if let (Some(obj), Some(meta)) = (
            compact_payload.as_object_mut(),
            outcome.broadcast_metadata(show_hint).as_object().cloned(),
        ) {
            obj.extend(meta);
        }
        broadcast(
            &self.state,
            "chat",
            compact_payload,
            BroadcastOpts::default(),
        )
        .await;

        // Notify any channel (Telegram, Discord, Matrix, WhatsApp, etc.)
        // that has pending reply targets on this session, so channel
        // users see "Conversation compacted (mode, tokens, hint)"
        // alongside the web UI's compact card.
        notify_channels_of_compaction(&self.state, &session_key, &outcome, show_hint).await;

        // Save compaction summary to memory file and trigger sync.
        if let Some(mm) = self.state.memory_manager() {
            let memory_dir = moltis_config::agent_workspace_dir(&session_agent_id).join("memory");
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
                    "# Compaction Summary\n\n- **Session**: {session_key}\n- **Timestamp**: {ts}\n\n{summary_for_memory}"
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
                summary_len: summary_for_memory.len(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "AfterCompaction hook failed");
            }
        }

        info!(session = %session_key, "chat.compact: done");
        Ok(serde_json::json!(compacted))
    }

    async fn context(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;

        // Session info
        let message_count = self.session_store.count(&session_key).await.unwrap_or(0);
        let session_entry = self.session_metadata.get(&session_key).await;
        let prompt_persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
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
            self.state.active_project_id(cid).await
        } else {
            None
        };
        let project_id =
            project_id.or_else(|| session_entry.as_ref().and_then(|e| e.project_id.clone()));

        let project_info = if let Some(pid) = project_id {
            match self
                .state
                .project_service()
                .get(serde_json::json!({"id": pid}))
                .await
            {
                Ok(val) => {
                    let dir = val.get("directory").and_then(|v| v.as_str());
                    let context_files = if let Some(d) = dir {
                        match moltis_projects::context::load_context_files(Path::new(d)) {
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
        let config = moltis_config::discover_and_load();
        let tools: Vec<Value> = if supports_tools {
            let registry_guard = self.tool_registry.read().await;
            let list_ctx = PolicyContext {
                agent_id: "main".into(),
                ..Default::default()
            };
            let effective_registry =
                apply_runtime_tool_filters(&registry_guard, &config, &[], mcp_disabled, &list_ctx);
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

        // Token usage from API-reported counts stored in messages.
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let usage = session_token_usage_from_messages(&messages);
        let total_tokens = usage.session_input_tokens + usage.session_output_tokens;
        let current_total_tokens =
            usage.current_request_input_tokens + usage.current_request_output_tokens;

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
        let sandbox_info = if let Some(router) = self.state.sandbox_router() {
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
        let sandbox_enabled = sandbox_info
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let host_is_root = detect_host_root_user().await;
        // Sandbox containers currently run as root by default.
        let exec_is_root = if sandbox_enabled {
            Some(true)
        } else {
            host_is_root
        };
        let exec_prompt_symbol = exec_is_root.map(|is_root| {
            if is_root {
                "#"
            } else {
                "$"
            }
        });
        let execution_info = serde_json::json!({
            "mode": if sandbox_enabled { "sandbox" } else { "host" },
            "hostIsRoot": host_is_root,
            "isRoot": exec_is_root,
            "promptSymbol": exec_prompt_symbol,
        });

        // Discover enabled skills/plugins (only if provider supports tools and
        // `[skills] enabled` is true — see #655).
        let skills_list: Vec<Value> = if supports_tools {
            discover_skills_if_enabled(&config)
                .await
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "description": s.description,
                        "source": s.source,
                    })
                })
                .collect()
        } else {
            vec![]
        };

        // MCP servers (only if provider supports tools)
        let mcp_servers = if supports_tools {
            self.state
                .mcp_service()
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
            "execution": execution_info,
            "promptMemory": prompt_persona.memory_status,
            "supportsTools": supports_tools,
            "tokenUsage": {
                "inputTokens": usage.session_input_tokens,
                "outputTokens": usage.session_output_tokens,
                "cacheReadTokens": usage.session_cache_read_tokens,
                "cacheWriteTokens": usage.session_cache_write_tokens,
                "total": total_tokens,
                "currentInputTokens": usage.current_request_input_tokens,
                "currentOutputTokens": usage.current_request_output_tokens,
                "currentCacheReadTokens": usage.current_request_cache_read_tokens,
                "currentCacheWriteTokens": usage.current_request_cache_write_tokens,
                "currentTotal": current_total_tokens,
                "estimatedNextInputTokens": usage.current_request_input_tokens,
                "contextWindow": context_window,
            },
        }))
    }

    async fn raw_prompt(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Resolve provider.
        let history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let provider = self
            .resolve_provider(&session_key, &history)
            .await
            .map_err(ServiceError::message)?;
        let tool_mode = effective_tool_mode(&*provider);
        let native_tools = matches!(tool_mode, ToolMode::Native);
        let tools_enabled = !matches!(tool_mode, ToolMode::Off);

        // Build runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        runtime_context.mode = resolve_prompt_mode_context(&persona.config, session_entry.as_ref());
        apply_request_runtime_context(&mut runtime_context.host, &params);

        // Resolve project context.
        let project_context = self
            .resolve_project_context(&session_key, conn_id.as_deref())
            .await;

        // Discover skills (gated on `[skills] enabled` — see #655).
        let discovered_skills = discover_skills_if_enabled(&persona.config).await;

        // Check MCP disabled.
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);

        let raw_prompt_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        // Build filtered tool registry.
        let policy_ctx =
            build_policy_context(&raw_prompt_agent_id, Some(&runtime_context), Some(&params));
        let filtered_registry = {
            let registry_guard = self.tool_registry.read().await;
            if tools_enabled {
                apply_runtime_tool_filters(
                    &registry_guard,
                    &persona.config,
                    &discovered_skills,
                    mcp_disabled,
                    &policy_ctx,
                )
            } else {
                registry_guard.clone_without(&[])
            }
        };

        let tool_count = filtered_registry.list_schemas().len();

        // Build the system prompt.
        let prompt_limits = prompt_build_limits_from_config(&persona.config);
        let prompt_build = if tools_enabled {
            build_system_prompt_with_session_runtime_details(
                &filtered_registry,
                native_tools,
                project_context.as_deref(),
                &discovered_skills,
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.boot_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
                prompt_limits,
                persona.guidelines_text.as_deref(),
            )
        } else {
            build_system_prompt_minimal_runtime_details(
                project_context.as_deref(),
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.boot_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
                prompt_limits,
                persona.guidelines_text.as_deref(),
            )
        };

        let truncated = prompt_build.metadata.truncated();
        let workspace_files = prompt_build.metadata.workspace_files.clone();
        let system_prompt = prompt_build.prompt;
        let char_count = system_prompt.len();

        Ok(serde_json::json!({
            "prompt": system_prompt,
            "charCount": char_count,
            "truncated": truncated,
            "workspaceFiles": workspace_files,
            "promptMemory": persona.memory_status,
            "native_tools": native_tools,
            "tools_enabled": tools_enabled,
            "tool_mode": format!("{:?}", tool_mode),
            "toolCount": tool_count,
        }))
    }

    /// Return the **full messages array** that would be sent to the LLM on the
    /// next call — system prompt + conversation history — in OpenAI format.
    async fn full_context(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Resolve provider.
        let history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let provider = self
            .resolve_provider(&session_key, &history)
            .await
            .map_err(ServiceError::message)?;
        let tool_mode = effective_tool_mode(&*provider);
        let native_tools = matches!(tool_mode, ToolMode::Native);
        let tools_enabled = !matches!(tool_mode, ToolMode::Off);

        // Build runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        runtime_context.mode = resolve_prompt_mode_context(&persona.config, session_entry.as_ref());
        apply_request_runtime_context(&mut runtime_context.host, &params);

        // Resolve project context.
        let project_context = self
            .resolve_project_context(&session_key, conn_id.as_deref())
            .await;

        // Discover skills (gated on `[skills] enabled` — see #655).
        let discovered_skills = discover_skills_if_enabled(&persona.config).await;

        // Check MCP disabled.
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);

        // Build filtered tool registry.
        let full_ctx_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let policy_ctx =
            build_policy_context(&full_ctx_agent_id, Some(&runtime_context), Some(&params));
        let filtered_registry = {
            let registry_guard = self.tool_registry.read().await;
            if tools_enabled {
                apply_runtime_tool_filters(
                    &registry_guard,
                    &persona.config,
                    &discovered_skills,
                    mcp_disabled,
                    &policy_ctx,
                )
            } else {
                registry_guard.clone_without(&[])
            }
        };

        // Build the system prompt.
        let prompt_limits = prompt_build_limits_from_config(&persona.config);
        let prompt_build = if tools_enabled {
            build_system_prompt_with_session_runtime_details(
                &filtered_registry,
                native_tools,
                project_context.as_deref(),
                &discovered_skills,
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.boot_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
                prompt_limits,
                persona.guidelines_text.as_deref(),
            )
        } else {
            build_system_prompt_minimal_runtime_details(
                project_context.as_deref(),
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.boot_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
                prompt_limits,
                persona.guidelines_text.as_deref(),
            )
        };

        let truncated = prompt_build.metadata.truncated();
        let workspace_files = prompt_build.metadata.workspace_files.clone();
        let system_prompt = prompt_build.prompt;
        let system_prompt_chars = system_prompt.len();

        // Keep raw assistant outputs (including provider/model/token metadata)
        // so the UI can show a debug view of what the LLM actually returned.
        let llm_outputs: Vec<Value> = history
            .iter()
            .filter(|entry| entry.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .cloned()
            .collect();

        // Build the full messages array: system prompt + conversation history.
        // `values_to_chat_messages` handles `tool_result` → `tool` conversion.
        let mut messages = Vec::with_capacity(1 + history.len());
        messages.push(ChatMessage::system(system_prompt));
        messages.extend(values_to_chat_messages(&history));

        let openai_messages: Vec<Value> = messages.iter().map(|m| m.to_openai_value()).collect();
        let message_count = openai_messages.len();
        let total_chars: usize = openai_messages
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_default().len())
            .sum();

        Ok(serde_json::json!({
            "messages": openai_messages,
            "llmOutputs": llm_outputs,
            "messageCount": message_count,
            "systemPromptChars": system_prompt_chars,
            "totalChars": total_chars,
            "truncated": truncated,
            "workspaceFiles": workspace_files,
            "promptMemory": persona.memory_status,
        }))
    }

    async fn refresh_prompt_memory(&self, params: Value) -> ServiceResult {
        let session_key = self.resolve_session_key_from_params(&params).await;
        let session_entry = self.session_metadata.get(&session_key).await;
        let agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let snapshot_cleared = clear_prompt_memory_snapshot(
            &session_key,
            &agent_id,
            self.session_state_store.as_deref(),
        )
        .await;
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;

        Ok(serde_json::json!({
            "ok": true,
            "sessionKey": session_key,
            "agentId": agent_id,
            "snapshotCleared": snapshot_cleared,
            "promptMemory": persona.memory_status,
        }))
    }

    async fn active(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'sessionKey' parameter".to_string())?;
        let active = self
            .active_runs_by_session
            .read()
            .await
            .contains_key(session_key);
        Ok(serde_json::json!({ "active": active }))
    }

    async fn active_session_keys(&self) -> Vec<String> {
        self.active_runs_by_session
            .read()
            .await
            .keys()
            .cloned()
            .collect()
    }

    async fn active_thinking_text(&self, session_key: &str) -> Option<String> {
        self.active_thinking_text
            .read()
            .await
            .get(session_key)
            .cloned()
    }

    async fn active_voice_pending(&self, session_key: &str) -> bool {
        self.active_reply_medium
            .read()
            .await
            .get(session_key)
            .is_some_and(|m| *m == ReplyMedium::Voice)
    }

    async fn peek(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        let active = self
            .active_runs_by_session
            .read()
            .await
            .contains_key(session_key);

        if !active {
            return Ok(serde_json::json!({ "active": false }));
        }

        let thinking_text = self
            .active_thinking_text
            .read()
            .await
            .get(session_key)
            .cloned();

        let tool_calls: Vec<ActiveToolCall> = self
            .active_tool_calls
            .read()
            .await
            .get(session_key)
            .cloned()
            .unwrap_or_default();

        Ok(serde_json::json!({
            "active": true,
            "sessionKey": session_key,
            "thinkingText": thinking_text,
            "toolCalls": tool_calls,
        }))
    }
}
