//! Automatic per-turn file checkpointing via the hook system.
//!
//! Registers a [`HookHandler`] that snapshots files before `Write`, `Edit`, and
//! `MultiEdit` tool calls. Checkpoints are grouped by run (turn) and exposed to
//! the user via the `/rollback` channel command.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use {
    async_trait::async_trait,
    moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload},
    serde_json::Value,
    time::OffsetDateTime,
    tokio::sync::Mutex,
    tracing::{debug, warn},
};

use crate::checkpoints::{CheckpointManager, TurnRecord};

/// File-mutating tool names that trigger automatic checkpoints.
const CHECKPOINT_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit"];

/// In-progress turn state for a single session.
struct ActiveTurn {
    run_id: String,
    session_key: String,
    checkpoint_ids: Vec<String>,
    source_paths: Vec<String>,
    created_at: i64,
}

/// Hook handler that automatically checkpoints files before mutation tools.
pub struct AutoCheckpointHook {
    manager: Arc<CheckpointManager>,
    /// Active turns keyed by session_key. Flushed when run_id changes.
    active: Mutex<HashMap<String, ActiveTurn>>,
}

impl AutoCheckpointHook {
    pub fn new(manager: Arc<CheckpointManager>) -> Self {
        Self {
            manager,
            active: Mutex::new(HashMap::new()),
        }
    }

    /// Extract the file path from tool arguments.
    fn extract_file_path(args: &Value) -> Option<&str> {
        // Write and Edit use "file_path"
        args.get("file_path")
            .and_then(|v| v.as_str())
            // MultiEdit uses "file_path" too
            .or_else(|| args.get("path").and_then(|v| v.as_str()))
    }

    /// Extract run_id from tool arguments (injected by tool context).
    fn extract_run_id(args: &Value) -> Option<&str> {
        args.get("_run_id").and_then(|v| v.as_str())
    }
}

#[async_trait]
impl HookHandler for AutoCheckpointHook {
    fn name(&self) -> &str {
        "auto_checkpoint"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::BeforeToolCall, HookEvent::AgentEnd]
    }

    fn priority(&self) -> i32 {
        // Run early so the checkpoint is created before the tool modifies the file.
        100
    }

    async fn handle(
        &self,
        _event: HookEvent,
        payload: &HookPayload,
    ) -> moltis_common::Result<HookAction> {
        // On AgentEnd, flush any in-progress turn so it is persisted.
        if let HookPayload::AgentEnd { session_key, .. } = payload {
            let mut active = self.active.lock().await;
            if let Some(turn) = active.remove(session_key)
                && !turn.checkpoint_ids.is_empty()
            {
                let manager = Arc::clone(&self.manager);
                let record = TurnRecord {
                    run_id: turn.run_id,
                    session_key: turn.session_key,
                    created_at: turn.created_at,
                    checkpoint_ids: turn.checkpoint_ids,
                    source_paths: turn.source_paths,
                };
                tokio::spawn(async move {
                    if let Err(e) = manager.append_turn(&record) {
                        warn!(error = %e, "failed to flush turn record at agent end");
                    }
                });
            }
            return Ok(HookAction::Continue);
        }

        let HookPayload::BeforeToolCall {
            session_key,
            tool_name,
            arguments,
            ..
        } = payload
        else {
            return Ok(HookAction::Continue);
        };

        if !CHECKPOINT_TOOLS.iter().any(|t| t == tool_name) {
            return Ok(HookAction::Continue);
        }

        let Some(file_path) = Self::extract_file_path(arguments) else {
            return Ok(HookAction::Continue);
        };

        let run_id = Self::extract_run_id(arguments)
            .unwrap_or("unknown")
            .to_string();

        // Snapshot the file before the tool modifies it.
        let path = PathBuf::from(file_path);
        let checkpoint = match self.manager.checkpoint_path(&path, tool_name).await {
            Ok(cp) => cp,
            Err(e) => {
                // Don't block the tool call if checkpointing fails.
                warn!(
                    file = %file_path,
                    tool = %tool_name,
                    error = %e,
                    "auto-checkpoint failed, tool will proceed without snapshot"
                );
                return Ok(HookAction::Continue);
            },
        };

        debug!(
            file = %file_path,
            tool = %tool_name,
            checkpoint_id = %checkpoint.id,
            "auto-checkpointed file before mutation"
        );

        // Track this checkpoint in the active turn for this session.
        let mut active = self.active.lock().await;
        let turn = active
            .entry(session_key.clone())
            .or_insert_with(|| ActiveTurn {
                run_id: run_id.clone(),
                session_key: session_key.clone(),
                checkpoint_ids: Vec::new(),
                source_paths: Vec::new(),
                created_at: OffsetDateTime::now_utc().unix_timestamp(),
            });

        // If the run_id changed, flush the previous turn and start a new one.
        if turn.run_id != run_id {
            let prev = std::mem::replace(turn, ActiveTurn {
                run_id: run_id.clone(),
                session_key: session_key.clone(),
                checkpoint_ids: Vec::new(),
                source_paths: Vec::new(),
                created_at: OffsetDateTime::now_utc().unix_timestamp(),
            });
            // Flush in background to avoid blocking the tool call.
            let self_ref = Arc::clone(&self.manager);
            let record = TurnRecord {
                run_id: prev.run_id,
                session_key: prev.session_key,
                created_at: prev.created_at,
                checkpoint_ids: prev.checkpoint_ids,
                source_paths: prev.source_paths,
            };
            tokio::spawn(async move {
                if let Err(e) = self_ref.append_turn(&record) {
                    warn!(error = %e, "failed to flush previous turn record");
                }
            });
        }

        turn.checkpoint_ids.push(checkpoint.id);
        turn.source_paths.push(file_path.to_string());

        Ok(HookAction::Continue)
    }
}
