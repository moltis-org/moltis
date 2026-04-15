use std::pin::Pin;

use {futures::StreamExt, secrecy::ExposeSecret, tokio_stream::Stream};

use tracing::{debug, trace};

use crate::{
    http::{retry_after_ms_from_headers, with_retry_after_marker},
    openai_compat::{
        ResponsesStreamState, SseLineResult, StreamingToolState, finalize_responses_stream,
        finalize_stream, process_openai_sse_line, process_responses_sse_line,
        split_responses_instructions_and_input, to_openai_tools, to_responses_api_tools,
    },
};

use moltis_agents::model::{ChatMessage, StreamEvent};

use super::OpenAiProvider;

impl OpenAiProvider {
    /// Stream using the OpenAI Responses API format (`/responses`) over SSE.
    #[allow(clippy::collapsible_if)]
    pub(super) fn stream_responses_sse(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let (instructions, input) = split_responses_instructions_and_input(messages);
            let mut body = serde_json::json!({
                "model": self.model,
                "input": input,
                "stream": true,
            });

            if let Some(instructions) = instructions {
                body["instructions"] = serde_json::Value::String(instructions);
            }

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                body["tool_choice"] = serde_json::json!("auto");
            }

            self.apply_reasoning_effort_responses(&mut body);

            debug!(
                model = %self.model,
                tools_count = tools.len(),
                "openai stream_responses_sse request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses stream request body");

            let url = self.responses_sse_url();
            let resp = match self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(with_retry_after_marker(
                            format!("HTTP {status}: {body_text}"),
                            retry_after_ms,
                        ));
                        return;
                    }
                    r
                }
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut state = ResponsesStreamState::default();
            let mut stream_done = false;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield StreamEvent::Error(e.to_string());
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                    else {
                        // Handle bare event types (e.g. "event: response.completed")
                        continue;
                    };

                    match process_responses_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            stream_done = true;
                            break;
                        }
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        SseLineResult::Skip => {}
                    }
                }
                if stream_done {
                    break;
                }
            }

            // Process any residual buffered line on EOF.
            if !stream_done {
                let line = buf.trim().to_string();
                if !line.is_empty()
                    && let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                {
                    match process_responses_sse_line(data, &mut state) {
                        SseLineResult::Done | SseLineResult::Skip => {}
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                    }
                }
            }

            // Finalize: emit pending ToolCallComplete events + Done with usage.
            for event in finalize_responses_stream(&mut state) {
                yield event;
            }
        })
    }

    #[allow(clippy::collapsible_if)]
    pub(super) fn stream_with_tools_sse(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let mut openai_messages = self.serialize_messages_for_request(&messages);
            self.apply_openrouter_cache_control(&mut openai_messages);
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });
            self.apply_system_prompt_rewrite(&mut body);

            if !tools.is_empty() {
                body["tools"] =
                    serde_json::Value::Array(to_openai_tools(&tools, self.needs_strict_tools()));
            }

            self.apply_reasoning_effort_chat(&mut body);

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                reasoning_effort = ?self.reasoning_effort,
                "openai stream_with_tools request (sse)"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai stream request body (sse)");

            let resp = match self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(with_retry_after_marker(
                            format!("HTTP {status}: {body_text}"),
                            retry_after_ms,
                        ));
                        return;
                    }
                    r
                }
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut state = StreamingToolState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield StreamEvent::Error(e.to_string());
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                    else {
                        continue;
                    };

                    match process_openai_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_stream(&mut state) {
                                yield event;
                            }
                            return;
                        }
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        SseLineResult::Skip => {}
                    }
                }
            }

            // Some OpenAI-compatible providers may close the stream without
            // an explicit [DONE] frame or trailing newline. Process any
            // residual buffered line and always finalize on EOF so usage
            // metadata still propagates.
            let line = buf.trim().to_string();
            if !line.is_empty()
                && let Some(data) = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
            {
                match process_openai_sse_line(data, &mut state) {
                    SseLineResult::Done => {
                        for event in finalize_stream(&mut state) {
                            yield event;
                        }
                        return;
                    }
                    SseLineResult::Events(events) => {
                        for event in events {
                            yield event;
                        }
                    }
                    SseLineResult::Skip => {}
                }
            }

            for event in finalize_stream(&mut state) {
                yield event;
            }
        })
    }
}
