use std::{collections::HashSet, pin::Pin, sync::mpsc, time::Duration};

use {
    async_trait::async_trait,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    futures::StreamExt,
    moltis_oauth::{OAuthFlow, TokenStore, load_oauth_config},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, info, trace, warn},
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

const CODEX_MODELS_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/models";
const CODEX_MODELS_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-5.3-codex", "GPT-5.3 Codex"),
    ("gpt-5.2-codex", "GPT-5.2 Codex"),
    ("gpt-5.2", "GPT-5.2"),
    ("gpt-5.1-codex-max", "GPT-5.1 Codex Max"),
    ("gpt-5.1-codex-mini", "GPT-5.1 Codex Mini"),
];

impl OpenAiCodexProvider {
    pub fn new(model: String) -> Self {
        Self {
            model,
            base_url: "https://chatgpt.com/backend-api".to_string(),
            client: reqwest::Client::new(),
            token_store: TokenStore::new(),
        }
    }

    async fn get_valid_token(&self) -> anyhow::Result<String> {
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
                .unwrap_or_default()
                .as_secs();
            if now + 300 >= expires_at {
                // Token expired or expiring — try refresh
                if let Some(ref refresh_token) = tokens.refresh_token {
                    debug!("refreshing openai-codex token");
                    let oauth_config = load_oauth_config("openai-codex")
                        .ok_or_else(|| anyhow::anyhow!("missing oauth config for openai-codex"))?;
                    let flow = OAuthFlow::new(oauth_config);
                    let refresh = refresh_token.expose_secret().clone();
                    let new_tokens = flow.refresh(&refresh).await?;
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
                        let content_blocks = match content {
                            UserContent::Text(t) => {
                                vec![serde_json::json!({"type": "input_text", "text": t})]
                            },
                            UserContent::Multimodal(parts) => {
                                let text_count = parts
                                    .iter()
                                    .filter(|p| matches!(p, crate::model::ContentPart::Text(_)))
                                    .count();
                                let image_count = parts
                                    .iter()
                                    .filter(|p| {
                                        matches!(p, crate::model::ContentPart::Image { .. })
                                    })
                                    .count();
                                debug!(
                                    text_count,
                                    image_count, "codex convert_messages: multimodal user content"
                                );
                                parts
                                    .iter()
                                    .map(|p| match p {
                                        crate::model::ContentPart::Text(t) => {
                                            serde_json::json!({"type": "input_text", "text": t})
                                        },
                                        crate::model::ContentPart::Image { media_type, data } => {
                                            let data_uri =
                                                format!("data:{media_type};base64,{data}");
                                            debug!(
                                                media_type,
                                                data_len = data.len(),
                                                "codex convert_messages: including input_image"
                                            );
                                            serde_json::json!({
                                                "type": "input_image",
                                                "image_url": data_uri,
                                            })
                                        },
                                    })
                                    .collect()
                            },
                        };
                        vec![serde_json::json!({
                            "role": "user",
                            "content": content_blocks,
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

    async fn post_responses_request(
        &self,
        token: &str,
        account_id: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .post(format!("{}/codex/responses", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("chatgpt-account-id", account_id)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "pi")
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
    }

    async fn post_responses_request_with_fallback(
        &self,
        token: &str,
        account_id: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<reqwest::Response> {
        let response = self
            .post_responses_request(token, account_id, &body)
            .await?;
        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        anyhow::bail!("openai-codex API error HTTP {status}: {body_text}");
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

fn default_model_catalog() -> Vec<super::DiscoveredModel> {
    DEFAULT_CODEX_MODELS
        .iter()
        .map(|(id, name)| super::DiscoveredModel::new(*id, *name))
        .collect()
}

fn formatted_model_name(model_id: &str) -> String {
    let mut out = Vec::new();
    for part in model_id.split('-') {
        let item = match part {
            "gpt" => "GPT".to_string(),
            "codex" => "Codex".to_string(),
            "mini" => "Mini".to_string(),
            "max" => "Max".to_string(),
            other => {
                if other.is_empty() {
                    continue;
                }
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => {
                        let mut chunk = String::new();
                        chunk.push(first.to_ascii_uppercase());
                        chunk.push_str(chars.as_str());
                        chunk
                    },
                    None => continue,
                }
            },
        };
        out.push(item);
    }
    if out.is_empty() {
        model_id.to_string()
    } else {
        out.join(" ")
    }
}

fn normalize_display_name(model_id: &str, display_name: Option<&str>) -> String {
    let normalized = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(model_id);
    if normalized == model_id {
        return formatted_model_name(model_id);
    }
    normalized.to_string()
}

fn is_likely_model_id(model_id: &str) -> bool {
    if model_id.is_empty() || model_id.len() > 120 {
        return false;
    }
    if model_id.chars().any(char::is_whitespace) {
        return false;
    }
    model_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<super::DiscoveredModel> {
    let obj = entry.as_object()?;
    let model_id = obj
        .get("id")
        .or_else(|| obj.get("slug"))
        .or_else(|| obj.get("model"))
        .and_then(serde_json::Value::as_str)?;

    if !is_likely_model_id(model_id) {
        return None;
    }

    let display_name = obj
        .get("display_name")
        .or_else(|| obj.get("displayName"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("title"))
        .and_then(serde_json::Value::as_str);

    let created_at = obj.get("created").and_then(serde_json::Value::as_i64);

    Some(
        super::DiscoveredModel::new(model_id, normalize_display_name(model_id, display_name))
            .with_created_at(created_at),
    )
}

fn collect_candidate_arrays<'a>(
    value: &'a serde_json::Value,
    out: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Array(items) => out.extend(items),
        serde_json::Value::Object(map) => {
            for key in ["models", "data", "items", "results", "available"] {
                if let Some(nested) = map.get(key) {
                    collect_candidate_arrays(nested, out);
                }
            }
        },
        _ => {},
    }
}

fn parse_models_payload(value: &serde_json::Value) -> Vec<super::DiscoveredModel> {
    let mut candidates = Vec::new();
    collect_candidate_arrays(value, &mut candidates);

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for entry in candidates {
        if let Some(model) = parse_model_entry(entry)
            && seen.insert(model.id.clone())
        {
            models.push(model);
        }
    }

    // Sort by created_at descending (newest first). Models without a
    // timestamp are placed after those with one, preserving relative order.
    models.sort_by(|a, b| match (a.created_at, b.created_at) {
        (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts), // newest first
        (Some(_), None) => std::cmp::Ordering::Less, // timestamp before no-timestamp
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    models
}

async fn fetch_models_from_api(
    access_token: String,
    account_id: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let url = format!("{CODEX_MODELS_ENDPOINT}?client_version={CODEX_MODELS_CLIENT_VERSION}");
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("originator", "pi")
        .header("accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("codex models API error HTTP {status}");
    }
    let payload: serde_json::Value = serde_json::from_str(&body)?;
    let models = parse_models_payload(&payload);
    if models.is_empty() {
        anyhow::bail!("codex models API returned no models");
    }
    Ok(models)
}

fn fetch_models_blocking(
    access_token: String,
    account_id: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(fetch_models_from_api(access_token, account_id)));
        let _ = tx.send(result);
    });
    rx.recv()
        .map_err(|err| anyhow::anyhow!("codex model discovery worker failed: {err}"))?
}

fn load_access_token_and_account_id() -> anyhow::Result<(String, String)> {
    let tokens = TokenStore::new()
        .load("openai-codex")
        .or_else(load_codex_cli_tokens)
        .ok_or_else(|| {
            debug!("openai-codex tokens not found in token store or codex CLI auth");
            anyhow::anyhow!("openai-codex tokens not found")
        })?;

    let access_token = tokens.access_token.expose_secret().clone();
    let account_id = OpenAiCodexProvider::extract_account_id(&access_token)?;
    Ok((access_token, account_id))
}

pub fn live_models() -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let (access_token, account_id) = load_access_token_and_account_id()?;
    let models = fetch_models_blocking(access_token, account_id)?;
    info!(
        model_count = models.len(),
        "loaded openai-codex live models"
    );
    Ok(models)
}

pub fn available_models() -> Vec<super::DiscoveredModel> {
    let fallback = default_model_catalog();
    let discovered = match live_models() {
        Ok(models) => models,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("tokens not found") || msg.contains("not logged in") {
                debug!(error = %err, "openai-codex not configured, using fallback catalog");
            } else {
                warn!(error = %err, "failed to fetch openai-codex models, using fallback catalog");
            }
            return fallback;
        },
    };

    let merged = super::merge_discovered_with_fallback_catalog(discovered, fallback);

    info!(
        model_count = merged.len(),
        "loaded openai-codex models catalog"
    );
    merged
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
        let token = self.get_valid_token().await?;
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
            .post_responses_request_with_fallback(&token, &account_id, body)
            .await?;

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
                ..Default::default()
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
            let token = match self.get_valid_token().await {
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
                .post_responses_request_with_fallback(&token, &account_id, body)
                .await
            {
                Ok(r) => r,
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
                        yield StreamEvent::Done(Usage { input_tokens, output_tokens, ..Default::default() });
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
                                yield StreamEvent::Done(Usage { input_tokens, output_tokens, ..Default::default() });
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    #[test]
    fn convert_messages_user_multimodal_with_image() {
        use crate::model::ContentPart;

        let messages = vec![ChatMessage::User {
            content: UserContent::Multimodal(vec![
                ContentPart::Text("describe this image".to_string()),
                ContentPart::Image {
                    media_type: "image/png".to_string(),
                    data: "ABC123".to_string(),
                },
            ]),
        }];
        let converted = OpenAiCodexProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        let content = &converted[0]["content"];
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "describe this image");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,ABC123");
    }

    #[test]
    fn parse_models_payload_from_models_array() {
        let value = serde_json::json!({
            "models": [
                {"id": "gpt-5.3", "name": "GPT-5.3"},
                {"id": "gpt-5.2-codex", "display_name": "GPT-5.2 Codex"}
            ]
        });
        let models = parse_models_payload(&value);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5.3");
        assert_eq!(models[0].display_name, "GPT-5.3");
        assert_eq!(models[1].id, "gpt-5.2-codex");
    }

    #[test]
    fn parse_models_payload_from_nested_data_array() {
        let value = serde_json::json!({
            "data": {
                "items": [
                    {"slug": "gpt-5.3-codex"},
                    {"model": "gpt-5.1-codex-mini", "title": "GPT-5.1 Codex Mini"}
                ]
            }
        });
        let models = parse_models_payload(&value);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5.3-codex");
        assert_eq!(models[0].display_name, "GPT 5.3 Codex");
        assert_eq!(models[1].id, "gpt-5.1-codex-mini");
    }

    #[test]
    fn parse_models_payload_ignores_invalid_ids_and_dedupes() {
        let value = serde_json::json!({
            "models": [
                {"id": "gpt-5.3"},
                {"id": "gpt-5.3", "name": "Duplicate"},
                {"id": "this has spaces"},
                {"id": ""}
            ]
        });
        let models = parse_models_payload(&value);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.3");
    }

    #[test]
    fn parse_models_payload_keeps_non_codex_and_codex_variants() {
        let value = serde_json::json!({
            "models": [
                {"id": "gpt-5.3", "name": "GPT-5.3"},
                {"id": "gpt-5.3-codex", "name": "GPT-5.3 Codex"}
            ]
        });
        let models = parse_models_payload(&value);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5.3");
        assert_eq!(models[1].id, "gpt-5.3-codex");
    }
}
