use std::sync::Arc;

use {
    moltis_channels::{Error as ChannelError, Result as ChannelResult},
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

// ── /btw — ephemeral side question ──────────────────────────────────────────

pub(in crate::channel_events) async fn handle_btw(
    state: &Arc<GatewayState>,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    if args.is_empty() {
        return Err(ChannelError::invalid_input(
            "usage: /btw <question>\nAsk a quick side question without tools or persisting to history.",
        ));
    }

    // Resolve a provider. Clone the registry Arc out of the inner lock
    // first, then drop `inner` before acquiring the registry read lock
    // to avoid nested lock contention.
    let registry = {
        let inner = state.inner.read().await;
        inner.llm_providers.clone()
    };
    let Some(registry) = registry else {
        return Err(ChannelError::unavailable("no LLM providers available"));
    };
    // Resolve session model via async DB lookup *before* acquiring the
    // registry read lock to avoid holding the lock across an await point.
    let session_model = if let Some(ref meta) = state.services.session_metadata {
        meta.get(session_key).await.and_then(|e| e.model.clone())
    } else {
        None
    };
    let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
        let reg = registry.read().await;
        let resolved = session_model
            .as_deref()
            .and_then(|id| reg.get(id))
            .or_else(|| reg.first());
        match resolved {
            Some(p) => p,
            None => return Err(ChannelError::unavailable("no LLM provider configured")),
        }
    };

    // Read recent session history for context (last ~20 messages).
    let context_msgs = if let Some(ref store) = state.services.session_store {
        let history = store.read(session_key).await.unwrap_or_default();
        let chat_msgs = moltis_agents::model::values_to_chat_messages(&history);
        let tail_start = chat_msgs.len().saturating_sub(20);
        chat_msgs[tail_start..].to_vec()
    } else {
        Vec::new()
    };

    // Build a minimal prompt and call the LLM with no tools.
    let system_prompt = "You are a helpful assistant. Answer the user's side question \
                         concisely. You have no tools available — answer from context only.";
    let mut messages = vec![moltis_agents::ChatMessage::system(system_prompt)];
    messages.extend(context_msgs);
    messages.push(moltis_agents::ChatMessage::user(args));

    match provider.complete(&messages, &[]).await {
        Ok(response) => {
            let text = response.text.as_deref().unwrap_or("(no response)");
            Ok(text.to_string())
        },
        Err(e) => Err(ChannelError::unavailable(format!(
            "btw LLM call failed: {e}"
        ))),
    }
}

// ── /fast — toggle fast/priority mode ───────────────────────────────────────

pub(in crate::channel_events) async fn handle_fast(
    state: &Arc<GatewayState>,
    _session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let current = state.is_fast_mode(session_key).await;

    match args {
        "" => {
            // Toggle
            let new_val = !current;
            state.set_fast_mode(session_key, new_val).await;
            broadcast(
                state,
                "session",
                serde_json::json!({
                    "kind": "patched",
                    "sessionKey": session_key,
                    "fastMode": new_val,
                }),
                BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                },
            )
            .await;
            if new_val {
                Ok("Fast mode enabled. Using priority processing where supported.".to_string())
            } else {
                Ok("Fast mode disabled. Using standard processing.".to_string())
            }
        },
        "on" | "fast" => {
            state.set_fast_mode(session_key, true).await;
            broadcast(
                state,
                "session",
                serde_json::json!({
                    "kind": "patched",
                    "sessionKey": session_key,
                    "fastMode": true,
                }),
                BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                },
            )
            .await;
            Ok("Fast mode enabled. Using priority processing where supported.".to_string())
        },
        "off" | "normal" => {
            state.set_fast_mode(session_key, false).await;
            broadcast(
                state,
                "session",
                serde_json::json!({
                    "kind": "patched",
                    "sessionKey": session_key,
                    "fastMode": false,
                }),
                BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                },
            )
            .await;
            Ok("Fast mode disabled. Using standard processing.".to_string())
        },
        "status" => {
            let label = if current {
                "enabled"
            } else {
                "disabled"
            };
            Ok(format!("Fast mode is {label}."))
        },
        _ => Err(ChannelError::invalid_input("usage: /fast [on|off|status]")),
    }
}

// ── /rollback — list or restore file checkpoints ────────────────────────────

pub(in crate::channel_events) async fn handle_rollback(
    state: &Arc<GatewayState>,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let data_dir = moltis_config::data_dir();
    let manager = moltis_tools::checkpoints::CheckpointManager::new(data_dir);

    if args.is_empty() {
        // List recent turns.
        let turns = manager
            .read_turns(10, Some(session_key))
            .map_err(|e| ChannelError::unavailable(format!("failed to read turns: {e}")))?;

        if turns.is_empty() {
            return Ok(
                "No file checkpoints yet. Checkpoints are created automatically before file writes."
                    .to_string(),
            );
        }

        let mut lines = vec!["Recent turns with file changes:".to_string()];
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        for (i, turn) in turns.iter().enumerate() {
            let age_secs = now.saturating_sub(turn.created_at);
            let age_str = format_age(age_secs);
            let file_count = turn.source_paths.len();
            let file_label = if file_count == 1 {
                "file"
            } else {
                "files"
            };
            let paths: Vec<&str> = turn
                .source_paths
                .iter()
                .map(|p| p.rsplit('/').next().unwrap_or(p.as_str()))
                .collect();
            let paths_preview = if paths.len() <= 3 {
                paths.join(", ")
            } else {
                format!("{}, {} +{} more", paths[0], paths[1], paths.len() - 2)
            };
            lines.push(format!(
                "{}. {age_str} \u{2014} {file_count} {file_label} ({paths_preview})",
                i + 1
            ));
        }
        lines.push("\nUse /rollback <N> to restore, /rollback diff <N> to preview.".to_string());
        return Ok(lines.join("\n"));
    }

    // /rollback diff <N>
    if let Some(rest) = args
        .strip_prefix("diff ")
        .or_else(|| args.strip_prefix("diff\t"))
    {
        let n: usize = rest
            .trim()
            .parse()
            .map_err(|_| ChannelError::invalid_input("usage: /rollback diff <N>"))?;
        let turns = manager
            .read_turns(n, Some(session_key))
            .map_err(|e| ChannelError::unavailable(format!("failed to read turns: {e}")))?;

        if n == 0 || n > turns.len() {
            return Err(ChannelError::invalid_input(format!(
                "invalid turn number. Use 1\u{2013}{}.",
                turns.len()
            )));
        }
        let turn = &turns[n - 1];
        let mut lines = vec![format!(
            "Turn {n} \u{2014} {} checkpoints:",
            turn.checkpoint_ids.len()
        )];
        for (id, path) in turn.checkpoint_ids.iter().zip(turn.source_paths.iter()) {
            let exists = std::path::Path::new(path).exists();
            let status = if exists {
                "exists"
            } else {
                "missing"
            };
            lines.push(format!(
                "  {path} ({status}) [cp:{id}]",
                id = &id[..8.min(id.len())]
            ));
        }
        return Ok(lines.join("\n"));
    }

    // /rollback <N> — restore all files from turn N
    let n: usize = args
        .trim()
        .parse()
        .map_err(|_| ChannelError::invalid_input("usage: /rollback [<N>|diff <N>]"))?;
    let turns = manager
        .read_turns(n, Some(session_key))
        .map_err(|e| ChannelError::unavailable(format!("failed to read turns: {e}")))?;

    if n == 0 || n > turns.len() {
        return Err(ChannelError::invalid_input(format!(
            "invalid turn number. Use 1\u{2013}{}.",
            turns.len()
        )));
    }
    let turn = &turns[n - 1];

    let mut restored = 0;
    let mut errors = Vec::new();
    for id in &turn.checkpoint_ids {
        match manager.restore(id).await {
            Ok(_) => restored += 1,
            Err(e) => errors.push(format!("{id}: {e}")),
        }
    }

    broadcast(
        state,
        "session",
        serde_json::json!({
            "kind": "rollback",
            "sessionKey": session_key,
            "turn": n,
            "restored": restored,
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;

    if errors.is_empty() {
        Ok(format!("Restored {restored} files from turn {n}."))
    } else {
        Ok(format!(
            "Restored {restored} files from turn {n}. {} errors:\n{}",
            errors.len(),
            errors.join("\n")
        ))
    }
}

fn format_age(secs: i64) -> String {
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

// ── /insights — session analytics ───────────────────────────────────────────

pub(in crate::channel_events) async fn handle_insights(
    state: &Arc<GatewayState>,
    args: &str,
) -> ChannelResult<String> {
    #[cfg(not(feature = "metrics"))]
    {
        let _ = (state, args);
        return Err(ChannelError::unavailable(
            "metrics feature is not enabled in this build",
        ));
    }

    #[cfg(feature = "metrics")]
    {
        let days: u64 = if args.is_empty() {
            30
        } else {
            args.trim()
                .parse()
                .map_err(|_| ChannelError::invalid_input("usage: /insights [days]"))?
        };

        let store = state
            .metrics_store
            .as_ref()
            .ok_or_else(|| ChannelError::unavailable("metrics store not available"))?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let since_ms = now_ms.saturating_sub(days * 24 * 60 * 60 * 1000);

        let history = store
            .load_history(since_ms, 100_000)
            .await
            .map_err(|e| ChannelError::external("loading metrics history", e))?;

        if history.is_empty() {
            return Ok(format!("No metrics data in the last {days} days."));
        }

        // Aggregate
        let mut total_completions: u64 = 0;
        let mut total_input_tokens: u64 = 0;
        let mut total_output_tokens: u64 = 0;
        let mut total_errors: u64 = 0;
        let mut total_tool_executions: u64 = 0;
        let mut total_tool_errors: u64 = 0;
        let mut provider_totals: std::collections::HashMap<String, (u64, u64, u64)> =
            std::collections::HashMap::new();

        // Use deltas between consecutive points (metrics are cumulative counters)
        let mut prev: Option<&crate::state::MetricsHistoryPoint> = None;
        for point in &history {
            if let Some(p) = prev {
                total_completions += point.llm_completions.saturating_sub(p.llm_completions);
                total_input_tokens += point.llm_input_tokens.saturating_sub(p.llm_input_tokens);
                total_output_tokens += point.llm_output_tokens.saturating_sub(p.llm_output_tokens);
                total_errors += point.llm_errors.saturating_sub(p.llm_errors);
                total_tool_executions += point.tool_executions.saturating_sub(p.tool_executions);
                total_tool_errors += point.tool_errors.saturating_sub(p.tool_errors);

                for (provider, tokens) in &point.by_provider {
                    let prev_tokens = p.by_provider.get(provider);
                    let delta_in = tokens
                        .input_tokens
                        .saturating_sub(prev_tokens.map_or(0, |pt| pt.input_tokens));
                    let delta_out = tokens
                        .output_tokens
                        .saturating_sub(prev_tokens.map_or(0, |pt| pt.output_tokens));
                    let delta_completions = tokens
                        .completions
                        .saturating_sub(prev_tokens.map_or(0, |pt| pt.completions));
                    let entry = provider_totals.entry(provider.clone()).or_default();
                    entry.0 += delta_in;
                    entry.1 += delta_out;
                    entry.2 += delta_completions;
                }
            }
            prev = Some(point);
        }

        let total_tokens = total_input_tokens + total_output_tokens;
        let first_ts = history.first().map(|p| p.timestamp).unwrap_or(0);
        let last_ts = history.last().map(|p| p.timestamp).unwrap_or(0);
        let span_hours = (last_ts.saturating_sub(first_ts)) as f64 / 3_600_000.0;

        let mut lines = Vec::new();
        lines.push(format!("Insights \u{2014} last {days} days"));
        lines.push(String::new());
        lines.push(format!(
            "LLM completions: {total_completions}  (errors: {total_errors})"
        ));
        lines.push(format!(
            "Tokens: {total_tokens} total ({total_input_tokens} in / {total_output_tokens} out)"
        ));
        lines.push(format!(
            "Tools: {total_tool_executions} executions  (errors: {total_tool_errors})"
        ));
        if span_hours > 0.0 {
            let completions_per_hour = total_completions as f64 / span_hours;
            lines.push(format!("Rate: {completions_per_hour:.1} completions/hour"));
        }

        if !provider_totals.is_empty() {
            lines.push(String::new());
            lines.push("By provider:".to_string());
            let mut providers: Vec<_> = provider_totals.into_iter().collect();
            providers.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)));
            for (provider, (input, output, completions)) in &providers {
                lines.push(format!(
                    "  {provider}: {completions} completions, {} tokens ({input} in / {output} out)",
                    input + output
                ));
            }
        }

        let data_points = history.len();
        lines.push(String::new());
        lines.push(format!(
            "Based on {data_points} data points over {span_hours:.1} hours."
        ));

        // Skill usage telemetry
        if let Some(store) = state.skill_usage_store.get() {
            let usage = store.get_all().await;
            if !usage.is_empty() {
                lines.push(String::new());
                lines.push("Skills:".to_string());

                let total_reads: u64 = usage.values().map(|e| e.read_count).sum();
                let total_writes: u64 = usage.values().map(|e| e.write_count).sum();
                lines.push(format!(
                    "  {total_reads} activations, {total_writes} modifications across {} skills",
                    usage.len()
                ));

                // Top 5 most-used skills by read_count
                let mut by_reads: Vec<_> = usage.iter().collect();
                by_reads.sort_by(|a, b| b.1.read_count.cmp(&a.1.read_count));
                if by_reads.iter().any(|(_, e)| e.read_count > 0) {
                    lines.push("  Most activated:".to_string());
                    for (name, entry) in by_reads.iter().take(5) {
                        if entry.read_count == 0 {
                            break;
                        }
                        lines.push(format!(
                            "    {name}: {} reads, {} writes",
                            entry.read_count, entry.write_count
                        ));
                    }
                }

                // Skills never read (created but unused)
                let never_read: Vec<_> = usage.iter().filter(|(_, e)| e.read_count == 0).collect();
                if !never_read.is_empty() {
                    lines.push(format!("  Never activated: {}", never_read.len()));
                }
            }
        }

        Ok(lines.join("\n"))
    }
}

// ── /steer — inject guidance into active run ────────────────────────────────

pub(in crate::channel_events) async fn handle_steer(
    state: &Arc<GatewayState>,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    if args.is_empty() {
        return Err(ChannelError::invalid_input(
            "usage: /steer <guidance>\nInject a steering note into the current agent run.",
        ));
    }

    // Check if there's an active run for this session.
    let chat = state.chat().await;
    let peek_res = chat
        .peek(serde_json::json!({ "sessionKey": session_key }))
        .await
        .map_err(|e| ChannelError::external("peek", e))?;

    let active = peek_res
        .get("active")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !active {
        return Err(ChannelError::invalid_input(
            "No active agent run to steer. Use /steer while the agent is working.",
        ));
    }

    state.set_steer_text(session_key, args.to_string()).await;

    broadcast(
        state,
        "chat",
        serde_json::json!({
            "sessionKey": session_key,
            "state": "steered",
            "text": args,
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;

    Ok(format!("Steering applied: {args}"))
}

// ── /queue — queue a message for the next agent turn ────────────────────────

pub(in crate::channel_events) async fn handle_queue(
    state: &Arc<GatewayState>,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    if args.is_empty() {
        return Err(ChannelError::invalid_input(
            "usage: /queue <message>\nQueue a message for the next agent turn without interrupting the current one.",
        ));
    }

    // Use the chat service's send method — when a run is active, it will
    // automatically queue the message according to MessageQueueMode.
    let chat = state.chat().await;
    let params = serde_json::json!({
        "text": args,
        "_session_key": session_key,
    });

    match chat.send(params).await {
        Ok(res) => {
            let queued = res.get("queued").and_then(|v| v.as_bool()).unwrap_or(false);
            if queued {
                Ok(format!("Queued for next turn: {args}"))
            } else {
                Ok("No active run \u{2014} message sent immediately.".to_string())
            }
        },
        Err(e) => Err(ChannelError::external("queue", e)),
    }
}
