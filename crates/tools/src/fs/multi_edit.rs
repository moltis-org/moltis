//! `MultiEdit` tool — atomic batch of sequential edits to a single file.
//!
//! All edits succeed or none apply. Each edit runs against the buffer
//! produced by the previous edit, so later edits see the output of earlier
//! ones. This is the right semantic for multi-step file surgery: the LLM
//! plans the whole sequence, and we guarantee rollback on any failure.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::sync::Arc,
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
        edit::{apply_edit, persist_atomic},
        sandbox_bridge::SandboxReadResult,
        shared::{
            DEFAULT_MAX_READ_BYTES, FsPathPolicy, FsState, canonicalize_existing,
            enforce_must_read_before_write, enforce_path_policy, ensure_regular_file,
            host_mutation_queue_key, note_fs_mutation, reject_if_symlink, require_absolute,
            require_fs_mutation_approval, sandbox_mutation_queue_key, session_key_from,
            with_fs_mutation_lock,
        },
    },
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

/// Native `MultiEdit` tool implementation.
#[derive(Default)]
pub struct MultiEditTool {
    fs_state: Option<FsState>,
    path_policy: Option<FsPathPolicy>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    approval_manager: Option<Arc<ApprovalManager>>,
    broadcaster: Option<Arc<dyn ApprovalBroadcaster>>,
}

impl MultiEditTool {
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

    /// Attach a [`CheckpointManager`] so MultiEdit backs up the target
    /// file before the batch lands.
    #[must_use]
    pub fn with_checkpoint_manager(mut self, manager: Arc<CheckpointManager>) -> Self {
        self.checkpoint_manager = Some(manager);
        self
    }

    /// Attach a shared [`SandboxRouter`]. Sandboxed sessions round-trip
    /// through Read+apply(*)+Write via the bridge.
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

    #[instrument(skip(self, edits), fields(file_path = %file_path, edit_count = edits.len()))]
    async fn multi_edit_impl(
        &self,
        file_path: &str,
        edits: Vec<ParsedEdit>,
        session_key: &str,
    ) -> Result<Value> {
        if edits.is_empty() {
            return Err(Error::message("'edits' must contain at least one edit"));
        }
        require_absolute(file_path, "file_path")?;
        let approval_request = format!("MultiEdit {file_path}");

        // Sandbox dispatch: read once, apply all edits in host memory,
        // write the final buffer back through the bridge atomically.
        // Path policy and must-read-before-write are enforced host-side.
        if let Some(ref router) = self.sandbox_router
            && router.is_sandboxed(session_key).await
        {
            if let Some(ref policy) = self.path_policy
                && let Some(payload) = enforce_path_policy(policy, std::path::Path::new(file_path))
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
                    let original = String::from_utf8(bytes).map_err(|e| {
                        Error::message(format!(
                            "sandbox file '{file_path}' is not valid UTF-8: {e}"
                        ))
                    })?;

                    let mut buffer = original;
                    let mut per_edit_replacements: Vec<usize> = Vec::with_capacity(edits.len());
                    let mut any_crlf_recovery = false;
                    for (idx, edit) in edits.iter().enumerate() {
                        let outcome = apply_edit(
                            &buffer,
                            &edit.old_string,
                            &edit.new_string,
                            edit.replace_all,
                        )
                        .map_err(|e| Error::message(format!("edit #{}: {e}", idx + 1)))?;
                        per_edit_replacements.push(outcome.replacements);
                        if outcome.recovered_via_crlf {
                            any_crlf_recovery = true;
                        }
                        buffer = outcome.content;
                    }

                    require_fs_mutation_approval(
                        self.approval_manager.as_ref(),
                        self.broadcaster.as_ref(),
                        &approval_request,
                    )
                    .await?;
                    if let Some(payload) =
                        sandbox_fs.write_file(file_path, buffer.as_bytes()).await?
                    {
                        return Ok(payload);
                    }
                    note_fs_mutation(self.fs_state.as_ref(), session_key, file_path);
                    Ok(json!({
                        "file_path": file_path,
                        "edits_applied": edits.len(),
                        "replacements_per_edit": per_edit_replacements,
                        "recovered_via_crlf": any_crlf_recovery,
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

        with_fs_mutation_lock(host_mutation_queue_key(&canonical), async {
            let original = tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| Error::message(format!("failed to read '{file_path}': {e}")))?;

            let mut buffer = original;
            let mut per_edit_replacements: Vec<usize> = Vec::with_capacity(edits.len());
            let mut any_crlf_recovery = false;

            for (idx, edit) in edits.iter().enumerate() {
                let outcome = apply_edit(
                    &buffer,
                    &edit.old_string,
                    &edit.new_string,
                    edit.replace_all,
                )
                .map_err(|e| Error::message(format!("edit #{}: {e}", idx + 1)))?;
                per_edit_replacements.push(outcome.replacements);
                if outcome.recovered_via_crlf {
                    any_crlf_recovery = true;
                }
                buffer = outcome.content;
            }

            require_fs_mutation_approval(
                self.approval_manager.as_ref(),
                self.broadcaster.as_ref(),
                &approval_request,
            )
            .await?;

            // Optional checkpoint before the whole batch lands.
            let checkpoint_id = if let Some(ref manager) = self.checkpoint_manager {
                Some(manager.checkpoint_path(&canonical, "MultiEdit").await?.id)
            } else {
                None
            };

            persist_atomic(&canonical, &buffer).await?;

            note_fs_mutation(self.fs_state.as_ref(), session_key, &canonical_str);

            #[cfg(feature = "metrics")]
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "MultiEdit".to_string(),
                labels::SUCCESS => "true".to_string()
            )
            .increment(1);

            Ok(json!({
                "file_path": canonical.to_string_lossy(),
                "edits_applied": edits.len(),
                "replacements_per_edit": per_edit_replacements,
                "recovered_via_crlf": any_crlf_recovery,
                "checkpoint_id": checkpoint_id,
            }))
        })
        .await
    }
}

#[derive(Debug, Clone)]
struct ParsedEdit {
    old_string: String,
    new_string: String,
    replace_all: bool,
}

fn parse_edits(raw: &Value) -> Result<Vec<ParsedEdit>> {
    let arr = raw
        .as_array()
        .ok_or_else(|| Error::message("'edits' must be an array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, entry) in arr.iter().enumerate() {
        let old_string = entry
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message(format!("edit #{}: missing 'old_string'", idx + 1)))?
            .to_string();
        let new_string = entry
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message(format!("edit #{}: missing 'new_string'", idx + 1)))?
            .to_string();
        let replace_all = entry
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.push(ParsedEdit {
            old_string,
            new_string,
            replace_all,
        });
    }
    Ok(out)
}

#[async_trait]
impl AgentTool for MultiEditTool {
    fn name(&self) -> &str {
        "MultiEdit"
    }

    fn description(&self) -> &str {
        "Apply multiple sequential edits to a single file as one atomic \
         operation. Each edit sees the output of previous edits. Either all \
         edits succeed or none are applied (full rollback on any failure). \
         Each edit follows the same uniqueness rules as `Edit`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "edits"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit."
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ordered list of edits to apply atomically.",
                    "items": {
                        "type": "object",
                        "required": ["old_string", "new_string"],
                        "properties": {
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean", "default": false }
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let file_path = params
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'file_path' parameter"))?;
        let edits_raw = params
            .get("edits")
            .ok_or_else(|| Error::message("missing 'edits' parameter"))?;
        let edits = parse_edits(edits_raw)?;
        let session_key = session_key_from(&params).to_string();

        match self.multi_edit_impl(file_path, edits, &session_key).await {
            Ok(value) => Ok(value),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "MultiEdit".to_string()
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

    #[tokio::test]
    async fn multi_edit_applies_sequential_edits() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "alpha beta gamma").await.unwrap();

        let tool = MultiEditTool::new();
        tool.execute(json!({
            "file_path": target.to_str().unwrap(),
            "edits": [
                { "old_string": "alpha", "new_string": "ALPHA" },
                { "old_string": "gamma", "new_string": "GAMMA" }
            ]
        }))
        .await
        .unwrap();

        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "ALPHA beta GAMMA");
    }

    #[tokio::test]
    async fn multi_edit_second_sees_first_output() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "one").await.unwrap();

        let tool = MultiEditTool::new();
        tool.execute(json!({
            "file_path": target.to_str().unwrap(),
            "edits": [
                { "old_string": "one", "new_string": "two" },
                { "old_string": "two", "new_string": "three" }
            ]
        }))
        .await
        .unwrap();

        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "three");
    }

    #[tokio::test]
    async fn multi_edit_rolls_back_on_any_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "alpha beta").await.unwrap();

        let tool = MultiEditTool::new();
        let err = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "edits": [
                    { "old_string": "alpha", "new_string": "ALPHA" },
                    { "old_string": "nope", "new_string": "NOPE" }
                ]
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("edit #2"));

        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "alpha beta");
    }

    #[tokio::test]
    async fn multi_edit_empty_array_errors() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "alpha").await.unwrap();

        let tool = MultiEditTool::new();
        let err = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "edits": []
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at least one edit"));
    }

    #[tokio::test]
    async fn multi_edit_replace_all_per_edit() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        tokio::fs::write(&target, "foo foo foo bar").await.unwrap();

        let tool = MultiEditTool::new();
        let value = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "edits": [
                    { "old_string": "foo", "new_string": "FOO", "replace_all": true },
                    { "old_string": "bar", "new_string": "BAR" }
                ]
            }))
            .await
            .unwrap();

        assert_eq!(value["replacements_per_edit"], json!([3, 1]));
        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "FOO FOO FOO BAR");
    }
}
