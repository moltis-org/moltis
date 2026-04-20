use secrecy::ExposeSecret;

use tracing::{debug, trace, warn};

use crate::{
    http::{retry_after_ms_from_headers, with_retry_after_marker},
    ollama::normalize_ollama_api_base_url,
    openai_compat::{
        parse_openai_compat_usage_from_payload, parse_tool_calls,
        split_responses_instructions_and_input, strip_think_tags, to_openai_tools,
        to_responses_api_tools,
    },
    raw_model_id,
};

use moltis_agents::model::{ChatMessage, CompletionResponse, Usage};

use super::OpenAiProvider;

fn is_chat_endpoint_unsupported_model_error(body_text: &str) -> bool {
    let lower = body_text.to_ascii_lowercase();
    lower.contains("not a chat model")
        || lower.contains("does not support chat")
        || lower.contains("only supported in v1/responses")
        || lower.contains("not supported in the v1/chat/completions endpoint")
        || lower.contains("input content or output modality contain audio")
        || lower.contains("requires audio")
}

fn should_warn_on_api_error(status: reqwest::StatusCode, body_text: &str) -> bool {
    if is_chat_endpoint_unsupported_model_error(body_text) {
        return false;
    }
    !matches!(status.as_u16(), 404)
}

impl OpenAiProvider {
    fn apply_probe_output_cap_chat(&self, body: &mut serde_json::Value) {
        let raw = raw_model_id(&self.model).to_ascii_lowercase();
        let capability = raw.rsplit('/').next().unwrap_or(raw.as_str());
        let uses_max_completion_tokens = capability.starts_with("gpt-5")
            || capability.starts_with("o1")
            || capability.starts_with("o3")
            || capability.starts_with("o4");
        if uses_max_completion_tokens {
            // GPT-5 and reasoning models need a higher minimum output cap.
            // Values below ~10 can trigger 400 errors on some models.
            body["max_completion_tokens"] = serde_json::json!(16);
        } else {
            body["max_tokens"] = serde_json::json!(1);
        }
    }

    pub(super) async fn probe_chat_completions(&self) -> anyhow::Result<()> {
        let messages = vec![ChatMessage::user("ping")];
        let mut openai_messages = self.serialize_messages_for_request(&messages);
        self.apply_openrouter_cache_control(&mut openai_messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        self.apply_system_prompt_rewrite(&mut body);
        // Probes only answer "can this model respond at all?".
        // Keep them cheap instead of mirroring full reasoning budgets.
        self.apply_probe_output_cap_chat(&mut body);

        debug!(model = %self.model, "openai probe request");
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai probe request body");

        let http_resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            if should_warn_on_api_error(status, &body_text) {
                warn!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    body = %body_text,
                    "openai probe API error"
                );
            } else {
                debug!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    "openai probe model unsupported for chat/completions endpoint"
                );
            }
            // Ollama's OpenAI-compat layer returns 404 for models that
            // exist but aren't wired to /v1/chat/completions.  Fall back
            // to the native `/api/show` endpoint before giving up.
            if status == reqwest::StatusCode::NOT_FOUND
                && self.provider_name.eq_ignore_ascii_case("ollama")
            {
                return self.probe_ollama_native().await;
            }

            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        Ok(())
    }

    /// Fallback probe for Ollama: POST `/api/show` with the model name.
    ///
    /// This confirms the model is installed and Ollama is reachable even when
    /// the OpenAI-compat `/v1/chat/completions` endpoint returns 404.
    async fn probe_ollama_native(&self) -> anyhow::Result<()> {
        let api_base = normalize_ollama_api_base_url(&self.base_url);
        let url = format!("{}/api/show", api_base.trim_end_matches('/'));

        debug!(model = %self.model, url = %url, "ollama native probe via /api/show");

        let mut req = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "name": self.model }));
        let key = self.api_key.expose_secret();
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let resp = req.send().await?;

        if resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Model '{}' not found. Make sure it is installed (ollama pull {}) \
             and try again. (Ollama /api/show returned HTTP {}: {})",
            self.model,
            self.model,
            status,
            body_text,
        )
    }

    pub(super) async fn probe_responses(&self) -> anyhow::Result<()> {
        let messages = vec![ChatMessage::user("ping")];
        let (instructions, input) = split_responses_instructions_and_input(messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
            "max_output_tokens": 1,
        });

        if let Some(instructions) = instructions {
            body["instructions"] = serde_json::Value::String(instructions);
        }

        self.apply_reasoning_effort_responses(&mut body);

        debug!(model = %self.model, "openai responses probe request");
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses probe request body");

        let url = self.responses_sse_url();
        let http_resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(
                status = %status,
                model = %self.model,
                provider = %self.provider_name,
                body = %body_text,
                "openai responses probe API error"
            );
            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        Ok(())
    }

    /// Non-streaming completion using the Chat Completions API.
    pub(super) async fn complete_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let mut openai_messages = self.serialize_messages_for_request(messages);
        self.apply_openrouter_cache_control(&mut openai_messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        self.apply_system_prompt_rewrite(&mut body);

        if !tools.is_empty() {
            body["tools"] =
                serde_json::Value::Array(to_openai_tools(tools, self.needs_strict_tools()));
        }

        self.apply_reasoning_effort_chat(&mut body);

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            reasoning_effort = ?self.reasoning_effort,
            "openai complete request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai request body");

        let http_resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            if should_warn_on_api_error(status, &body_text) {
                warn!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    body = %body_text,
                    "openai API error"
                );
            } else {
                debug!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    "openai model unsupported for chat/completions endpoint"
                );
            }
            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "openai raw response");

        let message = &resp["choices"][0]["message"];

        let text = message["content"].as_str().and_then(|s| {
            let (visible, _thinking) = strip_think_tags(s);
            if visible.is_empty() {
                None
            } else {
                Some(visible)
            }
        });
        let tool_calls = parse_tool_calls(message);

        let usage = parse_openai_compat_usage_from_payload(&resp).unwrap_or_default();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    /// Non-streaming completion using the Responses API.
    ///
    /// Sends `stream: true` and collects events into a single response, since
    /// many Responses API endpoints only support streaming.
    pub(super) async fn complete_responses(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let (instructions, input) = split_responses_instructions_and_input(messages.to_vec());
        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
            "stream": true,
        });
        if let Some(instructions) = instructions {
            body["instructions"] = serde_json::Value::String(instructions);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_responses_api_tools(tools));
            body["tool_choice"] = serde_json::json!("auto");
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "openai complete_responses request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses request body");

        let url = self.responses_sse_url();
        let http_resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("Responses API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        // Collect SSE events into text + tool calls.
        let mut text_buf = String::new();
        let mut fn_call_ids: Vec<String> = Vec::new();
        let mut fn_call_names: Vec<String> = Vec::new();
        let mut fn_call_args: Vec<String> = Vec::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut cache_read_tokens: u32 = 0;
        let cache_write_tokens: u32 = 0;

        let full_body = http_resp.text().await.unwrap_or_default();
        for line in full_body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(data) = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"))
            else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }

            let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };

            match evt["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(delta) = evt["delta"].as_str() {
                        text_buf.push_str(delta);
                    }
                },
                "response.output_item.added" => {
                    if evt["item"]["type"].as_str() == Some("function_call") {
                        fn_call_ids.push(evt["item"]["call_id"].as_str().unwrap_or("").to_string());
                        fn_call_names.push(evt["item"]["name"].as_str().unwrap_or("").to_string());
                        fn_call_args.push(String::new());
                    }
                },
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = evt["delta"].as_str()
                        && let Some(last) = fn_call_args.last_mut()
                    {
                        last.push_str(delta);
                    }
                },
                "response.completed" => {
                    if let Some(u) = evt["response"]["usage"].as_object() {
                        input_tokens =
                            u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        output_tokens =
                            u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        cache_read_tokens = u
                            .get("input_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                    }
                },
                "error" | "response.failed" => {
                    let msg = evt["error"]["message"]
                        .as_str()
                        .or_else(|| evt["response"]["error"]["message"].as_str())
                        .or_else(|| evt["message"].as_str())
                        .unwrap_or("unknown error");
                    anyhow::bail!("Responses API error: {msg}");
                },
                _ => {},
            }
        }

        let text = if text_buf.is_empty() {
            None
        } else {
            Some(text_buf)
        };

        let tool_calls: Vec<moltis_agents::model::ToolCall> = fn_call_ids
            .into_iter()
            .zip(fn_call_names)
            .zip(fn_call_args)
            .filter_map(|((id, name), args)| {
                let arguments: serde_json::Value = serde_json::from_str(&args)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                if name.is_empty() {
                    return None;
                }
                Some(moltis_agents::model::ToolCall {
                    id,
                    name,
                    arguments,
                    metadata: None,
                })
            })
            .collect();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage: Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            },
        })
    }
}
