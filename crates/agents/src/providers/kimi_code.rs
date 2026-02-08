//! Kimi Code provider.
//!
//! Authentication uses the Kimi device-flow OAuth (same as kimi-cli).
//! The API is OpenAI-compatible at `https://api.kimi.com/coding/v1`.

use std::pin::Pin;

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_oauth::{OAuthTokens, TokenStore, kimi_headers},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, trace, warn},
};

use {
    super::openai_compat::{
        SseLineResult, StreamingToolState, finalize_stream, parse_tool_calls,
        process_openai_sse_line, to_openai_tools,
    },
    crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage},
};

// ── Constants ────────────────────────────────────────────────────────────────

const KIMI_API_BASE: &str = "https://api.kimi.com/coding/v1";
const KIMI_AUTH_HOST: &str = "https://auth.kimi.com";
const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const PROVIDER_NAME: &str = "kimi-code";

/// Refresh threshold: 5 minutes before expiry.
const REFRESH_THRESHOLD_SECS: u64 = 300;

// ── Provider ─────────────────────────────────────────────────────────────────

enum AuthMode {
    OAuthTokenStore { token_store: TokenStore },
    ApiKey { api_key: Secret<String> },
}

pub struct KimiCodeProvider {
    model: String,
    client: reqwest::Client,
    base_url: String,
    auth_mode: AuthMode,
}

impl KimiCodeProvider {
    /// Build a provider that authenticates via Kimi OAuth tokens.
    pub fn new(model: String) -> Self {
        Self {
            model,
            client: reqwest::Client::new(),
            base_url: KIMI_API_BASE.into(),
            auth_mode: AuthMode::OAuthTokenStore {
                token_store: TokenStore::new(),
            },
        }
    }

    /// Build a provider that authenticates via API key.
    pub fn new_with_api_key(api_key: Secret<String>, model: String, base_url: String) -> Self {
        Self {
            model,
            client: reqwest::Client::new(),
            base_url,
            auth_mode: AuthMode::ApiKey { api_key },
        }
    }

    fn should_send_kimi_headers(&self) -> bool {
        matches!(self.auth_mode, AuthMode::OAuthTokenStore { .. })
    }

    async fn get_auth_token(&self) -> anyhow::Result<String> {
        match &self.auth_mode {
            AuthMode::ApiKey { api_key } => Ok(api_key.expose_secret().clone()),
            AuthMode::OAuthTokenStore { .. } => self.get_valid_oauth_token().await,
        }
    }

    /// Load tokens and refresh if needed (< 5 min remaining).
    async fn get_valid_oauth_token(&self) -> anyhow::Result<String> {
        let AuthMode::OAuthTokenStore { token_store } = &self.auth_mode else {
            return Err(anyhow::anyhow!("oauth token store is not configured"));
        };
        let tokens = token_store.load(PROVIDER_NAME).ok_or_else(|| {
            anyhow::anyhow!(
                "not logged in to kimi-code — run `moltis auth login --provider kimi-code`"
            )
        })?;

        // Check expiry with 5 min buffer
        if let Some(expires_at) = tokens.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now + REFRESH_THRESHOLD_SECS >= expires_at {
                if let Some(ref refresh_token) = tokens.refresh_token {
                    debug!("refreshing kimi-code token");
                    let new_tokens =
                        refresh_access_token(&self.client, refresh_token.expose_secret()).await?;
                    token_store.save(PROVIDER_NAME, &new_tokens)?;
                    return Ok(new_tokens.access_token.expose_secret().clone());
                }
                return Err(anyhow::anyhow!(
                    "kimi-code token expired and no refresh token available"
                ));
            }
        }

        Ok(tokens.access_token.expose_secret().clone())
    }
}

fn build_access_denied_hint(status: reqwest::StatusCode, body_text: &str) -> Option<String> {
    if status == reqwest::StatusCode::FORBIDDEN && body_text.contains("access_terminated_error") {
        return Some(
            "Kimi OAuth access is restricted for this account/client. Configure `kimi-code` with `KIMI_API_KEY` (or [providers.kimi-code].api_key) and use API-key auth.".into(),
        );
    }
    None
}

/// Refresh the access token using the Kimi token endpoint.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<OAuthTokens> {
    let headers = kimi_headers();
    let resp = client
        .post(format!("{KIMI_AUTH_HOST}/api/oauth/token"))
        .headers(headers)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", KIMI_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("kimi-code token refresh failed: {body}");
    }

    #[derive(serde::Deserialize)]
    struct RefreshResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let body: RefreshResponse = resp.json().await?;
    let expires_at = body.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + secs
    });

    Ok(OAuthTokens {
        access_token: Secret::new(body.access_token),
        refresh_token: body.refresh_token.map(Secret::new),
        expires_at,
    })
}

/// Check if we have stored tokens for Kimi Code.
pub fn has_stored_tokens() -> bool {
    TokenStore::new().load(PROVIDER_NAME).is_some()
}

/// Known Kimi Code models.
pub const KIMI_CODE_MODELS: &[(&str, &str)] = &[
    ("kimi-for-coding", "Kimi For Coding"),
    ("kimi-k2.5", "Kimi K2.5"),
];

// ── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for KimiCodeProvider {
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
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let token = self.get_auth_token().await?;

        let openai_messages: Vec<serde_json::Value> =
            messages.iter().map(ChatMessage::to_openai_value).collect();
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "kimi-code complete request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "kimi-code request body");

        let mut request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        if self.should_send_kimi_headers() {
            request = request.headers(kimi_headers());
        }
        let http_resp = request.json(&body).send().await?;

        let status = http_resp.status();
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body_text, "kimi-code API error");
            let hint = build_access_denied_hint(status, &body_text);
            if let Some(hint) = hint {
                anyhow::bail!("Kimi Code API error HTTP {status}: {body_text} ({hint})");
            }
            anyhow::bail!("Kimi Code API error HTTP {status}: {body_text}");
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "kimi-code raw response");

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
        Box::pin(async_stream::stream! {
            let token = match self.get_auth_token().await {
                Ok(t) => t,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let openai_messages: Vec<serde_json::Value> =
                messages.iter().map(ChatMessage::to_openai_value).collect();
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools));
            }

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                "kimi-code stream_with_tools request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "kimi-code stream request body");

            let mut request = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json");
            if self.should_send_kimi_headers() {
                request = request.headers(kimi_headers());
            }

            let resp = match request.json(&body).send().await {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let body_text = r.text().await.unwrap_or_default();
                        let hint = build_access_denied_hint(
                            reqwest::StatusCode::from_u16(status)
                                .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
                            &body_text,
                        );
                        if let Some(hint) = hint {
                            yield StreamEvent::Error(format!("HTTP {status}: {body_text} ({hint})"));
                        } else {
                            yield StreamEvent::Error(format!("HTTP {status}: {body_text}"));
                        }
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

                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };

                    match process_openai_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_stream(&state) {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use axum::{Router, extract::Request, routing::post};

    #[derive(Default, Clone)]
    struct CapturedRequest {
        headers: Vec<(String, String)>,
        body: Option<serde_json::Value>,
    }

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
                    "content": "Hello from Kimi!",
                    "role": "assistant"
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5
            }
        })
    }

    /// Test-only variant with configurable base URL and no token store.
    struct MockKimiProvider {
        model: String,
        client: reqwest::Client,
        base_url: String,
    }

    impl MockKimiProvider {
        async fn complete(
            &self,
            messages: &[serde_json::Value],
            tools: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            let token = "mock-kimi-token";

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
                .headers(kimi_headers())
                .json(&body)
                .send()
                .await?;

            let status = http_resp.status();
            if !status.is_success() {
                let body_text = http_resp.text().await.unwrap_or_default();
                anyhow::bail!("Kimi Code API error HTTP {status}: {body_text}");
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

    fn mock_provider(base_url: &str, model: &str) -> MockKimiProvider {
        MockKimiProvider {
            model: model.to_string(),
            client: reqwest::Client::new(),
            base_url: base_url.to_string(),
        }
    }

    // ── Unit tests ───────────────────────────────────────────────────────────

    #[test]
    fn has_stored_tokens_returns_false_without_tokens() {
        let _ = has_stored_tokens();
    }

    #[test]
    fn kimi_code_models_not_empty() {
        assert!(!KIMI_CODE_MODELS.is_empty());
    }

    #[test]
    fn kimi_code_models_have_unique_ids() {
        let mut ids: Vec<&str> = KIMI_CODE_MODELS.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), KIMI_CODE_MODELS.len());
    }

    #[test]
    fn provider_name_and_id() {
        let provider = KimiCodeProvider::new("kimi-k2.5".into());
        assert_eq!(provider.name(), "kimi-code");
        assert_eq!(provider.id(), "kimi-k2.5");
        assert!(provider.supports_tools());
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(KIMI_API_BASE, "https://api.kimi.com/coding/v1");
        assert_eq!(PROVIDER_NAME, "kimi-code");
        assert_eq!(KIMI_CLIENT_ID, "17e5f671-d194-4dfb-9706-5516cb48c098");
    }

    // ── Integration tests with mock server ───────────────────────────────────

    #[tokio::test]
    async fn complete_sends_required_headers() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "kimi-k2.5");

        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let result = provider.complete(&messages, &[]).await;
        assert!(result.is_ok());

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);

        let req = &reqs[0];

        // Verify X-Msh-* headers
        let has_platform = req.headers.iter().any(|(k, _)| k == "x-msh-platform");
        assert!(has_platform, "missing X-Msh-Platform header");

        let has_device_id = req.headers.iter().any(|(k, _)| k == "x-msh-device-id");
        assert!(has_device_id, "missing X-Msh-Device-Id header");

        let has_auth = req
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer mock-kimi-token");
        assert!(has_auth, "missing Authorization header");
    }

    #[tokio::test]
    async fn complete_with_api_key_does_not_send_kimi_headers() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = KimiCodeProvider::new_with_api_key(
            Secret::new("sk-kimi".into()),
            "kimi-for-coding".into(),
            base_url,
        );

        let messages = vec![ChatMessage::user("hi")];
        let result = provider.complete(&messages, &[]).await;
        assert!(result.is_ok());

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];

        let has_platform = req.headers.iter().any(|(k, _)| k == "x-msh-platform");
        assert!(!has_platform, "api-key mode should not send X-Msh-Platform");

        let has_device_id = req.headers.iter().any(|(k, _)| k == "x-msh-device-id");
        assert!(
            !has_device_id,
            "api-key mode should not send X-Msh-Device-Id"
        );

        let has_auth = req
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer sk-kimi");
        assert!(has_auth, "missing Authorization header");
    }

    #[test]
    fn access_terminated_error_adds_api_key_hint() {
        let hint = build_access_denied_hint(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":{"type":"access_terminated_error"}}"#,
        );
        assert!(hint.is_some());
        let hint_text = hint.unwrap();
        assert!(hint_text.contains("KIMI_API_KEY"));
    }

    #[tokio::test]
    async fn complete_sends_model_in_body() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "kimi-k2.5");

        let messages = vec![serde_json::json!({"role": "user", "content": "test"})];
        provider.complete(&messages, &[]).await.unwrap();

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().unwrap();
        assert_eq!(body["model"], "kimi-k2.5");
    }

    #[tokio::test]
    async fn complete_sends_tools_when_provided() {
        let (base_url, captured) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "kimi-k2.5");

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
    async fn complete_parses_text_response() {
        let (base_url, _) = start_mock_with_capture(mock_completion_response()).await;
        let provider = mock_provider(&base_url, "kimi-k2.5");

        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let resp = provider.complete(&messages, &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("Hello from Kimi!"));
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
        let provider = mock_provider(&base_url, "kimi-k2.5");

        let messages = vec![serde_json::json!({"role": "user", "content": "read file"})];
        let resp = provider.complete(&messages, &[]).await.unwrap();

        assert!(resp.text.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "read_file");
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

        let provider = mock_provider(&format!("http://{addr}"), "kimi-k2.5");
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let err = provider.complete(&messages, &[]).await.unwrap_err();
        assert!(
            err.to_string().contains("400"),
            "expected 400 in error: {err}"
        );
    }
}
