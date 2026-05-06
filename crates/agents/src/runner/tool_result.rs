//! Tool result sanitization: base64/hex blob stripping, truncation, multimodal content.

use std::fmt::Write;

/// Tag that starts a base64 data URI.
const BASE64_TAG: &str = "data:";
/// Marker between MIME type and base64 payload.
const BASE64_MARKER: &str = ";base64,";
/// Minimum length of a blob payload (base64 or hex) to be worth stripping.
const BLOB_MIN_LEN: usize = 200;

fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
}

/// Strip base64 data-URI blobs (e.g. `data:image/png;base64,AAAA...`) and
/// replace them with a short placeholder. Only targets payloads >= 200 chars.
fn strip_base64_blobs(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(BASE64_TAG) {
        result.push_str(&rest[..start]);
        let after_tag = &rest[start + BASE64_TAG.len()..];

        if let Some(marker_pos) = after_tag.find(BASE64_MARKER) {
            let mime_part = &after_tag[..marker_pos];
            let payload_start = marker_pos + BASE64_MARKER.len();
            let payload = &after_tag[payload_start..];
            let payload_len = payload.bytes().take_while(|b| is_base64_byte(*b)).count();

            if payload_len >= BLOB_MIN_LEN {
                let total_uri_len = BASE64_TAG.len() + payload_start + payload_len;
                // Provide a descriptive message based on MIME type
                if mime_part.starts_with("image/") {
                    result.push_str("[screenshot captured and displayed in UI]");
                } else {
                    let _ = write!(result, "[{mime_part} data removed — {total_uri_len} bytes]");
                }
                rest = &rest[start + total_uri_len..];
                continue;
            }
        }

        result.push_str(BASE64_TAG);
        rest = after_tag;
    }
    result.push_str(rest);
    result
}

/// Strip long hex sequences (>= 200 hex chars) that look like binary dumps.
fn strip_hex_blobs(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_ascii_hexdigit() {
            let mut end = start;
            while let Some(&(i, c)) = chars.peek() {
                if c.is_ascii_hexdigit() {
                    end = i + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let run = end - start;
            if run >= BLOB_MIN_LEN {
                let _ = write!(result, "[hex data removed — {run} chars]");
            } else {
                result.push_str(&input[start..end]);
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }
    result
}

/// Sanitize a tool result string before feeding it to the LLM.
///
/// 1. Strips base64 data URIs (>= 200 char payloads).
/// 2. Strips long hex sequences (>= 200 hex chars).
/// 3. Truncates the result to `max_bytes` (at a char boundary), appending a
///    truncation marker.
pub fn sanitize_tool_result(input: &str, max_bytes: usize) -> String {
    let mut result = strip_base64_blobs(input);
    result = strip_hex_blobs(&result);

    if result.len() <= max_bytes {
        return result;
    }

    let original_len = result.len();
    let mut end = max_bytes;
    while end > 0 && !result.is_char_boundary(end) {
        end -= 1;
    }
    result.truncate(end);
    let _ = write!(result, "\n\n[truncated — {original_len} bytes total]");
    result
}

// ── Multimodal tool result helpers ─────────────────────────────────────────

/// Image extracted from a tool result for multimodal handling.
#[derive(Debug)]
pub struct ExtractedImage {
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
    /// Base64-encoded image data
    pub data: String,
}

/// Extract image data URIs from text, returning the images and remaining text.
///
/// Searches for patterns like `data:image/png;base64,AAAA...` and extracts them.
/// Returns the list of images found and the text with images removed.
pub(crate) fn extract_images_from_text_impl(input: &str) -> (Vec<ExtractedImage>, String) {
    let mut images = Vec::new();
    let mut remaining = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(BASE64_TAG) {
        remaining.push_str(&rest[..start]);
        let after_tag = &rest[start + BASE64_TAG.len()..];

        // Check for image MIME type
        if let Some(marker_pos) = after_tag.find(BASE64_MARKER) {
            let mime_part = &after_tag[..marker_pos];

            // Only extract image/* MIME types
            if let Some(image_subtype) = mime_part.strip_prefix("image/") {
                let payload_start = marker_pos + BASE64_MARKER.len();
                let payload = &after_tag[payload_start..];
                let payload_len = payload.bytes().take_while(|b| is_base64_byte(*b)).count();

                if payload_len >= BLOB_MIN_LEN {
                    // Extract the image
                    let media_type = format!("image/{image_subtype}");
                    let data = payload[..payload_len].to_string();
                    images.push(ExtractedImage { media_type, data });

                    // Skip past the full data URI
                    let total_uri_len = BASE64_TAG.len() + payload_start + payload_len;
                    rest = &rest[start + total_uri_len..];
                    continue;
                }
            }
        }

        // Not an extractable image, keep the tag and continue
        remaining.push_str(BASE64_TAG);
        rest = after_tag;
    }
    remaining.push_str(rest);

    (images, remaining)
}

/// Test alias for extract_images_from_text_impl
#[allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
pub(crate) fn extract_images_from_text(input: &str) -> (Vec<ExtractedImage>, String) {
    extract_images_from_text_impl(input)
}

/// Convert a tool result to multimodal content for vision-capable providers.
///
/// For providers with `supports_vision() == true`, this extracts images from
/// the tool result and returns them as OpenAI-style content blocks:
/// ```json
/// [
///   { "type": "text", "text": "..." },
///   { "type": "image_url", "image_url": { "url": "data:image/png;base64,..." } }
/// ]
/// ```
///
/// For non-vision providers, returns a simple string with images stripped.
///
/// Note: Browser screenshots are pre-stripped by the browser tool to avoid
/// the LLM outputting the raw base64 in its response (the UI already displays
/// screenshots via WebSocket events).
pub fn tool_result_to_content(
    result: &str,
    max_bytes: usize,
    supports_vision: bool,
) -> serde_json::Value {
    if !supports_vision {
        // Non-vision provider: strip images and return string
        return serde_json::Value::String(sanitize_tool_result(result, max_bytes));
    }

    // Vision provider: extract images and create multimodal content
    let (images, text) = extract_images_from_text_impl(result);

    if images.is_empty() {
        // No images found, just sanitize and return string
        return serde_json::Value::String(sanitize_tool_result(result, max_bytes));
    }

    // Build multimodal content array
    let mut content_blocks = Vec::new();

    // Sanitize remaining text (strips any remaining hex blobs, truncates if needed)
    let sanitized_text = sanitize_tool_result(&text, max_bytes);
    if !sanitized_text.trim().is_empty() {
        content_blocks.push(serde_json::json!({
            "type": "text",
            "text": sanitized_text
        }));
    }

    // Add image blocks
    for image in images {
        // Reconstruct data URI for OpenAI format
        let data_uri = format!("data:{};base64,{}", image.media_type, image.data);
        content_blocks.push(serde_json::json!({
            "type": "image_url",
            "image_url": { "url": data_uri }
        }));
    }

    serde_json::json!(content_blocks)
}
