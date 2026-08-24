//! xAI Grok subscription OAuth provider.
//!
//! Authentication uses the public Grok CLI device-flow client against
//! `auth.x.ai`. Inference rides the SuperGrok CLI chat proxy at
//! `cli-chat-proxy.grok.com` (OpenAI-compatible chat completions) so requests
//! consume subscription quota rather than billed API credits.
//!
//! The existing API-key provider `xai` remains separate and targets
//! `https://api.x.ai/v1`.

use std::{pin::Pin, sync::Arc};

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_oauth::{OAuthTokens, TokenStore, xai_proxy_headers},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, trace, warn},
};

use {
    super::openai_compat::{
        SseLineResult, StreamingToolState, finalize_stream, parse_openai_compat_usage_from_payload,
        parse_tool_calls, process_openai_sse_line, to_openai_tools,
    },
    moltis_agents::model::{
        ChatMessage, CompletionResponse, LlmProvider, ReasoningEffort, StreamEvent,
    },
};

// ── Constants ────────────────────────────────────────────────────────────────

const XAI_PROXY_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
const XAI_AUTH_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const PROVIDER_NAME: &str = "xai-oauth";

/// Refresh threshold: 5 minutes before expiry.
const REFRESH_THRESHOLD_SECS: u64 = 300;

// ── Provider ─────────────────────────────────────────────────────────────────

pub struct XaiOauthProvider {
    model: String,
    client: &'static reqwest::Client,
    base_url: String,
    token_store: TokenStore,
    reasoning_effort: Option<ReasoningEffort>,
}

impl XaiOauthProvider {
    /// Build a provider that authenticates via xAI OAuth tokens.
    pub fn new(model: String) -> Self {
        Self {
            model,
            client: crate::shared_http_client(),
            base_url: XAI_PROXY_BASE.into(),
            token_store: TokenStore::new(),
            reasoning_effort: None,
        }
    }

    /// Override the proxy base URL (tests / custom deployments).
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    fn apply_reasoning_effort(&self, body: &mut serde_json::Value) {
        if let Some(effort) = self.reasoning_effort {
            // Effort-capable Grok models accept nested reasoning.effort.
            body["reasoning"] = serde_json::json!({ "effort": effort.as_str() });
        }
    }

    /// Load tokens and refresh if needed (< 5 min remaining).
    async fn get_valid_oauth_token(&self) -> anyhow::Result<String> {
        let tokens = self.token_store.load(PROVIDER_NAME).ok_or_else(|| {
            anyhow::anyhow!(
                "not logged in to xai-oauth — run `moltis auth login --provider xai-oauth`"
            )
        })?;

        if let Some(expires_at) = tokens.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now + REFRESH_THRESHOLD_SECS >= expires_at {
                if let Some(ref refresh_token) = tokens.refresh_token {
                    debug!("refreshing xai-oauth token");
                    let new_tokens =
                        refresh_access_token(self.client, refresh_token.expose_secret()).await?;
                    self.token_store.save(PROVIDER_NAME, &new_tokens)?;
                    return Ok(new_tokens.access_token.expose_secret().clone());
                }
                return Err(anyhow::anyhow!(
                    "xai-oauth token expired and no refresh token available — run `moltis auth login --provider xai-oauth`"
                ));
            }
        }

        Ok(tokens.access_token.expose_secret().clone())
    }
}

fn build_access_denied_hint(status: reqwest::StatusCode, body_text: &str) -> Option<String> {
    let lower = body_text.to_ascii_lowercase();
    if status == reqwest::StatusCode::FORBIDDEN
        || lower.contains("personal-team-blocked")
        || lower.contains("spending-limit")
    {
        return Some(
            "xAI subscription OAuth is not entitled for this account/tier on this surface. Use the API-key provider `xai` with `XAI_API_KEY`, or upgrade the subscription. Re-login will not fix entitlement errors.".into(),
        );
    }
    None
}

/// Refresh the access token using the xAI token endpoint.
///
/// xAI rotates refresh tokens — callers must persist the returned pair.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<OAuthTokens> {
    let resp = client
        .post(XAI_AUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("client_id", XAI_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        if status.as_u16() == 403 {
            anyhow::bail!(
                "xai-oauth token refresh forbidden (HTTP 403): subscription not entitled for API access. Use `XAI_API_KEY` / provider `xai` instead. Body: {body}"
            );
        }
        if status.as_u16() == 400 || status.as_u16() == 401 {
            anyhow::bail!(
                "xai-oauth token refresh failed (HTTP {status}): re-login required via `moltis auth login --provider xai-oauth`. Body: {body}"
            );
        }
        anyhow::bail!("xai-oauth token refresh failed (HTTP {status}): {body}");
    }

    #[derive(serde::Deserialize)]
    struct RefreshResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let body: RefreshResponse = resp.json().await?;
    // xAI rotates refresh tokens; requiring a replacement avoids leaving a
    // consumed token on disk.
    let Some(new_refresh) = body.refresh_token else {
        anyhow::bail!(
            "xai-oauth token refresh omitted refresh_token; refusing to persist a consumed token"
        );
    };
    let expires_at = body.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + secs
    });

    Ok(OAuthTokens {
        access_token: Secret::new(body.access_token),
        refresh_token: Some(Secret::new(new_refresh)),
        id_token: None,
        account_id: None,
        expires_at,
    })
}

/// Check if we have stored tokens for xAI OAuth.
pub fn has_stored_tokens() -> bool {
    TokenStore::new().load(PROVIDER_NAME).is_some()
}

/// Known SuperGrok / Heavy subscription models (fallback catalog).
pub const XAI_OAUTH_MODELS: &[(&str, &str)] = &[
    ("grok-4.5", "Grok 4.5"),
    ("grok-4.3", "Grok 4.3"),
    ("grok-build", "Grok Build"),
    ("grok-composer-2.5-fast", "Composer 2.5"),
    ("grok-4.20-0309-reasoning", "Grok 4.20 Reasoning"),
    ("grok-4.20-0309-non-reasoning", "Grok 4.20 Non-Reasoning"),
    ("grok-4.20-multi-agent-0309", "Grok 4.20 Multi-Agent"),
];

// ── LlmProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for XaiOauthProvider {
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
        let token = self.get_valid_oauth_token().await?;

        let openai_messages: Vec<serde_json::Value> =
            messages.iter().map(ChatMessage::to_openai_value).collect();
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        self.apply_reasoning_effort(&mut body);

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools, true));
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "xai-oauth complete request"
        );
        trace!(
            body_bytes = serde_json::to_vec(&body).map_or(0, |value| value.len()),
            "xai-oauth request body prepared"
        );

        let http_resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .headers(xai_proxy_headers(Some(&self.model)))
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body_bytes = body_text.len(), "xai-oauth API error");
            let hint = build_access_denied_hint(status, &body_text);
            if let Some(hint) = hint {
                anyhow::bail!(
                    "{}",
                    super::with_retry_after_marker(
                        format!("xAI OAuth API error HTTP {status}: {body_text} ({hint})"),
                        retry_after_ms,
                    )
                );
            }
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("xAI OAuth API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(
            response_bytes = serde_json::to_vec(&resp).map_or(0, |value| value.len()),
            "xai-oauth response received"
        );

        let message = &resp["choices"][0]["message"];
        let text = message["content"].as_str().map(|s| s.to_string());
        let tool_calls = parse_tool_calls(message);
        let usage = parse_openai_compat_usage_from_payload(&resp).unwrap_or_default();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let token = match self.get_valid_oauth_token().await {
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
            self.apply_reasoning_effort(&mut body);

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools, true));
            }

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                "xai-oauth stream_with_tools request"
            );
            trace!(body_bytes = serde_json::to_vec(&body).map_or(0, |value| value.len()), "xai-oauth stream request body prepared");

            let resp = match self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .headers(xai_proxy_headers(Some(&self.model)))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = super::retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        let status_code = reqwest::StatusCode::from_u16(status)
                            .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR);
                        let hint = build_access_denied_hint(status_code, &body_text);
                        if let Some(hint) = hint {
                            yield StreamEvent::Error(super::with_retry_after_marker(
                                format!("HTTP {status}: {body_text} ({hint})"),
                                retry_after_ms,
                            ));
                        } else {
                            yield StreamEvent::Error(super::with_retry_after_marker(
                                format!("HTTP {status}: {body_text}"),
                                retry_after_ms,
                            ));
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

    fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.reasoning_effort
    }

    fn with_reasoning_effort(
        self: Arc<Self>,
        effort: ReasoningEffort,
    ) -> Option<Arc<dyn LlmProvider>> {
        Some(Arc::new(Self {
            model: self.model.clone(),
            client: self.client,
            base_url: self.base_url.clone(),
            token_store: TokenStore::new(),
            reasoning_effort: Some(effort),
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xai_oauth_models_not_empty() {
        assert!(!XAI_OAUTH_MODELS.is_empty());
    }

    #[test]
    fn xai_oauth_models_have_unique_ids() {
        let mut ids: Vec<&str> = XAI_OAUTH_MODELS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), XAI_OAUTH_MODELS.len());
    }

    #[test]
    fn provider_name_and_model() {
        let provider = XaiOauthProvider::new("grok-4.5".into());
        assert_eq!(provider.name(), "xai-oauth");
        assert_eq!(provider.id(), "grok-4.5");
        assert!(provider.supports_tools());
    }

    #[test]
    fn entitlement_hint_on_forbidden() {
        let hint = build_access_denied_hint(
            reqwest::StatusCode::FORBIDDEN,
            "personal-team-blocked:spending-limit",
        );
        let msg = hint.expect("hint");
        assert!(msg.contains("XAI_API_KEY"));
        assert!(msg.contains("Re-login will not fix"));
    }
}
