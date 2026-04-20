use super::*;

/// Convenience wrapper matching the old stub signature.
pub async fn run_agent(_agent_id: &str, _session_key: &str, _message: &str) -> Result<String> {
    bail!("run_agent requires a configured provider and tool registry; use run_agent_loop instead")
}

/// Streaming variant of the agent loop.
///
/// Unlike `run_agent_loop_with_context`, this function uses streaming to send
/// text deltas to the UI as they arrive, providing a much better UX.
///
/// Tool calls are accumulated from the stream and executed after the stream
/// completes, then the loop continues with the next iteration.
pub async fn run_agent_loop_streaming(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_content: &UserContent,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
    tool_context: Option<serde_json::Value>,
    hook_registry: Option<Arc<HookRegistry>>,
) -> Result<AgentRunResult, AgentRunError> {
    let native_tools = provider.supports_tools();
    let config = moltis_config::discover_and_load();
    let max_tool_result_bytes = config.tools.max_tool_result_bytes;
    let max_auto_continues = config.tools.agent_max_auto_continues;
    let auto_continue_min_tool_calls = config.tools.agent_auto_continue_min_tool_calls;
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
        "starting streaming agent loop"
    );

    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(system_prompt)];

    // Insert conversation history before the current user message.
    if let Some(hist) = history {
        messages.extend(hist);
    }

    messages.push(ChatMessage::User {
        content: user_content.clone(),
        name: None,
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
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;
    let mut server_retries_remaining: u8 = 1;
    let mut rate_limit_retries_remaining: u8 = RATE_LIMIT_MAX_RETRIES;
    let mut rate_limit_backoff_ms: Option<u64> = None;
    let mut raw_llm_responses: Vec<serde_json::Value> = Vec::new();
    // Track answer text from iterations that also contained tool calls.
    // When the final iteration is empty (e.g. model stop after browser close),
    // this is used as the final response text instead of returning silent.
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
            warn!(
                "streaming agent loop exceeded max iterations ({})",
                max_iterations
            );
            return Err(AgentRunError::Other(anyhow::anyhow!(
                "agent loop exceeded max iterations ({})",
                max_iterations
            )));
        }

        // Re-compute schemas each iteration so activated tools appear immediately.
        // When the loop detector has escalated to stage 2, pass an empty tool
        // list for this single turn so the model is forced to respond in text
        // (issue #658).
        let schemas_for_api = if native_tools && !strip_tools_next_iter {
            tools.list_schemas()
        } else {
            vec![]
        };
        if strip_tools_next_iter {
            strip_tools_next_iter = false;
            loop_detector.clear_strip_tools();
        }

        enforce_tool_result_context_budget(
            &mut messages,
            &schemas_for_api,
            provider.context_window(),
        )?;

        if let Some(cb) = on_event {
            cb(RunnerEvent::Iteration(iterations));
        }

        info!(
            iteration = iterations,
            messages_count = messages.len(),
            "calling LLM (streaming)"
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

        // Use streaming API.
        #[cfg(feature = "metrics")]
        let iter_start = std::time::Instant::now();
        let mut stream = provider.stream_with_tools(messages.clone(), schemas_for_api.clone());

        // Accumulate answer text, reasoning text, and tool calls from the stream.
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Map streaming index → accumulated JSON args string.
        let mut tool_call_args: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        // Map streaming index → position in the `tool_calls` vec.
        // The streaming index may not start at 0 (e.g. Copilot proxying
        // Anthropic uses the content-block index, so a text block at index 0
        // pushes the tool_use to index 1).
        let mut stream_idx_to_vec_pos: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut stream_error: Option<String> = None;

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(text) => {
                    accumulated_text.push_str(&text);
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::TextDelta(text));
                    }
                },
                StreamEvent::ProviderRaw(raw) => {
                    if raw_llm_responses.len() < 256 {
                        raw_llm_responses.push(raw);
                    }
                },
                StreamEvent::ReasoningDelta(text) => {
                    accumulated_reasoning.push_str(&text);
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::ThinkingText(accumulated_reasoning.clone()));
                    }
                },
                StreamEvent::ToolCallStart {
                    id, name, index, ..
                } => {
                    let vec_pos = tool_calls.len();
                    debug!(tool = %name, id = %id, stream_index = index, vec_pos, "tool call started in stream");
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: serde_json::json!({}),
                        metadata: None,
                    });
                    stream_idx_to_vec_pos.insert(index, vec_pos);
                    tool_call_args.insert(index, String::new());
                },
                StreamEvent::ToolCallArgumentsDelta { index, delta } => {
                    if let Some(args) = tool_call_args.get_mut(&index) {
                        args.push_str(&delta);
                    }
                },
                StreamEvent::ToolCallComplete { index } => {
                    // Arguments are finalized after stream completes.
                    // Just log for now - we'll parse accumulated args later.
                    debug!(index, "tool call arguments complete");
                },
                StreamEvent::Done(usage) => {
                    input_tokens = usage.input_tokens;
                    output_tokens = usage.output_tokens;
                    debug!(input_tokens, output_tokens, "stream done");

                    #[cfg(feature = "metrics")]
                    {
                        let provider_name = provider.name().to_string();
                        let model_id = provider.id().to_string();
                        let duration = iter_start.elapsed().as_secs_f64();
                        counter!(
                            llm_metrics::COMPLETIONS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(1);
                        counter!(
                            llm_metrics::INPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.input_tokens));
                        counter!(
                            llm_metrics::OUTPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.output_tokens));
                        counter!(
                            llm_metrics::CACHE_READ_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.cache_read_tokens));
                        counter!(
                            llm_metrics::CACHE_WRITE_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.cache_write_tokens));
                        histogram!(
                            llm_metrics::COMPLETION_DURATION_SECONDS,
                            labels::PROVIDER => provider_name,
                            labels::MODEL => model_id
                        )
                        .record(duration);
                    }
                },
                StreamEvent::Error(msg) => {
                    stream_error = Some(msg);
                    break;
                },
            }
        }

        if let Some(cb) = on_event {
            cb(RunnerEvent::ThinkingDone);
        }

        // Handle stream errors — retry on transient failures/rate limits.
        if let Some(err) = stream_error {
            if is_context_window_error(&err) {
                return Err(AgentRunError::ContextWindowExceeded(err));
            }
            if let Some(delay_ms) = next_retry_delay_ms(
                &err,
                &mut server_retries_remaining,
                &mut rate_limit_retries_remaining,
                &mut rate_limit_backoff_ms,
            ) {
                // Don't count the failed attempt as an iteration.
                iterations -= 1;
                warn!(
                    error = %err,
                    delay_ms,
                    server_retries_remaining,
                    rate_limit_retries_remaining,
                    "transient LLM error, retrying after delay"
                );
                if let Some(cb) = on_event {
                    cb(RunnerEvent::RetryingAfterError {
                        error: err,
                        delay_ms,
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }
            return Err(AgentRunError::Other(anyhow::anyhow!(err)));
        }

        total_input_tokens = total_input_tokens.saturating_add(input_tokens);
        total_output_tokens = total_output_tokens.saturating_add(output_tokens);

        // Finalize tool call arguments from accumulated strings.
        // Use stream_idx_to_vec_pos to map streaming indices (which may not
        // start at 0) to the actual position in the tool_calls vec.
        for (stream_idx, args_str) in &tool_call_args {
            // Emit raw accumulated string at debug level so future variants of
            // "default to {} because no deltas arrived" can be diagnosed
            // without a repro (issue #658).
            debug!(
                stream_idx,
                args_str = %args_str,
                "finalizing tool call args"
            );
            if let Some(&vec_pos) = stream_idx_to_vec_pos.get(stream_idx)
                && vec_pos < tool_calls.len()
                && !args_str.is_empty()
                && let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str)
            {
                tool_calls[vec_pos].arguments = args;
            }
        }

        info!(
            iteration = iterations,
            has_text = !accumulated_text.is_empty(),
            tool_calls_count = tool_calls.len(),
            input_tokens,
            output_tokens,
            "streaming LLM response complete"
        );

        // Fallback: parse tool calls from model text if the provider returned
        // no structured tool calls (some providers/models emit text-based calls).
        if tool_calls.is_empty() && !accumulated_text.is_empty() {
            let (parsed, remaining) = parse_tool_calls_from_text(&accumulated_text);
            if !parsed.is_empty() {
                info!(
                    native_tools,
                    count = parsed.len(),
                    first_tool = %parsed[0].name,
                    "parsed tool call(s) from text fallback"
                );
                accumulated_text = remaining.unwrap_or_default();
                tool_calls = parsed;
            }
        }

        // One-shot retry for malformed tool calls in streaming mode.
        if tool_calls.is_empty()
            && looks_like_failed_tool_call(&Some(accumulated_text.clone()))
            && malformed_retry_count == 0
        {
            malformed_retry_count += 1;
            info!("detected malformed tool call in stream, requesting retry");
            messages.push(ChatMessage::assistant(&accumulated_text));
            messages.push(ChatMessage::user(MALFORMED_TOOL_RETRY_PROMPT));
            continue;
        }

        // Fallback: recover tool calls from XML blocks (<function_call>, <tool_call>).
        if !native_tools && tool_calls.is_empty() && !accumulated_text.is_empty() {
            let (cleaned, recovered) = recover_tool_calls_from_content(&accumulated_text);
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "recovered tool calls from XML blocks in streamed text"
                );
                accumulated_text = cleaned;
                tool_calls = recovered;
            }
        }

        // Final fallback: if the user turn is an explicit `/sh ...` command and
        // the model returned plain text, force one exec tool call so this path
        // is deterministic in the UI.
        if tool_calls.is_empty()
            && iterations == 1
            && total_tool_calls == 0
            && let Some(command) = explicit_shell_command.as_ref()
            && tools.get("exec").is_some()
        {
            info!(command = %command, "forcing exec tool call from explicit /sh command");
            // Preserve streamed reasoning/planning text on the assistant tool
            // message so providers that validate thinking history accept the
            // next iteration.
            tool_calls = vec![ToolCall {
                id: new_synthetic_tool_call_id("forced"),
                name: "exec".to_string(),
                arguments: serde_json::json!({ "command": command }),
                metadata: None,
            }];
        }

        if let Some(tc) = find_empty_tool_name_call(&tool_calls) {
            if has_named_tool_call(&tool_calls) {
                warn!(
                    tool_call_id = %tc.id,
                    "streamed tool call batch contains both empty and valid tool names; preserving valid sibling tool calls and falling back to normal tool error handling"
                );
            } else if empty_tool_name_retry_count == 0 {
                empty_tool_name_retry_count += 1;
                info!(tool_call_id = %tc.id, "detected structured tool call with empty name in stream, requesting retry");
                let retry_text = streaming_tool_call_message_content(
                    &mut last_answer_text,
                    &accumulated_text,
                    &accumulated_reasoning,
                );
                messages.push(ChatMessage::assistant(retry_text.unwrap_or_default()));
                messages.push(ChatMessage::user(empty_tool_name_retry_prompt(tc)));
                continue;
            }
            warn!(
                tool_call_id = %tc.id,
                "structured tool call in stream still has empty name after retry; falling back to normal tool error handling"
            );
        }

        // Dispatch AfterLLMCall hook — may block tool execution.
        if let Some(ref hooks) = hook_registry {
            let tc_json: Vec<serde_json::Value> = tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    })
                })
                .collect();
            let payload = HookPayload::AfterLLMCall {
                session_key: session_key_for_hooks.clone(),
                provider: provider.name().to_string(),
                model: provider.id().to_string(),
                text: if accumulated_text.is_empty() {
                    None
                } else {
                    Some(accumulated_text.clone())
                },
                tool_calls: tc_json,
                input_tokens,
                output_tokens,
                iteration: iterations,
            };
            match hooks.dispatch(&payload).await {
                Ok(HookAction::Block(reason)) => {
                    warn!(reason = %reason, "LLM response blocked by AfterLLMCall hook");
                    return Err(AgentRunError::Other(anyhow::anyhow!(
                        "blocked by AfterLLMCall hook: {reason}"
                    )));
                },
                Ok(HookAction::ModifyPayload(_)) => {
                    debug!("AfterLLMCall ModifyPayload ignored (response is typed)");
                },
                Ok(HookAction::Continue) => {},
                Err(e) => {
                    warn!(error = %e, "AfterLLMCall hook dispatch failed");
                },
            }
        }

        // If no tool calls, auto-continue or return the text response.
        if tool_calls.is_empty() {
            // Auto-continue: if the model made tool calls earlier in this run
            // and we haven't exhausted nudges, ask it to keep going. Suppress
            // the nudge when the model already produced a substantive final
            // answer — nudging in that case risks losing the answer (GH #628).
            if !is_substantive_answer_text(&accumulated_text)
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
                if !accumulated_text.is_empty() {
                    messages.push(ChatMessage::assistant(&accumulated_text));
                }
                messages.push(ChatMessage::user(AUTO_CONTINUE_NUDGE));
                continue;
            }

            // When the final iteration produced no text but a previous iteration
            // streamed answer text alongside tool calls, use that as the response.
            let final_text = if accumulated_text.is_empty() && !last_answer_text.is_empty() {
                std::mem::take(&mut last_answer_text)
            } else {
                accumulated_text
            };
            info!(
                iterations,
                tool_calls = total_tool_calls,
                "streaming agent loop complete — returning text"
            );
            return Ok(AgentRunResult {
                text: clean_response(&final_text),
                iterations,
                tool_calls_made: total_tool_calls,
                usage: Usage {
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    ..Default::default()
                },
                request_usage: Usage {
                    input_tokens,
                    output_tokens,
                    ..Default::default()
                },
                raw_llm_responses,
            });
        }

        // Append assistant message with tool calls.
        //
        // When the model emits explicit reasoning (extended thinking), use
        // that as the planning text and emit it as ThinkingText for the UI.
        // When there is only regular text alongside tool calls (no separate
        // reasoning), preserve it on the message for history but do NOT emit
        // it as ThinkingText — it was already streamed as TextDelta and is
        // likely the actual answer (e.g. a search result table produced
        // before a `browser close` cleanup call).
        let (text_for_msg, is_actual_reasoning) = if !accumulated_reasoning.is_empty() {
            (Some(accumulated_reasoning), true)
        } else if !accumulated_text.is_empty() {
            last_answer_text.clone_from(&accumulated_text);
            (Some(accumulated_text), false)
        } else {
            (None, false)
        };
        if let Some(ref text) = text_for_msg
            && is_actual_reasoning
            && let Some(cb) = on_event
        {
            cb(RunnerEvent::ThinkingText(text.clone()));
        }
        messages.push(ChatMessage::assistant_with_tools(
            text_for_msg,
            tool_calls.clone(),
        ));

        // Execute tool calls concurrently.
        total_tool_calls += tool_calls.len();

        // Build futures for all tool calls (executed concurrently).
        //
        // Pre-dispatch schema validation runs synchronously against each
        // tool's declared `parameters_schema`. Calls that fail validation
        // are short-circuited to a directive error response without invoking
        // `execute`, and the UI receives `ToolCallRejected` instead of the
        // misleading "executing" status (issue #658).
        let tool_futures: Vec<_> = tool_calls
            .iter()
            .map(|tc| {
                let sanitized = sanitize_tool_name(&tc.name);
                if *sanitized != tc.name {
                    debug!(original = %tc.name, sanitized = %sanitized, "sanitized mangled tool name");
                }
                let tool = tools.get(&sanitized);
                let mut args = tc.arguments.clone();

                let hook_registry = hook_registry.clone();
                let session_key = session_key_for_hooks.clone();
                let channel_for_hooks = channel_for_hooks.clone();
                let tc_name = sanitized.to_string();

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
                            }
                            Ok(HookAction::ModifyPayload(v)) => {
                                args = v;
                            }
                            Ok(HookAction::Continue) => {}
                            Err(e) => {
                                warn!(tool = %tc_name, error = %e, "BeforeToolCall hook dispatch failed");
                            }
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
                                    (false, serde_json::json!({ "result": val }), error_msg, false)
                                } else {
                                    (true, serde_json::json!({ "result": val }), None, false)
                                }
                            }
                            Err(e) => {
                                let err_str = e.to_string();
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
                            }
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
        // Intervention is derived from the detector's post-batch state
        // via `consume_pending_action()` below — see the non-streaming
        // path for the rationale (issue #658).
        for (tc, (success, mut result, error, rejected)) in tool_calls.iter().zip(results) {
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

            // Record outcome in the loop detector (issue #658).
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
