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

use super::{canonicalize_or_original, check_allowed_dir};

/// File size threshold (bytes) below which we count lines by reading.
/// Above this, line_count is omitted from the response to avoid loading
/// large binaries into memory just to count newlines.
const LINE_COUNT_MAX_BYTES: u64 = 1024 * 1024; // 1 MB

/// Retrieve file or directory metadata without reading content.
#[derive(Default)]
pub struct FileInfoTool {
    allowed_dirs: Vec<String>,
}

impl FileInfoTool {
    #[must_use]
    pub fn new() -> Self {
        Self {
            allowed_dirs: Vec::new(),
        }
    }

    /// Create a tool that restricts info queries to the given directories.
    ///
    /// Paths are canonicalized before checking, so symlinks cannot escape
    /// the boundary.  An empty `allowed_dirs` permits all paths (permissive
    /// mode, same as `new`).
    #[must_use]
    pub fn new_with_allowed_dirs(allowed_dirs: Vec<String>) -> Self {
        Self { allowed_dirs }
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

        // --- Resolve & validate path ---
        let resolved = canonicalize_or_original(std::path::Path::new(path));
        check_allowed_dir(&resolved, &self.allowed_dirs)?;

        let display_path = resolved.to_string_lossy().to_string();

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| Error::message(format!("cannot access '{display_path}': {e}")))?;

        let name = resolved
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
            let extension = resolved
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
                match tokio::fs::read_to_string(&resolved).await {
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
            let entry_count = match tokio::fs::read_dir(&resolved).await {
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
    use {super::*, std::io::Write};

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
        let mut f = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
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
        let f = tempfile::Builder::new().suffix("").tempfile().unwrap();

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

    // --- allowed_dirs containment tests ---

    #[tokio::test]
    async fn allowed_dirs_path_inside_is_allowed() {
        let allowed = tempfile::tempdir().unwrap();
        let file = allowed.path().join("info_test.txt");
        std::fs::write(&file, "data").unwrap();

        let tool =
            FileInfoTool::new_with_allowed_dirs(vec![allowed.path().to_str().unwrap().to_string()]);
        let result = tool
            .execute(json!({ "path": file.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(result["type"], "file");
    }

    #[tokio::test]
    async fn allowed_dirs_directory_inside_is_allowed() {
        let allowed = tempfile::tempdir().unwrap();
        std::fs::write(allowed.path().join("a.txt"), "a").unwrap();

        let tool =
            FileInfoTool::new_with_allowed_dirs(vec![allowed.path().to_str().unwrap().to_string()]);
        let result = tool
            .execute(json!({ "path": allowed.path().to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(result["type"], "directory");
        assert_eq!(result["entry_count"], 1);
    }

    #[tokio::test]
    async fn allowed_dirs_path_outside_is_rejected() {
        let allowed = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("outside.txt");
        std::fs::write(&file, "data").unwrap();

        let tool =
            FileInfoTool::new_with_allowed_dirs(vec![allowed.path().to_str().unwrap().to_string()]);
        let err = tool
            .execute(json!({ "path": file.to_str().unwrap() }))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("outside the allowed directories"), "{msg}");
    }

    #[tokio::test]
    async fn allowed_dirs_empty_allows_everything() {
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("anywhere.txt");
        std::fs::write(&file, "data").unwrap();

        let tool = FileInfoTool::new_with_allowed_dirs(vec![]);
        let result = tool
            .execute(json!({ "path": file.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(result["type"], "file");
    }

    #[tokio::test]
    async fn allowed_dirs_symlink_escape_is_blocked() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let outside_dir = outside.path().join("secret_dir");
        std::fs::create_dir_all(&outside_dir).unwrap();

        let link = allowed.path().join("sneaky");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_dir, &link).unwrap();

        let tool =
            FileInfoTool::new_with_allowed_dirs(vec![allowed.path().to_str().unwrap().to_string()]);
        let err = tool
            .execute(json!({ "path": link.to_str().unwrap() }))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("outside the allowed directories"), "{msg}");
    }

    #[tokio::test]
    async fn allowed_dirs_default_constructor_is_permissive() {
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("unrestricted.txt");
        std::fs::write(&file, "ok").unwrap();

        // Plain new() has no allowed_dirs — should be permissive.
        let tool = FileInfoTool::new();
        let result = tool
            .execute(json!({ "path": file.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(result["type"], "file");
    }
}
