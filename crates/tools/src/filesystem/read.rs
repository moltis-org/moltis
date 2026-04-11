//! `file_read` tool — read file contents with line-range control.
//!
//! Key behavioural difference from the MCP `read_text_file`: this tool
//! defaults to returning at most 150 lines (configurable) instead of the
//! entire file. Every response includes metadata (total_lines, byte_size,
//! truncated flag) so the model can decide whether it needs another call
//! without a separate `file_info` round-trip.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    tracing::instrument,
};

use crate::error::Error;

/// Default maximum lines per call when no config override is set.
const DEFAULT_MAX_LINES: usize = 150;

/// Maximum file size (bytes) that will be read into memory.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Read file contents with line-range control and metadata.
pub struct FileReadTool {
    max_lines: usize,
}

impl FileReadTool {
    #[must_use]
    pub fn new(max_lines: usize) -> Self {
        Self {
            max_lines: max_lines.clamp(1, 10_000),
        }
    }

    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_LINES)
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[async_trait]
impl AgentTool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read a file's contents with line-range control. Returns the requested \
         lines along with metadata (total_lines, byte_size, truncated). By \
         default returns at most 150 lines starting from line 1. Use \
         start_line and end_line to read a specific range, or set max_lines \
         to override the per-call limit."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed, inclusive). Default: 1"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (inclusive). Default: start_line + max_lines - 1"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum lines to return for this call. Overrides the configured default."
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

        let start_line = params
            .get("start_line")
            .and_then(Value::as_i64)
            .unwrap_or(1);
        let end_line = params.get("end_line").and_then(Value::as_i64);
        let max_lines = params
            .get("max_lines")
            .and_then(Value::as_i64)
            .map(|v| v as usize)
            .unwrap_or(self.max_lines);

        // --- Resolve display path ---
        let display_path = std::path::Path::new(path)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(path))
            .to_string_lossy()
            .to_string();

        // --- File metadata ---
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| Error::message(format!("cannot access '{path}': {e}")))?;

        if !meta.is_file() {
            return Err(Error::message(format!("'{path}' is not a regular file")).into());
        }

        let byte_size = meta.len();
        if byte_size > MAX_FILE_BYTES {
            return Err(Error::message(format!(
                "file is too large ({:.1} MB) — maximum is {:.0} MB",
                byte_size as f64 / (1024.0 * 1024.0),
                MAX_FILE_BYTES as f64 / (1024.0 * 1024.0),
            )).into());
        }

        // --- Read content ---
        let raw = tokio::fs::read(path)
            .await
            .map_err(|e| Error::message(format!("failed to read '{path}': {e}")))?;

        let content = String::from_utf8(raw)
            .map_err(|e| Error::message(format!(
                "'{path}' is not valid UTF-8 (first error at byte {})",
                e.utf8_error().valid_up_to()
            )))?;

        // --- Line logic ---
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // An empty file or a file with only a trailing newline has 0 lines.
        if total_lines == 0 {
            return Ok(json!({
                "content": "",
                "metadata": {
                    "path": display_path,
                    "total_lines": 0,
                    "start_line": 1,
                    "end_line": 0,
                    "byte_size": byte_size,
                    "truncated": false,
                }
            }));
        }

        // Clamp start_line to [1, total_lines].
        let start = if start_line < 1 {
            1
        } else if start_line as usize > total_lines {
            // Start is beyond the file — return empty content.
            return Ok(json!({
                "content": "",
                "metadata": {
                    "path": display_path,
                    "total_lines": total_lines,
                    "start_line": start_line as usize,
                    "end_line": total_lines,
                    "byte_size": byte_size,
                    "truncated": false,
                }
            }));
        } else {
            start_line as usize
        };

        // Compute end line.
        let end = match end_line {
            Some(e) if e < start as i64 => start,
            Some(e) => (e as usize).min(total_lines),
            None => (start + max_lines - 1).min(total_lines),
        };

        // Truncation only applies when the range was capped by max_lines,
        // not when the user explicitly requested a specific end_line.
        let truncated = end_line.is_none() && end < total_lines;
        let selected: Vec<&str> = lines[(start - 1)..end].to_vec();
        let result_content = selected.join("\n");

        Ok(json!({
            "content": result_content,
            "metadata": {
                "path": display_path,
                "total_lines": total_lines,
                "start_line": start,
                "end_line": end,
                "byte_size": byte_size,
                "truncated": truncated,
            }
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_file_with(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[tokio::test]
    async fn happy_path_default_read() {
        let lines: Vec<String> = (1..=200).map(|i| format!("line {i}")).collect();
        let f = tmp_file_with(&lines.join("\n"));

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["total_lines"], 200);
        assert_eq!(result["metadata"]["start_line"], 1);
        assert_eq!(result["metadata"]["end_line"], 150);
        assert_eq!(result["metadata"]["truncated"], true);
        assert!(result["content"].as_str().unwrap().starts_with("line 1\nline 2"));
        assert!(result["content"].as_str().unwrap().ends_with("line 150"));
    }

    #[tokio::test]
    async fn line_range() {
        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        let f = tmp_file_with(&lines.join("\n"));

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({
                "path": f.path().to_str().unwrap(),
                "start_line": 5,
                "end_line": 10,
            }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["start_line"], 5);
        assert_eq!(result["metadata"]["end_line"], 10);
        assert_eq!(result["metadata"]["truncated"], false);
        assert_eq!(
            result["content"].as_str().unwrap(),
            "line 5\nline 6\nline 7\nline 8\nline 9\nline 10"
        );
    }

    #[tokio::test]
    async fn start_line_beyond_file() {
        let f = tmp_file_with("line 1\nline 2\nline 3");

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({
                "path": f.path().to_str().unwrap(),
                "start_line": 100,
            }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["total_lines"], 3);
        assert_eq!(result["content"].as_str().unwrap(), "");
    }

    #[tokio::test]
    async fn empty_file() {
        let f = tmp_file_with("");

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["total_lines"], 0);
        assert_eq!(result["content"].as_str().unwrap(), "");
        assert_eq!(result["metadata"]["truncated"], false);
    }

    #[tokio::test]
    async fn non_existent_file() {
        let tool = FileReadTool::with_defaults();
        let err = tool
            .execute(json!({ "path": "/tmp/does-not-exist-file-read-test-98765.bin" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot access"));
    }

    #[tokio::test]
    async fn non_utf8_file_returns_error() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&[0x80, 0xFE, 0xFF]).unwrap();
        f.flush().unwrap();

        let tool = FileReadTool::with_defaults();
        let err = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));
    }

    #[tokio::test]
    async fn max_lines_override() {
        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        let f = tmp_file_with(&lines.join("\n"));

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({
                "path": f.path().to_str().unwrap(),
                "max_lines": 3,
            }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["end_line"], 3);
        assert_eq!(result["metadata"]["truncated"], true);
        assert_eq!(
            result["content"].as_str().unwrap(),
            "line 1\nline 2\nline 3"
        );
    }

    #[tokio::test]
    async fn metadata_accuracy() {
        let content = "hello\nworld\nfoo";
        let f = tmp_file_with(content);

        let tool = FileReadTool::with_defaults();
        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["total_lines"], 3);
        assert_eq!(result["metadata"]["byte_size"], content.len() as u64);
        assert_eq!(result["metadata"]["truncated"], false);
        assert_eq!(result["metadata"]["start_line"], 1);
        assert_eq!(result["metadata"]["end_line"], 3);
    }

    #[tokio::test]
    async fn missing_path_parameter() {
        let tool = FileReadTool::with_defaults();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'path'"));
    }

    #[tokio::test]
    async fn path_is_directory_returns_error() {
        let dir = tempfile::tempdir().unwrap();

        let tool = FileReadTool::with_defaults();
        let err = tool
            .execute(json!({ "path": dir.path().to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a regular file"));
    }

    #[tokio::test]
    async fn custom_max_lines_in_constructor() {
        let tool = FileReadTool::new(5);
        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        let f = tmp_file_with(&lines.join("\n"));

        let result = tool
            .execute(json!({ "path": f.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(result["metadata"]["end_line"], 5);
    }
}
