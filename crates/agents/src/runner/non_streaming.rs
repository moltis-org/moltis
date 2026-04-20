//! Non-streaming agent loop: `run_agent_loop` and `run_agent_loop_with_context`.

use std::sync::Arc;

use {
    anyhow::Result,
    tracing::{debug, info, trace, warn},
};

use moltis_common::hooks::{HookAction, HookPayload, HookRegistry};

use crate::{
    model::{ChatMessage, LlmProvider, ToolCall, UserContent},
    response_sanitizer::recover_tool_calls_from_content,
    tool_arg_validator::validate_tool_args,
    tool_loop_detector::ToolCallFingerprint,
    tool_parsing::{
        looks_like_failed_tool_call, new_synthetic_tool_call_id, parse_tool_calls_from_text,
    },
    tool_registry::ToolRegistry,
};

use super::{
    AUTO_CONTINUE_NUDGE, AgentRunError, AgentRunResult, MALFORMED_TOOL_RETRY_PROMPT, OnEvent,
    RunnerEvent, UsageAccumulator, apply_loop_detector_intervention,
    channel_binding_from_tool_context, dispatch_after_llm_call_hook, empty_tool_name_retry_prompt,
    explicit_shell_command_from_user_content, find_empty_tool_name_call, finish_agent_run,
    has_named_tool_call, is_substantive_answer_text, record_answer_text, resolve_tool_lookup,
    retry::{
        RATE_LIMIT_MAX_RETRIES, is_context_window_error, next_retry_delay_ms,
        resolve_agent_max_iterations,
    },
    sanitize_tool_name,
    tool_result::sanitize_tool_result,
};

use crate::tool_loop_detector::ToolLoopDetector;

/// Run the agent loop: send messages to the LLM, execute tool calls, repeat.
///
/// If `history` is provided, those messages are inserted between the system
/// prompt and the current user message, giving the LLM conversational context.
pub async fn run_agent_loop(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_content: &UserContent,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
) -> Result<AgentRunResult, AgentRunError> {
    run_agent_loop_with_context(
        provider,
        tools,
        system_prompt,
        user_content,
        on_event,
        history,
        None,
        None,
        None,
    )
    .await
}

/// Like `run_agent_loop` but accepts optional context values that are injected
/// into every tool call's parameters (e.g. `_session_key`).
pub async fn run_agent_loop_with_context(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_content: &UserContent,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
    tool_context: Option<serde_json::Value>,
    hook_registry: Option<Arc<HookRegistry>>,
    sender_name: Option<String>,
) -> Result<AgentRunResult, AgentRunError> {
    let native_tools = provider.supports_tools();
    let config = moltis_config::discover_and_load();
    let max_tool_result_bytes = config.tools.max_tool_result_bytes;
    let max_auto_continues = config.tools.agent_max_auto_continues;
    let auto_continue_min_tool_calls = config.tools.agent_auto_continue_min_tool_calls;
    let compaction_ratio = config.tools.tool_result_compaction_ratio as usize;
    let overflow_ratio = config.tools.preemptive_overflow_ratio as usize;
    let compaction_min_iterations = config.tools.compaction_min_iterations;
    let base_max_iterations = resolve_agent_max_iterations(config.tools.agent_max_iterations);
    // Lazy mode needs extra iterations for tool_search discovery round-trips.
    let max_iterations = if config.tools.registry_mode == moltis_config::ToolRegistryMode::Lazy {
        base_max_iterations * 3
    } else {
        base_max_iterations
    };

    let is_multimodal = matches!(user_content, UserContent::Multimodal(_));
    info!(
        provider = provider.name(),
        model = provider.id(),
        native_tools,
        tools_count = tools.list_names().len(),
        is_multimodal,
        "starting agent loop"
    );

    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(system_prompt)];

    // Insert conversation history before the current user message.
    if let Some(hist) = history {
        messages.extend(hist);
    }

    messages.push(ChatMessage::User {
        content: user_content.clone(),
        name: sender_name,
    });
    let explicit_shell_command = explicit_shell_command_from_user_content(user_content);

    // Extract session key once for hook payloads.
    let session_key_for_hooks = tool_context
        .as_ref()
        .and_then(|ctx| ctx.get("_session_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let channel_for_hooks =
        channel_binding_from_tool_context(&session_key_for_hooks, tool_context.as_ref());

    let mut iterations = 0;
    let mut total_tool_calls = 0;
    let mut usage_accumulator = UsageAccumulator::default();
    let mut server_retries_remaining: u8 = 1;
    let mut rate_limit_retries_remaining: u8 = RATE_LIMIT_MAX_RETRIES;
    let mut rate_limit_backoff_ms: Option<u64> = None;
    let mut last_answer_text = String::new();
    let mut malformed_retry_count: u8 = 0;
    let mut empty_tool_name_retry_count: u8 = 0;
    let mut auto_continue_count: usize = 0;
    let mut loop_detector = ToolLoopDetector::new(
        config.tools.agent_loop_detector_window,
        config.tools.agent_loop_detector_strip_tools_on_second_fire,
    );
    let mut strip_tools_next_iter = false;

    loop {
        iterations += 1;
        if iterations > max_iterations {
            warn!("agent loop exceeded max iterations ({})", max_iterations);
            return Err(AgentRunError::Other(anyhow::anyhow!(
                "agent loop exceeded max iterations ({})",
                max_iterations
            )));
        }

        // Re-compute schemas each iteration so activated tools appear immediately.
        // When the loop detector has escalated to stage 2, pass an empty tool
        // list for this single turn so the model is forced to respond in text.
        let schemas_for_api = if native_tools && !strip_tools_next_iter {
            tools.list_schemas()
        } else {
            vec![]
        };
        if strip_tools_next_iter {
            strip_tools_next_iter = false;
            loop_detector.clear_strip_tools();
        }

        let effective_ratio = if iterations > compaction_min_iterations {
            compaction_ratio
        } else {
            0 // skip compaction but still check for overflow
        };
        super::enforce_tool_result_context_budget(
            &mut messages,
            &schemas_for_api,
            provider.context_window(),
            effective_ratio,
            overflow_ratio,
        )?;

        if let Some(cb) = on_event {
            cb(RunnerEvent::Iteration(iterations));
        }

        info!(
            iteration = iterations,
            messages_count = messages.len(),
            "calling LLM"
        );
        trace!(iteration = iterations, messages = ?messages, "LLM request messages");

        // Dispatch BeforeLLMCall hook — may block the LLM call.
        if let Some(ref hooks) = hook_registry {
            let msgs_json: Vec<serde_json::Value> =
                messages.iter().map(|m| m.to_openai_value()).collect();
            let payload = HookPayload::BeforeLLMCall {
                session_key: session_key_for_hooks.clone(),
                provider: provider.name().to_string(),
                model: provider.id().to_string(),
                messages: serde_json::Value::Array(msgs_json),
                tool_count: schemas_for_api.len(),
                iteration: iterations,
            };
            match hooks.dispatch(&payload).await {
                Ok(HookAction::Block(reason)) => {
                    warn!(reason = %reason, "LLM call blocked by BeforeLLMCall hook");
                    return Err(AgentRunError::Other(anyhow::anyhow!(
                        "blocked by BeforeLLMCall hook: {reason}"
                    )));
                },
                Ok(HookAction::ModifyPayload(_)) => {
                    debug!("BeforeLLMCall ModifyPayload ignored (messages are typed)");
                },
                Ok(HookAction::Continue) => {},
                Err(e) => {
                    warn!(error = %e, "BeforeLLMCall hook dispatch failed");
                },
            }
        }

        if let Some(cb) = on_event {
            cb(RunnerEvent::Thinking);
        }

        let mut response = match provider.complete(&messages, &schemas_for_api).await {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                if is_context_window_error(&msg) {
                    return Err(AgentRunError::ContextWindowExceeded(msg));
                }
                if let Some(delay_ms) = next_retry_delay_ms(
                    &msg,
                    &mut server_retries_remaining,
                    &mut rate_limit_retries_remaining,
                    &mut rate_limit_backoff_ms,
                ) {
                    iterations -= 1;
                    warn!(
                        error = %msg,
                        delay_ms,
                        server_retries_remaining,
                        rate_limit_retries_remaining,
                        "transient LLM error, retrying after delay"
                    );
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::RetryingAfterError {
                            error: msg,
                            delay_ms,
                        });
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                return Err(AgentRunError::Other(e));
            },
        };

        if let Some(cb) = on_event {
            cb(RunnerEvent::ThinkingDone);
        }

        usage_accumulator.record_request(response.usage.clone());

        info!(
            iteration = iterations,
            has_text = response.text.is_some(),
            tool_calls_count = response.tool_calls.len(),
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            "LLM response received"
        );
        if let Some(ref text) = response.text {
            trace!(iteration = iterations, text = %text, "LLM response text");
        }

        // Fallback: parse tool calls from model text if the provider returned
        // no structured tool calls (some providers/models emit text-based calls).
        if response.tool_calls.is_empty()
            && let Some(ref text) = response.text
        {
            let (parsed, remaining) = parse_tool_calls_from_text(text);
            if !parsed.is_empty() {
                info!(
                    native_tools,
                    count = parsed.len(),
                    first_tool = %parsed[0].name,
                    "parsed tool call(s) from text fallback"
                );
                response.text = remaining;
                response.tool_calls = parsed;
            }
        }

        // One-shot retry for malformed tool calls: if the text looks like a
        // failed tool call attempt, ask the model to retry with exact format.
        if response.tool_calls.is_empty()
            && looks_like_failed_tool_call(&response.text)
            && malformed_retry_count == 0
        {
            malformed_retry_count += 1;
            info!("detected malformed tool call, requesting retry");
            messages.push(ChatMessage::assistant(
                response.text.as_deref().unwrap_or(""),
            ));
            messages.push(ChatMessage::user(MALFORMED_TOOL_RETRY_PROMPT));
            continue;
        }

        // Fallback: recover tool calls from XML blocks (<function_call>, <tool_call>).
        if !native_tools
            && response.tool_calls.is_empty()
            && let Some(ref text) = response.text
        {
            let (cleaned, recovered) = recover_tool_calls_from_content(text);
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "recovered tool calls from XML blocks in response text"
                );
                response.text = if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                };
                response.tool_calls = recovered;
            }
        }

        // Final fallback: if the user turn is an explicit `/sh ...` command and
        // the model returned plain text, force one exec tool call so this path
        // is deterministic in the UI.
        if response.tool_calls.is_empty()
            && iterations == 1
            && total_tool_calls == 0
            && let Some(command) = explicit_shell_command.as_ref()
            && tools.get("exec").is_some()
        {
            info!(command = %command, "forcing exec tool call from explicit /sh command");
            // Preserve the model's planning/reasoning text on the assistant
            // tool-call message. Some providers (e.g. Moonshot thinking mode)
            // require this history field for follow-up tool turns.
            response.tool_calls = vec![ToolCall {
                id: new_synthetic_tool_call_id("forced"),
                name: "exec".to_string(),
                arguments: serde_json::json!({ "command": command }),
                metadata: None,
            }];
        }

        if let Some(tc) = find_empty_tool_name_call(&response.tool_calls) {
            if has_named_tool_call(&response.tool_calls) {
                warn!(
                    tool_call_id = %tc.id,
                    "structured tool call batch contains both empty and valid tool names; preserving valid sibling tool calls and falling back to normal tool error handling"
                );
            } else if empty_tool_name_retry_count == 0 {
                empty_tool_name_retry_count += 1;
                info!(tool_call_id = %tc.id, "detected structured tool call with empty name, requesting retry");
                record_answer_text(&mut last_answer_text, &response.text);
                messages.push(ChatMessage::assistant(
                    response.text.as_deref().unwrap_or(""),
                ));
                messages.push(ChatMessage::user(empty_tool_name_retry_prompt(tc)));
                continue;
            }
            warn!(
                tool_call_id = %tc.id,
                "structured tool call still has empty name after retry; falling back to normal tool error handling"
            );
        }

        for tc in &response.tool_calls {
            info!(
                iteration = iterations,
                tool_name = %tc.name,
                arguments = %tc.arguments,
                "LLM requested tool call"
            );
        }

        // Dispatch AfterLLMCall hook — may block tool execution.
        dispatch_after_llm_call_hook(
            hook_registry.as_ref(),
            &session_key_for_hooks,
            provider.name(),
            provider.id(),
            response.text.clone(),
            &response.tool_calls,
            &response.usage,
            iterations,
        )
        .await?;

        // If no tool calls, auto-continue or return the text response.
        if response.tool_calls.is_empty() {
            let response_text = response
                .text
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_default();

            // Auto-continue: if the model made tool calls earlier in this run
            // and we haven't exhausted nudges, ask it to keep going. Suppress
            // the nudge when the model already produced a substantive final
            // answer — nudging in that case risks losing the answer (GH #628).
            if !is_substantive_answer_text(&response_text)
                && total_tool_calls > 0
                && total_tool_calls >= auto_continue_min_tool_calls
                && auto_continue_count < max_auto_continues
            {
                auto_continue_count += 1;
                info!(
                    iterations,
                    auto_continue_count, "model stopped without tool calls, auto-continuing"
                );
                if let Some(cb) = on_event {
                    cb(RunnerEvent::AutoContinue {
                        iteration: iterations,
                        max_iterations,
                    });
                }
                if !response_text.is_empty() {
                    messages.push(ChatMessage::assistant(&response_text));
                }
                messages.push(ChatMessage::user(AUTO_CONTINUE_NUDGE));
                continue;
            }

            let text = if !response_text.is_empty() {
                response_text
            } else {
                std::mem::take(&mut last_answer_text)
            };

            info!(
                iterations,
                tool_calls = total_tool_calls,
                "agent loop complete — returning text"
            );
            return Ok(finish_agent_run(
                text,
                iterations,
                total_tool_calls,
                &usage_accumulator,
                Vec::new(),
            ));
        }

        // Append assistant message with tool calls.
        // Save any answer text for fallback — when the final iteration returns
        // empty, this becomes the result. Don't emit as ThinkingText because
        // it may be the actual answer (e.g. a table produced before a cleanup
        // tool call like `browser close`).
        record_answer_text(&mut last_answer_text, &response.text);
        messages.push(ChatMessage::assistant_with_tools(
            response.text.clone(),
            response.tool_calls.clone(),
        ));

        // Execute tool calls concurrently.
        total_tool_calls += response.tool_calls.len();

        // Build futures for all tool calls (executed concurrently).
        //
        // Pre-dispatch schema validation runs synchronously against each
        // tool's declared `parameters_schema`. Calls that fail validation
        // are short-circuited to a directive error response — the tool's
        // `execute` method is never invoked, and the UI receives a
        // `ToolCallRejected` event instead of the misleading
        // `ToolCallStart`/"executing" status (issue #658).
        let tool_futures: Vec<_> = response
            .tool_calls
            .iter()
            .map(|tc| {
                let sanitized = sanitize_tool_name(&tc.name);
                if *sanitized != tc.name {
                    debug!(original = %tc.name, sanitized = %sanitized, "sanitized mangled tool name");
                }
                let (tool, resolved_name) = resolve_tool_lookup(tools, sanitized.as_ref());
                if resolved_name.as_ref() != sanitized.as_ref() {
                    debug!(original = %sanitized, resolved = %resolved_name, "resolved legacy tool alias");
                }
                let mut args = tc.arguments.clone();

                // Dispatch BeforeToolCall hook — may block or modify arguments.
                let hook_registry = hook_registry.clone();
                let session_key = session_key_for_hooks.clone();
                let channel_for_hooks = channel_for_hooks.clone();
                let tc_name = resolved_name.to_string();
                let _tc_id = tc.id.clone();

                if let Some(ref ctx) = tool_context
                    && let (Some(args_obj), Some(ctx_obj)) = (args.as_object_mut(), ctx.as_object())
                {
                    for (k, v) in ctx_obj {
                        args_obj.insert(k.clone(), v.clone());
                    }
                }

                // Pre-dispatch validation against the tool's schema.
                let validation_error: Option<String> = if let Some(ref t) = tool {
                    let schema = t.parameters_schema();
                    match validate_tool_args(&schema, &args) {
                        Ok(()) => None,
                        Err(e) => {
                            warn!(
                                tool = %tc_name,
                                summary = %e.short_summary(),
                                "tool call rejected by pre-dispatch schema validation"
                            );
                            Some(e.to_llm_error_message(&tc_name))
                        },
                    }
                } else {
                    None
                };

                // Emit ToolCallStart only for calls that will actually run.
                // Rejected calls get a single `ToolCallRejected` event after
                // the concurrent batch completes (handled in the result loop).
                if validation_error.is_none() {
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::ToolCallStart {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            arguments: args.clone(),
                        });
                    }
                    info!(tool = %tc_name, id = %tc.id, args = %args, "executing tool");
                }

                async move {
                    if let Some(err_msg) = validation_error {
                        return (
                            false,
                            serde_json::json!({ "error": err_msg.clone() }),
                            Some(err_msg),
                            true,
                        );
                    }
                    // Run BeforeToolCall hook.
                    if let Some(ref hooks) = hook_registry {
                        let payload = HookPayload::BeforeToolCall {
                            session_key: session_key.clone(),
                            tool_name: tc_name.clone(),
                            arguments: args.clone(),
                            channel: channel_for_hooks.clone(),
                        };
                        match hooks.dispatch(&payload).await {
                            Ok(HookAction::Block(reason)) => {
                                warn!(tool = %tc_name, reason = %reason, "tool call blocked by hook");
                                let err_str = format!("blocked by hook: {reason}");
                                return (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                    false,
                                );
                            },
                            Ok(HookAction::ModifyPayload(v)) => {
                                args = v;
                            },
                            Ok(HookAction::Continue) => {},
                            Err(e) => {
                                warn!(tool = %tc_name, error = %e, "BeforeToolCall hook dispatch failed");
                            },
                        }
                    }

                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(val) => {
                                // Check if the result indicates a logical failure
                                // (e.g., BrowserResponse with success: false)
                                let has_error = val.get("error").is_some()
                                    || val.get("success") == Some(&serde_json::json!(false));
                                let error_msg = if has_error {
                                    val.get("error")
                                        .and_then(|e| e.as_str())
                                        .map(String::from)
                                } else {
                                    None
                                };

                                // Dispatch AfterToolCall hook.
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: !has_error,
                                        result: Some(val.clone()),
                                        channel: channel_for_hooks.clone(),
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }

                                if has_error {
                                    // Tool executed but returned an error in the result
                                    (false, serde_json::json!({ "result": val }), error_msg, false)
                                } else {
                                    (true, serde_json::json!({ "result": val }), None, false)
                                }
                            },
                            Err(e) => {
                                let err_str = e.to_string();
                                // Dispatch AfterToolCall hook on failure.
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: false,
                                        result: None,
                                        channel: channel_for_hooks.clone(),
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }
                                (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                    false,
                                )
                            },
                        }
                    } else {
                        let err_str = format!("unknown tool: {tc_name}");
                        (
                            false,
                            serde_json::json!({ "error": err_str }),
                            Some(err_str),
                            false,
                        )
                    }
                }
            })
            .collect();

        // Execute all tools concurrently and collect results in order.
        let results = futures::future::join_all(tool_futures).await;

        // Process results in original order: emit events, append messages.
        // The loop detector records each outcome as it is processed; the
        // authoritative intervention decision is derived AFTER the loop from
        // the detector's post-batch state via `consume_pending_action()`.
        // This avoids two edge cases that per-call return values hit in
        // mixed batches:
        //   1. Trailing success after a triggering failure must NOT leave a
        //      stale intervention — the reset() abandons it cleanly.
        //   2. A batch that races through both escalation stages must still
        //      deliver the stage-1 nudge first, not skip straight to strip.
        for (tc, (success, mut result, error, rejected)) in response.tool_calls.iter().zip(results)
        {
            if success {
                info!(tool = %tc.name, id = %tc.id, "tool execution succeeded");
                trace!(tool = %tc.name, result = %result, "tool result");
            } else if rejected {
                warn!(
                    tool = %tc.name,
                    id = %tc.id,
                    "tool call rejected before execution by pre-dispatch validation"
                );
            } else {
                warn!(tool = %tc.name, id = %tc.id, error = %error.as_deref().unwrap_or(""), "tool execution failed");
            }

            // Record outcome in the loop detector. Use explicit success/failure
            // constructors so tools returning `{success: false}` without an
            // `error` field still register as failures.
            if loop_detector.is_enabled() {
                let fp = if success {
                    ToolCallFingerprint::success(&tc.name, &tc.arguments)
                } else {
                    ToolCallFingerprint::failure(&tc.name, &tc.arguments, error.as_deref())
                };
                let _ = loop_detector.record(fp);
            }

            if let Some(cb) = on_event {
                if rejected {
                    cb(RunnerEvent::ToolCallRejected {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                        error: error.clone().unwrap_or_default(),
                    });
                } else {
                    cb(RunnerEvent::ToolCallEnd {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        success,
                        error,
                        result: if success {
                            result.get("result").cloned()
                        } else {
                            None
                        },
                    });
                }
            }

            // Dispatch ToolResultPersist hook — the last opportunity for a handler
            // to sanitize, redact, or block attacker-controlled tool output before
            // it enters the messages array and is reasoned on by the next LLM
            // iteration. Block substitutes an error marker instead of aborting the
            // run, so a single hostile tool result cannot kill a long-running
            // autonomous agent.
            if let Some(ref hooks) = hook_registry {
                let payload = HookPayload::ToolResultPersist {
                    session_key: session_key_for_hooks.clone(),
                    tool_name: sanitize_tool_name(&tc.name).into_owned(),
                    result: result.clone(),
                    channel: channel_for_hooks.clone(),
                };
                match hooks.dispatch(&payload).await {
                    Ok(HookAction::ModifyPayload(v)) => {
                        debug!(tool = %tc.name, "ToolResultPersist replaced tool result");
                        result = v;
                    },
                    Ok(HookAction::Block(reason)) => {
                        warn!(tool = %tc.name, reason = %reason, "ToolResultPersist blocked result — substituting error marker");
                        result = serde_json::json!({
                            "error": format!("blocked by hook: {reason}")
                        });
                    },
                    Ok(HookAction::Continue) => {},
                    Err(e) => {
                        warn!(tool = %tc.name, error = %e, "ToolResultPersist hook dispatch failed");
                    },
                }
            }

            // Always sanitize tool results as strings - most LLM APIs don't support
            // multimodal content in tool results. Images are stripped but the UI
            // still receives them via ToolCallEnd event.
            let tool_result_str = sanitize_tool_result(&result.to_string(), max_tool_result_bytes);
            debug!(
                tool = %tc.name,
                id = %tc.id,
                result_len = tool_result_str.len(),
                "appending tool result to messages"
            );
            trace!(tool = %tc.name, content = %tool_result_str, "tool result message content");

            messages.push(ChatMessage::tool(&tc.id, &tool_result_str));
        }

        // Apply loop-detector intervention if one fired during this batch.
        apply_loop_detector_intervention(
            &mut loop_detector,
            &mut messages,
            &mut strip_tools_next_iter,
            on_event,
        );
    }
}

/// Convenience wrapper matching the old stub signature.
pub async fn run_agent(_agent_id: &str, _session_key: &str, _message: &str) -> Result<String> {
    anyhow::bail!(
        "run_agent requires a configured provider and tool registry; use run_agent_loop instead"
    )
}
