use {
    moltis_agents::model::{CompletionResponse, StreamEvent},
    secrecy::Secret,
    tracing::{debug, info, warn},
};

pub(super) fn is_responses_api_required_error(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("unsupported_api_for_model")
        || lower.contains("not accessible via the /chat/completions")
}

pub(super) fn completion_to_stream_events(completion: CompletionResponse) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if let Some(text) = completion.text {
        events.push(StreamEvent::Delta(text));
    }
    for (index, tool_call) in completion.tool_calls.into_iter().enumerate() {
        events.push(StreamEvent::ToolCallStart {
            id: tool_call.id,
            name: tool_call.name,
            index,
            metadata: tool_call.metadata,
        });
        events.push(StreamEvent::ToolCallArgumentsDelta {
            index,
            delta: tool_call.arguments.to_string(),
        });
        events.push(StreamEvent::ToolCallComplete { index });
    }
    events.push(StreamEvent::Done(completion.usage));
    events
}

#[derive(serde::Deserialize)]
pub(super) struct CopilotTokenResponse {
    pub(super) token: Secret<String>,
    pub(super) expires_at: u64,
    pub(super) sku: Option<String>,
    pub(super) endpoints: Option<CopilotTokenEndpoints>,
    #[serde(rename = "proxy-ep")]
    pub(super) proxy_ep: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct CopilotTokenEndpoints {
    pub(super) api: Option<String>,
}

impl std::fmt::Debug for CopilotTokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopilotTokenResponse")
            .field("token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("sku", &self.sku)
            .field("endpoints", &self.endpoints)
            .field("proxy_ep", &self.proxy_ep)
            .finish()
    }
}

fn response_header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

pub(super) fn log_copilot_response(
    response: &reqwest::Response,
    operation: &str,
    endpoint: Option<&str>,
    enterprise: Option<bool>,
) {
    let status = response.status();
    let response_host = response.url().host_str().unwrap_or("unknown");
    let response_path = response.url().path();
    let content_type = response_header(response, "content-type");
    let server = response_header(response, "server");
    let request_id = response_header(response, "x-github-request-id");
    let via = response_header(response, "via");

    if status == reqwest::StatusCode::MISDIRECTED_REQUEST {
        warn!(
            operation,
            status = %status,
            endpoint = ?endpoint,
            enterprise = ?enterprise,
            response_url = %response.url(),
            response_host,
            response_path,
            content_type = ?content_type,
            server = ?server,
            request_id = ?request_id,
            via = ?via,
            "github-copilot returned 421 Misdirected Request; check enterprise proxy routing and cached token metadata"
        );
    } else {
        debug!(
            operation,
            status = %status,
            endpoint = ?endpoint,
            enterprise = ?enterprise,
            response_host,
            response_path,
            content_type = ?content_type,
            server = ?server,
            request_id = ?request_id,
            via = ?via,
            "github-copilot HTTP response"
        );
    }
}

pub(super) fn log_copilot_request(
    operation: &str,
    endpoint: &str,
    model: Option<&str>,
    enterprise: Option<bool>,
    stream: bool,
    messages_count: Option<usize>,
    tools_count: Option<usize>,
) {
    info!(
        operation,
        endpoint,
        model,
        enterprise,
        stream,
        messages_count,
        tools_count,
        editor_version = super::provider::EDITOR_VERSION,
        user_agent = super::provider::COPILOT_USER_AGENT,
        "github-copilot HTTP request"
    );
}

pub(super) fn log_copilot_chat_error(
    operation: &str,
    endpoint: &str,
    response_url: &str,
    model: &str,
    enterprise: bool,
    stream: bool,
    status: u16,
    body: &str,
) {
    const MAX_ERROR_BODY_LENGTH: usize = 4_096;
    let body = body.trim();
    let body = if body.len() > MAX_ERROR_BODY_LENGTH {
        let end = body
            .char_indices()
            .take_while(|(index, _)| *index < MAX_ERROR_BODY_LENGTH)
            .last()
            .map_or(0, |(index, character)| index + character.len_utf8());
        format!("{}...[truncated]", &body[..end])
    } else {
        body.to_string()
    };

    warn!(
        operation,
        endpoint,
        response_url,
        model,
        enterprise,
        stream,
        status,
        error_body = %body,
        "github-copilot chat request failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_response_preserves_enterprise_diagnostics() {
        let response: CopilotTokenResponse = serde_json::from_value(serde_json::json!({
            "token": "secret-token",
            "expires_at": 1_800_000_000_u64,
            "endpoints": { "api": "https://api.enterprise.githubcopilot.com" },
            "proxy-ep": "proxy.enterprise.githubcopilot.com",
            "sku": "copilot_enterprise_seat_multi_quota"
        }))
        .unwrap_or_else(|err| panic!("token response should deserialize: {err}"));

        assert_eq!(
            response.proxy_ep.as_deref(),
            Some("proxy.enterprise.githubcopilot.com")
        );
        assert_eq!(
            response.sku.as_deref(),
            Some("copilot_enterprise_seat_multi_quota")
        );
        assert_eq!(
            response
                .endpoints
                .as_ref()
                .and_then(|endpoints| endpoints.api.as_deref()),
            Some("https://api.enterprise.githubcopilot.com")
        );
        assert!(!format!("{response:?}").contains("secret-token"));
    }

    #[test]
    fn chat_error_body_is_bounded_at_utf8_boundary() {
        let body = "é".repeat(3_000);
        log_copilot_chat_error(
            "stream_chat_completions",
            "https://proxy.enterprise.githubcopilot.com",
            "https://proxy.enterprise.githubcopilot.com/chat/completions",
            "claude-sonnet-4",
            true,
            true,
            421,
            &body,
        );
    }

    #[test]
    fn completion_to_stream_events_preserves_terminal_usage() {
        let completion = CompletionResponse {
            text: Some("hello".to_string()),
            tool_calls: Vec::new(),
            usage: Default::default(),
        };
        let events = completion_to_stream_events(completion);
        assert!(matches!(events.first(), Some(StreamEvent::Delta(text)) if text == "hello"));
        assert!(matches!(events.last(), Some(StreamEvent::Done(_))));
    }
}
