//! `send_image` tool — send a local image file to the current conversation's
//! channel (e.g. Telegram).
//!
//! Returns a `{ "screenshot": "data:{mime};base64,..." }` payload that the
//! chat runner picks up and routes through `send_screenshot_to_channels`.

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    base64::Engine as _,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::path::Path,
    tracing::debug,
};

/// 20 MB — Telegram's maximum photo upload size.
const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Image-sending tool.
#[derive(Default)]
pub struct SendImageTool;

impl SendImageTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// Map a file extension to its MIME type.
fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

#[async_trait]
impl AgentTool for SendImageTool {
    fn name(&self) -> &str {
        "send_image"
    }

    fn description(&self) -> &str {
        "Send a local image file to the current conversation's channel (e.g. Telegram). \
         Supported formats: PNG, JPEG, GIF, WebP. Maximum size: 20 MB."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute file path to the image (e.g. /tmp/chart.png)"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional text caption to send with the image"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'path' parameter"))?;

        let caption = params.get("caption").and_then(Value::as_str).unwrap_or("");

        // Resolve extension and validate MIME.
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!("file has no extension — supported: png, jpg, jpeg, gif, webp")
            })?;

        let mime = mime_from_extension(ext).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported image format '.{ext}' — supported: png, jpg, jpeg, gif, webp"
            )
        })?;

        // Check file metadata before reading.
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| anyhow::anyhow!("cannot access '{path}': {e}"))?;

        if !meta.is_file() {
            bail!("'{path}' is not a regular file");
        }

        if meta.len() > MAX_FILE_SIZE {
            bail!(
                "file is too large ({:.1} MB) — maximum is {:.0} MB",
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
            );
        }

        // Read and encode.
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read '{path}': {e}"))?;

        // Post-read size guard against TOCTOU races (file replaced between
        // metadata check and read).
        if bytes.len() as u64 > MAX_FILE_SIZE {
            bail!(
                "file is too large ({:.1} MB) — maximum is {:.0} MB",
                bytes.len() as f64 / (1024.0 * 1024.0),
                MAX_FILE_SIZE as f64 / (1024.0 * 1024.0),
            );
        }

        debug!(
            path,
            mime,
            size = bytes.len(),
            "send_image: encoded file as data URI"
        );

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        drop(bytes);
        let data_uri = format!("data:{mime};base64,{b64}");

        let mut result = json!({
            "screenshot": data_uri,
            "sent": true,
        });

        if !caption.is_empty() {
            result["caption"] = Value::String(caption.to_string());
        }

        Ok(result)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::io::Write};

    #[test]
    fn mime_lookup_covers_supported_formats() {
        assert_eq!(mime_from_extension("png"), Some("image/png"));
        assert_eq!(mime_from_extension("PNG"), Some("image/png"));
        assert_eq!(mime_from_extension("jpg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("jpeg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("gif"), Some("image/gif"));
        assert_eq!(mime_from_extension("webp"), Some("image/webp"));
        assert_eq!(mime_from_extension("bmp"), None);
        assert_eq!(mime_from_extension("svg"), None);
    }

    #[tokio::test]
    async fn rejects_missing_path_parameter() {
        let tool = SendImageTool::new();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'path'"));
    }

    #[tokio::test]
    async fn rejects_unsupported_extension() {
        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/image.bmp" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported image format"));
    }

    #[tokio::test]
    async fn rejects_file_without_extension() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("has no extension"));
    }

    #[tokio::test]
    async fn rejects_nonexistent_file() {
        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/does-not-exist-12345.png" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot access"));
    }

    #[tokio::test]
    async fn rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        // Rename dir to have a .png extension so it passes the MIME check.
        let png_dir = dir.path().parent().unwrap().join("test-dir.png");
        std::fs::create_dir_all(&png_dir).unwrap();

        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": png_dir.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a regular file"));

        std::fs::remove_dir(&png_dir).unwrap();
    }

    #[tokio::test]
    async fn encodes_valid_png_as_data_uri() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        tmp.write_all(&[0x89, b'P', b'N', b'G']).unwrap();

        let tool = SendImageTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        let screenshot = result["screenshot"].as_str().unwrap();
        assert!(screenshot.starts_with("data:image/png;base64,"));
        assert_eq!(result["sent"], true);
        assert!(result.get("caption").is_none());
    }

    #[tokio::test]
    async fn includes_caption_when_provided() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".jpg").unwrap();
        tmp.write_all(&[0xFF, 0xD8, 0xFF]).unwrap();

        let tool = SendImageTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap(), "caption": "Hello" }))
            .await
            .unwrap();

        assert!(
            result["screenshot"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert_eq!(result["caption"], "Hello");
    }

    #[tokio::test]
    async fn rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");

        // Create a sparse file that reports > 20 MB without writing all bytes.
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_SIZE + 1).unwrap();

        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("too large"));
    }
}
