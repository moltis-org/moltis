//! `Write` tool — atomic file writes.
//!
//! Writes the full contents of a file via a temp-file + rename sequence so
//! partial writes are never visible. Refuses to follow symlinks by default.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::io::Write as _,
    tracing::instrument,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, labels, tools as tools_metrics};

use std::sync::Arc;

use crate::{
    Result,
    approval::ApprovalManager,
    checkpoints::CheckpointManager,
    error::Error,
    exec::ApprovalBroadcaster,
    fs::shared::{
        FsPathPolicy, FsState, canonicalize_for_create, enforce_must_read_before_write,
        enforce_path_policy, host_mutation_queue_key, note_fs_mutation, reject_if_symlink,
        require_absolute, require_fs_mutation_approval, sandbox_mutation_queue_key,
        session_key_from, with_fs_mutation_lock,
    },
    sandbox::{SandboxRouter, file_system::sandbox_file_system_for_session},
};

/// Native `Write` tool implementation.
#[derive(Default)]
pub struct WriteTool {
    fs_state: Option<FsState>,
    path_policy: Option<FsPathPolicy>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    approval_manager: Option<Arc<ApprovalManager>>,
    broadcaster: Option<Arc<dyn ApprovalBroadcaster>>,
}

impl WriteTool {
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

    /// Attach a [`CheckpointManager`] so Write backs up the target
    /// file before overwriting. Does nothing for new files.
    #[must_use]
    pub fn with_checkpoint_manager(mut self, manager: Arc<CheckpointManager>) -> Self {
        self.checkpoint_manager = Some(manager);
        self
    }

    /// Attach a shared [`SandboxRouter`]. Sandboxed sessions dispatch
    /// through the bridge.
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

    #[instrument(skip(self, content), fields(file_path = %file_path, bytes = content.len()))]
    async fn write_impl(&self, file_path: &str, content: &str, session_key: &str) -> Result<Value> {
        require_absolute(file_path, "file_path")?;
        let approval_request = format!("Write {file_path}");

        // Sandbox dispatch: round-trip through the bridge when sandboxed.
        // Symlink check is handled by the bridge script. Host-side
        // canonicalization doesn't apply (the path lives inside the
        // container). Path policy and must-read-before-write are
        // enforced host-side before dispatching.
        if let Some(ref router) = self.sandbox_router
            && router.is_sandboxed(session_key).await
        {
            if let Some(ref policy) = self.path_policy
                && let Some(payload) = enforce_path_policy(policy, std::path::Path::new(file_path))
            {
                return Ok(payload);
            }
            return with_fs_mutation_lock(
                sandbox_mutation_queue_key(session_key, file_path),
                async {
                    // must-read-before-write: skip for sandbox Write because we
                    // can't cheaply check whether the file exists inside the
                    // container (that would cost an extra exec round-trip). New
                    // files would be falsely blocked. Edit/MultiEdit always read
                    // before writing so they get the check naturally.
                    require_fs_mutation_approval(
                        self.approval_manager.as_ref(),
                        self.broadcaster.as_ref(),
                        &approval_request,
                    )
                    .await?;
                    let sandbox_fs = sandbox_file_system_for_session(router, session_key).await?;
                    if let Some(payload) =
                        sandbox_fs.write_file(file_path, content.as_bytes()).await?
                    {
                        return Ok(payload);
                    }
                    note_fs_mutation(self.fs_state.as_ref(), session_key, file_path);
                    Ok(json!({
                        "file_path": file_path,
                        "bytes_written": content.len(),
                        "checkpoint_id": Value::Null,
                    }))
                },
            )
            .await;
        }

        let canonical = canonicalize_for_create(file_path).await?;
        let canonical_str = canonical
            .to_str()
            .ok_or_else(|| Error::message("file_path contains invalid UTF-8"))?
            .to_string();

        // Path policy check runs on the canonicalized path so the allow-
        // list evaluates after symlink resolution.
        if let Some(ref policy) = self.path_policy
            && let Some(payload) = enforce_path_policy(policy, &canonical)
        {
            return Ok(payload);
        }

        with_fs_mutation_lock(host_mutation_queue_key(&canonical), async {
            let target_exists = tokio::fs::try_exists(&canonical).await.unwrap_or(false);
            let mut checkpoint_id: Option<String> = None;

            if target_exists {
                // Reject symlinks so we don't unknowingly write through to
                // another location. A new file naturally isn't a symlink.
                reject_if_symlink(&canonical_str).await?;

                // Must-read-before-write: reject if the target exists and the
                // session hasn't read it. Skip this check for new files —
                // there's nothing to have read.
                if let Some(payload) = enforce_must_read_before_write(
                    self.fs_state.as_ref(),
                    session_key,
                    &canonical_str,
                ) {
                    return Ok(payload);
                }
            }

            require_fs_mutation_approval(
                self.approval_manager.as_ref(),
                self.broadcaster.as_ref(),
                &approval_request,
            )
            .await?;

            // Optional checkpoint backup before the mutation lands.
            if target_exists && let Some(ref manager) = self.checkpoint_manager {
                let record = manager.checkpoint_path(&canonical, "Write").await?;
                checkpoint_id = Some(record.id);
            }

            let parent = canonical
                .parent()
                .ok_or_else(|| Error::message(format!("'{file_path}' has no parent directory")))?;

            let bytes = content.as_bytes().to_vec();
            let canonical_for_blocking = canonical.clone();
            let parent_owned = parent.to_path_buf();

            // tempfile's persist + sync on a blocking thread so we stay async-safe.
            tokio::task::spawn_blocking(move || -> Result<()> {
                let mut tmp = tempfile::NamedTempFile::new_in(&parent_owned).map_err(|e| {
                    Error::message(format!(
                        "failed to create temp file in '{}': {e}",
                        parent_owned.display()
                    ))
                })?;
                tmp.write_all(&bytes)
                    .map_err(|e| Error::message(format!("failed to write temp file: {e}")))?;
                tmp.as_file()
                    .sync_all()
                    .map_err(|e| Error::message(format!("failed to fsync temp file: {e}")))?;
                tmp.persist(&canonical_for_blocking).map_err(|e| {
                    Error::message(format!(
                        "failed to persist file '{}': {e}",
                        canonical_for_blocking.display()
                    ))
                })?;
                Ok(())
            })
            .await
            .map_err(|e| Error::message(format!("blocking write task failed: {e}")))??;

            note_fs_mutation(self.fs_state.as_ref(), session_key, &canonical_str);

            #[cfg(feature = "metrics")]
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "Write".to_string(),
                labels::SUCCESS => "true".to_string()
            )
            .increment(1);

            Ok(json!({
                "file_path": canonical.to_string_lossy(),
                "bytes_written": content.len(),
                "checkpoint_id": checkpoint_id,
            }))
        })
        .await
    }
}

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Atomically write a file to the local filesystem. Parent directories \
         must already exist. Refuses to follow symlinks. The entire file is \
         replaced — use `Edit` for surgical changes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Full file contents to write."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let file_path = params
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'file_path' parameter"))?;
        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::message("missing 'content' parameter"))?;
        let session_key = session_key_from(&params).to_string();

        match self.write_impl(file_path, content, &session_key).await {
            Ok(value) => Ok(value),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "Write".to_string()
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
    async fn write_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.txt");

        let tool = WriteTool::new();
        let value = tool
            .execute(json!({
                "file_path": target.to_str().unwrap(),
                "content": "hello world\n",
            }))
            .await
            .unwrap();

        assert_eq!(value["bytes_written"], 12);
        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "hello world\n");
    }

    #[tokio::test]
    async fn write_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.txt");
        tokio::fs::write(&target, "old contents").await.unwrap();

        let tool = WriteTool::new();
        tool.execute(json!({
            "file_path": target.to_str().unwrap(),
            "content": "new contents",
        }))
        .await
        .unwrap();

        let contents = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(contents, "new contents");
    }

    #[tokio::test]
    async fn write_rejects_symlink_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.txt");
        tokio::fs::write(&real, "real").await.unwrap();
        let link = dir.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        {
            // Symlink creation on windows requires privileges; skip.
            return;
        }

        let tool = WriteTool::new();
        let err = tool
            .execute(json!({
                "file_path": link.to_str().unwrap(),
                "content": "nope",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("symbolic link"));
    }

    #[tokio::test]
    async fn write_missing_parent_errors() {
        let tool = WriteTool::new();
        let err = tool
            .execute(json!({
                "file_path": "/tmp/definitely-missing-parent-ab91x/out.txt",
                "content": "x",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot resolve parent"));
    }

    #[tokio::test]
    async fn write_missing_content_errors() {
        let tool = WriteTool::new();
        let err = tool
            .execute(json!({ "file_path": "/tmp/x" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'content'"));
    }
}
