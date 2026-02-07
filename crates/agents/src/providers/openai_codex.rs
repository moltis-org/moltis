use std::pin::Pin;

use {
    async_trait::async_trait,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    futures::StreamExt,
    moltis_oauth::{OAuthFlow, TokenStore, load_oauth_config},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, info, trace},
};

use crate::{
    model::{
        ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage, UserContent,
    },
    providers::openai_compat::to_responses_api_tools,
};

pub struct OpenAiCodexProvider {
    model: String,
    base_url: String,
    client: reqwest::Client,
    token_store: TokenStore,
}

impl OpenAiCodexProvider {
    pub fn new(model: String) -> Self {
        Self {
            model,
            base_url: "https://chatgpt.com/backend-api".to_string(),
            client: reqwest::Client::new(),
            token_store: TokenStore::new(),
        }
    }

    fn get_valid_token(&self) -> anyhow::Result<String> {
        let tokens = self
            .token_store
            .load("openai-codex")
            .or_else(load_codex_cli_tokens)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "not logged in to openai-codex — run `moltis auth login --provider openai-codex`"
                )
            })?;

        // Check expiry with 5 min buffer
        if let Some(expires_at) = tokens.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now + 300 >= expires_at {
                // Token expired or expiring — try refresh
                if let Some(ref refresh_token) = tokens.refresh_token {
                    debug!("refreshing openai-codex token");
                    let rt = tokio::runtime::Handle::current();
                    let oauth_config = load_oauth_config("openai-codex")
                        .ok_or_else(|| anyhow::anyhow!("missing oauth config for openai-codex"))?;
                    let flow = OAuthFlow::new(oauth_config);
                    let refresh = refresh_token.expose_secret().clone();
                    let new_tokens = std::thread::scope(|_| rt.block_on(flow.refresh(&refresh)))?;
                    self.token_store.save("openai-codex", &new_tokens)?;
                    return Ok(new_tokens.access_token.expose_secret().clone());
                }
                return Err(anyhow::anyhow!(
                    "openai-codex token expired and no refresh token available"
                ));
            }
        }

        Ok(tokens.access_token.expose_secret().clone())
    }

    fn extract_account_id(jwt: &str) -> anyhow::Result<String> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() < 2 {
            anyhow::bail!("invalid JWT format");
        }
        let payload = URL_SAFE_NO_PAD.decode(parts[1]).or_else(|_| {
            // Try with padding
            let padded = match parts[1].len() % 4 {
                2 => format!("{}==", parts[1]),
                3 => format!("{}=", parts[1]),
                _ => parts[1].to_string(),
            };
            base64::engine::general_purpose::STANDARD.decode(&padded)
        })?;
        let claims: serde_json::Value = serde_json::from_slice(&payload)?;
        let account_id = claims["https://api.openai.com/auth"]["chatgpt_account_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing chatgpt_account_id in JWT claims"))?;
        Ok(account_id.to_string())
    }

    fn convert_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .flat_map(|msg| {
                match msg {
                    ChatMessage::System { .. } => {
                        // System messages are extracted as instructions; skip here
                        vec![]
                    },
                    ChatMessage::User { content } => {
                        let text = match content {
                            UserContent::Text(t) => t.clone(),
                            UserContent::Multimodal(parts) => {
                                // Flatten multimodal to text for the Codex API
                                parts
                                    .iter()
                                    .filter_map(|p| match p {
                                        crate::model::ContentPart::Text(t) => Some(t.as_str()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            },
                        };
                        vec![serde_json::json!({
                            "role": "user",
                            "content": [{"type": "input_text", "text": text}]
                        })]
                    },
                    ChatMessage::Assistant {
                        content,
                        tool_calls,
                    } => {
                        if !tool_calls.is_empty() {
                            let mut items: Vec<serde_json::Value> = vec![];
                            for tc in tool_calls {
                                items.push(serde_json::json!({
                                    "type": "function_call",
                                    "call_id": tc.id,
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }));
                            }
                            // Also include text content if present
                            if let Some(text) = content
                                && !text.is_empty()
                            {
                                items.insert(
                                    0,
                                    serde_json::json!({
                                        "type": "message",
                                        "role": "assistant",
                                        "content": [{"type": "output_text", "text": text}]
                                    }),
                                );
                            }
                            items
                        } else {
                            let text = content.as_deref().unwrap_or("");
                            vec![serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": text}]
                            })]
                        }
                    },
                    ChatMessage::Tool {
                        tool_call_id,
                        content,
                    } => {
                        vec![serde_json::json!({
                            "type": "function_call_output",
                            "call_id": tool_call_id,
                            "output": content,
                        })]
                    },
                }
            })
            .collect()
    }
}

/// Parse tokens from Codex CLI auth.json content.
fn parse_codex_cli_tokens(data: &str) -> Option<moltis_oauth::OAuthTokens> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;
    let tokens = json.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str()?.to_string();
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(moltis_oauth::OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        expires_at: None,
    })
}

/// Try to load tokens from the Codex CLI file at `~/.codex/auth.json`.
fn load_codex_cli_tokens() -> Option<moltis_oauth::OAuthTokens> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home)
        .join(".codex")
        .join("auth.json");
    let data = std::fs::read_to_string(path).ok()?;
    parse_codex_cli_tokens(&data)
}

pub fn has_stored_tokens() -> bool {
    TokenStore::new().load("openai-codex").is_some() || load_codex_cli_tokens().is_some()
}

#[async_trait]
impl LlmProvider for OpenAiCodexProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let token = self.get_valid_token()?;
        let account_id = Self::extract_account_id(&token)?;

        // Extract system message as instructions; pass the rest as input
        let instructions = messages
            .iter()
            .find_map(|m| match m {
                ChatMessage::System { content } => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or("You are a helpful assistant.");
        let non_system: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| !matches!(m, ChatMessage::System { .. }))
            .cloned()
            .collect();
        let input = Self::convert_messages(&non_system);

        // The Codex API requires stream=true, so we stream and collect.
        let mut body = serde_json::json!({
            "model": self.model,
            "store": false,
            "stream": true,
            "input": input,
            "instructions": instructions,
            "text": {"verbosity": "medium"},
            "include": ["reasoning.encrypted_content"],
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_responses_api_tools(tools));
            body["tool_choice"] = serde_json::json!("auto");
        }

        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai-codex request body");

        let http_resp = self
            .client
            .post(format!("{}/codex/responses", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("chatgpt-account-id", &account_id)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "pi")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            tracing::warn!(status = %status, body = %body_text, "openai-codex API error");
            anyhow::bail!("openai-codex API error HTTP {status}: {body_text}");
        }

        // Collect the SSE stream into a final response
        let mut text_buf = String::new();
        let mut tool_calls: Vec<ToolCall> = vec![];
        // Track in-progress function calls by index
        let mut fn_call_ids: Vec<String> = vec![];
        let mut fn_call_names: Vec<String> = vec![];
        let mut fn_call_args: Vec<String> = vec![];
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        let mut byte_stream = http_resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf = buf[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }
                let Some(data) = line.strip_prefix("data: ") else {
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
                            fn_call_ids
                                .push(evt["item"]["call_id"].as_str().unwrap_or("").to_string());
                            fn_call_names
                                .push(evt["item"]["name"].as_str().unwrap_or("").to_string());
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
                    "response.function_call_arguments.done" => {
                        // function call complete — will be collected at the end
                    },
                    "response.completed" => {
                        if let Some(u) = evt["response"]["usage"].as_object() {
                            input_tokens =
                                u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            output_tokens =
                                u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        }
                    },
                    "error" | "response.failed" => {
                        let msg = evt["error"]["message"]
                            .as_str()
                            .or_else(|| evt["message"].as_str())
                            .unwrap_or("unknown error");
                        anyhow::bail!("openai-codex stream error: {msg}");
                    },
                    _ => {},
                }
            }
        }

        // Build tool calls from collected parts
        for i in 0..fn_call_ids.len() {
            let args_str = &fn_call_args[i];
            let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
            tool_calls.push(ToolCall {
                id: fn_call_ids[i].clone(),
                name: fn_call_names[i].clone(),
                arguments,
            });
        }

        let text = if text_buf.is_empty() {
            None
        } else {
            Some(text_buf)
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage: Usage {
                input_tokens,
                output_tokens,
            },
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        info!(
            tools_received = tools.len(),
            "stream_with_tools entry (before async_stream)"
        );
        Box::pin(async_stream::stream! {
            let token = match self.get_valid_token() {
                Ok(t) => t,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let account_id = match Self::extract_account_id(&token) {
                Ok(id) => id,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let instructions = messages
                .iter()
                .find_map(|m| match m {
                    ChatMessage::System { content } => Some(content.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "You are a helpful assistant.".to_string());
            let non_system: Vec<ChatMessage> = messages
                .iter()
                .filter(|m| !matches!(m, ChatMessage::System { .. }))
                .cloned()
                .collect();
            let input = Self::convert_messages(&non_system);

            let mut body = serde_json::json!({
                "model": self.model,
                "store": false,
                "stream": true,
                "input": input,
                "instructions": instructions,
                "text": {"verbosity": "medium"},
                "include": ["reasoning.encrypted_content"],
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                body["tool_choice"] = serde_json::json!("auto");
            }

            info!(
                model = %self.model,
                messages_count = messages.len(),
                tools_count = tools.len(),
                "openai-codex stream_with_tools request"
            );
            debug!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai-codex stream request body");

            let resp = match self
                .client
                .post(format!("{}/codex/responses", self.base_url))
                .header("Authorization", format!("Bearer {token}"))
                .header("chatgpt-account-id", &account_id)
                .header("OpenAI-Beta", "responses=experimental")
                .header("originator", "pi")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(format!("HTTP {status}: {body_text}"));
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
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;

            // Track tool calls being streamed (index -> (id, name))
            let mut tool_calls: std::collections::HashMap<usize, (String, String)> =
                std::collections::HashMap::new();
            let mut current_tool_index: usize = 0;

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

                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };

                    if data == "[DONE]" {
                        // Emit completion for any pending tool calls
                        for index in tool_calls.keys() {
                            yield StreamEvent::ToolCallComplete { index: *index };
                        }
                        yield StreamEvent::Done(Usage { input_tokens, output_tokens });
                        return;
                    }

                    if let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) {
                        let evt_type = evt["type"].as_str().unwrap_or("");
                        trace!(evt_type = %evt_type, evt = %evt, "openai-codex stream event");

                        match evt_type {
                            "response.output_text.delta" => {
                                if let Some(delta) = evt["delta"].as_str() {
                                    if !delta.is_empty() {
                                        yield StreamEvent::Delta(delta.to_string());
                                    }
                                }
                            }
                            "response.output_item.added" => {
                                // New output item - could be text or function_call
                                if evt["item"]["type"].as_str() == Some("function_call") {
                                    let id = evt["item"]["call_id"].as_str().unwrap_or("").to_string();
                                    let name = evt["item"]["name"].as_str().unwrap_or("").to_string();
                                    let index = current_tool_index;
                                    current_tool_index += 1;
                                    tool_calls.insert(index, (id.clone(), name.clone()));
                                    yield StreamEvent::ToolCallStart { id, name, index };
                                }
                            }
                            "response.function_call_arguments.delta" => {
                                if let Some(delta) = evt["delta"].as_str() {
                                    if !delta.is_empty() {
                                        // Find the index for this tool call (use the most recent one)
                                        let index = if current_tool_index > 0 {
                                            current_tool_index - 1
                                        } else {
                                            0
                                        };
                                        yield StreamEvent::ToolCallArgumentsDelta {
                                            index,
                                            delta: delta.to_string(),
                                        };
                                    }
                                }
                            }
                            "response.function_call_arguments.done" => {
                                // Function call arguments complete - tool call will be finalized at [DONE]
                            }
                            "response.completed" => {
                                if let Some(u) = evt["response"]["usage"].as_object() {
                                    input_tokens = u.get("input_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                    output_tokens = u.get("output_tokens")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as u32;
                                }
                                // Emit completion for any pending tool calls
                                for index in tool_calls.keys() {
                                    yield StreamEvent::ToolCallComplete { index: *index };
                                }
                                yield StreamEvent::Done(Usage { input_tokens, output_tokens });
                                return;
                            }
                            "error" | "response.failed" => {
                                let msg = evt["error"]["message"]
                                    .as_str()
                                    .or_else(|| evt["message"].as_str())
                                    .unwrap_or("unknown error");
                                yield StreamEvent::Error(msg.to_string());
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_cli_tokens_full() {
        let json = r#"{
            "last_refresh": "2026-01-27T04:54:45Z",
            "OPENAI_API_KEY": null,
            "tokens": {
                "access_token": "test_access_token",
                "account_id": "some-account-id",
                "id_token": "some-id-token",
                "refresh_token": "test_refresh_token"
            }
        }"#;
        let tokens = parse_codex_cli_tokens(json).unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "test_access_token");
        assert_eq!(
            tokens
                .refresh_token
                .as_ref()
                .map(|s| s.expose_secret().as_str()),
            Some("test_refresh_token")
        );
        assert_eq!(tokens.expires_at, None);
    }

    #[test]
    fn parse_codex_cli_tokens_no_refresh() {
        let json = r#"{
            "tokens": {
                "access_token": "tok123"
            }
        }"#;
        let tokens = parse_codex_cli_tokens(json).unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "tok123");
        assert!(tokens.refresh_token.is_none());
    }

    #[test]
    fn parse_codex_cli_tokens_missing_tokens_field() {
        let json = r#"{"OPENAI_API_KEY": "sk-test"}"#;
        assert!(parse_codex_cli_tokens(json).is_none());
    }

    #[test]
    fn parse_codex_cli_tokens_invalid_json() {
        assert!(parse_codex_cli_tokens("not json").is_none());
    }

    #[test]
    fn parse_codex_cli_tokens_null_access_token() {
        let json = r#"{"tokens": {"access_token": null}}"#;
        assert!(parse_codex_cli_tokens(json).is_none());
    }

    #[test]
    fn convert_messages_user_and_assistant() {
        let messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi there"),
        ];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["content"][0]["type"], "input_text");
        assert_eq!(converted[1]["type"], "message");
        assert_eq!(converted[1]["content"][0]["text"], "hi there");
    }

    #[test]
    fn convert_messages_tool_call_and_result() {
        let messages = vec![
            ChatMessage::assistant_with_tools(None, vec![ToolCall {
                id: "call_1".to_string(),
                name: "get_time".to_string(),
                arguments: serde_json::json!({}),
            }]),
            ChatMessage::tool("call_1", "12:00"),
        ];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["type"], "function_call");
        assert_eq!(converted[0]["call_id"], "call_1");
        assert_eq!(converted[0]["name"], "get_time");
        assert_eq!(converted[1]["type"], "function_call_output");
        assert_eq!(converted[1]["call_id"], "call_1");
        assert_eq!(converted[1]["output"], "12:00");
    }

    // ── Array Content Handling Tests ───────────────────────────────────
    // These tests verify that the Codex provider correctly handles array
    // content (multimodal) in tool results, which can occur even when we
    // send string content due to model behavior or content format.

    #[test]
    fn convert_messages_tool_result_with_string_content() {
        // Standard case: tool result content is a string
        let messages = vec![ChatMessage::tool(
            "call_123",
            "Command executed successfully",
        )];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["call_id"], "call_123");
        assert_eq!(converted[0]["output"], "Command executed successfully");
    }

    #[test]
    fn convert_messages_tool_result_with_serialized_array_content() {
        // ChatMessage::Tool always has String content. If the caller serialized
        // array content into a JSON string, it passes through unchanged.
        let array_content = serde_json::json!([
            {"type": "text", "text": "Screenshot captured"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,ABC123"}}
        ])
        .to_string();
        let messages = vec![ChatMessage::tool("call_456", &array_content)];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["call_id"], "call_456");
        let output = converted[0]["output"].as_str().unwrap();
        assert!(
            output.contains("Screenshot captured"),
            "output should contain text: {output}"
        );
        assert!(
            output.contains("image_url"),
            "output should contain image type: {output}"
        );
    }

    #[test]
    fn convert_messages_tool_result_with_empty_content() {
        // ChatMessage::Tool content is a String, so "null" equivalent is empty string
        let messages = vec![ChatMessage::tool("call_789", "")];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["call_id"], "call_789");
        assert_eq!(converted[0]["output"], "");
    }

    #[test]
    fn convert_messages_tool_result_with_json_object_content() {
        // ChatMessage::Tool content is a String; caller serializes structured data
        let object_content =
            serde_json::json!({"result": "success", "data": [1, 2, 3]}).to_string();
        let messages = vec![ChatMessage::tool("call_abc", &object_content)];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["call_id"], "call_abc");
        let output = converted[0]["output"].as_str().unwrap();
        assert!(output.contains("success"), "output should contain result");
        assert!(
            output.contains("[1,2,3]"),
            "output should contain data array"
        );
    }

    #[test]
    fn convert_messages_preserves_tool_call_id() {
        // Verify that tool_call_id is correctly preserved for various content types
        let test_cases = vec![
            ("call_str", "simple string"),
            ("call_empty", ""),
            ("call_unicode", "日本語テスト"),
        ];

        for (call_id, content) in test_cases {
            let messages = vec![ChatMessage::tool(call_id, content)];
            let converted = OpenAiCodexProvider::convert_messages(&messages);
            assert_eq!(
                converted[0]["call_id"], call_id,
                "call_id should be preserved for content: {content}"
            );
        }
    }

    #[test]
    fn convert_messages_empty_array_content() {
        // ChatMessage::Tool content is a String; caller serializes empty array as "[]"
        let messages = vec![ChatMessage::tool("call_empty_arr", "[]")];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["output"], "[]");
    }

    #[test]
    fn convert_messages_mixed_conversation_with_tool_content() {
        // Full conversation with various message types
        let tool_output = serde_json::json!([
            {"type": "text", "text": "Screenshot taken"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,XYZ"}}
        ])
        .to_string();
        let messages = vec![
            ChatMessage::user("Take a screenshot"),
            ChatMessage::assistant_with_tools(None, vec![ToolCall {
                id: "call_screenshot".to_string(),
                name: "browser_screenshot".to_string(),
                arguments: serde_json::json!({}),
            }]),
            ChatMessage::tool("call_screenshot", &tool_output),
            ChatMessage::assistant("Here is the screenshot."),
        ];

        let converted = OpenAiCodexProvider::convert_messages(&messages);

        // Verify all messages are converted
        assert_eq!(converted.len(), 4);

        // User message
        assert_eq!(converted[0]["content"][0]["type"], "input_text");
        assert_eq!(converted[0]["content"][0]["text"], "Take a screenshot");

        // Tool call
        assert_eq!(converted[1]["type"], "function_call");
        assert_eq!(converted[1]["name"], "browser_screenshot");

        // Tool result with serialized array content
        assert_eq!(converted[2]["type"], "function_call_output");
        let output = converted[2]["output"].as_str().unwrap();
        assert!(output.contains("Screenshot taken"));
        assert!(output.contains("image_url"));

        // Assistant response
        assert_eq!(converted[3]["type"], "message");
        assert_eq!(
            converted[3]["content"][0]["text"],
            "Here is the screenshot."
        );
    }
}
