//! `send_image` tool — send a local image file to the current conversation's
//! channel (e.g. Telegram).
//!
//! Returns a `{ "screenshot": "data:{mime};base64,..." }` payload that the
//! chat runner picks up and routes through `send_screenshot_to_channels`.

use {
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::{path::Path, sync::Arc},
    tracing::debug,
};

use crate::error::Error;

use crate::{file_io, sandbox::SandboxRouter};

/// Image-sending tool.
#[derive(Default)]
pub struct SendImageTool {
    sandbox_router: Option<Arc<SandboxRouter>>,
}

impl SendImageTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a sandbox router for per-session dynamic sandbox resolution.
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }
}

/// Map a file extension to its image MIME type.
///
/// Delegates to `moltis_media::mime::mime_from_extension` but only accepts
/// image MIME types — returns `None` for documents, audio, etc.
fn mime_from_extension(ext: &str) -> Option<&'static str> {
    moltis_media::mime::mime_from_extension(ext).filter(|m| m.starts_with("image/"))
}

#[async_trait]
impl AgentTool for SendImageTool {
    fn name(&self) -> &str {
        "send_image"
    }

    fn description(&self) -> &str {
        "Send a local image file to the current conversation's channel (e.g. Telegram). \
         Supported formats: PNG, JPEG, GIF, WebP, PPM. Maximum size: 20 MB."
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

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'path' parameter"))?;

        let caption = params.get("caption").and_then(Value::as_str).unwrap_or("");
        let session_key = params
            .get("_session_key")
            .and_then(Value::as_str)
            .unwrap_or("main");

        // Resolve extension and validate MIME.
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| {
                Error::message("file has no extension — supported: png, jpg, jpeg, gif, webp, ppm")
            })?;

        let mime = mime_from_extension(ext).ok_or_else(|| {
            Error::message(format!(
                "unsupported image format '.{ext}' — supported: png, jpg, jpeg, gif, webp, ppm"
            ))
        })?;

        let bytes = file_io::read_file_for_session(
            self.sandbox_router.as_ref(),
            session_key,
            path,
            "send_image",
        )
        .await?;

        debug!(
            path,
            session_key,
            mime,
            size = bytes.len(),
            "send_image: encoded file as data URI"
        );

        let b64 = BASE64.encode(&bytes);
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
    use {
        super::*,
        crate::{
            Result,
            exec::{ExecOpts, ExecResult},
            sandbox::{Sandbox, SandboxConfig, SandboxId},
        },
        std::io::Write,
    };

    struct StubSandbox;

    #[async_trait]
    impl Sandbox for StubSandbox {
        fn backend_name(&self) -> &'static str {
            "stub"
        }

        async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
            Ok(())
        }

        async fn exec(
            &self,
            _id: &SandboxId,
            command: &str,
            _opts: &ExecOpts,
        ) -> Result<ExecResult> {
            if command.contains("/tmp/rex_image.png") {
                return Ok(ExecResult {
                    stdout: BASE64.encode([0x89, b'P', b'N', b'G']),
                    stderr: String::new(),
                    exit_code: 0,
                });
            }

            Ok(ExecResult {
                stdout: String::new(),
                stderr: "path is not a regular file".to_string(),
                exit_code: 2,
            })
        }

        async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn mime_lookup_covers_supported_formats() {
        assert_eq!(mime_from_extension("png"), Some("image/png"));
        assert_eq!(mime_from_extension("PNG"), Some("image/png"));
        assert_eq!(mime_from_extension("jpg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("jpeg"), Some("image/jpeg"));
        assert_eq!(mime_from_extension("gif"), Some("image/gif"));
        assert_eq!(mime_from_extension("webp"), Some("image/webp"));
        assert_eq!(mime_from_extension("ppm"), Some("image/x-portable-pixmap"));
        assert_eq!(mime_from_extension("bmp"), Some("image/bmp"));
        assert_eq!(mime_from_extension("svg"), Some("image/svg+xml"));
        assert_eq!(mime_from_extension("qqqq"), None);
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
            .execute(json!({ "path": "/tmp/image.qqqq" }))
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
    async fn encodes_ppm_as_data_uri() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".ppm").unwrap();
        tmp.write_all(b"P3\n1 1\n255\n255 0 0\n").unwrap();

        let tool = SendImageTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        let screenshot = result["screenshot"].as_str().unwrap_or_default();
        assert!(screenshot.starts_with("data:image/x-portable-pixmap;base64,"));
    }

    #[tokio::test]
    async fn rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");

        // Create a sparse file that reports > 20 MB without writing all bytes.
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(file_io::MAX_FILE_SIZE + 1).unwrap();

        let tool = SendImageTool::new();
        let err = tool
            .execute(json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn reads_sandbox_path_when_session_is_sandboxed() {
        let backend: Arc<dyn Sandbox> = Arc::new(StubSandbox);
        let router = Arc::new(SandboxRouter::with_backend(
            SandboxConfig::default(),
            backend,
        ));

        let tool = SendImageTool::new().with_sandbox_router(router);
        let result = tool
            .execute(json!({
                "_session_key": "session:abc",
                "path": "/tmp/rex_image.png"
            }))
            .await
            .unwrap();

        let screenshot = result["screenshot"].as_str().unwrap_or_default();
        assert!(screenshot.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn sandbox_missing_file_returns_sandbox_error() {
        let backend: Arc<dyn Sandbox> = Arc::new(StubSandbox);
        let router = Arc::new(SandboxRouter::with_backend(
            SandboxConfig::default(),
            backend,
        ));

        let tool = SendImageTool::new().with_sandbox_router(router);
        let err = tool
            .execute(json!({
                "_session_key": "session:abc",
                "path": "/tmp/missing.png"
            }))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("in sandbox"));
    }
}
