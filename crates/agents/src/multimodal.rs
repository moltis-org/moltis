//! Typed multimodal content structures for LLM providers.
//!
//! This module provides strongly-typed data structures for multimodal content
//! (text + images) that can be sent to various LLM providers. Using typed structs
//! instead of raw JSON prevents missing fields at compile time and documents the
//! expected API formats.
//!
//! References:
//! - OpenAI: <https://platform.openai.com/docs/guides/vision>
//! - Anthropic: <https://docs.anthropic.com/claude/docs/vision>
//! - Gemini: <https://ai.google.dev/gemini-api/docs/vision>

use serde::{Deserialize, Serialize};

// ============================================================================
// OpenAI Multimodal Types (Chat Completions API)
// ============================================================================

/// Content block for OpenAI multimodal messages.
///
/// OpenAI uses a discriminated union with `type` field:
/// ```json
/// { "type": "text", "text": "..." }
/// { "type": "image_url", "image_url": { "url": "data:image/png;base64,..." } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiContent {
    /// Text content block.
    Text {
        /// The text content.
        text: String,
    },
    /// Image URL content block.
    ImageUrl {
        /// The image URL details.
        image_url: OpenAiImageUrl,
    },
}

/// Image URL details for OpenAI vision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiImageUrl {
    /// Data URI (data:image/png;base64,...) or HTTP URL.
    pub url: String,
    /// Optional detail level: "auto", "low", or "high".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl OpenAiContent {
    /// Create a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create an image URL content block from a data URI.
    pub fn image_data_uri(data_uri: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: OpenAiImageUrl {
                url: data_uri.into(),
                detail: None,
            },
        }
    }

    /// Create an image URL content block with detail level.
    pub fn image_with_detail(url: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: OpenAiImageUrl {
                url: url.into(),
                detail: Some(detail.into()),
            },
        }
    }
}

// ============================================================================
// Anthropic Multimodal Types (Messages API)
// ============================================================================

/// Content block for Anthropic multimodal messages.
///
/// Anthropic uses a discriminated union with `type` field:
/// ```json
/// { "type": "text", "text": "..." }
/// { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "..." } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContent {
    /// Text content block.
    Text {
        /// The text content.
        text: String,
    },
    /// Image content block.
    Image {
        /// The image source.
        source: AnthropicImageSource,
    },
}

/// Image source for Anthropic vision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicImageSource {
    /// Base64-encoded image data.
    Base64 {
        /// MIME type: "image/png", "image/jpeg", "image/gif", "image/webp".
        media_type: String,
        /// Base64-encoded image data (without the data URI prefix).
        data: String,
    },
}

impl AnthropicContent {
    /// Create a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create an image content block from base64 data.
    pub fn image_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Image {
            source: AnthropicImageSource::Base64 {
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }
}

// ============================================================================
// Tool Result Content Types
// ============================================================================

/// Tool result content that can be either plain text or multimodal.
///
/// Most LLM APIs expect tool results to be strings. For vision-capable models,
/// we *could* send multimodal content, but currently most APIs don't support
/// images in tool results (only in user messages).
///
/// This enum documents the possible formats for future use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ToolResultContent {
    /// Plain text result (most common, all providers support this).
    Text(String),
    /// OpenAI-style multimodal content array.
    OpenAiMultimodal(Vec<OpenAiContent>),
    /// Anthropic-style multimodal content array.
    AnthropicMultimodal(Vec<AnthropicContent>),
}

impl ToolResultContent {
    /// Create a plain text tool result.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Create an OpenAI-style multimodal tool result.
    pub fn openai_multimodal(blocks: Vec<OpenAiContent>) -> Self {
        Self::OpenAiMultimodal(blocks)
    }

    /// Create an Anthropic-style multimodal tool result.
    pub fn anthropic_multimodal(blocks: Vec<AnthropicContent>) -> Self {
        Self::AnthropicMultimodal(blocks)
    }
}

// ============================================================================
// Conversion Helpers
// ============================================================================

/// Parse a data URI into its components.
///
/// Returns `Some((media_type, data))` for valid data URIs like:
/// - `data:image/png;base64,iVBORw0KGgo...`
/// - `data:image/jpeg;base64,/9j/4AAQ...`
///
/// Returns `None` for invalid or non-base64 data URIs.
pub fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if media_type.is_empty() || data.is_empty() {
        return None;
    }
    Some((media_type, data))
}

/// Build a data URI from media type and base64 data.
pub fn build_data_uri(media_type: &str, data: &str) -> String {
    format!("data:{media_type};base64,{data}")
}

/// Longest-edge pixel cap for images embedded into model context.
///
/// Vision models downsample internally (Claude effectively caps the long edge
/// near ~1568px / ~1.15 MP), so a 4032×3024 phone photo embedded at full
/// resolution is pure token waste — as base64 a single such image can exceed
/// the entire context budget on its own. Capping the long edge keeps one image
/// to roughly tens of thousands of tokens instead of hundreds of thousands.
pub const MAX_IMAGE_EDGE: u32 = 1568;

/// Skip the decode/resize cost for images whose base64 is already small enough
/// that they cannot meaningfully threaten the context budget (~64K tokens).
const DOWNSCALE_SKIP_BELOW_BASE64_LEN: usize = 256 * 1024;

/// Downscale an oversized base64 image so it doesn't blow the context window.
///
/// When the decoded image's longest edge exceeds [`MAX_IMAGE_EDGE`], resize it
/// down (preserving aspect ratio), re-encode as JPEG, and return the new
/// `(media_type, base64_data)` — the media type is always `image/jpeg`.
///
/// Returns `None` (caller keeps the original) when the image is already within
/// budget, is too small to bother decoding, can't be decoded, or re-encoding
/// would not shrink it. Failures are swallowed deliberately: a botched
/// downscale must never drop the image — better an oversized image than none.
#[must_use]
pub fn downscale_base64_image(b64: &str) -> Option<(String, String)> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    // Cheap guard: small payloads are already well under budget — never pay the
    // decode cost for them (this runs on every history load).
    if b64.len() < DOWNSCALE_SKIP_BELOW_BASE64_LEN {
        return None;
    }
    let bytes = STANDARD.decode(b64).ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    if decoded.width().max(decoded.height()) <= MAX_IMAGE_EDGE {
        return None;
    }
    let resized = decoded.resize(
        MAX_IMAGE_EDGE,
        MAX_IMAGE_EDGE,
        image::imageops::FilterType::Triangle,
    );
    // JPEG has no alpha channel — flatten to RGB before encoding.
    let rgb = image::DynamicImage::ImageRgb8(resized.to_rgb8());
    let mut out = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    let shrunk = out.into_inner();
    if shrunk.len() >= bytes.len() {
        return None; // re-encode didn't actually help; keep the original
    }
    Some(("image/jpeg".to_string(), STANDARD.encode(shrunk)))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // ── OpenAI Content Tests ───────────────────────────────────────────

    #[test]
    fn openai_text_content_serializes_correctly() {
        let content = OpenAiContent::text("Hello, world!");
        let json = serde_json::to_value(&content).unwrap();

        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "Hello, world!");
    }

    #[test]
    fn openai_image_content_serializes_correctly() {
        let content = OpenAiContent::image_data_uri("data:image/png;base64,iVBORw0KGgo");
        let json = serde_json::to_value(&content).unwrap();

        assert_eq!(json["type"], "image_url");
        assert_eq!(
            json["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo"
        );
        assert!(json["image_url"].get("detail").is_none());
    }

    #[test]
    fn openai_image_with_detail_serializes_correctly() {
        let content = OpenAiContent::image_with_detail("data:image/png;base64,AAAA", "high");
        let json = serde_json::to_value(&content).unwrap();

        assert_eq!(json["type"], "image_url");
        assert_eq!(json["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(json["image_url"]["detail"], "high");
    }

    #[test]
    fn openai_content_deserializes_text() {
        let json = serde_json::json!({
            "type": "text",
            "text": "Hello"
        });
        let content: OpenAiContent = serde_json::from_value(json).unwrap();
        assert!(matches!(content, OpenAiContent::Text { text } if text == "Hello"));
    }

    #[test]
    fn openai_content_deserializes_image() {
        let json = serde_json::json!({
            "type": "image_url",
            "image_url": { "url": "data:image/png;base64,ABC" }
        });
        let content: OpenAiContent = serde_json::from_value(json).unwrap();
        assert!(
            matches!(content, OpenAiContent::ImageUrl { image_url } if image_url.url == "data:image/png;base64,ABC")
        );
    }

    #[test]
    fn openai_multimodal_message_format() {
        // Test the expected format for a multimodal user message
        let content = vec![
            OpenAiContent::text("What is in this image?"),
            OpenAiContent::image_data_uri("data:image/png;base64,iVBORw0KGgo"),
        ];
        let message = serde_json::json!({
            "role": "user",
            "content": content,
        });

        assert_eq!(message["role"], "user");
        assert!(message["content"].is_array());
        assert_eq!(message["content"].as_array().unwrap().len(), 2);
        assert_eq!(message["content"][0]["type"], "text");
        assert_eq!(message["content"][1]["type"], "image_url");
    }

    // ── Anthropic Content Tests ────────────────────────────────────────

    #[test]
    fn anthropic_text_content_serializes_correctly() {
        let content = AnthropicContent::text("Hello, world!");
        let json = serde_json::to_value(&content).unwrap();

        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "Hello, world!");
    }

    #[test]
    fn anthropic_image_content_serializes_correctly() {
        let content = AnthropicContent::image_base64("image/png", "iVBORw0KGgo");
        let json = serde_json::to_value(&content).unwrap();

        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/png");
        assert_eq!(json["source"]["data"], "iVBORw0KGgo");
    }

    #[test]
    fn anthropic_content_deserializes_text() {
        let json = serde_json::json!({
            "type": "text",
            "text": "Hello"
        });
        let content: AnthropicContent = serde_json::from_value(json).unwrap();
        assert!(matches!(content, AnthropicContent::Text { text } if text == "Hello"));
    }

    #[test]
    fn anthropic_content_deserializes_image() {
        let json = serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": "ABC123"
            }
        });
        let content: AnthropicContent = serde_json::from_value(json).unwrap();
        match content {
            AnthropicContent::Image { source } => match source {
                AnthropicImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/jpeg");
                    assert_eq!(data, "ABC123");
                },
            },
            _ => panic!("expected image content"),
        }
    }

    #[test]
    fn anthropic_multimodal_message_format() {
        // Test the expected format for a multimodal user message
        let content = vec![
            AnthropicContent::text("What is in this image?"),
            AnthropicContent::image_base64("image/png", "iVBORw0KGgo"),
        ];
        let message = serde_json::json!({
            "role": "user",
            "content": content,
        });

        assert_eq!(message["role"], "user");
        assert!(message["content"].is_array());
        assert_eq!(message["content"].as_array().unwrap().len(), 2);
        assert_eq!(message["content"][0]["type"], "text");
        assert_eq!(message["content"][1]["type"], "image");
        assert_eq!(message["content"][1]["source"]["type"], "base64");
    }

    // ── Tool Result Content Tests ──────────────────────────────────────

    #[test]
    fn tool_result_text_serializes_as_string() {
        let result = ToolResultContent::text("Command executed successfully");
        let json = serde_json::to_value(&result).unwrap();

        assert!(json.is_string());
        assert_eq!(json.as_str().unwrap(), "Command executed successfully");
    }

    #[test]
    fn tool_result_openai_multimodal_serializes_as_array() {
        let result = ToolResultContent::openai_multimodal(vec![
            OpenAiContent::text("Screenshot captured"),
            OpenAiContent::image_data_uri("data:image/png;base64,ABC"),
        ]);
        let json = serde_json::to_value(&result).unwrap();

        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn tool_result_anthropic_multimodal_serializes_as_array() {
        let result = ToolResultContent::anthropic_multimodal(vec![
            AnthropicContent::text("Screenshot captured"),
            AnthropicContent::image_base64("image/png", "ABC"),
        ]);
        let json = serde_json::to_value(&result).unwrap();

        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    // ── Data URI Parsing Tests ─────────────────────────────────────────

    #[test]
    fn parse_data_uri_png() {
        let uri = "data:image/png;base64,iVBORw0KGgo";
        let (media_type, data) = parse_data_uri(uri).unwrap();
        assert_eq!(media_type, "image/png");
        assert_eq!(data, "iVBORw0KGgo");
    }

    #[test]
    fn parse_data_uri_jpeg() {
        let uri = "data:image/jpeg;base64,/9j/4AAQ";
        let (media_type, data) = parse_data_uri(uri).unwrap();
        assert_eq!(media_type, "image/jpeg");
        assert_eq!(data, "/9j/4AAQ");
    }

    #[test]
    fn parse_data_uri_webp() {
        let uri = "data:image/webp;base64,UklGR";
        let (media_type, data) = parse_data_uri(uri).unwrap();
        assert_eq!(media_type, "image/webp");
        assert_eq!(data, "UklGR");
    }

    #[test]
    fn parse_data_uri_invalid() {
        assert!(parse_data_uri("not a data uri").is_none());
        assert!(parse_data_uri("data:").is_none());
        assert!(parse_data_uri("data:image/png").is_none());
        assert!(parse_data_uri("data:;base64,ABC").is_none());
        assert!(parse_data_uri("data:image/png;base64,").is_none());
    }

    #[test]
    fn build_data_uri_roundtrip() {
        let uri = build_data_uri("image/png", "iVBORw0KGgo");
        let (media_type, data) = parse_data_uri(&uri).unwrap();
        assert_eq!(media_type, "image/png");
        assert_eq!(data, "iVBORw0KGgo");
    }

    // ── Provider Format Compatibility Tests ────────────────────────────

    #[test]
    fn openai_format_matches_api_spec() {
        // Verify our types match the OpenAI API specification for vision
        // https://platform.openai.com/docs/guides/vision
        let content = vec![
            OpenAiContent::text("What's in this image?"),
            OpenAiContent::ImageUrl {
                image_url: OpenAiImageUrl {
                    url: "https://example.com/image.png".into(),
                    detail: Some("high".into()),
                },
            },
        ];

        let json = serde_json::to_value(&content).unwrap();
        let arr = json.as_array().unwrap();

        // First element: text
        assert_eq!(arr[0]["type"], "text");
        assert!(arr[0]["text"].is_string());

        // Second element: image_url
        assert_eq!(arr[1]["type"], "image_url");
        assert!(arr[1]["image_url"].is_object());
        assert!(arr[1]["image_url"]["url"].is_string());
        assert_eq!(arr[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn anthropic_format_matches_api_spec() {
        // Verify our types match the Anthropic API specification for vision
        // https://docs.anthropic.com/claude/docs/vision
        let content = vec![
            AnthropicContent::text("What's in this image?"),
            AnthropicContent::Image {
                source: AnthropicImageSource::Base64 {
                    media_type: "image/png".into(),
                    data: "iVBORw0KGgo".into(),
                },
            },
        ];

        let json = serde_json::to_value(&content).unwrap();
        let arr = json.as_array().unwrap();

        // First element: text
        assert_eq!(arr[0]["type"], "text");
        assert!(arr[0]["text"].is_string());

        // Second element: image
        assert_eq!(arr[1]["type"], "image");
        assert!(arr[1]["source"].is_object());
        assert_eq!(arr[1]["source"]["type"], "base64");
        assert_eq!(arr[1]["source"]["media_type"], "image/png");
        assert!(arr[1]["source"]["data"].is_string());
    }

    // ── Edge Case Tests ────────────────────────────────────────────────

    #[test]
    fn empty_multimodal_content_arrays() {
        let empty_openai: Vec<OpenAiContent> = vec![];
        let json = serde_json::to_value(&empty_openai).unwrap();
        assert!(json.is_array());
        assert!(json.as_array().unwrap().is_empty());

        let empty_anthropic: Vec<AnthropicContent> = vec![];
        let json = serde_json::to_value(&empty_anthropic).unwrap();
        assert!(json.is_array());
        assert!(json.as_array().unwrap().is_empty());
    }

    #[test]
    fn text_only_multimodal_content() {
        // Single text block should still serialize as array
        let content = vec![OpenAiContent::text("Just text, no images")];
        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 1);
    }

    #[test]
    fn image_only_multimodal_content() {
        // Single image block (no text) should still serialize as array
        let content = vec![OpenAiContent::image_data_uri("data:image/png;base64,ABC")];
        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["type"], "image_url");
    }

    #[test]
    fn special_characters_in_text() {
        let content = OpenAiContent::text("Text with \"quotes\" and \\ backslash and 日本語");
        let json = serde_json::to_value(&content).unwrap();
        let text = json["text"].as_str().unwrap();
        assert!(text.contains("\"quotes\""));
        assert!(text.contains("\\"));
        assert!(text.contains("日本語"));
    }

    #[test]
    fn large_base64_data() {
        // Test with a realistic base64 payload size (similar to a small screenshot)
        let large_data = "A".repeat(10_000);
        let content = OpenAiContent::image_data_uri(format!("data:image/png;base64,{large_data}"));
        let json = serde_json::to_value(&content).unwrap();
        let url = json["image_url"]["url"].as_str().unwrap();
        assert!(url.len() > 10_000);
        assert!(url.contains(&large_data));
    }

    /// Encode a high-entropy `w`×`h` image as PNG, base64. High entropy keeps PNG
    /// from compressing it away, so the payload reliably clears the skip
    /// threshold — mirroring a real phone photo.
    fn noisy_png_base64(w: u32, h: u32) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let buf = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([
                (x.wrapping_mul(73).wrapping_add(y.wrapping_mul(151)) & 0xFF) as u8,
                (x.wrapping_mul(199).wrapping_add(y.wrapping_mul(37)) & 0xFF) as u8,
                (x.wrapping_add(y).wrapping_mul(91) & 0xFF) as u8,
            ])
        });
        let dynimg = image::DynamicImage::ImageRgb8(buf);
        let mut out = std::io::Cursor::new(Vec::new());
        dynimg.write_to(&mut out, image::ImageFormat::Png).unwrap();
        STANDARD.encode(out.into_inner())
    }

    fn decode_dims(b64: &str) -> (u32, u32) {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let bytes = STANDARD.decode(b64).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn downscale_caps_oversized_image_to_max_edge() {
        let original = noisy_png_base64(2400, 1800);
        assert!(
            original.len() >= DOWNSCALE_SKIP_BELOW_BASE64_LEN,
            "test fixture must exceed the skip threshold"
        );

        let (media_type, shrunk) =
            downscale_base64_image(&original).expect("oversized image should downscale");

        assert_eq!(media_type, "image/jpeg");
        assert!(
            shrunk.len() < original.len(),
            "downscaled payload must be smaller"
        );
        let (w, h) = decode_dims(&shrunk);
        assert!(
            w.max(h) <= MAX_IMAGE_EDGE,
            "long edge {}px must be capped at {MAX_IMAGE_EDGE}",
            w.max(h)
        );
        // Aspect ratio (4:3) preserved within rounding.
        assert_eq!(w, MAX_IMAGE_EDGE);
        assert_eq!(h, 1176);
    }

    #[test]
    fn downscale_skips_small_payloads_without_decoding() {
        // Below the skip threshold → returns None even though it isn't valid image
        // bytes (proves we never attempted a decode).
        let small = "A".repeat(1_000);
        assert!(downscale_base64_image(&small).is_none());
    }

    #[test]
    fn downscale_leaves_already_small_dimension_images_alone() {
        // A large *payload* but within the edge cap: must not be re-encoded.
        let within_cap = noisy_png_base64(1500, 1200);
        if within_cap.len() >= DOWNSCALE_SKIP_BELOW_BASE64_LEN {
            assert!(
                downscale_base64_image(&within_cap).is_none(),
                "image within MAX_IMAGE_EDGE must be left untouched"
            );
        }
    }
}
