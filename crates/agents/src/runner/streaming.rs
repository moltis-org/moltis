//! Streaming variant of the agent loop.

use std::{collections::HashSet, sync::Arc};

use {
    anyhow::Result,
    tracing::{debug, info, trace, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, llm as llm_metrics};

use futures::StreamExt;

use moltis_common::hooks::{HookAction, HookPayload, HookRegistry};

use crate::{
    model::{
        AgentToolControls, ChatMessage, LlmProvider, ProviderAttemptEvent, ProviderIdentity,
        StreamEvent, ToolCall, ToolChoice, TrackedStreamEvent, Usage, UserContent,
        decode_tool_call_arguments_from_str, push_capped_provider_raw_event,
    },
    response_sanitizer::{clean_response, recover_tool_calls_from_content},
    tool_arg_validator::{coerce_scalar_args, validate_tool_args},
    tool_loop_detector::ToolCallFingerprint,
    tool_parsing::{
        looks_like_failed_tool_call, new_synthetic_tool_call_id, parse_tool_calls_from_text,
    },
    tool_registry::ToolRegistry,
};

use super::{
    AUTO_CONTINUE_NUDGE, AgentLoopLimits, AgentRunError, AgentRunResult,
    MALFORMED_TOOL_RETRY_PROMPT, OnEvent, RunnerEvent, UsageAccumulator,
    apply_before_llm_call_modify_payload, apply_loop_detector_intervention,
    channel_binding_from_tool_context, dispatch_after_llm_call_hook,
    dispatch_before_agent_start_hook, empty_tool_name_retry_prompt,
    explicit_shell_command_from_user_content, find_empty_tool_name_call, finish_agent_run,
    has_named_tool_call, instrumentation, is_substantive_answer_text, log_tool_argument_diagnostic,
    resolve_tool_lookup,
    retry::{
        RATE_LIMIT_MAX_RETRIES, is_context_window_error, next_retry_delay_ms,
        resolve_agent_max_iterations,
    },
    sanitize_tool_name, streaming_tool_call_message_content,
    tool_result::{sanitize_tool_result, tool_result_failure},
};

use crate::tool_loop_detector::ToolLoopDetector;

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
    sender_name: Option<String>,
    steer_inbox: Option<super::SteerInbox>,
) -> Result<AgentRunResult, AgentRunError> {
    run_agent_loop_streaming_with_limits(
        provider,
        tools,
        system_prompt,
        user_content,
        on_event,
        history,
        tool_context,
        hook_registry,
        sender_name,
        steer_inbox,
        Default::default(),
    )
    .await
}

pub async fn run_agent_loop_streaming_with_limits(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_content: &UserContent,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
    tool_context: Option<serde_json::Value>,
    hook_registry: Option<Arc<HookRegistry>>,
    sender_name: Option<String>,
    steer_inbox: Option<super::SteerInbox>,
    limits: AgentLoopLimits,
) -> Result<AgentRunResult, AgentRunError> {
    let native_tools = provider.supports_tools();
    let config = moltis_config::discover_and_load();
    let max_tool_result_bytes = config.tools.max_tool_result_bytes;
    let max_auto_continues = config.tools.agent_max_auto_continues;
    let auto_continue_min_tool_calls = config.tools.agent_auto_continue_min_tool_calls;
    let compaction_ratio = config.tools.tool_result_compaction_ratio as usize;
    let overflow_ratio = config.tools.preemptive_overflow_ratio as usize;
    let compaction_min_iterations = config.tools.compaction_min_iterations;
    let configured_max_iterations = limits
        .max_iterations
        .unwrap_or(config.tools.agent_max_iterations);
    let base_max_iterations = resolve_agent_max_iterations(configured_max_iterations);
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
    let trace_correlation_key = tool_context
        .as_ref()
        .and_then(|ctx| ctx.get("_trace_correlation_key"))
        .and_then(|value| value.as_str());
    let channel_for_hooks =
        channel_binding_from_tool_context(&session_key_for_hooks, tool_context.as_ref());

    // Instrumentation. `None` when no backend is configured or the turn was
    // not sampled, in which case every call below is skipped.
    let turn_recorder = Arc::new(instrumentation::begin_turn(
        &session_key_for_hooks,
        trace_correlation_key,
        channel_for_hooks.as_ref(),
        provider.name(),
        provider.id(),
        user_content,
        instrumentation::recorder_settings(&config.instrumentation),
        config.instrumentation.environment.clone(),
        instrumentation::release(&config.instrumentation),
    ));

    let result: std::result::Result<AgentRunResult, AgentRunError> = async {
    dispatch_before_agent_start_hook(
        hook_registry.as_ref(),
        &session_key_for_hooks,
        provider.id(),
    )
    .await?;

    let mut iterations = 0;
    let mut total_tool_calls = 0;
    let mut usage_accumulator = UsageAccumulator::default();
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
    let tool_controls = AgentToolControls::from_tool_context(tool_context.as_ref());
    let active_tool_names = tool_controls
        .active_tools
        .as_ref()
        .map(|names| names.iter().cloned().collect::<HashSet<_>>());

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
            let schemas = if let Some(active) = active_tool_names.as_ref() {
                tools.list_schemas_allowed_by(|name| active.contains(name))
            } else {
                tools.list_schemas()
            };
            match tool_controls.tool_choice.as_ref() {
                Some(ToolChoice::None) => vec![],
                Some(ToolChoice::Any) if schemas.is_empty() => {
                    return Err(AgentRunError::Other(anyhow::anyhow!(
                        "tool_choice any requires at least one active tool"
                    )));
                },
                Some(ToolChoice::Tool { name }) => {
                    if !schemas.iter().any(|schema| {
                        schema.get("name").and_then(serde_json::Value::as_str) == Some(name)
                    }) {
                        return Err(AgentRunError::Other(anyhow::anyhow!(
                            "forced tool_choice references unavailable tool: {name}"
                        )));
                    }
                    schemas
                },
                _ => schemas,
            }
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
            "calling LLM (streaming)"
        );
        trace!(
            iteration = iterations,
            messages_count = messages.len(),
            "LLM request prepared"
        );

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
                Ok(HookAction::ModifyPayload(modified_payload)) => {
                    apply_before_llm_call_modify_payload(&mut messages, modified_payload);
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
        let mut generation_step = None;
        let mut selected_identity: Option<ProviderIdentity> = None;

        let mut stream = provider.stream_with_tools_and_options_tracked(
            messages.clone(),
            schemas_for_api.clone(),
            tool_controls.clone(),
        );

        // Accumulate answer text, reasoning text, and tool calls from the stream.
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Map streaming index -> accumulated JSON args string.
        let mut tool_call_args: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        // Map streaming index -> position in the `tool_calls` vec.
        // The streaming index may not start at 0 (e.g. Copilot proxying
        // Anthropic uses the content-block index, so a text block at index 0
        // pushes the tool_use to index 1).
        let mut stream_idx_to_vec_pos: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut request_usage = Usage::default();
        let mut stream_error: Option<String> = None;
        let mut finish_reason: Option<String> = None;
        let mut reasoning_tokens: u32 = 0;

        while let Some(event) = stream.next().await {
            match event {
                TrackedStreamEvent::Attempt(ProviderAttemptEvent::Started(identity)) => {
                    if let Some(recorder) = turn_recorder.as_ref().as_ref() {
                        recorder.set_metadata(
                            "provider",
                            serde_json::Value::String(identity.provider.clone()),
                        );
                        recorder.set_metadata(
                            "model",
                            serde_json::Value::String(identity.model.clone()),
                        );
                    }
                    generation_step = instrumentation::begin_generation_step(
                        turn_recorder.as_ref().as_ref(),
                        &identity,
                        provider.as_ref(),
                        &tool_controls,
                        iterations,
                        schemas_for_api.len(),
                        &messages,
                    );
                    selected_identity = Some(identity);
                },
                TrackedStreamEvent::Attempt(ProviderAttemptEvent::Failed { error, .. }) => {
                    if let Some(mut step) = generation_step.take() {
                        step.fail(error);
                        let mut usage = instrumentation::to_token_usage(&request_usage);
                        usage.reasoning = reasoning_tokens;
                        step.set_usage(usage);
                        step.finish();
                    }
                    request_usage = Usage::default();
                    finish_reason = None;
                    reasoning_tokens = 0;
                },
                TrackedStreamEvent::Event(StreamEvent::Delta(text)) => {
                    if let Some(step) = generation_step.as_mut() {
                        step.mark_first_token();
                    }
                    accumulated_text.push_str(&text);
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::TextDelta(text.clone()));
                        cb(RunnerEvent::FinalText(text));
                    }
                },
                TrackedStreamEvent::Event(StreamEvent::ProviderRaw(raw)) => {
                    if let Some(reason) = instrumentation::finish_reason(&raw) {
                        finish_reason = Some(reason);
                    }
                    if let Some(tokens) = instrumentation::reasoning_tokens(&raw) {
                        reasoning_tokens = tokens;
                    }
                    push_capped_provider_raw_event(&mut raw_llm_responses, raw);
                },
                TrackedStreamEvent::Event(StreamEvent::ReasoningDelta(text)) => {
                    if let Some(step) = generation_step.as_mut() {
                        step.mark_first_token();
                    }
                    accumulated_reasoning.push_str(&text);
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::ThinkingText(accumulated_reasoning.clone()));
                    }
                },
                TrackedStreamEvent::Event(StreamEvent::ToolCallStart {
                    id,
                    name,
                    index,
                    metadata,
                }) => {
                    if let Some(step) = generation_step.as_mut() {
                        step.mark_first_token();
                    }
                    let vec_pos = tool_calls.len();
                    debug!(tool = %name, id = %id, stream_index = index, vec_pos, "tool call started in stream");
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: serde_json::json!({}),
                        argument_diagnostic: None,
                        metadata,
                    });
                    stream_idx_to_vec_pos.insert(index, vec_pos);
                    tool_call_args.insert(index, String::new());
                },
                TrackedStreamEvent::Event(StreamEvent::ToolCallArgumentsDelta {
                    index,
                    delta,
                }) => {
                    if let Some(step) = generation_step.as_mut() {
                        step.mark_first_token();
                    }
                    if let Some(args) = tool_call_args.get_mut(&index) {
                        args.push_str(&delta);
                    }
                },
                TrackedStreamEvent::Event(StreamEvent::ToolCallComplete { index }) => {
                    // Arguments are finalized after stream completes.
                    // Just log for now - we'll parse accumulated args later.
                    debug!(index, "tool call arguments complete");
                },
                TrackedStreamEvent::Event(StreamEvent::Done(usage)) => {
                    request_usage = usage.clone();
                    debug!(
                        input_tokens = request_usage.input_tokens,
                        output_tokens = request_usage.output_tokens,
                        cache_read_tokens = request_usage.cache_read_tokens,
                        cache_write_tokens = request_usage.cache_write_tokens,
                        "stream done"
                    );

                    #[cfg(feature = "metrics")]
                    {
                        let provider_name = selected_identity
                            .as_ref()
                            .map_or_else(|| provider.name().to_string(), |id| id.provider.clone());
                        let model_id = selected_identity
                            .as_ref()
                            .map_or_else(|| provider.id().to_string(), |id| id.model.clone());
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
                TrackedStreamEvent::Event(StreamEvent::Error(msg)) => {
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
            // Close the generation as failed before any early return, so a
            // failed call is visible in the backend rather than silently
            // ending as a span with no error on it.
            if let Some(mut step) = generation_step.take() {
                step.fail(err.clone());
                let mut usage = instrumentation::to_token_usage(&request_usage);
                usage.reasoning = reasoning_tokens;
                step.set_usage(usage);
                step.finish();
            }
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

        if let Some(mut step) = generation_step.take() {
            let mut usage = instrumentation::to_token_usage(&request_usage);
            usage.reasoning = reasoning_tokens;
            step.set_usage(usage);
            step.set_output(serde_json::json!({
                "text": accumulated_text,
                "reasoning": accumulated_reasoning,
                "tool_calls": tool_calls.iter().map(|call| serde_json::json!({
                    "id": call.id,
                    "name": call.name,
                })).collect::<Vec<_>>(),
            }));
            if !tool_calls.is_empty() {
                step.set_metadata("tool_calls", serde_json::json!(tool_calls.len()));
            }
            if let Some(reason) = finish_reason {
                step.set_metadata("finish_reason", serde_json::Value::String(reason));
            }
            step.finish();
        }

        usage_accumulator.record_request(request_usage.clone());

        // Finalize tool call arguments from accumulated strings.
        // Use stream_idx_to_vec_pos to map streaming indices (which may not
        // start at 0) to the actual position in the tool_calls vec.
        for (stream_idx, args_str) in &tool_call_args {
            debug!(
                stream_idx,
                argument_bytes = args_str.len(),
                "finalizing tool call args"
            );
            if let Some(&vec_pos) = stream_idx_to_vec_pos.get(stream_idx)
                && vec_pos < tool_calls.len()
            {
                let decoded = decode_tool_call_arguments_from_str(args_str);
                tool_calls[vec_pos].arguments = decoded.arguments;
                tool_calls[vec_pos].argument_diagnostic = decoded.diagnostic;
            }
        }

        info!(
            iteration = iterations,
            has_text = !accumulated_text.is_empty(),
            tool_calls_count = tool_calls.len(),
            input_tokens = request_usage.input_tokens,
            output_tokens = request_usage.output_tokens,
            cache_read_tokens = request_usage.cache_read_tokens,
            cache_write_tokens = request_usage.cache_write_tokens,
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
            info!(
                command_bytes = command.len(),
                "forcing exec tool call from explicit /sh command"
            );
            // Preserve streamed reasoning/planning text on the assistant tool
            // message so providers that validate thinking history accept the
            // next iteration.
            tool_calls = vec![ToolCall {
                id: new_synthetic_tool_call_id("forced"),
                name: "exec".to_string(),
                arguments: serde_json::json!({ "command": command }),
                argument_diagnostic: None,
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

        dispatch_after_llm_call_hook(
            hook_registry.as_ref(),
            &session_key_for_hooks,
            provider.name(),
            provider.id(),
            (!accumulated_text.is_empty()).then(|| accumulated_text.clone()),
            &tool_calls,
            &request_usage,
            iterations,
        )
        .await?;

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
            let used_fallback_text = accumulated_text.is_empty() && !last_answer_text.is_empty();
            let final_text = if used_fallback_text {
                std::mem::take(&mut last_answer_text)
            } else {
                accumulated_text
            };
            if used_fallback_text {
                let streamed_final_text = clean_response(&final_text);
                if let Some(cb) = on_event
                    && !streamed_final_text.is_empty()
                {
                    cb(RunnerEvent::FinalText(streamed_final_text));
                }
            }
            info!(
                iterations,
                tool_calls = total_tool_calls,
                "streaming agent loop complete — returning text"
            );
            return Ok(finish_agent_run(
                final_text,
                iterations,
                total_tool_calls,
                &usage_accumulator,
                raw_llm_responses,
            ));
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
            && let Some(cb) = on_event
        {
            if is_actual_reasoning {
                cb(RunnerEvent::ThinkingText(text.clone()));
            } else {
                cb(RunnerEvent::ProgressText(text.clone()));
            }
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
                let tool_turn_recorder = Arc::clone(&turn_recorder);
                let sanitized = sanitize_tool_name(&tc.name);
                if *sanitized != tc.name {
                    debug!(original = %tc.name, sanitized = %sanitized, "sanitized mangled tool name");
                }
                let (tool, resolved_name) = resolve_tool_lookup(tools, sanitized.as_ref());
                if resolved_name.as_ref() != sanitized.as_ref() {
                    debug!(original = %sanitized, resolved = %resolved_name, "resolved legacy tool alias");
                }
                let mut args = tc.arguments.clone();
                let tool_hidden = matches!(tool_controls.tool_choice, Some(ToolChoice::None))
                    || active_tool_names
                        .as_ref()
                        .is_some_and(|active| !active.contains(resolved_name.as_ref()));

                let hook_registry = hook_registry.clone();
                let session_key = session_key_for_hooks.clone();
                let channel_for_hooks = channel_for_hooks.clone();
                let tc_name = resolved_name.to_string();

                if let Some(ref ctx) = tool_context
                    && let (Some(args_obj), Some(ctx_obj)) = (args.as_object_mut(), ctx.as_object())
                {
                    for (k, v) in ctx_obj {
                        args_obj.insert(k.clone(), v.clone());
                    }
                }
                log_tool_argument_diagnostic(&tc_name, tc.argument_diagnostic.as_ref());

                // Pre-dispatch validation against the tool's schema.
                let validation_error: Option<String> = if tool_hidden {
                    let reason = if matches!(tool_controls.tool_choice, Some(ToolChoice::None)) {
                        format!(
                            "tool `{tc_name}` cannot be called: tool use is disabled for this turn"
                        )
                    } else {
                        format!(
                            "tool `{tc_name}` is not active for this turn; choose one of the currently available tools"
                        )
                    };
                    Some(reason)
                } else if let Some(ref t) = tool {
                    let schema = t.parameters_schema();
                    coerce_scalar_args(&schema, &mut args);
                    match validate_tool_args(&schema, &args) {
                        Ok(()) => None,
                        Err(e) => {
                            warn!(
                                tool = %tc_name,
                                summary = %e.short_summary_with_argument_diagnostic(
                                    tc.argument_diagnostic.as_ref(),
                                ),
                                "tool call rejected by pre-dispatch schema validation"
                            );
                            Some(e.to_llm_error_message_with_argument_diagnostic(
                                &tc_name,
                                tc.argument_diagnostic.as_ref(),
                            ))
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
                            metadata: tc.metadata.clone(),
                        });
                    }
                    info!(tool = %tc_name, id = %tc.id, args = %args, "executing tool");
                }

                async move {
                    let mut tool_step = tool_turn_recorder.as_ref().as_ref().map(|recorder| {
                        recorder.step(
                            instrumentation::tool_observation_kind(&tc_name),
                            tc_name.clone(),
                        )
                    });
                    if let Some(err_msg) = validation_error {
                        if let Some(step) = tool_step.as_mut() {
                            step.set_input(args);
                        }
                        return (
                            false,
                            serde_json::json!({ "error": err_msg.clone() }),
                            Some(err_msg),
                            true,
                            tool_step,
                        );
                    }
                    if let Some(step) = tool_step.as_mut() {
                        step.set_input(args.clone());
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
                                    tool_step,
                                );
                            }
                            Ok(HookAction::ModifyPayload(v)) => {
                                args = v;
                                if let Some(ref tool) = tool {
                                    let schema = tool.parameters_schema();
                                    coerce_scalar_args(&schema, &mut args);
                                    if let Err(e) = validate_tool_args(&schema, &args) {
                                        let err_str = e.to_llm_error_message(&tc_name);
                                        warn!(
                                            tool = %tc_name,
                                            summary = %e.short_summary(),
                                            "tool call rejected after BeforeToolCall hook modified arguments"
                                        );
                                        return (
                                            false,
                                            serde_json::json!({ "error": err_str.clone() }),
                                            Some(err_str),
                                            false,
                                            tool_step,
                                        );
                                    }
                                }
                            }
                            Ok(HookAction::Continue) => {}
                            Err(e) => {
                                warn!(tool = %tc_name, error = %e, "BeforeToolCall hook dispatch failed");
                            }
                        }
                    }

                    if let Some(step) = tool_step.as_mut() {
                        step.set_input(args.clone());
                    }

                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(val) => {
                                let error = tool_result_failure(&val);
                                let success = error.is_none();
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success,
                                        result: Some(val.clone()),
                                        channel: channel_for_hooks.clone(),
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }

                                (
                                    success,
                                    serde_json::json!({ "result": val }),
                                    error,
                                    false,
                                    tool_step,
                                )
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
                                    tool_step,
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
                            tool_step,
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
        for (tc, (success, mut result, error, rejected, mut tool_step)) in
            tool_calls.iter().zip(results)
        {
            if success {
                info!(tool = %tc.name, id = %tc.id, "tool execution succeeded");
                trace!(tool = %tc.name, "tool result available");
            } else if rejected {
                warn!(
                    tool = %tc.name,
                    id = %tc.id,
                    "tool call rejected before execution by pre-dispatch validation"
                );
            } else {
                warn!(tool = %tc.name, id = %tc.id, has_error = error.is_some(), "tool execution failed");
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
                        error: error.clone(),
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
            instrumentation::finish_tool_step(
                tool_step.take(),
                &tool_result_str,
                success,
                error.as_deref(),
                &result,
            );
            debug!(
                tool = %tc.name,
                id = %tc.id,
                result_len = tool_result_str.len(),
                "appending tool result to messages"
            );
            trace!(tool = %tc.name, content_bytes = tool_result_str.len(), "tool result message prepared");

            messages.push(ChatMessage::tool(&tc.id, &tool_result_str));
        }

        // Apply loop-detector intervention if one fired during this batch.
        apply_loop_detector_intervention(
            &mut loop_detector,
            &mut messages,
            &mut strip_tools_next_iter,
            on_event,
        );

        // Drain any pending /steer text and inject as a system note.
        // Uses system role to avoid consecutive-user-message violations
        // with strict providers that enforce role alternation.
        if let Some(ref inbox) = steer_inbox {
            let mut guard = inbox.lock().await;
            if !guard.is_empty() {
                let combined = guard.drain(..).collect::<Vec<_>>().join("\n");
                debug!(steer_bytes = combined.len(), "injecting /steer guidance");
                messages.push(ChatMessage::system(format!(
                    "[Steering note from the user — adjust your approach accordingly]: {combined}"
                )));
            }
        }
    }
    }
    .await;

    instrumentation::finish_turn(turn_recorder.as_ref().as_ref(), &result);
    result
}
