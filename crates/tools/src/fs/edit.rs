//! `Edit` tool — exact-match string replacement with uniqueness enforcement.
//!
//! The uniqueness requirement is the main correctness win over shell `sed`:
//! if `old_string` matches more than once, the edit is rejected unless
//! `replace_all=true`. The error payload includes the match count so the LLM
//! can fix its input.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::{io::Write as _, path::Path, sync::Arc},
    tracing::instrument,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, labels, tools as tools_metrics};

use crate::{
    Result,
    approval::ApprovalManager,
    checkpoints::CheckpointManager,
    error::Error,
    exec::ApprovalBroadcaster,
    fs::{
        sandbox_bridge::SandboxReadResult,
        shared::{
            DEFAULT_MAX_READ_BYTES, FsPathPolicy, FsState, canonicalize_existing,
            check_file_modified_since_read, enforce_must_read_before_write, enforce_path_policy,
            ensure_regular_file, host_mutation_queue_key, note_fs_mutation, reject_if_symlink,
            require_absolute, require_fs_mutation_approval, sandbox_mutation_queue_key,
            session_key_from, with_fs_mutation_lock,
        },
    },
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

/// Outcome of a successful [`apply_edit`] call.
///
/// Carries enough metadata for the tool response to describe how the edit
/// landed, including whether CRLF recovery fired.
#[derive(Debug, Clone)]
pub(crate) struct EditOutcome {
    pub content: String,
    pub replacements: usize,
    pub recovered_via_crlf: bool,
}

/// Apply a single exact-match edit to `content`, enforcing uniqueness semantics.
///
/// Returns an [`EditOutcome`] on success. Errors surface the number of matches
/// so the caller (and the LLM via the tool result) can tell why the edit
/// failed.
///
/// # CRLF recovery
///
/// [`ReadTool`](super::read::ReadTool) strips `\r` from CRLF files so the LLM
/// sees them as LF. If the LLM then crafts an `Edit` with a `\n`-only
/// `old_string` against a CRLF file, the literal match will miss. On zero
/// literal hits we retry with `\r\n` substituted for every `\n` in the needle
/// and `new_string`, so CRLF round-trips work transparently through the
/// Read + Edit pipeline.
pub(crate) fn apply_edit(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<EditOutcome> {
    if old_string.is_empty() {
        return Err(Error::message("'old_string' must not be empty"));
    }
    if old_string == new_string {
        return Err(Error::message("'new_string' must differ from 'old_string'"));
    }

    let match_count = content.matches(old_string).count();
    if match_count > 0 {
        return finish_edit(
            content,
            old_string,
            new_string,
            replace_all,
            match_count,
            false,
        );
    }

    // Recovery 1: CRLF-ify the needle and try again.
    if content.contains("\r\n") && old_string.contains('\n') && !old_string.contains("\r\n") {
        let crlf_old = old_string.replace('\n', "\r\n");
        let crlf_new = new_string.replace('\n', "\r\n");
        let crlf_count = content.matches(&crlf_old).count();
        if crlf_count > 0 {
            return finish_edit(content, &crlf_old, &crlf_new, replace_all, crlf_count, true);
        }
    }

    // Recovery 2: smart-quote normalization (ported from Claude Code's
    // J_6/As4). Fold curly quotes to straight quotes in both the needle
    // and the file content. If the normalized versions match, splice
    // `new_string` into the *original* content at the matched positions
    // so the file's curly-quote style outside the replaced span is
    // preserved.
    let norm_old = normalize_smart_quotes(old_string);
    let norm_content = normalize_smart_quotes(content);
    if norm_old != old_string || norm_content != content {
        let sq_count = norm_content.matches(&norm_old).count();
        if sq_count > 0 {
            if sq_count > 1 && !replace_all {
                return Err(Error::message(format!(
                    "'old_string' matches {sq_count} locations in the file (after \
                     smart-quote normalization); refusing to edit. Supply a larger \
                     `old_string` with more context to make the match unique, or \
                     set `replace_all=true`."
                )));
            }
            // Map normalized char offsets back to original byte offsets
            // so we splice into the real content without corrupting
            // surrounding curly quotes.
            let updated =
                splice_via_normalized(content, &norm_content, &norm_old, new_string, replace_all);
            let replacements = if replace_all {
                sq_count
            } else {
                1
            };
            return Ok(EditOutcome {
                content: updated,
                replacements,
                recovered_via_crlf: true, // reuse flag for "recovery fired"
            });
        }
    }

    Err(Error::message(
        "'old_string' not found in file — edit refused",
    ))
}

/// Fold Unicode curly quotes to their ASCII equivalents.
///
/// Ported from Claude Code's `As4` function:
/// `\u{2018}`→`'`, `\u{2019}`→`'`, `\u{201C}`→`"`, `\u{201D}`→`"`
/// Splice `new_string` into the original `content` at positions found
/// via the normalized (smart-quote-folded) copies. This avoids the bug
/// where passing `&norm_content` to `finish_edit` would silently
/// convert every curly quote in the entire file to ASCII.
fn splice_via_normalized(
    content: &str,
    norm_content: &str,
    norm_old: &str,
    new_string: &str,
    replace_all: bool,
) -> String {
    // Smart-quote folding is 1-char → 1-char so char-count is preserved
    // between content and norm_content. We find match char-offsets in
    // norm_content and map them back to byte-offsets in content.
    let content_chars: Vec<(usize, char)> = content.char_indices().collect();
    let mut result = String::with_capacity(content.len());
    let mut last_byte = 0;
    let norm_old_chars = norm_old.chars().count();

    for (char_offset, _) in norm_content.match_indices(norm_old) {
        // char_offset is a BYTE offset in norm_content. Convert to
        // a char index, then map to a byte offset in content.
        let match_char_start = norm_content[..char_offset].chars().count();
        let match_char_end = match_char_start + norm_old_chars;

        let orig_byte_start = content_chars
            .get(match_char_start)
            .map(|(i, _)| *i)
            .unwrap_or(content.len());
        let orig_byte_end = content_chars
            .get(match_char_end)
            .map(|(i, _)| *i)
            .unwrap_or(content.len());

        result.push_str(&content[last_byte..orig_byte_start]);
        result.push_str(new_string);
        last_byte = orig_byte_end;

        if !replace_all {
            break;
        }
    }
    result.push_str(&content[last_byte..]);
    result
}

fn normalize_smart_quotes(s: &str) -> String {
    s.replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"")
}

fn finish_edit(
    content: &str,
    needle: &str,
    replacement: &str,
    replace_all: bool,
    match_count: usize,
    recovered_via_crlf: bool,
) -> Result<EditOutcome> {
    if match_count > 1 && !replace_all {
        return Err(Error::message(format!(
            "'old_string' matches {match_count} locations in the file; \
             refusing to edit. Supply a larger `old_string` with more \
             context to make the match unique, or set `replace_all=true`."
        )));
    }

    let updated = if replace_all {
        content.replace(needle, replacement)
    } else {
        content.replacen(needle, replacement, 1)
    };
    let replacements = if replace_all {
        match_count
    } else {
        1
    };

    Ok(EditOutcome {
        content: updated,
        replacements,
        recovered_via_crlf,
    })
}

/// Native `Edit` tool implementation.
#[derive(Default)]
pub struct EditTool {
    fs_state: Option<FsState>,
    path_policy: Option<FsPathPolicy>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    approval_manager: Option<Arc<ApprovalManager>>,
    broadcaster: Option<Arc<dyn ApprovalBroadcaster>>,
}

impl EditTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach shared [`FsState`] for must-read-before-write enforcement.
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

    /// Attach a [`CheckpointManager`] so Edit backs up the target file
    /// before mutating.
    #[must_use]
    pub fn with_checkpoint_manager(mut self, manager: Arc<CheckpointManager>) -> Self {
        self.checkpoint_manager = Some(manager);
        self
    }

    /// Attach a shared [`SandboxRouter`]. Sandboxed sessions round-trip
    /// through Read+apply+Write via the bridge.
    #[must_use]
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    /// Attach approval gating for file mutations.
    #[must_use]
    pub fn with_approval(
        mut self,
        manager: Arc<ApprovalManager>,
        broadcaster: Arc<dyn ApprovalBroadcaster>,
    ) -> Self {
        self.approval_manager = Some(manager);
        self.broadcaster = Some(broadcaster);
        self
    }

    #[instrument(skip(self, old_string, new_string), fields(file_path = %file_path, replace_all))]
    async fn edit_impl(
        &self,
        file_path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
        session_key: &str,
    ) -> Result<Value> {
        require_absolute(file_path, "file_path")?;
        let approval_request = format!("Edit {file_path}");

        // Sandbox dispatch: read through the bridge, apply in host
        // memory, write through the bridge. Path policy and must-read-
        // before-write are enforced host-side before dispatching;
        // symlink check is handled by the bridge script.
        if let Some(ref router) = self.sandbox_router
            && router.is_sandboxed(session_key).await
        {
            if let Some(ref policy) = self.path_policy
                && let Some(payload) = enforce_path_policy(policy, Path::new(file_path))
            {
                return Ok(payload);
            }
            if let Some(payload) =
                enforce_must_read_before_write(self.fs_state.as_ref(), session_key, file_path)
            {
                return Ok(payload);
            }
            return with_fs_mutation_lock(
                sandbox_mutation_queue_key(session_key, file_path),
                async {
                    let sandbox_fs = sandbox_file_system_for_session(router, session_key).await?;
                    let read_result = sandbox_fs
                        .read_file(file_path, DEFAULT_MAX_READ_BYTES)
                        .await?;
                    let bytes = match read_result {
                        SandboxReadResult::Ok(bytes) => bytes,
                        other => {
                            return Ok(other
                                .into_typed_payload(file_path, DEFAULT_MAX_READ_BYTES)
                                .unwrap_or(json!({})));
                        },
                    };
                    let content = String::from_utf8(bytes).map_err(|e| {
                        Error::message(format!(
                            "sandbox file '{file_path}' is not valid UTF-8: {e}"
                        ))
                    })?;
                    let outcome = apply_edit(&content, old_string, new_string, replace_all)?;
                    require_fs_mutation_approval(
                        self.approval_manager.as_ref(),
                        self.broadcaster.as_ref(),
                        &approval_request,
                    )
                    .await?;
                    if let Some(payload) = sandbox_fs
                        .write_file(file_path, outcome.content.as_bytes())
                        .await?
                    {
                        return Ok(payload);
                    }
                    note_fs_mutation(self.fs_state.as_ref(), session_key, file_path);
                    Ok(json!({
                        "file_path": file_path,
                        "replacements": outcome.replacements,
                        "replace_all": replace_all,
                        "recovered_via_crlf": outcome.recovered_via_crlf,
                        "checkpoint_id": Value::Null,
                    }))
                },
            )
            .await;
        }

        reject_if_symlink(file_path).await?;
        let canonical = canonicalize_existing(file_path).await?;
        ensure_regular_file(&canonical).await?;

        let canonical_str = canonical
            .to_str()
            .ok_or_else(|| Error::message("file_path contains invalid UTF-8"))?
            .to_string();

        if let Some(ref policy) = self.path_policy
            && let Some(payload) = enforce_path_policy(policy, &canonical)
        {
            return Ok(payload);
        }

        if let Some(payload) =
            enforce_must_read_before_write(self.fs_state.as_ref(), session_key, &canonical_str)
        {
            return Ok(payload);
        }

        // Detect linter/hook modifications between Read and this Edit.
        if let Some(payload) = check_file_modified_since_read(
            self.fs_state.as_ref(),
            session_key,
            &canonical,
            file_path,
        ) {
            return Ok(payload);
        }

        with_fs_mutation_lock(host_mutation_queue_key(&canonical), async {
            let content = tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| Error::message(format!("failed to read '{file_path}': {e}")))?;

            let outcome = apply_edit(&content, old_string, new_string, replace_all)?;

            require_fs_mutation_approval(
                self.approval_manager.as_ref(),
                self.broadcaster.as_ref(),
                &approval_request,
            )
            .await?;

            // Optional checkpoint before the mutation lands.
            let checkpoint_id = if let Some(ref manager) = self.checkpoint_manager {
                Some(manager.checkpoint_path(&canonical, "Edit").await?.id)
            } else {
                None
            };

            persist_atomic(&canonical, &outcome.content).await?;

            note_fs_mutation(self.fs_state.as_ref(), session_key, &canonical_str);

            #[cfg(feature = "metrics")]
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "Edit".to_string(),
                labels::SUCCESS => "true".to_string()
            )
            .increment(1);

            Ok(json!({
                "file_path": canonical.to_string_lossy(),
                "replacements": outcome.replacements,
                "replace_all": replace_all,
                "recovered_via_crlf": outcome.recovered_via_crlf,
                "checkpoint_id": checkpoint_id,
            }))
        })
        .await
    }
}

/// Persist `content` to `path` atomically via a sibling temp file + rename.
pub(crate) async fn persist_atomic(path: &Path, content: &str) -> Result<()> {
    let path = path.to_path_buf();
    let parent = path
        .parent()
        .ok_or_else(|| Error::message(format!("'{}' has no parent", path.display())))?
        .to_path_buf();
    let bytes = content.as_bytes().to_vec();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut tmp = tempfile::NamedTempFile::new_in(&parent).map_err(|e| {
            Error::message(format!(
                "failed to create temp file in '{}': {e}",
                parent.display()
            ))
        })?;
        tmp.write_all(&bytes)
            .map_err(|e| Error::message(format!("failed to write temp file: {e}")))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| Error::message(format!("failed to fsync temp file: {e}")))?;
        tmp.persist(&path)
            .map_err(|e| Error::message(format!("failed to persist '{}': {e}", path.display())))?;
        Ok(())
    })
    .await
    .map_err(|e| Error::message(format!("blocking persist task failed: {e}")))?
}

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Exact-match string replacement in a file. Refuses to edit if \
         `old_string` is not unique in the file unless `replace_all=true`. \
         This uniqueness requirement prevents the most common class of \
         edit mistakes. Supply enough context in `old_string` to make the \
         match unique."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "old_string", "new_string"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit."
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to replace. Must be unique in the file unless replace_all=true."
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text. Must differ from old_string."
                },
                "replace_all": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, replace every occurrence of old_string instead of requiring uniqueness."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        // Accept Claude Code's parameter aliases for robustness.
        let file_path = params
            .get("file_path")
            .or_else(|| params.get("filePath"))
            .or_else(|| params.get("filepath"))
            .or_else(|| params.get("path"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'file_path' parameter"))?;
        let old_string = params
            .get("old_string")
            .or_else(|| params.get("old_str"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'old_string' parameter"))?;
        let new_string = params
            .get("new_string")
            .or_else(|| params.get("new_str"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'new_string' parameter"))?;
        let replace_all = params
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let session_key = session_key_from(&params).to_string();

        match self
            .edit_impl(file_path, old_string, new_string, replace_all, &session_key)
            .await
        {
            Ok(value) => Ok(value),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "Edit".to_string()
                )
                .increment(1);
                Err(e.into())
            },
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edit_unique_match() {
        let content = "hello world";
        let outcome = apply_edit(content, "world", "rust", false).unwrap();
        assert_eq!(outcome.content, "hello rust");
        assert_eq!(outcome.replacements, 1);
        assert!(!outcome.recovered_via_crlf);
    }

    #[test]
    fn apply_edit_rejects_non_unique() {
        let content = "foo foo foo";
        let err = apply_edit(content, "foo", "bar", false).unwrap_err();
        assert!(err.to_string().contains("matches 3 locations"));
    }

    #[test]
    fn apply_edit_replace_all() {
        let content = "foo foo foo";
        let outcome = apply_edit(content, "foo", "bar", true).unwrap();
        assert_eq!(outcome.content, "bar bar bar");
        assert_eq!(outcome.replacements, 3);
    }

    #[test]
    fn apply_edit_rejects_empty_old_string() {
        let err = apply_edit("x", "", "y", false).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn apply_edit_rejects_identical_strings() {
        let err = apply_edit("abc", "abc", "abc", false).unwrap_err();
        assert!(err.to_string().contains("must differ"));
    }

    #[test]
    fn apply_edit_rejects_no_match() {
        let err = apply_edit("hello", "world", "rust", false).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn apply_edit_crlf_recovery_single_line() {
        // File is CRLF; needle is LF-only (as Read would have returned).
        let content = "one\r\ntwo\r\nthree\r\n";
        let outcome = apply_edit(content, "one\ntwo", "ONE\nTWO", false).unwrap();
        assert!(outcome.recovered_via_crlf);
        assert_eq!(outcome.replacements, 1);
        assert_eq!(outcome.content, "ONE\r\nTWO\r\nthree\r\n");
    }

    #[test]
    fn apply_edit_crlf_recovery_preserves_file_line_endings() {
        // New-string is also LF-only; recovery CRLF-ifies it so the file
        // stays CRLF throughout after the edit.
        let content = "alpha\r\nbeta\r\n";
        let outcome = apply_edit(content, "alpha\nbeta", "ALPHA\nBETA", false).unwrap();
        assert_eq!(outcome.content, "ALPHA\r\nBETA\r\n");
    }

    #[test]
    fn apply_edit_no_recovery_when_lf_file() {
        // LF file + LF needle — happy path, recovery does not fire.
        let content = "alpha\nbeta\n";
        let outcome = apply_edit(content, "alpha\nbeta", "ALPHA\nBETA", false).unwrap();
        assert!(!outcome.recovered_via_crlf);
    }

    #[test]
    fn apply_edit_no_recovery_when_needle_already_crlf() {
        // File CRLF, needle CRLF — literal match wins, no recovery fires.
        let content = "one\r\ntwo\r\n";
        let outcome = apply_edit(content, "one\r\ntwo", "ONE\r\nTWO", false).unwrap();
        assert!(!outcome.recovered_via_crlf);
    }

    #[test]
    fn apply_edit_crlf_recovery_still_fails_if_no_match() {
        // Not present in any form.
        let content = "alpha\r\nbeta\r\n";
        let err = apply_edit(content, "delta\nepsilon", "X\nY", false).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn edit_tool_updates_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "alpha beta gamma").await.unwrap();

        let tool = EditTool::new();
        let value = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "old_string": "beta",
                "new_string": "BETA",
            }))
            .await
            .unwrap();

        assert_eq!(value["replacements"], 1);
        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "alpha BETA gamma");
    }

    #[tokio::test]
    async fn edit_tool_rejects_non_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "foo foo").await.unwrap();

        let tool = EditTool::new();
        let err = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "bar",
            }))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("matches 2 locations"));
        // File must be unchanged.
        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "foo foo");
    }

    #[tokio::test]
    async fn edit_tool_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "foo foo foo").await.unwrap();

        let tool = EditTool::new();
        let value = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "bar",
                "replace_all": true,
            }))
            .await
            .unwrap();

        assert_eq!(value["replacements"], 3);
        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "bar bar bar");
    }

    #[tokio::test]
    async fn edit_tool_recovers_crlf_file_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("crlf.txt");
        tokio::fs::write(&target, "line one\r\nline two\r\nline three\r\n")
            .await
            .unwrap();

        // LLM sends LF-only needle (matching what Read stripped).
        let tool = EditTool::new();
        let value = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "old_string": "line one\nline two",
                "new_string": "LINE ONE\nLINE TWO",
            }))
            .await
            .unwrap();

        assert_eq!(value["replacements"], 1);
        assert_eq!(value["recovered_via_crlf"], true);

        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "LINE ONE\r\nLINE TWO\r\nline three\r\n");
    }

    #[tokio::test]
    async fn edit_tool_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        tokio::fs::write(&real, "foo").await.unwrap();
        let link = dir.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        return;

        let tool = EditTool::new();
        let err = tool
            .execute(json!({
                "file_path": link.to_str().unwrap(),
                "old_string": "foo",
                "new_string": "bar",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("symbolic link"));
    }
}
