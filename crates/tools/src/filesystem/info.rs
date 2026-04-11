//! `file_info` tool — retrieve file/directory metadata without reading content.
//!
//! Provides size, line count, type, extension, modification time, and
//! permissions. Useful for deciding whether to read a file at all.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    time::format_description::well_known::Rfc3339,
    tracing::instrument,
};

use crate::error::Error;

/// File size threshold (bytes) below which we count lines by reading.
/// Above this, line_count is omitted from the response to avoid loading
/// large binaries into memory just to count newlines.
const LINE_COUNT_MAX_BYTES: u64 = 1024 * 1024; // 1 MB

/// Retrieve file or directory metadata without reading content.
#[derive(Default)]
pub struct FileInfoTool;

impl FileInfoTool {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentTool for FileInfoTool {
    fn name(&self) -> &str {
        "file_info"
    }

    fn description(&self) -> &str {
        "Retrieve detailed metadata about a file or directory. Returns size, \
         type, line count (for files under 1 MB), extension, last modified \
         time, and read/write permissions. Does not return file content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory"
                }
            }
        })
    }

    #[instrument(skip(self, params))]
    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'path' parameter"))?;

        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| Error::message(format!("cannot access '{path}': {e}")))?;

        let display_path = std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(path))
            .to_string_lossy()
            .to_string();

        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| display_path.clone());

        let size_bytes = meta.len();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| {
                let dt = time::OffsetDateTime::from(t);
                dt.format(&Rfc3339).ok()
            })
            .unwrap_or_default();

        let readable = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o444 != 0
        };
        let writable = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o222 != 0
        };

        if meta.is_file() {
            let extension = std::path::Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut result = json!({
                "path": display_path,
                "name": name,
                "type": "file",
                "size_bytes": size_bytes,
                "extension": extension,
                "last_modified": modified,
                "readable": readable,
                "writable": writable,
            });

            // Count lines only for files under the threshold.
            if size_bytes <= LINE_COUNT_MAX_BYTES {
                match tokio::fs::read_to_string(path).await {
                    Ok(content) => {
                        let line_count = content.lines().count();
                        result["line_count"] = json!(line_count);
                    },
                    Err(_) => {
                        // Non-UTF-8 file — skip line count.
                    },
                }
            }

            Ok(result)
        } else if meta.is_dir() {
            let entry_count = match tokio::fs::read_dir(path).await {
                Ok(mut rd) => {
                    let mut count = 0usize;
                    while let Ok(Some(_)) = rd.next_entry().await {
                        count = count.saturating_add(1);
                    }
                    count
                },
                Err(_) => 0,
            };

            Ok(json!({
                "path": display_path,
                "name": name,
                "type": "directory",
                "size_bytes": size_bytes,
                "entry_count": entry_count,
                "last_modified": modified,
                "readable": readable,
                "writable": writable,
            }))
        } else {
            Ok(json!({
                "path": display_path,
                "name": name,
                "type": "special",
                "size_bytes": size_bytes,
                "last_modified": modified,
                "readable": readable,
                "writable": writable,
            }))
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn file_info_basic() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "hello\nworld\nfoo").unwrap();
        f.flush().unwrap();

        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["type"], "file");
        assert_eq!(result["line_count"], 3);
        assert!(result["readable"].as_bool().unwrap());
        assert!(result["writable"].as_bool().unwrap());
        assert!(result["size_bytes"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn file_info_extension() {
        let mut f = tempfile::Builder::new()
            .suffix(".rs")
            .tempfile()
            .unwrap();
        write!(f, "fn main() {{}}").unwrap();
        f.flush().unwrap();

        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["extension"], "rs");
    }

    #[tokio::test]
    async fn file_info_no_extension() {
        let f = tempfile::Builder::new()
            .suffix("")
            .tempfile()
            .unwrap();

        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["extension"], "");
    }

    #[tokio::test]
    async fn directory_info() {
        let dir = tempfile::tempdir().unwrap();
        // Create a few children.
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.rs"), "b").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": dir.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["type"], "directory");
        assert_eq!(result["entry_count"], 3);
        // Directories should not have line_count or extension.
        assert!(result.get("line_count").is_none());
        assert!(result.get("extension").is_none());
    }

    #[tokio::test]
    async fn non_existent_path() {
        let tool = FileInfoTool::new();
        let err = tool
            .execute(json!({ "path": "/tmp/does-not-exist-file-info-test-98765" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot access"));
    }

    #[tokio::test]
    async fn missing_path_parameter() {
        let tool = FileInfoTool::new();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'path'"));
    }

    #[tokio::test]
    async fn binary_file_skips_line_count() {
        // Create a binary file >1MB so line_count is skipped.
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("large.bin");
        let f = std::fs::File::create(&bin_path).unwrap();
        f.set_len(LINE_COUNT_MAX_BYTES + 1).unwrap();

        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": bin_path.to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["type"], "file");
        assert!(result.get("line_count").is_none());
    }
}
