use {
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    secrecy::ExposeSecret,
    serde_json::Value,
    tracing::debug,
};

use crate::openai_codex::OpenAiCodexProvider;

const CODEX_IMAGE_RESPONSES_MODEL: &str = "gpt-5.4";
const DEFAULT_IMAGE_MODEL: &str = "gpt-image-2";
const DEFAULT_SIZE: &str = "1024x1024";
const DEFAULT_QUALITY: &str = "medium";
const DEFAULT_OUTPUT_FORMAT: &str = "png";
const DEFAULT_BACKGROUND: &str = "opaque";
const MAX_IMAGE_SSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_SSE_EVENTS: usize = 512;
const MAX_IMAGE_BASE64_CHARS: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    pub model: String,
    pub size: String,
    pub quality: String,
    pub output_format: String,
    pub background: String,
}

impl ImageGenerationRequest {
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            model: DEFAULT_IMAGE_MODEL.to_string(),
            size: DEFAULT_SIZE.to_string(),
            quality: DEFAULT_QUALITY.to_string(),
            output_format: DEFAULT_OUTPUT_FORMAT.to_string(),
            background: DEFAULT_BACKGROUND.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub media_type: String,
    pub data: String,
}

impl GeneratedImage {
    #[must_use]
    pub fn data_uri(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenerationResult {
    pub model: String,
    pub images: Vec<GeneratedImage>,
}

pub struct OpenAiCodexImageProvider {
    base_url: String,
    client: &'static reqwest::Client,
    auth: OpenAiCodexProvider,
}

impl Default for OpenAiCodexImageProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAiCodexImageProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client: crate::shared_http_client(),
            auth: OpenAiCodexProvider::new(CODEX_IMAGE_RESPONSES_MODEL.to_string()),
        }
    }

    pub async fn generate_image(
        &self,
        request: ImageGenerationRequest,
    ) -> anyhow::Result<ImageGenerationResult> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            anyhow::bail!("image prompt is required");
        }

        let tokens = self.auth.get_valid_tokens().await?;
        let token = tokens.access_token.expose_secret().clone();
        let account_id = OpenAiCodexProvider::resolve_account_id(&tokens)?;
        let body = build_codex_image_request_body(&request);

        let response = self
            .client
            .post(format!("{}/responses", self.base_url.trim_end_matches('/')))
            .header("Authorization", format!("Bearer {token}"))
            .header("chatgpt-account-id", account_id)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "pi")
            .header("accept", "text/event-stream")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let retry_after_ms = super::retry_after_ms_from_headers(response.headers());
        let body_text = read_limited_response_body(response).await?;
        if !status.is_success() {
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("openai-codex image generation API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        extract_codex_image_generation_result(&body_text, &request.model, &request.output_format)
    }
}

fn build_codex_image_request_body(request: &ImageGenerationRequest) -> Value {
    serde_json::json!({
        "model": CODEX_IMAGE_RESPONSES_MODEL,
        "store": false,
        "stream": true,
        "instructions": "You are an image generation assistant.",
        "input": [{
            "role": "user",
            "content": [{"type": "input_text", "text": request.prompt.trim()}],
        }],
        "tools": [{
            "type": "image_generation",
            "model": request.model,
            "size": request.size,
            "quality": request.quality,
            "output_format": request.output_format,
            "background": request.background,
        }],
        "tool_choice": {"type": "image_generation"},
    })
}

async fn read_limited_response_body(response: reqwest::Response) -> anyhow::Result<String> {
    use futures::TryStreamExt as _;

    let mut buf = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.try_next().await? {
        if buf.len() + chunk.len() > MAX_IMAGE_SSE_BYTES {
            anyhow::bail!("openai-codex image generation response exceeded size limit");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[derive(Debug, Default)]
struct ParsedImageEvent {
    event_type: Option<String>,
    item_type: Option<String>,
    result: Option<String>,
    response_output: Vec<(Option<String>, Option<String>)>,
    error_message: Option<String>,
}

fn parse_image_events(body: &str) -> anyhow::Result<Vec<ParsedImageEvent>> {
    let mut events = Vec::new();
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data: ").map(str::trim) else {
            continue;
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value = match serde_json::from_str::<Value>(data) {
            Ok(value) => value,
            Err(err) => {
                debug!(error = %err, "ignoring non-json codex image SSE payload");
                continue;
            },
        };
        if events.len() >= MAX_IMAGE_SSE_EVENTS {
            anyhow::bail!("openai-codex image generation response exceeded event limit");
        }
        events.push(parse_image_event_value(&value));
    }
    Ok(events)
}

fn parse_image_event_value(value: &Value) -> ParsedImageEvent {
    let response_output = value
        .get("response")
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item.get("type")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        item.get("result")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    ParsedImageEvent {
        event_type: value
            .get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        item_type: value
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        result: value
            .get("item")
            .and_then(|item| item.get("result"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        response_output,
        error_message: value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
            .map(ToString::to_string),
    }
}

fn media_type_for_output_format(output_format: &str) -> &'static str {
    match output_format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn generated_image_from_base64(data: &str, output_format: &str) -> anyhow::Result<GeneratedImage> {
    if data.len() > MAX_IMAGE_BASE64_CHARS {
        anyhow::bail!("openai-codex image generation result exceeded size limit");
    }
    BASE64.decode(data)?;
    Ok(GeneratedImage {
        media_type: media_type_for_output_format(output_format).to_string(),
        data: data.to_string(),
    })
}

fn extract_codex_image_generation_result(
    body: &str,
    model: &str,
    output_format: &str,
) -> anyhow::Result<ImageGenerationResult> {
    let events = parse_image_events(body)?;
    if let Some(failure) = events.iter().find(|event| {
        matches!(
            event.event_type.as_deref(),
            Some("response.failed" | "error")
        )
    }) {
        anyhow::bail!(
            "{}",
            failure
                .error_message
                .as_deref()
                .unwrap_or("openai-codex image generation failed")
        );
    }

    let mut images: Vec<GeneratedImage> = events
        .iter()
        .filter(|event| {
            event.event_type.as_deref() == Some("response.output_item.done")
                && event.item_type.as_deref() == Some("image_generation_call")
        })
        .filter_map(|event| event.result.as_deref())
        .map(|result| generated_image_from_base64(result, output_format))
        .collect::<anyhow::Result<Vec<_>>>()?;

    if images.is_empty() {
        images = events
            .iter()
            .flat_map(|event| &event.response_output)
            .filter(|(item_type, _)| item_type.as_deref() == Some("image_generation_call"))
            .filter_map(|(_, result)| result.as_deref())
            .map(|result| generated_image_from_base64(result, output_format))
            .collect::<anyhow::Result<Vec<_>>>()?;
    }

    if images.is_empty() {
        anyhow::bail!("openai-codex image generation returned no images");
    }

    Ok(ImageGenerationResult {
        model: model.to_string(),
        images,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn builds_codex_image_generation_body() {
        let request = ImageGenerationRequest {
            prompt: " draw a fox ".into(),
            model: "gpt-image-2".into(),
            size: "1536x1024".into(),
            quality: "high".into(),
            output_format: "webp".into(),
            background: "opaque".into(),
        };
        let body = build_codex_image_request_body(&request);
        assert_eq!(body["model"], CODEX_IMAGE_RESPONSES_MODEL);
        assert_eq!(body["input"][0]["content"][0]["text"], "draw a fox");
        assert_eq!(body["tools"][0]["type"], "image_generation");
        assert_eq!(body["tools"][0]["model"], "gpt-image-2");
        assert_eq!(body["tools"][0]["quality"], "high");
        assert_eq!(body["tool_choice"]["type"], "image_generation");
    }

    #[test]
    fn parses_output_item_done_image_result() {
        let png = BASE64.encode(b"png");
        let body = format!(
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"image_generation_call\",\"result\":\"{png}\"}}}}\n\n"
        );
        let result = extract_codex_image_generation_result(&body, "gpt-image-2", "png").unwrap();
        assert_eq!(result.model, "gpt-image-2");
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].media_type, "image/png");
        assert_eq!(result.images[0].data, png);
    }

    #[test]
    fn parses_completed_response_image_result() {
        let jpg = BASE64.encode(b"jpg");
        let body = format!(
            "data: {{\"type\":\"response.completed\",\"response\":{{\"output\":[{{\"type\":\"image_generation_call\",\"result\":\"{jpg}\"}}]}}}}\n\n"
        );
        let result = extract_codex_image_generation_result(&body, "gpt-image-2", "jpeg").unwrap();
        assert_eq!(result.images[0].media_type, "image/jpeg");
        assert_eq!(result.images[0].data, jpg);
    }

    #[test]
    fn surfaces_failed_event_message() {
        let body = "data: {\"type\":\"response.failed\",\"error\":{\"message\":\"blocked\"}}\n\n";
        let err = extract_codex_image_generation_result(body, "gpt-image-2", "png")
            .expect_err("failure should error");
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn enforces_event_limit_before_pushing_extra_event() {
        let event = "data: {\"type\":\"response.created\"}\n";
        let body = event.repeat(MAX_IMAGE_SSE_EVENTS + 1);
        let err = parse_image_events(&body).expect_err("too many events should fail");
        assert!(err.to_string().contains("exceeded event limit"));
    }
}
