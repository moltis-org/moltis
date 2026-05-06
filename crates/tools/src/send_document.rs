//! `send_document` tool — send a local file (PDF, CSV, etc.) to the current
//! conversation's channel (e.g. Telegram).
//!
//! When a [`SessionStore`] is attached the tool saves the raw bytes to the
//! session's media directory and returns a lightweight path reference:
//! ```json
//! { "document_ref": "media/main/a1b2c3d4_report.pdf", "mime_type": "…", … }
//! ```
//! The chat runner reads from disk only when uploading to a channel, avoiding
//! the previous pattern of base64-encoding the entire file into JSON.
//!
//! Falls back to a `data:` URI when no store is available (tests, CLI-only).

use {
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    moltis_agents::tool_registry::AgentTool,
    moltis_media::mime::mime_from_extension,
    moltis_sessions::store::SessionStore,
    serde_json::{Value, json},
    std::{path::Path, sync::Arc},
    tracing::debug,
    uuid::Uuid,
};

use crate::error::Error;

use crate::{file_io, sandbox::SandboxRouter};

/// Extensions that are blocked for security reasons.
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "bat", "cmd", "com", "msi", "scr", "pif", // Windows executables
    "sh", "bash", "zsh", "csh", "ksh", // Shell scripts
    "dll", "so", "dylib", // Shared libraries
    "app", "dmg", "pkg", // macOS
    "deb", "rpm", // Linux packages
    "ps1", "psm1", "psd1", // PowerShell
    "vbs", "vbe", "js", "jse", "wsf", "wsh", // Script engines
    "reg", "inf", // Windows registry / setup
    "py", "pyw", "php", "rb", "pl", "lua", "tcl", // Interpreted scripts
];

/// Document-sending tool.
#[derive(Default)]
pub struct SendDocumentTool {
    sandbox_router: Option<Arc<SandboxRouter>>,
    session_store: Option<Arc<SessionStore>>,
}

impl SendDocumentTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a sandbox router for per-session dynamic sandbox resolution.
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    /// Attach a session store so files are saved to the media directory
    /// instead of being base64-encoded into the JSON result.
    pub fn with_session_store(mut self, store: Arc<SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }
}

#[async_trait]
impl AgentTool for SendDocumentTool {
    fn name(&self) -> &str {
        "send_document"
    }

    fn description(&self) -> &str {
        "Send a local file (PDF, CSV, DOCX, TXT, JSON, ZIP, etc.) to the current \
         conversation's channel. Use send_image for image files. Maximum size: 20 MB."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute file path to the document (e.g. /tmp/report.pdf)"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional text caption to send with the document"
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

        let file_path = Path::new(path);

        // Resolve extension.
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| {
                Error::message(
                    "file has no extension — cannot determine document type. \
                     Supported: pdf, csv, txt, json, docx, xlsx, zip, and more.",
                )
            })?;

        let ext_lower = ext.to_ascii_lowercase();

        // Security: reject dangerous executable extensions.
        if BLOCKED_EXTENSIONS.contains(&ext_lower.as_str()) {
            return Err(Error::message(format!(
                "file extension '.{ext}' is blocked for security reasons"
            ))
            .into());
        }

        // Resolve MIME type.
        let mime = mime_from_extension(&ext_lower).ok_or_else(|| {
            Error::message(format!(
                "unsupported file extension '.{ext}' — supported: pdf, csv, txt, json, \
                 docx, xlsx, pptx, zip, html, xml, rtf, md, and more"
            ))
        })?;

        // Images should use send_image instead.
        if mime.starts_with("image/") {
            return Err(Error::message(
                "use the send_image tool for image files (PNG, JPEG, GIF, WebP, PPM)",
            )
            .into());
        }

        let bytes = file_io::read_file_for_session(
            self.sandbox_router.as_ref(),
            session_key,
            path,
            "send_document",
        )
        .await?;

        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.bin")
            .to_string();

        let size_bytes = bytes.len();

        // When a session store is available, save to disk and return a
        // lightweight path reference. The chat runner reads from disk only
        // when uploading to a channel.
        if let Some(ref store) = self.session_store {
            let uuid_prefix = &Uuid::new_v4().as_simple().to_string()[..8];
            let unique_filename = format!("{uuid_prefix}_{filename}");

            let media_ref = store
                .save_media(session_key, &unique_filename, &bytes)
                .await
                .map_err(|e| {
                    Error::message(format!("failed to save document to media dir: {e}"))
                })?;

            debug!(
                path,
                session_key,
                mime,
                filename,
                size = size_bytes,
                media_ref,
                "send_document: saved file to media dir"
            );

            let mut result = json!({
                "document_ref": media_ref,
                "mime_type": mime,
                "filename": filename,
                "size_bytes": size_bytes,
                "sent": true,
            });

            if !caption.is_empty() {
                result["caption"] = Value::String(caption.to_string());
            }

            return Ok(result);
        }

        // Fallback: encode as data URI when no session store is available.
        debug!(
            path,
            session_key,
            mime,
            filename,
            size = size_bytes,
            "send_document: encoded file as data URI (no session store)"
        );

        let b64 = BASE64.encode(&bytes);
        let data_uri = format!("data:{mime};base64,{b64}");

        let mut result = json!({
            "document": data_uri,
            "filename": filename,
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
            if command.contains("/tmp/report.pdf") {
                return Ok(ExecResult {
                    stdout: BASE64.encode(b"%PDF-1.4"),
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

    #[tokio::test]
    async fn rejects_missing_path_parameter() {
        let tool = SendDocumentTool::new();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'path'"));
    }

    #[tokio::test]
    async fn rejects_blocked_extension_exe() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/malware.exe" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked for security"));
    }

    #[tokio::test]
    async fn rejects_blocked_extension_sh() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/script.sh" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked for security"));
    }

    #[tokio::test]
    async fn rejects_blocked_extension_bat() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/run.bat" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked for security"));
    }

    #[tokio::test]
    async fn rejects_blocked_extension_dll() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/lib.dll" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked for security"));
    }

    #[tokio::test]
    async fn rejects_unsupported_extension() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/data.qqqq" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unsupported file extension"));
    }

    #[tokio::test]
    async fn rejects_image_extension() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/photo.png" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("send_image"));
    }

    #[tokio::test]
    async fn rejects_file_without_extension() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/noext" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("has no extension"));
    }

    #[tokio::test]
    async fn rejects_nonexistent_file() {
        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/does-not-exist-99999.pdf" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot access"));
    }

    #[tokio::test]
    async fn rejects_directory() {
        let dir = tempfile::tempdir().unwrap();
        let pdf_dir = dir.path().parent().unwrap().join("test-dir.pdf");
        std::fs::create_dir_all(&pdf_dir).unwrap();

        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": pdf_dir.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a regular file"));

        std::fs::remove_dir(&pdf_dir).unwrap();
    }

    #[tokio::test]
    async fn rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.pdf");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(file_io::MAX_FILE_SIZE + 1).unwrap();

        let tool = SendDocumentTool::new();
        let err = tool
            .execute(json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("too large"));
    }

    #[tokio::test]
    async fn encodes_valid_pdf() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(b"%PDF-1.4").unwrap();

        let tool = SendDocumentTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        let doc = result["document"].as_str().unwrap();
        assert!(doc.starts_with("data:application/pdf;base64,"));
        assert_eq!(result["sent"], true);
        assert!(result["filename"].as_str().unwrap().ends_with(".pdf"));
        assert!(result.get("caption").is_none());
    }

    #[tokio::test]
    async fn encodes_valid_csv() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(b"a,b,c\n1,2,3\n").unwrap();

        let tool = SendDocumentTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        let doc = result["document"].as_str().unwrap();
        assert!(doc.starts_with("data:text/csv;base64,"));
        assert!(result["filename"].as_str().unwrap().ends_with(".csv"));
    }

    #[tokio::test]
    async fn includes_caption_and_filename() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".json").unwrap();
        tmp.write_all(b"{}").unwrap();

        let tool = SendDocumentTool::new();
        let result = tool
            .execute(json!({
                "path": tmp.path().to_str().unwrap(),
                "caption": "Here is the data"
            }))
            .await
            .unwrap();

        assert_eq!(result["caption"], "Here is the data");
        assert!(result["filename"].as_str().unwrap().ends_with(".json"));
    }

    #[tokio::test]
    async fn sandbox_read_success() {
        let backend: Arc<dyn Sandbox> = Arc::new(StubSandbox);
        let router = Arc::new(SandboxRouter::with_backend(
            SandboxConfig::default(),
            backend,
        ));

        let tool = SendDocumentTool::new().with_sandbox_router(router);
        let result = tool
            .execute(json!({
                "_session_key": "session:abc",
                "path": "/tmp/report.pdf"
            }))
            .await
            .unwrap();

        let doc = result["document"].as_str().unwrap_or_default();
        assert!(doc.starts_with("data:application/pdf;base64,"));
        assert_eq!(result["filename"], "report.pdf");
    }

    #[tokio::test]
    async fn sandbox_missing_file_error() {
        let backend: Arc<dyn Sandbox> = Arc::new(StubSandbox);
        let router = Arc::new(SandboxRouter::with_backend(
            SandboxConfig::default(),
            backend,
        ));

        let tool = SendDocumentTool::new().with_sandbox_router(router);
        let err = tool
            .execute(json!({
                "_session_key": "session:abc",
                "path": "/tmp/missing.pdf"
            }))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("in sandbox"));
    }

    // ---- Session-store (document_ref) path tests ----

    #[tokio::test]
    async fn returns_document_ref_with_session_store() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp_dir.path().to_path_buf()));

        let mut tmp = tempfile::NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(b"%PDF-1.4").unwrap();

        let tool = SendDocumentTool::new().with_session_store(Arc::clone(&store));
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        // Should have document_ref, NOT document.
        assert!(
            result.get("document").is_none(),
            "should not contain data URI"
        );
        let media_ref = result["document_ref"].as_str().unwrap();
        assert!(
            media_ref.starts_with("media/"),
            "ref should start with media/"
        );
        assert!(media_ref.ends_with(".pdf"), "ref should end with .pdf");
        assert_eq!(result["mime_type"], "application/pdf");
        assert_eq!(result["sent"], true);
        assert_eq!(result["size_bytes"], 8); // len of "%PDF-1.4"

        // The file should actually exist on disk.
        let ref_filename = media_ref.rsplit('/').next().unwrap();
        let bytes = store.read_media("main", ref_filename).await.unwrap();
        assert_eq!(bytes, b"%PDF-1.4");
    }

    #[tokio::test]
    async fn document_ref_filename_has_uuid_prefix() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp_dir.path().to_path_buf()));

        let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
        tmp.write_all(b"a,b\n1,2\n").unwrap();

        let tool = SendDocumentTool::new().with_session_store(store);
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        let media_ref = result["document_ref"].as_str().unwrap();
        let ref_filename = media_ref.rsplit('/').next().unwrap();
        // UUID prefix is 8 hex chars + underscore + original filename.
        assert!(
            ref_filename.contains('_'),
            "should have UUID prefix separator"
        );
        let after_prefix = ref_filename
            .split('_')
            .skip(1)
            .collect::<Vec<_>>()
            .join("_");
        assert!(after_prefix.ends_with(".csv"));
    }

    #[tokio::test]
    async fn document_ref_includes_caption() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp_dir.path().to_path_buf()));

        let mut tmp = tempfile::NamedTempFile::with_suffix(".json").unwrap();
        tmp.write_all(b"{}").unwrap();

        let tool = SendDocumentTool::new().with_session_store(store);
        let result = tool
            .execute(json!({
                "path": tmp.path().to_str().unwrap(),
                "caption": "Data export"
            }))
            .await
            .unwrap();

        assert!(result.get("document_ref").is_some());
        assert_eq!(result["caption"], "Data export");
    }

    #[tokio::test]
    async fn unique_filenames_across_calls() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp_dir.path().to_path_buf()));

        let mut tmp = tempfile::NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(b"%PDF-1.4").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tool = SendDocumentTool::new().with_session_store(Arc::clone(&store));
        let r1 = tool.execute(json!({ "path": &path })).await.unwrap();
        let r2 = tool.execute(json!({ "path": &path })).await.unwrap();

        let ref1 = r1["document_ref"].as_str().unwrap();
        let ref2 = r2["document_ref"].as_str().unwrap();
        assert_ne!(ref1, ref2, "each call should produce a unique media ref");
    }

    #[tokio::test]
    async fn fallback_data_uri_without_session_store() {
        // Existing behaviour when no session store is attached.
        let mut tmp = tempfile::NamedTempFile::with_suffix(".pdf").unwrap();
        tmp.write_all(b"%PDF-1.4").unwrap();

        let tool = SendDocumentTool::new();
        let result = tool
            .execute(json!({ "path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert!(result.get("document_ref").is_none());
        let doc = result["document"].as_str().unwrap();
        assert!(doc.starts_with("data:application/pdf;base64,"));
    }
}
