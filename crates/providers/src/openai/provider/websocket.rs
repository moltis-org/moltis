use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
};

use {
    futures::{SinkExt, StreamExt},
    secrecy::ExposeSecret,
    tokio_stream::Stream,
    tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

use tracing::{debug, trace};

use crate::{
    openai_compat::{
        parse_openai_compat_usage, responses_output_index, split_responses_instructions_and_input,
        to_responses_api_tools,
    },
    ws_pool,
};

use moltis_agents::model::{ChatMessage, StreamEvent, Usage};

use super::OpenAiProvider;

impl OpenAiProvider {
    pub(super) fn is_openai_platform_base_url(&self) -> bool {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(ToString::to_string))
            .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
    }

    pub(super) fn responses_websocket_url(&self) -> crate::error::Result<String> {
        let mut base = self.base_url.trim_end_matches('/').to_string();
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }
        let url = format!("{base}/responses");
        if let Some(rest) = url.strip_prefix("https://") {
            return Ok(format!("wss://{rest}"));
        }
        if let Some(rest) = url.strip_prefix("http://") {
            return Ok(format!("ws://{rest}"));
        }
        Err(crate::error::Error::message(format!(
            "invalid OpenAI base_url for websocket mode: expected http:// or https://, got {}",
            self.base_url
        )))
    }

    #[allow(clippy::collapsible_if)]
    pub(super) fn stream_with_tools_websocket(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
        fallback_to_sse: bool,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        // Synchronous pre-flight: URL, request, auth header, pool key.
        // Fail fast and fall back to SSE before entering the async generator,
        // which avoids cloning messages/tools for the four sync-check paths.
        let (request, pool_key) = match (|| -> crate::error::Result<_> {
            if !self.is_openai_platform_base_url() {
                return Err(crate::error::Error::message(format!(
                    "websocket mode is only supported for api.openai.com (got {})",
                    self.base_url
                )));
            }
            let ws_url = self.responses_websocket_url()?;
            let pk = ws_pool::PoolKey::new(&ws_url, &self.api_key);
            let mut req = ws_url.as_str().into_client_request()?;
            let auth = format!("Bearer {}", self.api_key.expose_secret());
            req.headers_mut()
                .insert("Authorization", HeaderValue::from_str(&auth)?);
            req.headers_mut()
                .insert("OpenAI-Beta", HeaderValue::from_static("responses=v1"));
            Ok((req, pk))
        })() {
            Ok(r) => r,
            Err(err) => {
                if fallback_to_sse {
                    debug!(error = %err, "websocket setup failed, falling back to sse");
                    return self.stream_with_tools_sse(messages, tools);
                }
                return Box::pin(async_stream::stream! {
                    yield StreamEvent::Error(err.to_string());
                });
            },
        };

        Box::pin(async_stream::stream! {
            // Try the pool first; fall back to a fresh connection.
            let (mut ws_stream, created_at) = if let Some(pooled) = ws_pool::shared_ws_pool().checkout(&pool_key).await {
                pooled
            } else {
                match tokio_tungstenite::connect_async(request).await {
                    Ok((ws, _)) => (ws, std::time::Instant::now()),
                    Err(err) => {
                        if fallback_to_sse {
                            debug!(error = %err, "websocket connect failed, falling back to sse");
                            let mut sse = self.stream_with_tools_sse(messages, tools);
                            while let Some(event) = sse.next().await {
                                yield event;
                            }
                        } else {
                            yield StreamEvent::Error(err.to_string());
                        }
                        return;
                    }
                }
            };

            let (instructions, input) = split_responses_instructions_and_input(messages);
            let mut response_payload = serde_json::json!({
                "model": self.model,
                "stream": true,
                "store": false,
                "input": input,
            });
            if let Some(instructions) = instructions {
                response_payload["instructions"] = serde_json::Value::String(instructions);
            }
            if !tools.is_empty() {
                response_payload["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                response_payload["tool_choice"] = serde_json::json!("auto");
            }

            self.apply_reasoning_effort_responses(&mut response_payload);

            let create_event = serde_json::json!({
                "type": "response.create",
                "response": response_payload,
            });

            debug!(
                model = %self.model,
                tools_count = tools.len(),
                reasoning_effort = ?self.reasoning_effort,
                "openai stream_with_tools request (websocket)"
            );
            trace!(event = %create_event, "openai websocket create event");

            if let Err(err) = ws_stream
                .send(Message::Text(create_event.to_string().into()))
                .await
            {
                yield StreamEvent::Error(format!("websocket send failed: {err}"));
                return;
            }

            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_read_tokens: u32 = 0;
            let mut cache_write_tokens: u32 = 0;
            let mut current_tool_index: usize = 0;
            let mut tool_calls: HashMap<usize, (String, String)> = HashMap::new();
            let mut completed_tool_calls: HashSet<usize> = HashSet::new();
            let mut clean_completion = false;

            while let Some(frame) = ws_stream.next().await {
                let text = match frame {
                    Ok(Message::Text(t)) => t.to_string(),
                    Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                    Ok(Message::Ping(p)) => {
                        if let Err(err) = ws_stream.send(Message::Pong(p)).await {
                            yield StreamEvent::Error(err.to_string());
                            return;
                        }
                        continue;
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(err) => {
                        yield StreamEvent::Error(err.to_string());
                        return;
                    }
                };

                let Ok(evt) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                trace!(event = %evt, "openai websocket event");

                match evt["type"].as_str().unwrap_or("") {
                    "response.output_text.delta" => {
                        if let Some(delta) = evt["delta"].as_str()
                            && !delta.is_empty()
                        {
                            yield StreamEvent::Delta(delta.to_string());
                        }
                    }
                    "response.output_item.added" => {
                        if evt["item"]["type"].as_str() == Some("function_call") {
                            let id = evt["item"]["call_id"].as_str().unwrap_or("").to_string();
                            let name = evt["item"]["name"].as_str().unwrap_or("").to_string();
                            let index = responses_output_index(&evt, current_tool_index);
                            current_tool_index = current_tool_index.max(index + 1);
                            tool_calls.insert(index, (id.clone(), name.clone()));
                            yield StreamEvent::ToolCallStart { id, name, index, metadata: None };
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        if let Some(delta) = evt["delta"].as_str()
                            && !delta.is_empty()
                        {
                            let index = responses_output_index(&evt, current_tool_index.saturating_sub(1));
                            yield StreamEvent::ToolCallArgumentsDelta {
                                index,
                                delta: delta.to_string(),
                            };
                        }
                    }
                    "response.function_call_arguments.done" => {
                        let index = responses_output_index(&evt, current_tool_index.saturating_sub(1));
                        if completed_tool_calls.insert(index) {
                            yield StreamEvent::ToolCallComplete { index };
                        }
                    }
                    "response.completed" => {
                        if let Some(usage) = evt.get("response").and_then(|response| response.get("usage")) {
                            let parsed = parse_openai_compat_usage(usage);
                            input_tokens = parsed.input_tokens;
                            output_tokens = parsed.output_tokens;
                            cache_read_tokens = parsed.cache_read_tokens;
                            cache_write_tokens = parsed.cache_write_tokens;
                        }
                        let mut pending: Vec<usize> = tool_calls.keys().copied().collect();
                        pending.sort_unstable();
                        for index in pending {
                            if completed_tool_calls.insert(index) {
                                yield StreamEvent::ToolCallComplete { index };
                            }
                        }
                        clean_completion = true;
                        break;
                    }
                    "error" | "response.failed" => {
                        let msg = evt["error"]["message"]
                            .as_str()
                            .or_else(|| evt["response"]["error"]["message"].as_str())
                            .or_else(|| evt["message"].as_str())
                            .unwrap_or("unknown error");
                        yield StreamEvent::Error(msg.to_string());
                        return;
                    }
                    _ => {}
                }
            }

            // Emit any remaining tool-call completions (fallback for broken streams).
            if !clean_completion {
                let mut pending: Vec<usize> = tool_calls.keys().copied().collect();
                pending.sort_unstable();
                for index in pending {
                    if completed_tool_calls.insert(index) {
                        yield StreamEvent::ToolCallComplete { index };
                    }
                }
            }

            // Return healthy connections to the pool; drop on error / close.
            if clean_completion {
                ws_pool::shared_ws_pool()
                    .return_conn(pool_key, ws_stream, created_at)
                    .await;
            }

            yield StreamEvent::Done(Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            });
        })
    }
}
