//! `Read` tool — typed, line-numbered file reads.
//!
//! Matches Claude Code's `Read` tool schema: `file_path`, optional `offset`
//! and `limit`. Returns a structured payload with `content` (cat -n style),
//! `total_lines`, and `truncated` flags so the LLM can tell whether it has
//! the full file.
//!
//! Format-specific dispatchers live in submodules so new formats (e.g.
//! `.ipynb`, `.docx`) can be added without growing this file.

pub(crate) mod image;
pub(crate) mod pdf;

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Map, Value, json},
    std::{path::Path, sync::Arc},
    tokio::fs,
    tracing::instrument,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, labels, tools as tools_metrics};

use crate::{
    Result,
    error::Error,
    fs::{
        sandbox_bridge::SandboxReadResult,
        shared::{
            BinaryPolicy, DEFAULT_MAX_READ_BYTES, DEFAULT_READ_LINE_LIMIT, FsErrorKind,
            FsPathPolicy, FsState, MAX_READ_OUTPUT_BYTES, READ_LOOP_THRESHOLD,
            compute_adaptive_read_cap, enforce_path_policy, format_numbered_lines_with_cap,
            fs_error_payload, io_error_to_typed_payload, is_binary_extension, looks_binary,
            require_absolute, session_key_from,
        },
    },
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

const MAX_AUTO_PAGED_READS: usize = 8;

struct TextReadPage {
    content: String,
    total_lines: usize,
    start_line: usize,
    rendered_lines: usize,
    truncated: bool,
}

fn next_offset_for_text_read(
    start_line: usize,
    rendered_lines: usize,
    total_lines: usize,
    truncated: bool,
) -> Option<usize> {
    if !truncated || rendered_lines == 0 {
        return None;
    }

    let next_offset = start_line.saturating_add(rendered_lines);
    (next_offset <= total_lines).then_some(next_offset)
}

fn text_payload(
    file_path: &str,
    content: String,
    total_lines: usize,
    start_line: usize,
    rendered_lines: usize,
    truncated: bool,
) -> Value {
    let mut payload = Map::from_iter([
        ("kind".to_string(), json!("text")),
        ("file_path".to_string(), json!(file_path)),
        ("content".to_string(), json!(content)),
        ("total_lines".to_string(), json!(total_lines)),
        ("start_line".to_string(), json!(start_line)),
        ("rendered_lines".to_string(), json!(rendered_lines)),
        ("truncated".to_string(), json!(truncated)),
    ]);

    if let Some(next_offset) =
        next_offset_for_text_read(start_line, rendered_lines, total_lines, truncated)
    {
        payload.insert("next_offset".to_string(), json!(next_offset));
        payload.insert(
            "continuation_hint".to_string(),
            json!(format!(
                "File output was truncated. Re-run Read with offset={next_offset} to continue."
            )),
        );
    }

    Value::Object(payload)
}

fn maybe_add_loop_warning(
    fs_state: Option<&FsState>,
    payload: &mut Value,
    file_path: &str,
    offset: usize,
    limit: usize,
    session_key: &str,
) {
    let Some(state) = fs_state else {
        return;
    };
    let guard = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let n = guard.consecutive_reads(session_key);
    if n >= READ_LOOP_THRESHOLD
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(
            "loop_warning".into(),
            json!(format!(
                "This exact read (file_path={file_path}, offset={offset}, limit={limit}) \
                 has been repeated {n} times with no intervening edit. The \
                 file hasn't changed — stop re-reading it and make progress on the task."
            )),
        );
    }
}

/// Native `Read` tool implementation.
#[derive(Default)]
pub struct ReadTool {
    fs_state: Option<FsState>,
    path_policy: Option<FsPathPolicy>,
    binary_policy: BinaryPolicy,
    sandbox_router: Option<Arc<SandboxRouter>>,
    /// Override for the file-size gate. `None` → `DEFAULT_MAX_READ_BYTES`.
    max_read_bytes: Option<u64>,
    /// Optional context window in tokens. When set, Read's byte cap
    /// scales adaptively with the model's working set
    /// (`ctx * 4 chars * 0.2`, clamped to `[50 KB, 512 KB]`).
    context_window_tokens: Option<u64>,
}

impl ReadTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a shared [`FsState`] for per-session read tracking and
    /// re-read loop detection.
    #[must_use]
    pub fn with_fs_state(mut self, state: FsState) -> Self {
        self.fs_state = Some(state);
        self
    }

    /// Attach an allow/deny path policy.
    #[must_use]
    pub fn with_path_policy(mut self, policy: FsPathPolicy) -> Self {
        self.path_policy = Some(policy);
        self
    }

    /// Override the binary-file handling policy. Default is
    /// [`BinaryPolicy::Reject`] which returns a typed marker without
    /// content.
    #[must_use]
    pub fn with_binary_policy(mut self, policy: BinaryPolicy) -> Self {
        self.binary_policy = policy;
        self
    }

    /// Attach a shared [`SandboxRouter`]. When the router marks a
    /// session as sandboxed, Read dispatches through the bridge
    /// instead of touching the host filesystem.
    #[must_use]
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    /// Override the maximum file size `Read` will accept. Larger files
    /// return a typed `too_large` payload. Wired from
    /// `[tools.fs].max_read_bytes`. Default: `DEFAULT_MAX_READ_BYTES`.
    #[must_use]
    pub fn with_max_read_bytes(mut self, max: u64) -> Self {
        self.max_read_bytes = Some(max);
        self
    }

    /// Configure the model context window in tokens. Enables the
    /// adaptive byte cap so per-Read payloads scale with the model's
    /// working set instead of using a fixed ceiling.
    #[must_use]
    pub fn with_context_window_tokens(mut self, tokens: u64) -> Self {
        self.context_window_tokens = Some(tokens);
        self
    }

    /// Effective file-size cap: config override or default.
    fn effective_max_read_bytes(&self) -> u64 {
        self.max_read_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES)
    }

    fn effective_render_byte_cap(&self) -> usize {
        self.context_window_tokens
            .map(compute_adaptive_read_cap)
            .unwrap_or(MAX_READ_OUTPUT_BYTES)
    }

    #[instrument(skip(self), fields(file_path = %file_path))]
    async fn read_impl(
        &self,
        file_path: &str,
        offset: usize,
        limit: usize,
        session_key: &str,
    ) -> Result<Value> {
        require_absolute(file_path, "file_path")?;

        // Sandbox dispatch: if the session is sandboxed, round-trip through
        // the bridge and render the resulting bytes with the same logic as
        // the host path. Path-policy and binary detection still run on
        // host-side types, so both paths look identical to the LLM.
        if let Some(ref router) = self.sandbox_router
            && router.is_sandboxed(session_key).await
        {
            if let Some(ref policy) = self.path_policy
                && let Some(payload) = enforce_path_policy(policy, Path::new(file_path))
            {
                return Ok(payload);
            }
            let max = self.effective_max_read_bytes();
            let sandbox_fs = sandbox_file_system_for_session(router, session_key).await?;
            let result = sandbox_fs.read_file(file_path, max).await?;
            match result {
                SandboxReadResult::Ok(bytes) => {
                    return Ok(self.render_bytes_to_payload(
                        file_path,
                        offset,
                        limit,
                        &bytes,
                        session_key,
                        true,
                        None, // sandbox mtime unavailable
                    ));
                },
                other => {
                    return Ok(other
                        .into_typed_payload(file_path, max)
                        .unwrap_or(json!({})));
                },
            }
        }

        // Stat first so we can surface not_found / permission_denied as
        // typed Ok payloads rather than Err strings. The chat loop strips
        // Err detail via err.to_string(); typed payloads survive as JSON.
        let meta = match fs::metadata(file_path).await {
            Ok(m) => m,
            Err(e) => {
                if let Some(payload) = io_error_to_typed_payload(&e, file_path) {
                    return Ok(payload);
                }
                return Err(Error::message(format!("cannot stat '{file_path}': {e}")));
            },
        };

        if !meta.is_file() {
            return Ok(fs_error_payload(
                FsErrorKind::NotRegularFile,
                file_path,
                "path is not a regular file",
                None,
            ));
        }

        // Path policy check: canonicalize first so allowlist globs evaluate
        // against the resolved path, not whatever the LLM supplied.
        if let Some(ref policy) = self.path_policy {
            let canonical = fs::canonicalize(file_path)
                .await
                .unwrap_or_else(|_| std::path::PathBuf::from(file_path));
            if let Some(payload) = enforce_path_policy(policy, &canonical) {
                return Ok(payload);
            }
        }

        let size = meta.len();
        let max_read = self.effective_max_read_bytes();
        if size > max_read {
            return Ok(json!({
                "kind": FsErrorKind::TooLarge.as_str(),
                "file_path": file_path,
                "error": format!(
                    "file is too large ({:.1} MB) — maximum is {:.0} MB",
                    size as f64 / (1024.0 * 1024.0),
                    max_read as f64 / (1024.0 * 1024.0),
                ),
                "bytes": size,
                "max_bytes": max_read,
            }));
        }

        let bytes = match fs::read(file_path).await {
            Ok(b) => b,
            Err(e) => {
                if let Some(payload) = io_error_to_typed_payload(&e, file_path) {
                    return Ok(payload);
                }
                return Err(Error::message(format!("failed to read '{file_path}': {e}")));
            },
        };

        let mtime = meta.modified().ok();
        Ok(self.render_bytes_to_payload(
            file_path,
            offset,
            limit,
            &bytes,
            session_key,
            false,
            mtime,
        ))
    }

    fn parse_text_page(payload: &Value) -> Option<TextReadPage> {
        if payload.get("kind").and_then(Value::as_str) != Some("text") {
            return None;
        }
        Some(TextReadPage {
            content: payload.get("content")?.as_str()?.to_string(),
            total_lines: payload.get("total_lines")?.as_u64()? as usize,
            start_line: payload.get("start_line")?.as_u64()? as usize,
            rendered_lines: payload.get("rendered_lines")?.as_u64()? as usize,
            truncated: payload.get("truncated")?.as_bool()?,
        })
    }

    async fn read_with_auto_paging(
        &self,
        file_path: &str,
        offset: usize,
        page_limit: usize,
        session_key: &str,
    ) -> Result<Value> {
        let max_bytes = self.effective_render_byte_cap();
        let mut next_offset = offset;
        let mut aggregated_content = String::new();
        let mut aggregated_bytes = 0usize;
        let mut rendered_lines_total = 0usize;
        let mut total_lines = 0usize;
        let mut start_line = offset;
        let mut truncated = false;

        for page_index in 0..MAX_AUTO_PAGED_READS {
            let payload = self
                .read_impl(file_path, next_offset, page_limit, session_key)
                .await?;
            let Some(page) = Self::parse_text_page(&payload) else {
                return Ok(payload);
            };

            if page_index == 0 {
                total_lines = page.total_lines;
                start_line = page.start_line;
            }

            let page_bytes = page.content.len();
            if !aggregated_content.is_empty()
                && aggregated_bytes.saturating_add(page_bytes) > max_bytes
            {
                truncated = true;
                break;
            }

            aggregated_bytes = aggregated_bytes.saturating_add(page_bytes);
            aggregated_content.push_str(&page.content);
            rendered_lines_total = rendered_lines_total.saturating_add(page.rendered_lines);

            if !page.truncated {
                truncated = false;
                break;
            }

            let candidate_next_offset = page.start_line.saturating_add(page.rendered_lines);
            if page.rendered_lines == 0
                || candidate_next_offset > page.total_lines
                || page_index == MAX_AUTO_PAGED_READS - 1
            {
                truncated = true;
                break;
            }

            next_offset = candidate_next_offset;

            if aggregated_bytes >= max_bytes {
                truncated = true;
                break;
            }
        }

        let mut payload = text_payload(
            file_path,
            aggregated_content,
            total_lines,
            start_line,
            rendered_lines_total,
            truncated,
        );
        maybe_add_loop_warning(
            self.fs_state.as_ref(),
            &mut payload,
            file_path,
            offset,
            page_limit,
            session_key,
        );
        Ok(payload)
    }

    /// Render raw file bytes into the typed Read payload.
    ///
    /// Shared by the host and sandbox branches so the LLM-facing shape
    /// is identical across routing modes. `from_sandbox` controls the
    /// loop-tracker key (canonicalize is a no-op against sandbox paths
    /// since the LLM-supplied string is already absolute and untouched).
    fn render_bytes_to_payload(
        &self,
        file_path: &str,
        offset: usize,
        limit: usize,
        bytes: &[u8],
        session_key: &str,
        from_sandbox: bool,
        mtime: Option<std::time::SystemTime>,
    ) -> Value {
        // Record the read in the tracker BEFORE any early return (including
        // binary). An operator with must_read_before_write + binary_policy=base64
        // needs binary Reads to count as "this session has read the file."
        if let Some(ref state) = self.fs_state {
            let tracker_path = if from_sandbox {
                std::path::PathBuf::from(file_path)
            } else {
                std::fs::canonicalize(file_path)
                    .unwrap_or_else(|_| std::path::PathBuf::from(file_path))
            };
            let mut guard = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _consecutive = guard.record_read(session_key, tracker_path, offset, limit, mtime);
        }

        if is_binary_extension(file_path) || looks_binary(bytes) {
            #[cfg(feature = "metrics")]
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "Read".to_string(),
                labels::SUCCESS => "binary".to_string()
            )
            .increment(1);
            return match self.binary_policy {
                BinaryPolicy::Reject => json!({
                    "kind": "binary",
                    "file_path": file_path,
                    "bytes": bytes.len(),
                    "message": "file appears to be binary; content not returned (binary_policy = reject)",
                }),
                BinaryPolicy::Base64 => {
                    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
                    json!({
                        "kind": "binary",
                        "file_path": file_path,
                        "bytes": bytes.len(),
                        "base64": BASE64.encode(bytes),
                    })
                },
            };
        }

        // Lossy decode so we never fail on invalid UTF-8 — surface
        // whatever the LLM can see and let it decide.
        let text = String::from_utf8(bytes.to_vec()).unwrap_or_else(|e| {
            let bytes = e.into_bytes();
            String::from_utf8_lossy(&bytes).into_owned()
        });

        // Adaptive byte cap: when the operator has told us the model's
        // context window, scale per-call output so Read can't consume
        // more than ~20% of the model's working set. Otherwise fall
        // back to the fixed default.
        let byte_cap = self.effective_render_byte_cap();
        let rendered = format_numbered_lines_with_cap(&text, offset, limit, byte_cap);

        #[cfg(feature = "metrics")]
        counter!(
            tools_metrics::EXECUTIONS_TOTAL,
            labels::TOOL => "Read".to_string(),
            labels::SUCCESS => "true".to_string()
        )
        .increment(1);

        let mut payload = text_payload(
            file_path,
            rendered.text,
            rendered.total_lines,
            rendered.start_line,
            rendered.rendered_lines,
            rendered.truncated,
        );

        // Loop warning: the read was already recorded at the top of
        // this method. Check the consecutive count (which was bumped
        // during that first record_read) and surface a warning if the
        // same (path, offset, limit) has been repeated too many times.
        maybe_add_loop_warning(
            self.fs_state.as_ref(),
            &mut payload,
            file_path,
            offset,
            limit,
            session_key,
        );

        payload
    }
}

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Read a file from the local filesystem with line-numbered output. \
         Supports `offset` (1-indexed line to start at) and `limit` (max lines \
         to return) for paginating large files. Returns structured JSON with \
         the file's content, total line count, and truncation flag. Binary \
         files return a typed marker instead of garbage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-indexed line number to start reading from."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum number of lines to return (default 2000)."
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files (e.g. '1-5', '3', '10-20'). Only applicable to .pdf files. Maximum 20 pages per request."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let file_path = params
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'file_path' parameter"))?;
        let explicit_offset = params.get("offset").is_some();
        let explicit_limit = params.get("limit").is_some();
        let offset = params
            .get("offset")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(1)
            .max(1);
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_READ_LINE_LIMIT)
            .max(1);
        let pages = params
            .get("pages")
            .and_then(Value::as_str)
            .map(str::to_string);
        let session_key = session_key_from(&params).to_string();

        let lower = file_path.to_ascii_lowercase();
        let is_special = lower.ends_with(".pdf") || is_image_extension(&lower);

        // PDF and image dispatches bypass read_impl, so the three
        // gates that live inside read_impl must run here first:
        //  1. Path policy
        //  2. Sandbox routing (PDF/image extraction is host-only for
        //     now — return a typed payload if sandboxed)
        //  3. FsState read recording (so must-read-before-write works)
        if is_special {
            if let Some(ref policy) = self.path_policy {
                let p = Path::new(file_path);
                let canonical = fs::canonicalize(p)
                    .await
                    .unwrap_or_else(|_| p.to_path_buf());
                if let Some(payload) = enforce_path_policy(policy, &canonical) {
                    return Ok(payload);
                }
            }
            if let Some(ref router) = self.sandbox_router
                && router.is_sandboxed(&session_key).await
            {
                // PDF extraction and image resize run host-side; we
                // can't invoke them inside a container. Return a clear
                // typed payload so the LLM knows to fall back to the
                // raw binary Read path.
                return Ok(json!({
                    "kind": "unsupported_in_sandbox",
                    "file_path": file_path,
                    "error": "PDF and image processing is not available for sandboxed sessions. \
                              Use Read without a .pdf/.png/.jpg extension or access the file \
                              from a non-sandboxed session.",
                }));
            }
            // Record in FsState so must-read-before-write passes for
            // subsequent writes to this path.
            if let Some(ref state) = self.fs_state {
                let canonical = fs::canonicalize(file_path)
                    .await
                    .unwrap_or_else(|_| std::path::PathBuf::from(file_path));
                let mtime = fs::metadata(file_path)
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok());
                let mut guard = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.record_read(&session_key, canonical, offset, limit, mtime);
            }
        }

        // PDF dispatch.
        if lower.ends_with(".pdf") {
            return match read_pdf(file_path, pages.as_deref()).await {
                Ok(value) => Ok(value),
                Err(e) => {
                    #[cfg(feature = "metrics")]
                    counter!(
                        tools_metrics::EXECUTION_ERRORS_TOTAL,
                        labels::TOOL => "Read".to_string()
                    )
                    .increment(1);
                    Err(e.into())
                },
            };
        }

        // Image dispatch.
        if is_image_extension(&lower) {
            return match image::read_image(file_path).await {
                Ok(value) => Ok(value),
                Err(e) => {
                    #[cfg(feature = "metrics")]
                    counter!(
                        tools_metrics::EXECUTION_ERRORS_TOTAL,
                        labels::TOOL => "Read".to_string()
                    )
                    .increment(1);
                    Err(e.into())
                },
            };
        }

        let should_auto_page = !explicit_limit && !explicit_offset;
        let result = if should_auto_page {
            self.read_with_auto_paging(file_path, offset, limit, &session_key)
                .await
        } else {
            self.read_impl(file_path, offset, limit, &session_key).await
        };

        match result {
            Ok(value) => Ok(value),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "Read".to_string()
                )
                .increment(1);
                Err(e.into())
            },
        }
    }
}

use {image::is_image_extension, pdf::read_pdf};

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::io::Write};

    #[tokio::test]
    async fn read_small_text_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"line one\nline two\nline three\n").unwrap();

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "text");
        assert_eq!(value["total_lines"], 3);
        assert_eq!(value["rendered_lines"], 3);
        assert_eq!(value["truncated"], false);
        assert!(value["content"].as_str().unwrap().contains("→line one"));
    }

    #[tokio::test]
    async fn read_paginated() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(tmp, "line {i}").unwrap();
        }

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({
                "file_path": tmp.path().to_str().unwrap(),
                "offset": 3,
                "limit": 2,
            }))
            .await
            .unwrap();

        assert_eq!(value["total_lines"], 10);
        assert_eq!(value["rendered_lines"], 2);
        assert_eq!(value["start_line"], 3);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["next_offset"], 5);
        assert_eq!(
            value["continuation_hint"],
            "File output was truncated. Re-run Read with offset=5 to continue."
        );
        let content = value["content"].as_str().unwrap();
        assert!(content.contains("line 3"));
        assert!(content.contains("line 4"));
        assert!(!content.contains("line 5"));
    }

    #[tokio::test]
    async fn read_auto_pages_when_limit_is_omitted() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=3_000 {
            writeln!(tmp, "line {i}").unwrap();
        }

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "text");
        assert_eq!(value["total_lines"], 3_000);
        assert_eq!(value["start_line"], 1);
        assert_eq!(value["rendered_lines"], 3_000);
        assert_eq!(value["truncated"], false);
        let content = value["content"].as_str().unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2001"));
        assert!(content.contains("line 3000"));
    }

    #[tokio::test]
    async fn read_explicit_limit_disables_auto_paging() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=3_000 {
            writeln!(tmp, "line {i}").unwrap();
        }

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({
                "file_path": tmp.path().to_str().unwrap(),
                "limit": 100,
            }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "text");
        assert_eq!(value["total_lines"], 3_000);
        assert_eq!(value["rendered_lines"], 100);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["next_offset"], 101);
        assert_eq!(
            value["continuation_hint"],
            "File output was truncated. Re-run Read with offset=101 to continue."
        );
        let content = value["content"].as_str().unwrap();
        assert!(content.contains("line 100"));
        assert!(!content.contains("line 101"));
    }

    #[tokio::test]
    async fn read_auto_paged_truncation_exposes_next_offset() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=2_000 {
            writeln!(tmp, "{i:04}: {}", "x".repeat(120)).unwrap();
        }

        let tool = ReadTool::new().with_context_window_tokens(1);
        let value = tool
            .execute(json!({ "file_path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "text");
        assert_eq!(value["truncated"], true);
        let next_offset = value["next_offset"]
            .as_u64()
            .expect("truncated auto-paged read should advertise next_offset");
        assert!(next_offset > 1);
        assert_eq!(
            value["continuation_hint"],
            format!(
                "File output was truncated. Re-run Read with offset={next_offset} to continue."
            )
        );
    }

    #[tokio::test]
    async fn read_binary_base64_policy_returns_encoded_bytes() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&[0u8, 1, 2, 3, 0, 4, 5]).unwrap();

        let tool = ReadTool::new().with_binary_policy(BinaryPolicy::Base64);
        let value = tool
            .execute(json!({ "file_path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "binary");
        assert_eq!(value["bytes"], 7);
        let encoded = value["base64"].as_str().unwrap();
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        let decoded = BASE64.decode(encoded).unwrap();
        assert_eq!(decoded, [0u8, 1, 2, 3, 0, 4, 5]);
    }

    #[tokio::test]
    async fn read_binary_returns_typed_marker() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&[0u8, 1, 2, 3, 0, 4, 5]).unwrap();

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": tmp.path().to_str().unwrap() }))
            .await
            .unwrap();

        assert_eq!(value["kind"], "binary");
        assert_eq!(value["bytes"], 7);
    }

    #[tokio::test]
    async fn read_missing_file_returns_typed_not_found() {
        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": "/tmp/does-not-exist-read-test-xyz-123" }))
            .await
            .unwrap();
        assert_eq!(value["kind"], "not_found");
        assert_eq!(value["file_path"], "/tmp/does-not-exist-read-test-xyz-123");
    }

    #[tokio::test]
    async fn read_directory_returns_typed_not_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": dir.path().to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(value["kind"], "not_regular_file");
    }

    #[tokio::test]
    async fn read_too_large_returns_typed_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let f = std::fs::File::create(&path).unwrap();
        // One byte past the cap.
        f.set_len(DEFAULT_MAX_READ_BYTES + 1).unwrap();

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": path.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(value["kind"], "too_large");
        assert!(value["bytes"].as_u64().unwrap() > DEFAULT_MAX_READ_BYTES);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_permission_denied_returns_typed_payload() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        fs::write(&path, "secret").await.unwrap();
        let mut perms = fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&path, perms).await.unwrap();

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": path.to_str().unwrap() }))
            .await
            .unwrap();

        // Root bypasses permission checks; tolerate either typed error
        // or a successful text read so the test is CI-safe.
        let kind = value["kind"].as_str().unwrap();
        assert!(
            kind == "permission_denied" || kind == "text",
            "unexpected kind: {kind}"
        );

        // Restore perms so tempdir cleanup works.
        let mut restore = fs::metadata(&path).await.unwrap().permissions();
        restore.set_mode(0o644);
        let _ = fs::set_permissions(&path, restore).await;
    }

    #[tokio::test]
    async fn read_relative_path_errors() {
        let tool = ReadTool::new();
        let err = tool
            .execute(json!({ "file_path": "relative.txt" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be an absolute path"));
    }

    #[test]
    fn parse_page_range_single_page() {
        let (start, end) = pdf::parse_page_range("3").unwrap();
        assert_eq!((start, end), (3, 3));
    }

    #[test]
    fn parse_page_range_range() {
        let (start, end) = pdf::parse_page_range("2-5").unwrap();
        assert_eq!((start, end), (2, 5));
    }

    #[test]
    fn parse_page_range_zero_rejected() {
        assert!(pdf::parse_page_range("0").is_err());
        assert!(pdf::parse_page_range("0-5").is_err());
    }

    #[test]
    fn parse_page_range_inverted_rejected() {
        assert!(pdf::parse_page_range("5-2").is_err());
    }

    #[tokio::test]
    async fn read_pdf_dispatches_for_pdf_extension() {
        // Create a minimal valid PDF in a tempfile. This is the
        // smallest valid PDF structure that pdf-extract can parse.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pdf");
        // Minimal PDF with one page containing "Hello PDF"
        let pdf_bytes = b"%PDF-1.0\n\
            1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj \
            2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj \
            3 0 obj<</Type/Page/MediaBox[0 0 612 792]/Parent 2 0 R/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj \
            4 0 obj<</Length 44>>stream\nBT /F1 12 Tf 100 700 Td (Hello PDF) Tj ET\nendstream\nendobj \
            5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj \
            xref\n0 6\n\
            0000000000 65535 f \n\
            0000000009 00000 n \n\
            0000000058 00000 n \n\
            0000000115 00000 n \n\
            0000000266 00000 n \n\
            0000000360 00000 n \n\
            trailer<</Size 6/Root 1 0 R>>\nstartxref\n424\n%%EOF";
        std::fs::write(&path, pdf_bytes).unwrap();

        let tool = ReadTool::new();
        let value = tool
            .execute(json!({ "file_path": path.to_str().unwrap() }))
            .await
            .unwrap();

        // pdf-extract might succeed or return pdf_error on a minimal
        // PDF; either way we should get a structured Ok payload.
        let kind = value["kind"].as_str().unwrap_or("unknown");
        assert!(
            kind == "pdf" || kind == "pdf_error",
            "unexpected PDF response kind '{kind}': {value}"
        );
    }

    #[tokio::test]
    async fn read_missing_file_path_errors() {
        let tool = ReadTool::new();
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("missing 'file_path'"));
    }
}
