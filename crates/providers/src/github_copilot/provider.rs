use std::{pin::Pin, sync::mpsc, time::Duration};

use super::catalog::default_model_catalog;

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_oauth::{OAuthTokens, TokenStore},
    secrecy::{ExposeSecret, Secret},
    tokio_stream::Stream,
    tracing::{debug, info, trace, warn},
};

use {
    super::{
        super::openai_compat::{
            ResponsesStreamState, SseLineResult, StreamingToolState, finalize_responses_stream,
            finalize_stream, parse_openai_compat_usage_from_payload, parse_responses_completion,
            parse_tool_calls, process_openai_sse_line, process_responses_sse_line,
            split_responses_instructions_and_input, to_openai_tools, to_responses_api_tools,
        },
        diagnostics::{
            CopilotTokenResponse, completion_to_stream_events, is_responses_api_required_error,
            log_copilot_chat_error, log_copilot_request, log_copilot_response,
        },
        discovery::parse_models_payload,
        endpoints::{CopilotEndpoint, endpoint_from_cached_metadata, endpoint_from_metadata},
    },
    moltis_agents::model::{
        ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage,
        decode_tool_call_arguments_from_str,
    },
};

const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_API_BASE: &str = "https://api.individual.githubcopilot.com";

const PROVIDER_NAME: &str = "github-copilot";

pub(super) const EDITOR_VERSION: &str = "vscode/1.96.2";
pub(super) const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

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

pub(super) struct CopilotAuth {
    token: Secret<String>,
    base_url: String,
    is_enterprise: bool,
}

pub struct GitHubCopilotProvider {
    model: String,
    model_capabilities: super::super::ModelCapabilities,
    client: &'static reqwest::Client,
    token_store: TokenStore,
}

impl GitHubCopilotProvider {
    pub fn new(model: String) -> Self {
        let model_capabilities = super::super::ModelCapabilities::infer(&model);
        let mut model_capabilities = model_capabilities;
        if let Some(context_window) =
            super::super::model_capabilities::context_window_fallback_for_model(
                super::super::model_capabilities::ContextWindowFallbackScope::GitHubCopilot,
                &model,
            )
        {
            model_capabilities.context_window = context_window;
        }
        Self {
            model,
            model_capabilities,
            client: crate::shared_http_client(),
            token_store: TokenStore::new(),
        }
    }

    pub fn new_with_capabilities(
        model: String,
        capabilities: super::super::ModelCapabilities,
    ) -> Self {
        Self {
            model_capabilities: capabilities,
            ..Self::new(model)
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
            tokio::time::sleep(Duration::from_secs(interval)).await;

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
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                },
                Some(err) => anyhow::bail!("GitHub device flow error: {err}"),
                None => anyhow::bail!("unexpected response from GitHub token endpoint"),
            }
        }
    }

    async fn get_copilot_auth(&self) -> anyhow::Result<CopilotAuth> {
        fetch_copilot_auth_with_fallback(self.client, &self.token_store).await
    }

    async fn refresh_copilot_auth_after_misdirected(&self) -> anyhow::Result<Option<CopilotAuth>> {
        let Some(token_store) = token_store_with_provider_tokens(&self.token_store) else {
            return Ok(None);
        };
        warn!(
            "github-copilot individual endpoint returned 421; refreshing cached API token metadata"
        );
        if let Err(error) = token_store.delete("github-copilot-api") {
            warn!(%error, "failed to delete cached github-copilot API token before refresh");
        }
        let auth = fetch_copilot_auth(self.client, &token_store).await?;
        Ok(auth.is_enterprise.then_some(auth))
    }
}

fn home_token_store_if_different() -> Option<TokenStore> {
    let home = moltis_config::user_global_config_dir_if_different()?;
    Some(TokenStore::with_path(home.join("oauth_tokens.json")))
}

fn token_store_with_provider_tokens(primary: &TokenStore) -> Option<TokenStore> {
    debug!("checking primary token store for {PROVIDER_NAME}");
    if primary.load(PROVIDER_NAME).is_some() {
        debug!("found {PROVIDER_NAME} tokens in primary store");
        return Some(primary.clone());
    }
    if let Some(home_store) = home_token_store_if_different() {
        debug!("checking home token store for {PROVIDER_NAME}");
        if home_store.load(PROVIDER_NAME).is_some() {
            debug!("found {PROVIDER_NAME} tokens in home store");
            return Some(home_store);
        }
    }
    debug!("{PROVIDER_NAME} tokens not found in any store");
    None
}

/// Check if we have stored GitHub tokens for Copilot.
pub fn has_stored_tokens() -> bool {
    let found = token_store_with_provider_tokens(&TokenStore::new()).is_some();
    if found {
        debug!("{PROVIDER_NAME} stored tokens found");
    } else {
        debug!("{PROVIDER_NAME} stored tokens not found");
    }
    found
}

fn copilot_auth_from_parts(
    token: Secret<String>,
    endpoint: Option<CopilotEndpoint>,
) -> CopilotAuth {
    if let Some(endpoint) = endpoint {
        return CopilotAuth {
            token,
            base_url: endpoint.base_url,
            is_enterprise: endpoint.is_enterprise,
        };
    }

    CopilotAuth {
        token,
        base_url: COPILOT_API_BASE.to_string(),
        is_enterprise: false,
    }
}

pub(super) async fn fetch_copilot_auth(
    client: &reqwest::Client,
    token_store: &TokenStore,
) -> anyhow::Result<CopilotAuth> {
    let tokens = token_store.load(PROVIDER_NAME).ok_or_else(|| {
        anyhow::anyhow!("not logged in to github-copilot — run OAuth device flow first")
    })?;

    // The `access_token` stored is the GitHub user token.
    // We exchange it for a short-lived Copilot API token and cache it.
    // Endpoint metadata is persisted in the `account_id` field.
    if let Some(copilot_tokens) = token_store.load("github-copilot-api")
        && let Some(expires_at) = copilot_tokens.expires_at
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now + 60 < expires_at {
            let token = copilot_tokens.access_token.clone();
            let endpoint_metadata = copilot_tokens.account_id.as_deref();
            let endpoint = endpoint_from_cached_metadata(endpoint_metadata);
            let endpoint_metadata_valid = endpoint.is_some();
            let endpoint_metadata_present =
                endpoint_metadata.is_some_and(|value| !value.trim().is_empty());
            debug!(
                token_source = "cache",
                cached_token_expires_at = expires_at,
                cached_endpoint_metadata = ?endpoint_metadata,
                cached_endpoint_metadata_present = endpoint_metadata_present,
                cached_endpoint_metadata_valid = endpoint_metadata_valid,
                "using cached github-copilot API token"
            );

            if endpoint_metadata_valid {
                let auth = copilot_auth_from_parts(token, endpoint);
                info!(
                    token_source = "cache",
                    endpoint = %auth.base_url,
                    enterprise = auth.is_enterprise,
                    endpoint_metadata_present,
                    endpoint_metadata_valid,
                    "github-copilot authentication resolved"
                );
                return Ok(auth);
            }

            debug!(
                token_source = "exchange",
                cached_token_expires_at = expires_at,
                cached_endpoint_metadata = ?endpoint_metadata,
                cached_endpoint_metadata_present = endpoint_metadata_present,
                "cached github-copilot API token lacks valid endpoint metadata"
            );
        }

        debug!(
            token_source = "exchange",
            cached_token_expires_at = expires_at,
            cached_endpoint_metadata = ?copilot_tokens.account_id,
            "cached github-copilot API token is expired or near expiry"
        );
    } else {
        debug!(
            token_source = "exchange",
            "no cached github-copilot API token"
        );
    }

    let resp = client
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

    log_copilot_response(&resp, "token_exchange", Some(COPILOT_TOKEN_URL), None);
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Copilot token exchange failed: {body}");
    }

    let response_body = resp.text().await?;
    let copilot_resp: CopilotTokenResponse =
        serde_json::from_str(&response_body).map_err(|error| {
            anyhow::anyhow!(
                "failed to parse Copilot token exchange response: {error} (response_bytes={})",
                response_body.len()
            )
        })?;
    let endpoint_api = copilot_resp
        .endpoints
        .as_ref()
        .and_then(|endpoints| endpoints.api.as_deref());
    let endpoint = endpoint_from_metadata(endpoint_api, copilot_resp.proxy_ep.as_deref());
    let endpoint_metadata_valid = endpoint.is_some();
    debug!(
        token_source = "exchange",
        endpoint_api = ?endpoint_api,
        proxy_ep = ?copilot_resp.proxy_ep,
        endpoint_metadata_valid,
        expires_at = copilot_resp.expires_at,
        sku = ?copilot_resp.sku,
        "received github-copilot API token metadata"
    );
    let _ = token_store.save("github-copilot-api", &OAuthTokens {
        access_token: copilot_resp.token.clone(),
        refresh_token: None,
        id_token: None,
        // NOTE: account_id is repurposed here to persist the API endpoint
        // metadata so it can be recovered from the token cache.
        account_id: endpoint
            .as_ref()
            .map(|endpoint| endpoint.cache_value.clone())
            .or_else(|| Some(COPILOT_API_BASE.to_string())),
        expires_at: Some(copilot_resp.expires_at),
    });

    let auth = copilot_auth_from_parts(copilot_resp.token, endpoint);
    info!(
        token_source = "exchange",
        endpoint = %auth.base_url,
        enterprise = auth.is_enterprise,
        endpoint_metadata_present = endpoint_api.is_some() || copilot_resp.proxy_ep.is_some(),
        endpoint_metadata_valid,
        "github-copilot authentication resolved"
    );
    Ok(auth)
}

async fn fetch_copilot_auth_with_fallback(
    client: &reqwest::Client,
    primary_store: &TokenStore,
) -> anyhow::Result<CopilotAuth> {
    let Some(token_store) = token_store_with_provider_tokens(primary_store) else {
        anyhow::bail!("not logged in to github-copilot — run OAuth device flow first");
    };
    fetch_copilot_auth(client, &token_store).await
}

async fn fetch_models_from_api(
    client: &reqwest::Client,
    auth: &CopilotAuth,
) -> anyhow::Result<Vec<super::super::DiscoveredModel>> {
    log_copilot_request(
        "models",
        &auth.base_url,
        None,
        Some(auth.is_enterprise),
        false,
        None,
        None,
    );
    let response = client
        .get(format!("{}/models", auth.base_url))
        .header(
            "Authorization",
            format!("Bearer {}", auth.token.expose_secret()),
        )
        .header("Accept", "application/json")
        .header("Editor-Version", EDITOR_VERSION)
        .header("User-Agent", COPILOT_USER_AGENT)
        .send()
        .await?;
    log_copilot_response(
        &response,
        "models",
        Some(&auth.base_url),
        Some(auth.is_enterprise),
    );
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("copilot models API error HTTP {status}");
    }
    let payload: serde_json::Value = serde_json::from_str(&body)?;
    let models = parse_models_payload(&payload);
    if models.is_empty() {
        anyhow::bail!("copilot models API returned no models");
    }
    Ok(models)
}

/// Spawn model discovery in a background thread and return the receiver
/// immediately, without blocking.
pub fn start_model_discovery() -> mpsc::Receiver<anyhow::Result<Vec<super::super::DiscoveredModel>>>
{
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| {
                rt.block_on(async {
                    let client = reqwest::Client::builder()
                        .timeout(Duration::from_secs(8))
                        .build()?;
                    let token_store = TokenStore::new();
                    let auth = fetch_copilot_auth_with_fallback(&client, &token_store).await?;
                    fetch_models_from_api(&client, &auth).await
                })
            });
        let _ = tx.send(result);
    });
    rx
}

fn fetch_models_blocking() -> anyhow::Result<Vec<super::super::DiscoveredModel>> {
    start_model_discovery()
        .recv()
        .map_err(|err| anyhow::anyhow!("copilot model discovery worker failed: {err}"))?
}

pub fn live_models() -> anyhow::Result<Vec<super::super::DiscoveredModel>> {
    let models = fetch_models_blocking()?;
    debug!(
        model_count = models.len(),
        "loaded github-copilot live models"
    );
    Ok(models)
}

pub fn available_models() -> Vec<super::super::DiscoveredModel> {
    let fallback = default_model_catalog();
    let discovered = match live_models() {
        Ok(models) => models,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("not logged in") || msg.contains("tokens not found") {
                debug!(error = %err, "github-copilot not configured, using fallback catalog");
            } else {
                warn!(error = %err, "failed to fetch github-copilot models, using fallback catalog");
            }
            return fallback;
        },
    };

    super::super::merge_discovered_with_fallback_catalog(discovered, fallback)
}

// ── Enterprise streaming-to-sync bridge ──────────────────────────────────────

/// Send a streaming chat completion request and collect the SSE events into a
/// single [`CompletionResponse`].  Used for enterprise proxy endpoints that
/// reject non-streaming requests.
async fn collect_streamed_completion(
    client: &reqwest::Client,
    auth: &CopilotAuth,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> anyhow::Result<CompletionResponse> {
    let openai_messages: Vec<serde_json::Value> =
        messages.iter().map(ChatMessage::to_openai_value).collect();
    let mut body = serde_json::json!({
        "model": model,
        "messages": openai_messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(to_openai_tools(tools, true));
    }

    log_copilot_request(
        "enterprise_chat_completions",
        &auth.base_url,
        Some(model),
        Some(auth.is_enterprise),
        true,
        Some(messages.len()),
        Some(tools.len()),
    );
    let http_resp = client
        .post(format!("{}/chat/completions", auth.base_url))
        .header(
            "Authorization",
            format!("Bearer {}", auth.token.expose_secret()),
        )
        .header("content-type", "application/json")
        .header("Editor-Version", EDITOR_VERSION)
        .header("User-Agent", COPILOT_USER_AGENT)
        .json(&body)
        .send()
        .await?;

    log_copilot_response(
        &http_resp,
        "enterprise_chat_completions",
        Some(&auth.base_url),
        Some(auth.is_enterprise),
    );
    let status = http_resp.status();
    if !status.is_success() {
        let retry_after_ms = super::super::retry_after_ms_from_headers(http_resp.headers());
        let body_text = http_resp.text().await.unwrap_or_default();
        warn!(status = %status, body = %body_text, "github-copilot enterprise API error");
        anyhow::bail!(
            "{}",
            super::super::with_retry_after_marker(
                format!("GitHub Copilot API error HTTP {status}: {body_text}"),
                retry_after_ms,
            )
        );
    }

    let mut byte_stream = http_resp.bytes_stream();
    let mut buf = String::new();
    let mut state = StreamingToolState::default();
    let mut events: Vec<StreamEvent> = Vec::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        let mut offset = 0usize;
        while let Some(pos) = buf[offset..].find('\n') {
            let pos = offset + pos;
            let line = buf[offset..pos].trim();
            offset = pos + 1;

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
                    extend_events_or_error(&mut events, finalize_stream(&mut state))?;
                    return Ok(stream_events_to_completion(events));
                },
                SseLineResult::Events(new_events) => {
                    extend_events_or_error(&mut events, new_events)?;
                },
                SseLineResult::Skip => {},
            }
        }
        if offset > 0 {
            buf.drain(..offset);
        }
    }

    let line = buf.trim();
    if !line.is_empty()
        && let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
    {
        match process_openai_sse_line(data, &mut state) {
            SseLineResult::Done => {
                extend_events_or_error(&mut events, finalize_stream(&mut state))?;
                return Ok(stream_events_to_completion(events));
            },
            SseLineResult::Events(new_events) => {
                extend_events_or_error(&mut events, new_events)?;
            },
            SseLineResult::Skip => {},
        }
    }
    extend_events_or_error(&mut events, finalize_stream(&mut state))?;
    Ok(stream_events_to_completion(events))
}

fn extend_events_or_error(
    events: &mut Vec<StreamEvent>,
    new_events: Vec<StreamEvent>,
) -> anyhow::Result<()> {
    for event in new_events {
        if let StreamEvent::Error(msg) = &event {
            anyhow::bail!("{msg}");
        }
        events.push(event);
    }
    Ok(())
}

async fn collect_streamed_responses_completion(
    client: &reqwest::Client,
    auth: &CopilotAuth,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> anyhow::Result<CompletionResponse> {
    let (instructions, input) = split_responses_instructions_and_input(messages.to_vec());

    let mut body = serde_json::json!({
        "model": model,
        "stream": true,
        "input": input,
    });
    if let Some(instructions) = instructions {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(to_responses_api_tools(tools));
        body["tool_choice"] = serde_json::json!("auto");
    }

    log_copilot_request(
        "enterprise_responses",
        &auth.base_url,
        Some(model),
        Some(auth.is_enterprise),
        true,
        Some(messages.len()),
        Some(tools.len()),
    );
    let http_resp = client
        .post(format!("{}/responses", auth.base_url))
        .header(
            "Authorization",
            format!("Bearer {}", auth.token.expose_secret()),
        )
        .header("content-type", "application/json")
        .header("Editor-Version", EDITOR_VERSION)
        .header("User-Agent", COPILOT_USER_AGENT)
        .json(&body)
        .send()
        .await?;

    log_copilot_response(
        &http_resp,
        "enterprise_responses",
        Some(&auth.base_url),
        Some(auth.is_enterprise),
    );
    let status = http_resp.status();
    if !status.is_success() {
        let retry_after_ms = super::super::retry_after_ms_from_headers(http_resp.headers());
        let body_text = http_resp.text().await.unwrap_or_default();
        warn!(status = %status, body = %body_text, "github-copilot enterprise responses API error");
        anyhow::bail!(
            "{}",
            super::super::with_retry_after_marker(
                format!("GitHub Copilot Responses API error HTTP {status}: {body_text}"),
                retry_after_ms,
            )
        );
    }

    let mut byte_stream = http_resp.bytes_stream();
    let mut buf = String::new();
    let mut state = ResponsesStreamState::default();
    let mut events: Vec<StreamEvent> = Vec::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        let mut offset = 0usize;
        while let Some(pos) = buf[offset..].find('\n') {
            let pos = offset + pos;
            let line = buf[offset..pos].trim();
            offset = pos + 1;

            if line.is_empty() {
                continue;
            }

            let Some(data) = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"))
            else {
                continue;
            };

            match process_responses_sse_line(data, &mut state) {
                SseLineResult::Done => {
                    extend_events_or_error(&mut events, finalize_responses_stream(&mut state))?;
                    return Ok(stream_events_to_completion(events));
                },
                SseLineResult::Events(new_events) => {
                    extend_events_or_error(&mut events, new_events)?;
                },
                SseLineResult::Skip => {},
            }
        }
        if offset > 0 {
            buf.drain(..offset);
        }
    }

    let line = buf.trim();
    if !line.is_empty()
        && let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
    {
        match process_responses_sse_line(data, &mut state) {
            SseLineResult::Done => {
                extend_events_or_error(&mut events, finalize_responses_stream(&mut state))?;
                return Ok(stream_events_to_completion(events));
            },
            SseLineResult::Events(new_events) => {
                extend_events_or_error(&mut events, new_events)?;
            },
            SseLineResult::Skip => {},
        }
    }

    extend_events_or_error(&mut events, finalize_responses_stream(&mut state))?;
    Ok(stream_events_to_completion(events))
}

/// Collapse a collected list of [`StreamEvent`]s into a [`CompletionResponse`].
fn stream_events_to_completion(events: Vec<StreamEvent>) -> CompletionResponse {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut usage = Usage::default();

    // Track in-progress tool calls by index.
    let mut pending_tools: Vec<(String, String, String)> = Vec::new(); // (id, name, args)

    for event in events {
        match event {
            StreamEvent::Delta(s) => text_parts.push(s),
            StreamEvent::ToolCallStart {
                id, name, index, ..
            } => {
                while pending_tools.len() <= index {
                    pending_tools.push((String::new(), String::new(), String::new()));
                }
                pending_tools[index].0 = id;
                pending_tools[index].1 = name;
            },
            StreamEvent::ToolCallArgumentsDelta { index, delta } => {
                if let Some(entry) = pending_tools.get_mut(index) {
                    entry.2.push_str(&delta);
                }
            },
            StreamEvent::ToolCallComplete { index } => {
                if let Some(entry) = pending_tools.get(index) {
                    let decoded = decode_tool_call_arguments_from_str(&entry.2);
                    tool_calls.push(ToolCall {
                        id: entry.0.clone(),
                        name: entry.1.clone(),
                        arguments: decoded.arguments,
                        argument_diagnostic: decoded.diagnostic,
                        metadata: None,
                    });
                }
            },
            StreamEvent::Done(u) => usage = u,
            StreamEvent::Error(_)
            | StreamEvent::ProviderRaw(_)
            | StreamEvent::ReasoningDelta(_) => {},
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    CompletionResponse {
        text,
        tool_calls,
        usage,
    }
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
        self.model_capabilities.tools
    }

    fn context_window(&self) -> u32 {
        self.model_capabilities.context_window
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        if self.model_capabilities.requires_responses_api {
            return self.complete_responses(messages, tools).await;
        }

        let auth = self.get_copilot_auth().await?;

        // Enterprise proxy only supports streaming — delegate to the
        // streaming path and collect the result.
        if auth.is_enterprise {
            return collect_streamed_completion(self.client, &auth, &self.model, messages, tools)
                .await;
        }

        let openai_messages: Vec<serde_json::Value> =
            messages.iter().map(ChatMessage::to_openai_value).collect();
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools, true));
        }

        log_copilot_request(
            "chat_completions",
            &auth.base_url,
            Some(&self.model),
            Some(auth.is_enterprise),
            false,
            Some(messages.len()),
            Some(tools.len()),
        );
        let http_resp = self
            .client
            .post(format!("{}/chat/completions", auth.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", auth.token.expose_secret()),
            )
            .header("content-type", "application/json")
            .header("Editor-Version", EDITOR_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .json(&body)
            .send()
            .await?;

        log_copilot_response(
            &http_resp,
            "chat_completions",
            Some(&auth.base_url),
            Some(auth.is_enterprise),
        );
        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = super::super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();

            if status == reqwest::StatusCode::MISDIRECTED_REQUEST
                && !auth.is_enterprise
                && let Some(refreshed_auth) = self.refresh_copilot_auth_after_misdirected().await?
            {
                return collect_streamed_completion(
                    self.client,
                    &refreshed_auth,
                    &self.model,
                    messages,
                    tools,
                )
                .await;
            }

            // Fallback: if the model requires Responses API, retry with it.
            if status == reqwest::StatusCode::BAD_REQUEST
                && is_responses_api_required_error(&body_text)
            {
                debug!(
                    model = %self.model,
                    "chat completions returned unsupported_api_for_model, retrying with responses API"
                );
                return self.complete_responses(messages, tools).await;
            }

            warn!(status = %status, body = %body_text, "github-copilot API error");
            anyhow::bail!(
                "{}",
                super::super::with_retry_after_marker(
                    format!("GitHub Copilot API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "github-copilot raw response");

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
        if self.model_capabilities.requires_responses_api {
            return self.stream_responses_api(messages, tools);
        }
        self.stream_chat_completions(messages, tools)
    }
}

impl GitHubCopilotProvider {
    /// Non-streaming completion via the Responses API (`/responses`).
    async fn complete_responses(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let auth = self.get_copilot_auth().await?;

        if auth.is_enterprise {
            return collect_streamed_responses_completion(
                self.client,
                &auth,
                &self.model,
                messages,
                tools,
            )
            .await;
        }

        let (instructions, input) = split_responses_instructions_and_input(messages.to_vec());

        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
        });
        if let Some(instructions) = instructions {
            body["instructions"] = serde_json::Value::String(instructions);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_responses_api_tools(tools));
            body["tool_choice"] = serde_json::json!("auto");
        }

        log_copilot_request(
            "responses",
            &auth.base_url,
            Some(&self.model),
            Some(auth.is_enterprise),
            false,
            Some(messages.len()),
            Some(tools.len()),
        );
        let http_resp = self
            .client
            .post(format!("{}/responses", auth.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", auth.token.expose_secret()),
            )
            .header("content-type", "application/json")
            .header("Editor-Version", EDITOR_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .json(&body)
            .send()
            .await?;

        log_copilot_response(
            &http_resp,
            "responses",
            Some(&auth.base_url),
            Some(auth.is_enterprise),
        );
        let status = http_resp.status();
        if !status.is_success() {
            let retry_after_ms = super::super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::MISDIRECTED_REQUEST
                && !auth.is_enterprise
                && let Some(refreshed_auth) = self.refresh_copilot_auth_after_misdirected().await?
            {
                return collect_streamed_responses_completion(
                    self.client,
                    &refreshed_auth,
                    &self.model,
                    messages,
                    tools,
                )
                .await;
            }
            warn!(status = %status, body = %body_text, "github-copilot responses API error");
            anyhow::bail!(
                "{}",
                super::super::with_retry_after_marker(
                    format!("GitHub Copilot Responses API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "github-copilot responses raw response");

        Ok(parse_responses_completion(&resp))
    }

    /// Streaming via the Responses API (`/responses`) with SSE.
    fn stream_responses_api(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let auth = match self.get_copilot_auth().await {
                Ok(a) => a,
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let original_messages = messages.clone();
            let (instructions, input) = split_responses_instructions_and_input(messages);

            let mut body = serde_json::json!({
                "model": self.model,
                "stream": true,
                "input": input,
            });
            if let Some(instructions) = instructions {
                body["instructions"] = serde_json::Value::String(instructions);
            }
            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                body["tool_choice"] = serde_json::json!("auto");
            }

            log_copilot_request(
                "stream_responses_api",
                &auth.base_url,
                Some(&self.model),
                Some(auth.is_enterprise),
                true,
                Some(input.len()),
                Some(tools.len()),
            );
            let resp = match self
                .client
                .post(format!("{}/responses", auth.base_url))
                .header("Authorization", format!("Bearer {}", auth.token.expose_secret()))
                .header("content-type", "application/json")
                .header("Editor-Version", EDITOR_VERSION)
                .header("User-Agent", COPILOT_USER_AGENT)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    log_copilot_response(
                        &r,
                        "responses",
                        Some(&auth.base_url),
                        Some(auth.is_enterprise),
                    );
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = super::super::retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        if status == reqwest::StatusCode::MISDIRECTED_REQUEST.as_u16()
                            && !auth.is_enterprise
                        {
                            match self.refresh_copilot_auth_after_misdirected().await {
                                Ok(Some(refreshed_auth)) => {
                                    match collect_streamed_responses_completion(
                                        self.client,
                                        &refreshed_auth,
                                        &self.model,
                                        &original_messages,
                                        &tools,
                                    )
                                    .await
                                    {
                                        Ok(completion) => for event in completion_to_stream_events(completion) {
                                            yield event;
                                        },
                                        Err(error) => yield StreamEvent::Error(error.to_string()),
                                    }
                                    return;
                                },
                                Ok(None) => {},
                                Err(error) => {
                                    yield StreamEvent::Error(error.to_string());
                                    return;
                                },
                            }
                        }
                        yield StreamEvent::Error(super::super::with_retry_after_marker(
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

                    match process_responses_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_responses_stream(&mut state) {
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

            // Process any remaining data in the buffer.
            let line = buf.trim().to_string();
            if !line.is_empty()
                && let Some(data) = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
            {
                match process_responses_sse_line(data, &mut state) {
                    SseLineResult::Done => {
                        for event in finalize_responses_stream(&mut state) {
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

            for event in finalize_responses_stream(&mut state) {
                yield event;
            }
        })
    }

    /// Streaming via the Chat Completions API (`/chat/completions`) with SSE.
    #[allow(clippy::collapsible_if)]
    fn stream_chat_completions(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let auth = match self.get_copilot_auth().await {
                Ok(a) => a,
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
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools, true));
            }

            log_copilot_request(
                "stream_chat_completions",
                &auth.base_url,
                Some(&self.model),
                Some(auth.is_enterprise),
                true,
                Some(openai_messages.len()),
                Some(tools.len()),
            );
            let resp = match self
                .client
                .post(format!("{}/chat/completions", auth.base_url))
                .header("Authorization", format!("Bearer {}", auth.token.expose_secret()))
                .header("content-type", "application/json")
                .header("Editor-Version", EDITOR_VERSION)
                .header("User-Agent", COPILOT_USER_AGENT)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    log_copilot_response(
                        &r,
                        "stream_chat_completions",
                        Some(&auth.base_url),
                        Some(auth.is_enterprise),
                    );
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = super::super::retry_after_ms_from_headers(r.headers());
                        let response_url = r.url().to_string();
                        let body_text = r.text().await.unwrap_or_default();

                        log_copilot_chat_error(
                            "stream_chat_completions",
                            &auth.base_url,
                            &response_url,
                            &self.model,
                            auth.is_enterprise,
                            true,
                            status,
                            &body_text,
                        );

                        // Fallback: if this is an unsupported API error,
                        // switch to Responses API streaming.
                        if status == 400
                            && is_responses_api_required_error(&body_text)
                        {
                            debug!(
                                model = %self.model,
                                "chat completions returned unsupported_api_for_model, \
                                 falling back to responses API streaming"
                            );
                            let mut responses_stream =
                                self.stream_responses_api(messages, tools);
                            while let Some(event) = responses_stream.next().await {
                                yield event;
                            }
                            return;
                        }

                        yield StreamEvent::Error(super::super::with_retry_after_marker(
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
