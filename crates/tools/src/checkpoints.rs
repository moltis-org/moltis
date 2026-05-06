use std::{
    fs,
    path::{Path, PathBuf},
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::{Value, json},
    time::OffsetDateTime,
    uuid::Uuid,
};

use moltis_agents::tool_registry::AgentTool;

use crate::{
    Error, Result,
    params::{require_str, str_param, u64_param},
};

const DEFAULT_LIST_LIMIT: usize = 20;
const MAX_LIST_LIMIT: usize = 100;
/// Prune when checkpoint count exceeds this threshold.
const CLEANUP_CHECKPOINT_THRESHOLD: usize = 500;
/// Fraction of checkpoints to remove when pruning.
const CLEANUP_PRUNE_RATIO: f64 = 0.2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSourceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub id: String,
    pub created_at: i64,
    pub reason: String,
    pub source_path: String,
    pub source_kind: Option<CheckpointSourceKind>,
    pub existed: bool,
    pub backup_path: Option<String>,
}

impl CheckpointRecord {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "createdAt": self.created_at,
            "reason": self.reason,
            "sourcePath": self.source_path,
            "sourceKind": self.source_kind,
            "existed": self.existed,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointManager {
    base_dir: PathBuf,
}

impl CheckpointManager {
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            base_dir: data_dir.join("checkpoints"),
        }
    }

    pub async fn checkpoint_path(&self, source: &Path, reason: &str) -> Result<CheckpointRecord> {
        let base_dir = self.base_dir.clone();
        let source = source.to_path_buf();
        let reason = reason.to_string();

        tokio::task::spawn_blocking(move || checkpoint_path_blocking(&base_dir, &source, &reason))
            .await
            .map_err(|error| Error::message(format!("checkpoint task failed: {error}")))?
    }

    pub async fn list(
        &self,
        limit: usize,
        path_contains: Option<&str>,
    ) -> Result<Vec<CheckpointRecord>> {
        let base_dir = self.base_dir.clone();
        let path_contains = path_contains.map(|value| value.to_lowercase());
        let limit = limit.clamp(1, MAX_LIST_LIMIT);

        tokio::task::spawn_blocking(move || {
            let mut items = read_all_manifests(&base_dir)?;
            if let Some(filter) = path_contains {
                items.retain(|item| item.source_path.to_lowercase().contains(&filter));
            }
            items.sort_by(|lhs, rhs| {
                rhs.created_at
                    .cmp(&lhs.created_at)
                    .then_with(|| lhs.id.cmp(&rhs.id))
            });
            items.truncate(limit);
            Ok(items)
        })
        .await
        .map_err(|error| Error::message(format!("checkpoint list task failed: {error}")))?
    }

    pub async fn restore(&self, id: &str) -> Result<CheckpointRecord> {
        let base_dir = self.base_dir.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || restore_checkpoint_blocking(&base_dir, &id))
            .await
            .map_err(|error| Error::message(format!("checkpoint restore task failed: {error}")))?
    }
}

// ── Turn index ──────────────────────────────────────────────────────────────

/// A group of checkpoints created during a single agent turn (run).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub run_id: String,
    pub session_key: String,
    pub created_at: i64,
    pub checkpoint_ids: Vec<String>,
    /// Source paths for display (parallel to checkpoint_ids).
    pub source_paths: Vec<String>,
}

impl CheckpointManager {
    /// Path to the turns index file.
    fn turns_path(&self) -> PathBuf {
        self.base_dir.join("turns.jsonl")
    }

    /// Append a turn record to the index.
    pub fn append_turn(&self, record: &TurnRecord) -> Result<()> {
        fs::create_dir_all(&self.base_dir)?;
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.turns_path())?;
        std::io::Write::write_all(&mut file, line.as_bytes())?;
        Ok(())
    }

    /// Read turn records (newest first), optionally filtered by session key.
    pub fn read_turns(&self, limit: usize, session_key: Option<&str>) -> Result<Vec<TurnRecord>> {
        let path = self.turns_path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut turns: Vec<TurnRecord> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter(|turn: &TurnRecord| session_key.is_none_or(|sk| turn.session_key == sk))
            .collect();
        turns.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        turns.truncate(limit);
        Ok(turns)
    }

    /// Remove old checkpoints when the total count exceeds the threshold.
    ///
    /// Deletes the oldest checkpoints (by `created_at`) until the count is
    /// under the threshold. Also removes corresponding turn records.
    pub async fn cleanup_if_needed(&self) -> Result<usize> {
        let base_dir = self.base_dir.clone();
        tokio::task::spawn_blocking(move || cleanup_blocking(&base_dir))
            .await
            .map_err(|e| Error::message(format!("checkpoint cleanup failed: {e}")))?
    }
}

fn cleanup_blocking(base_dir: &Path) -> Result<usize> {
    let mut all = read_all_manifests(base_dir)?;
    if all.len() <= CLEANUP_CHECKPOINT_THRESHOLD {
        return Ok(0);
    }

    // Sort oldest first, delete the oldest CLEANUP_PRUNE_RATIO fraction.
    all.sort_by_key(|r| r.created_at);
    let to_remove = (all.len() as f64 * CLEANUP_PRUNE_RATIO).ceil() as usize;
    let removed_ids: std::collections::HashSet<String> =
        all.iter().take(to_remove).map(|r| r.id.clone()).collect();

    let mut deleted = 0;
    for id in &removed_ids {
        let dir = base_dir.join(id);
        if dir.is_dir() {
            if let Err(e) = fs::remove_dir_all(&dir) {
                tracing::warn!(checkpoint = %id, error = %e, "failed to remove old checkpoint");
            } else {
                deleted += 1;
            }
        }
    }

    // Rewrite turns.jsonl without references to deleted checkpoints.
    let turns_path = base_dir.join("turns.jsonl");
    if turns_path.exists()
        && let Ok(content) = fs::read_to_string(&turns_path)
    {
        let filtered: Vec<String> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let mut turn: TurnRecord = serde_json::from_str(line).ok()?;
                let orig_len = turn.checkpoint_ids.len();
                let mut new_ids = Vec::new();
                let mut new_paths = Vec::new();
                for (id, path) in turn
                    .checkpoint_ids
                    .into_iter()
                    .zip(turn.source_paths.into_iter())
                {
                    if !removed_ids.contains(&id) {
                        new_ids.push(id);
                        new_paths.push(path);
                    }
                }
                if new_ids.is_empty() && orig_len > 0 {
                    return None; // entire turn pruned
                }
                turn.checkpoint_ids = new_ids;
                turn.source_paths = new_paths;
                serde_json::to_string(&turn).ok()
            })
            .collect();
        let _ = fs::write(&turns_path, filtered.join("\n") + "\n");
    }

    if deleted > 0 {
        tracing::info!(
            deleted,
            remaining = all.len() - deleted,
            "pruned old checkpoints"
        );
    }
    Ok(deleted)
}

pub struct CheckpointsListTool {
    manager: CheckpointManager,
}

impl CheckpointsListTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            manager: CheckpointManager::new(data_dir),
        }
    }
}

#[async_trait]
impl AgentTool for CheckpointsListTool {
    fn name(&self) -> &str {
        "checkpoints_list"
    }

    fn description(&self) -> &str {
        "List recent automatic checkpoints created before built-in file mutations."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum checkpoints returned (default: 20, max: 100)."
                },
                "path_contains": {
                    "type": "string",
                    "description": "Optional case-insensitive substring filter on the source path."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let limit = u64_param(&params, "limit", DEFAULT_LIST_LIMIT as u64) as usize;
        let path_contains = str_param(&params, "path_contains");
        let checkpoints = self.manager.list(limit, path_contains).await?;

        Ok(json!({
            "count": checkpoints.len(),
            "checkpoints": checkpoints.into_iter().map(|item| item.to_json()).collect::<Vec<_>>(),
        }))
    }
}

pub struct CheckpointRestoreTool {
    manager: CheckpointManager,
}

impl CheckpointRestoreTool {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            manager: CheckpointManager::new(data_dir),
        }
    }
}

#[async_trait]
impl AgentTool for CheckpointRestoreTool {
    fn name(&self) -> &str {
        "checkpoint_restore"
    }

    fn description(&self) -> &str {
        "Restore a built-in file or directory mutation from an automatic checkpoint."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Checkpoint ID returned by checkpoints_list or a tool result."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let id = require_str(&params, "id")?;
        let checkpoint = self.manager.restore(id).await?;

        Ok(json!({
            "restored": true,
            "checkpoint": checkpoint.to_json(),
        }))
    }
}

fn checkpoint_path_blocking(
    base_dir: &Path,
    source: &Path,
    reason: &str,
) -> Result<CheckpointRecord> {
    let id = Uuid::new_v4().simple().to_string();
    let checkpoint_dir = base_dir.join(&id);
    fs::create_dir_all(&checkpoint_dir)?;

    let metadata = match fs::symlink_metadata(source) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    let mut record = CheckpointRecord {
        id,
        created_at: OffsetDateTime::now_utc().unix_timestamp(),
        reason: reason.to_string(),
        source_path: source.to_string_lossy().into_owned(),
        source_kind: None,
        existed: metadata.is_some(),
        backup_path: None,
    };

    if let Some(metadata) = metadata {
        if metadata.file_type().is_symlink() {
            return Err(Error::message(format!(
                "refusing to checkpoint symlink path '{}'",
                source.display()
            )));
        }

        let snapshot_root = checkpoint_dir.join("snapshot");
        match classify_source_kind(&metadata, source)? {
            CheckpointSourceKind::File => {
                let backup = snapshot_root.join("file");
                if let Some(parent) = backup.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(source, &backup)?;
                record.source_kind = Some(CheckpointSourceKind::File);
                record.backup_path = Some("snapshot/file".to_string());
            },
            CheckpointSourceKind::Directory => {
                let backup = snapshot_root.join("dir");
                copy_dir_recursive(source, &backup)?;
                record.source_kind = Some(CheckpointSourceKind::Directory);
                record.backup_path = Some("snapshot/dir".to_string());
            },
        }
    }

    write_manifest(&checkpoint_dir, &record)?;
    Ok(record)
}

fn restore_checkpoint_blocking(base_dir: &Path, id: &str) -> Result<CheckpointRecord> {
    validate_checkpoint_id(id)?;
    let checkpoint_dir = base_dir.join(id);
    let record = read_manifest(&checkpoint_dir)?;
    let source = PathBuf::from(&record.source_path);

    remove_existing_path(&source)?;

    if record.existed {
        let backup_rel = record
            .backup_path
            .as_ref()
            .ok_or_else(|| Error::message("checkpoint is missing backup data"))?;
        let backup = checkpoint_dir.join(backup_rel);
        match record
            .source_kind
            .ok_or_else(|| Error::message("checkpoint is missing source kind"))?
        {
            CheckpointSourceKind::File => {
                if let Some(parent) = source.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&backup, &source)?;
            },
            CheckpointSourceKind::Directory => {
                copy_dir_recursive(&backup, &source)?;
            },
        }
    }

    Ok(record)
}

fn validate_checkpoint_id(id: &str) -> Result<()> {
    let is_valid = id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !is_valid {
        return Err(Error::message(format!("invalid checkpoint id '{id}'")));
    }
    Ok(())
}

fn classify_source_kind(metadata: &fs::Metadata, source: &Path) -> Result<CheckpointSourceKind> {
    if metadata.is_file() {
        return Ok(CheckpointSourceKind::File);
    }
    if metadata.is_dir() {
        return Ok(CheckpointSourceKind::Directory);
    }
    Err(Error::message(format!(
        "unsupported checkpoint target '{}'",
        source.display()
    )))
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&src_path)?;

        if metadata.file_type().is_symlink() {
            return Err(Error::message(format!(
                "refusing to checkpoint symlink path '{}'",
                src_path.display()
            )));
        }

        if metadata.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dst_path)?;
        } else {
            return Err(Error::message(format!(
                "unsupported checkpoint entry '{}'",
                src_path.display()
            )));
        }
    }
    Ok(())
}

fn remove_existing_path(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    if metadata.file_type().is_symlink() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }

    Ok(())
}

fn write_manifest(checkpoint_dir: &Path, record: &CheckpointRecord) -> Result<()> {
    let manifest_path = checkpoint_dir.join("manifest.json");
    let payload = serde_json::to_vec_pretty(record)?;
    fs::write(manifest_path, payload)?;
    Ok(())
}

fn read_manifest(checkpoint_dir: &Path) -> Result<CheckpointRecord> {
    let manifest_path = checkpoint_dir.join("manifest.json");
    let payload = fs::read(&manifest_path)?;
    Ok(serde_json::from_slice(&payload)?)
}

fn read_all_manifests(base_dir: &Path) -> Result<Vec<CheckpointRecord>> {
    let mut items = Vec::new();
    let entries = match fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(items),
        Err(error) => return Err(error.into()),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let checkpoint_dir = entry.path();
        match read_manifest(&checkpoint_dir) {
            Ok(record) => items.push(record),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    checkpoint_dir = %checkpoint_dir.display(),
                    "failed to load checkpoint manifest"
                );
            },
        }
    }

    Ok(items)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checkpoint_round_trip_restores_file_content() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = CheckpointManager::new(tmp.path().to_path_buf());
        let path = tmp.path().join("target.txt");
        fs::write(&path, "before\n").unwrap();

        let checkpoint = manager.checkpoint_path(&path, "test.file").await.unwrap();
        fs::write(&path, "after\n").unwrap();

        manager.restore(&checkpoint.id).await.unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "before\n");
    }

    #[tokio::test]
    async fn checkpoint_round_trip_restores_directory_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = CheckpointManager::new(tmp.path().to_path_buf());
        let dir = tmp.path().join("skill");
        fs::create_dir_all(dir.join("templates")).unwrap();
        fs::write(dir.join("SKILL.md"), "v1\n").unwrap();
        fs::write(dir.join("templates/prompt.txt"), "hello\n").unwrap();

        let checkpoint = manager.checkpoint_path(&dir, "test.dir").await.unwrap();
        fs::write(dir.join("SKILL.md"), "v2\n").unwrap();
        fs::remove_file(dir.join("templates/prompt.txt")).unwrap();
        fs::write(dir.join("notes.txt"), "new\n").unwrap();

        manager.restore(&checkpoint.id).await.unwrap();

        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "v1\n");
        assert_eq!(
            fs::read_to_string(dir.join("templates/prompt.txt")).unwrap(),
            "hello\n"
        );
        assert!(!dir.join("notes.txt").exists());
    }

    #[tokio::test]
    async fn restore_removes_paths_that_did_not_exist_at_checkpoint_time() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = CheckpointManager::new(tmp.path().to_path_buf());
        let path = tmp.path().join("created-later.txt");

        let checkpoint = manager.checkpoint_path(&path, "test.absent").await.unwrap();
        fs::write(&path, "hello\n").unwrap();

        manager.restore(&checkpoint.id).await.unwrap();

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn checkpoints_list_filters_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = CheckpointManager::new(tmp.path().to_path_buf());
        let alpha = tmp.path().join("alpha.txt");
        let beta = tmp.path().join("beta.txt");
        fs::write(&alpha, "alpha\n").unwrap();
        fs::write(&beta, "beta\n").unwrap();

        manager.checkpoint_path(&alpha, "alpha").await.unwrap();
        manager.checkpoint_path(&beta, "beta").await.unwrap();

        let filtered = manager.list(20, Some("beta")).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].source_path.ends_with("beta.txt"));
    }

    #[tokio::test]
    async fn restore_rejects_non_checkpoint_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = CheckpointManager::new(tmp.path().to_path_buf());

        let error = manager.restore("../../etc/passwd").await.unwrap_err();
        assert!(error.to_string().contains("invalid checkpoint id"));
    }
}
