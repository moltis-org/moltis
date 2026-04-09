//! Inbound attachment downloading and outbound media handling.
//!
//! Handles downloading file attachments from inbound Teams messages and
//! building outbound media activities with proper attachment encoding.

use {
    base64::{Engine, engine::general_purpose::STANDARD as BASE64},
    secrecy::ExposeSecret,
    tracing::{debug, warn},
};

/// An attachment from an inbound Teams activity.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ActivityAttachment {
    /// Attachment content type (e.g., "image/png", "application/pdf").
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,

    /// URL to download the attachment content.
    #[serde(rename = "contentUrl")]
    pub content_url: Option<String>,

    /// Attachment filename.
    pub name: Option<String>,

    /// Inline content (for Adaptive Card attachments, etc.).
    pub content: Option<serde_json::Value>,
}

/// Downloaded attachment data.
#[derive(Debug)]
pub struct DownloadedAttachment {
    pub media_type: String,
    pub data: Vec<u8>,
    pub filename: Option<String>,
}

/// Maximum attachment download size (50 MB).
const MAX_DOWNLOAD_SIZE: usize = 50 * 1024 * 1024;

/// Download an attachment from its content URL using the bot's access token.
pub async fn download_attachment(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    attachment: &ActivityAttachment,
) -> anyhow::Result<Option<DownloadedAttachment>> {
    let url = match attachment.content_url.as_deref() {
        Some(u) if !u.is_empty() => u,
        _ => return Ok(None),
    };

    // Skip non-downloadable attachments (like Adaptive Cards).
    let content_type = attachment
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    if content_type.starts_with("application/vnd.microsoft.card.") {
        return Ok(None);
    }

    debug!(url, "downloading Teams attachment");

    let resp = http
        .get(url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        warn!(url, %status, "failed to download Teams attachment");
        return Ok(None);
    }

    // Check content length.
    if let Some(len) = resp.content_length()
        && len as usize > MAX_DOWNLOAD_SIZE
    {
        warn!(url, len, "Teams attachment too large, skipping");
        return Ok(None);
    }

    let actual_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(content_type)
        .to_string();

    let data = resp.bytes().await?.to_vec();
    if data.len() > MAX_DOWNLOAD_SIZE {
        warn!(url, size = data.len(), "Teams attachment exceeded max size");
        return Ok(None);
    }

    Ok(Some(DownloadedAttachment {
        media_type: actual_type,
        data,
        filename: attachment.name.clone(),
    }))
}

/// Build a Bot Framework attachment for sending an image inline (base64).
///
/// Works for DM chats. Group chats may need SharePoint/OneDrive instead.
pub fn build_inline_image_attachment(data: &[u8], media_type: &str) -> serde_json::Value {
    let b64 = BASE64.encode(data);
    let data_url = format!("data:{media_type};base64,{b64}");
    serde_json::json!({
        "contentType": media_type,
        "contentUrl": data_url,
    })
}

/// Build a Bot Framework activity with an inline image attachment.
pub fn build_media_activity(
    text: &str,
    data: &[u8],
    media_type: &str,
    reply_to: Option<&str>,
) -> serde_json::Value {
    let attachment = build_inline_image_attachment(data, media_type);
    let mut activity = serde_json::json!({
        "type": "message",
        "attachments": [attachment],
    });
    if !text.is_empty() {
        activity["text"] = serde_json::Value::String(text.to_string());
    }
    if let Some(reply_id) = reply_to {
        activity["replyToId"] = serde_json::Value::String(reply_id.to_string());
    }
    activity
}

/// Check if a content type represents an image that can be sent inline.
pub fn is_inline_image(content_type: &str) -> bool {
    content_type.starts_with("image/")
}

/// Build a URL-based attachment (for external URLs).
pub fn build_url_attachment(
    url: &str,
    content_type: &str,
    name: Option<&str>,
) -> serde_json::Value {
    let mut att = serde_json::json!({
        "contentType": content_type,
        "contentUrl": url,
    });
    if let Some(n) = name {
        att["name"] = serde_json::Value::String(n.to_string());
    }
    att
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn inline_image_attachment_format() {
        let data = b"fake-png-data";
        let att = build_inline_image_attachment(data, "image/png");
        assert_eq!(att["contentType"], "image/png");
        let url = att["contentUrl"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn media_activity_with_text() {
        let activity = build_media_activity("caption", b"data", "image/jpeg", Some("reply-1"));
        assert_eq!(activity["type"], "message");
        assert_eq!(activity["text"], "caption");
        assert_eq!(activity["replyToId"], "reply-1");
        assert!(activity["attachments"].is_array());
    }

    #[test]
    fn media_activity_no_text() {
        let activity = build_media_activity("", b"data", "image/png", None);
        assert!(activity.get("text").is_none());
        assert!(activity.get("replyToId").is_none());
    }

    #[test]
    fn is_inline_image_check() {
        assert!(is_inline_image("image/png"));
        assert!(is_inline_image("image/jpeg"));
        assert!(!is_inline_image("application/pdf"));
        assert!(!is_inline_image("text/plain"));
    }

    #[test]
    fn url_attachment_with_name() {
        let att = build_url_attachment(
            "https://example.com/file.pdf",
            "application/pdf",
            Some("report.pdf"),
        );
        assert_eq!(att["contentType"], "application/pdf");
        assert_eq!(att["contentUrl"], "https://example.com/file.pdf");
        assert_eq!(att["name"], "report.pdf");
    }

    #[test]
    fn deserialize_activity_attachment() {
        let json = serde_json::json!({
            "contentType": "image/png",
            "contentUrl": "https://example.com/image.png",
            "name": "screenshot.png",
        });
        let att: ActivityAttachment = serde_json::from_value(json).unwrap();
        assert_eq!(att.content_type.as_deref(), Some("image/png"));
        assert_eq!(att.name.as_deref(), Some("screenshot.png"));
    }
}
