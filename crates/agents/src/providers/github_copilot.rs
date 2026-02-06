//! GitHub Copilot provider.
//!
//! Authentication uses the GitHub device-flow OAuth to obtain a GitHub token,
//! then exchanges it for a short-lived Copilot API token via
//! `https://api.github.com/copilot_internal/v2/token`.
//!
//! The Copilot API itself is OpenAI-compatible (`/chat/completions`).

use std::pin::Pin;

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_oauth::{OAuthTokens, TokenStore},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, trace, warn},
};

use crate::model::{CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage};

// ── Constants ────────────────────────────────────────────────────────────────

/// GitHub OAuth app client ID for Copilot (VS Code's public client ID).
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_API_BASE: &str = "https://api.individual.githubcopilot.com";

const PROVIDER_NAME: &str = "github-copilot";

/// Required headers for the Copilot chat completions API.
/// The API rejects requests without `Editor-Version`.
const EDITOR_VERSION: &str = "vscode/1.96.2";
const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

// ── Device flow types ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
}

#[derive(Debug, serde::Deserialize)]
struct GithubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: u64,
}

// ── Provider ─────────────────────────────────────────────────────────────────

pub struct GitHubCopilotProvider {
    model: String,
    client: reqwest::Client,
    token_store: TokenStore,
}

impl GitHubCopilotProvider {
    pub fn new(model: String) -> Self {
        Self {
            model,
            client: reqwest::Client::new(),
            token_store: TokenStore::new(),
        }
    }

    /// Start the GitHub device-flow: request a device code from GitHub.
    pub async fn request_device_code(
        client: &reqwest::Client,
    ) -> anyhow::Result<DeviceCodeResponse> {
        let resp = client
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .form(&[("client_id", GITHUB_CLIENT_ID), ("scope", "")])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GitHub device code request failed: {body}");
        }

        Ok(resp.json().await?)
    }

    /// Poll GitHub for the access token after the user has entered the code.
    pub async fn poll_for_token(
        client: &reqwest::Client,
        device_code: &str,
        interval: u64,
    ) -> anyhow::Result<String> {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

            let resp = client
                .post(GITHUB_TOKEN_URL)
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", GITHUB_CLIENT_ID),
                    ("device_code", device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await?;

            let body: GithubTokenResponse = resp.json().await?;

            if let Some(token) = body.access_token {
                return Ok(token);
            }

            match body.error.as_deref() {
                Some("authorization_pending") => continue,
                Some("slow_down") => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                },
                Some(err) => anyhow::bail!("GitHub device flow error: {err}"),
                None => anyhow::bail!("unexpected response from GitHub token endpoint"),
            }
        }
    }

    /// Get a valid Copilot API token, exchanging the GitHub token if needed.
    async fn get_valid_copilot_token(&self) -> anyhow::Result<String> {
        let tokens = self.token_store.load(PROVIDER_NAME).ok_or_else(|| {
            anyhow::anyhow!("not logged in to github-copilot — run OAuth device flow first")
        })?;

        // The `access_token` stored is the GitHub user token.
        // We need to exchange it for a Copilot API token each time
        // (the Copilot token is short-lived ~30 min).
        // We store the Copilot token in a separate key for caching.
        if let Some(copilot_tokens) = self.token_store.load("github-copilot-api")
            && let Some(expires_at) = copilot_tokens.expires_at
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now + 60 < expires_at {
                return Ok(copilot_tokens.access_token.expose_secret().clone());
            }
        }

        // Exchange GitHub token for Copilot API token
        let resp = self
            .client
            .get(COPILOT_TOKEN_URL)
            .header(
                "Authorization",
                format!("token {}", tokens.access_token.expose_secret()),
            )
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                "moltis/0.1.0 (GitHub Copilot compatible client)",
            )
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Copilot token exchange failed: {body}");
        }

        let copilot_resp: CopilotTokenResponse = resp.json().await?;

        // Cache the Copilot API token
        let _ = self.token_store.save("github-copilot-api", &OAuthTokens {
            access_token: Secret::new(copilot_resp.token.clone()),
            refresh_token: None,
            expires_at: Some(copilot_resp.expires_at),
        });

        Ok(copilot_resp.token)
    }
}

/// Check if we have stored GitHub tokens for Copilot.
pub fn has_stored_tokens() -> bool {
    TokenStore::new().load(PROVIDER_NAME).is_some()
}

/// Known Copilot models.
/// The list is intentionally broad; if a model isn't available for the user's
/// plan Copilot will return an error.
pub const COPILOT_MODELS: &[(&str, &str)] = &[
    ("gpt-4o", "GPT-4o (Copilot)"),
    ("gpt-4.1", "GPT-4.1 (Copilot)"),
    ("gpt-4.1-mini", "GPT-4.1 Mini (Copilot)"),
    ("gpt-4.1-nano", "GPT-4.1 Nano (Copilot)"),
    ("o1", "o1 (Copilot)"),
    ("o1-mini", "o1-mini (Copilot)"),
    ("o3-mini", "o3-mini (Copilot)"),
    ("claude-sonnet-4", "Claude Sonnet 4 (Copilot)"),
    ("gemini-2.0-flash", "Gemini 2.0 Flash (Copilot)"),
];

// ── Parse helpers (reuse OpenAI format) ──────────────────────────────────────

fn to_openai_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["parameters"],
                }
            })
        })
        .collect()
}

fn parse_tool_calls(message: &serde_json::Value) -> Vec<ToolCall> {
    message["tool_calls"]
        .as_array()
        .map(|tcs| {
            tcs.iter()
                .filter_map(|tc| {
                    let id = tc["id"].as_str()?.to_string();
                    let name = tc["function"]["name"].as_str()?.to_string();
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for GitHubCopilotProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let token = self.get_valid_copilot_token().await?;

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "github-copilot complete request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "github-copilot request body");

        let http_resp = self
            .client
            .post(format!("{COPILOT_API_BASE}/chat/completions"))
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("Editor-Version", EDITOR_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body_text, "github-copilot API error");
            anyhow::bail!("GitHub Copilot API error HTTP {status}: {body_text}");
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "github-copilot raw response");

        let message = &resp["choices"][0]["message"];

        let text = message["content"].as_str().map(|s| s.to_string());
        let tool_calls = parse_tool_calls(message);

        let usage = Usage {
            input_tokens: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream(
        &self,
        messages: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools(
        &self,
        messages: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let token = match self.get_valid_copilot_token().await {
                Ok(t) => t,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let mut body = serde_json::json!({
                "model": self.model,
                "messages": messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools));
            }

            debug!(
                model = %self.model,
                messages_count = messages.len(),
                tools_count = tools.len(),
                "github-copilot stream_with_tools request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "github-copilot stream request body");

            let resp = match self
                .client
                .post(format!("{COPILOT_API_BASE}/chat/completions"))
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("Editor-Version", EDITOR_VERSION)
                .header("User-Agent", COPILOT_USER_AGENT)
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
            let mut started_tool_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

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
                        yield StreamEvent::Done(Usage { input_tokens, output_tokens });
                        return;
                    }

                    if let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(u) = evt.get("usage").filter(|u| !u.is_null()) {
                            input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                            output_tokens = u["completion_tokens"].as_u64().unwrap_or(0) as u32;
                        }

                        let delta = &evt["choices"][0]["delta"];

                        if let Some(text) = delta["content"].as_str() {
                            if !text.is_empty() {
                                yield StreamEvent::Delta(text.to_string());
                            }
                        }

                        if let Some(tool_calls) = delta["tool_calls"].as_array() {
                            for tc in tool_calls {
                                let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                if !started_tool_indices.contains(&index) {
                                    if let Some(id) = tc["id"].as_str() {
                                        let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                        started_tool_indices.insert(index);
                                        yield StreamEvent::ToolCallStart { id: id.to_string(), name, index };
                                    }
                                }
                                if let Some(args_delta) = tc["function"]["arguments"].as_str() {
                                    if !args_delta.is_empty() {
                                        yield StreamEvent::ToolCallArgumentsDelta { index, delta: args_delta.to_string() };
                                    }
                                }
                            }
                        }

                        if let Some(finish) = evt["choices"][0]["finish_reason"].as_str() {
                            if finish == "tool_calls" {
                                for &idx in &started_tool_indices {
                                    yield StreamEvent::ToolCallComplete { index: idx };
                                }
                            }
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

    use std::sync::{Arc, Mutex};

    use axum::{Router, extract::Request, routing::post};

    /// Captured request data for assertions.
    #[derive(Default, Clone)]
    struct CapturedRequest {
        headers: Vec<(String, String)>,
        body: Option<serde_json::Value>,
    }

    /// Start a mock HTTP server, returning (base_url, captured_requests).
    async fn start_mock_with_capture(
        response_body: serde_json::Value,
    ) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();
        let resp_body = response_body.clone();

        let app = Router::new().route(
            "/chat/completions",
            post(move |req: Request| {
                let cap = captured_clone.clone();
                let resp = resp_body.clone();
                async move {
                    let headers: Vec<(String, String)> = req
                        .headers()
                        .iter()
                        .map(|(k, v)| {
                            (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())
                        })
                        .collect();

                    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    let body: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();

                    cap.lock().unwrap().push(CapturedRequest { headers, body });

                    axum::Json(resp)
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}"), captured)
    }

    fn mock_completion_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Hello from Copilot!",
                    "role": "assistant"
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5
            }
        })
    }

    /// Create a provider that talks to a local mock server instead of the real
    /// Copilot API, and doesn't need stored OAuth tokens.
    fn mock_provider(base_url: &str, model: &str) -> MockCopilotProvider {
        MockCopilotProvider {
            model: model.to_string(),
            client: reqwest::Client::new(),
            base_url: base_url.to_string(),
        }
    }

    /// A test-only variant that uses a configurable base URL and a supplied
    /// token instead of the token store.
    struct MockCopilotProvider {
        model: String,
        client: reqwest::Client,
        base_url: String,
    }

    impl MockCopilotProvider {
        async fn complete(
            &self,
            messages: &[serde_json::Value],
            tools: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            let token = "mock-copilot-token";

            let mut body = serde_json::json!({
                "model": self.model,
                "messages": messages,
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
            }

            let http_resp = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("Editor-Version", EDITOR_VERSION)
                .header("User-Agent", COPILOT_USER_AGENT)
                .json(&body)
                .send()
                .await?;

            let status = http_resp.status();
            if !status.is_success() {
                let body_text = http_resp.text().await.unwrap_or_default();
                anyhow::bail!("Copilot API error HTTP {status}: {body_text}");
            }

            let resp = http_resp.json::<serde_json::Value>().await?;
            let message = &resp["choices"][0]["message"];
            let text = message["content"].as_str().map(|s| s.to_string());
            let tool_calls = parse_tool_calls(message);
            let usage = Usage {
                input_tokens: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            };

            Ok(CompletionResponse {
                text,
                tool_calls,
                usage,
            })
        }
    }

    // ── Unit tests ───────────────────────────────────────────────────────────

    #[test]
    fn has_stored_tokens_returns_false_without_tokens() {
        let _ = has_stored_tokens();
    }

    #[test]
    fn copilot_models_not_empty() {
        assert!(!COPILOT_MODELS.is_empty());
    }

    #[test]
    fn copilot_models_have_unique_ids() {
        let mut ids: Vec<&str> = COPILOT_MODELS.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), COPILOT_MODELS.len());
    }

    #[test]
    fn to_openai_tools_converts_correctly() {
        let tools = vec![serde_json::json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {"type": "object"}
        })];
        let converted = to_openai_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["function"]["name"], "test_tool");
    }

    #[test]
    fn to_openai_tools_empty_input() {
        let converted = to_openai_tools(&[]);
        assert!(converted.is_empty());
    }

    #[test]
    fn parse_tool_calls_empty() {
        let msg = serde_json::json!({"content": "hello"});
        assert!(parse_tool_calls(&msg).is_empty());
    }

    #[test]
    fn parse_tool_calls_with_calls() {
        let msg = serde_json::json!({
            "tool_calls": [{
                "id": "call_1",
                "function": {
                    "name": "get_weather",
                    "arguments": "{\"city\":\"SF\"}"
                }
            }]
        });
        let calls = parse_tool_calls(&msg);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "SF");
    }

    #[test]
    fn parse_tool_calls_multiple() {
        let msg = serde_json::json!({
            "tool_calls": [
                {
                    "id": "call_1",
                    "function": {
                        "name": "tool_a",
                        "arguments": "{}"
                    }
                },
                {
                    "id": "call_2",
                    "function": {
                        "name": "tool_b",
                        "arguments": "{\"x\":1}"
                    }
                }
            ]
        });
        let calls = parse_tool_calls(&msg);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool_a");
        assert_eq!(calls[1].name, "tool_b");
    }

    #[test]
    fn provider_name_and_id() {
        let provider = GitHubCopilotProvider::new("gpt-4o".into());
        assert_eq!(provider.name(), "github-copilot");
        assert_eq!(provider.id(), "gpt-4o");
        assert!(provider.supports_tools());
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(COPILOT_API_BASE, "https://api.individual.githubcopilot.com");
        assert_eq!(EDITOR_VERSION, "vscode/1.96.2");
        assert!(!COPILOT_USER_AGENT.is_empty());
        assert_eq!(PROVIDER_NAME, "github-copilot");
    }

    // ── Integration tests with mock server ───────────────────────────────────

    #[tokio::test]
    async fn complete_sends_required_headers() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let result = provider.complete(&messages, &[]).await;
        assert!(result.is_ok());

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);

        let req = &reqs[0];

        // Verify required headers
        let has_editor_version = req
            .headers
            .iter()
            .any(|(k, v)| k == "editor-version" && v == EDITOR_VERSION);
        assert!(
            has_editor_version,
            "missing Editor-Version header; got: {:?}",
            req.headers
        );

        let has_user_agent = req
            .headers
            .iter()
            .any(|(k, v)| k == "user-agent" && v == COPILOT_USER_AGENT);
        assert!(
            has_user_agent,
            "missing User-Agent header; got: {:?}",
            req.headers
        );

        let has_auth = req
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer mock-copilot-token");
        assert!(has_auth, "missing Authorization header");

        let has_content_type = req
            .headers
            .iter()
            .any(|(k, v)| k == "content-type" && v == "application/json");
        assert!(has_content_type, "missing content-type header");
    }

    #[tokio::test]
    async fn complete_sends_model_in_body() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4.1");

        let messages = vec![serde_json::json!({"role": "user", "content": "test"})];
        provider.complete(&messages, &[]).await.unwrap();

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().unwrap();
        assert_eq!(body["model"], "gpt-4.1");
        assert_eq!(body["messages"][0]["content"], "test");
    }

    #[tokio::test]
    async fn complete_sends_tools_when_provided() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "test"})];
        let tools = vec![serde_json::json!({
            "name": "read_file",
            "description": "Read a file",
            "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
        })];
        provider.complete(&messages, &tools).await.unwrap();

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().unwrap();
        let tools_arr = body["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 1);
        assert_eq!(tools_arr[0]["type"], "function");
        assert_eq!(tools_arr[0]["function"]["name"], "read_file");
    }

    #[tokio::test]
    async fn complete_omits_tools_when_empty() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "test"})];
        provider.complete(&messages, &[]).await.unwrap();

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().unwrap();
        assert!(body.get("tools").is_none());
    }

    #[tokio::test]
    async fn complete_parses_text_response() {
        let (base_url, _) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let resp = provider.complete(&messages, &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("Hello from Copilot!"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
    }

    #[tokio::test]
    async fn complete_parses_tool_call_response() {
        let response = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/tmp/test.txt\"}"
                        }
                    }]
                }
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10}
        });

        let (base_url, _) = start_mock_with_capture(response).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "read file"})];
        let resp = provider.complete(&messages, &[]).await.unwrap();

        assert!(resp.text.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].arguments["path"], "/tmp/test.txt");
    }

    #[tokio::test]
    async fn complete_handles_server_error() {
        let app = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    "bad request: missing something",
                )
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let provider = mock_provider(&format!("http://{addr}"), "gpt-4o");
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let err = provider.complete(&messages, &[]).await.unwrap_err();
        assert!(
            err.to_string().contains("400"),
            "expected 400 in error: {err}"
        );
    }

    #[tokio::test]
    async fn complete_does_not_send_copilot_integration_id() {
        // Regression: the API rejects requests with an unknown
        // Copilot-Integration-Id header.
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "gpt-4o");

        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        provider.complete(&messages, &[]).await.unwrap();

        let reqs = captured.lock().unwrap();
        let has_integration_id = reqs[0]
            .headers
            .iter()
            .any(|(k, _)| k == "copilot-integration-id");
        assert!(
            !has_integration_id,
            "copilot-integration-id header should NOT be sent"
        );
    }
}
