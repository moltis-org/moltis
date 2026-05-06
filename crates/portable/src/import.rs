//! Import a `.tar.gz` archive into a Moltis instance.

use std::{
    io::Read,
    path::{Component, Path},
};

use {
    flate2::read::GzDecoder,
    serde::{Deserialize, Serialize},
    tar::Archive,
    tracing::{debug, info, warn},
};

use crate::manifest::{ExportManifest, FORMAT_VERSION};

/// Drop guard that removes a temporary file when it goes out of scope.
struct TempFileGuard(std::path::PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// How to handle conflicts with existing data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictStrategy {
    /// Keep existing data, skip conflicting imports.
    #[default]
    Skip,
    /// Replace existing data with imported data.
    Overwrite,
}

/// Options for importing an archive.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub conflict: ConflictStrategy,
    pub dry_run: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            conflict: ConflictStrategy::Skip,
            dry_run: false,
        }
    }
}

/// A single item that was imported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedItem {
    pub category: String,
    pub path: String,
    pub action: String,
}

/// Result of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub manifest: ExportManifest,
    pub imported: Vec<ImportedItem>,
    pub skipped: Vec<ImportedItem>,
    pub warnings: Vec<String>,
}

/// Import a `.tar.gz` archive into the given config and data directories.
pub async fn import_archive<R: Read>(
    config_dir: &Path,
    data_dir: &Path,
    opts: &ImportOptions,
    reader: R,
) -> anyhow::Result<ImportResult> {
    let decoder = GzDecoder::new(reader);
    let mut archive = Archive::new(decoder);

    let mut manifest: Option<ExportManifest> = None;
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    let mut db_snapshots: Vec<(String, Vec<u8>)> = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let raw_path = entry.path()?.to_path_buf();

        // Strip the top-level prefix directory (moltis-backup-YYYYMMDD-HHMMSS/).
        let stripped = strip_archive_prefix(&raw_path);

        // Validate path safety — reject traversal attacks.
        if !is_safe_path(stripped) {
            warnings.push(format!("skipped unsafe path: {}", raw_path.display()));
            continue;
        }

        let stripped_str = stripped.display().to_string();

        if stripped_str == "manifest.json" {
            let m: ExportManifest = serde_json::from_reader(&mut entry)?;
            if m.format_version > FORMAT_VERSION {
                anyhow::bail!(
                    "archive format version {} is newer than supported version {FORMAT_VERSION}",
                    m.format_version
                );
            }
            manifest = Some(m);
            continue;
        }

        if stripped_str.starts_with("config/") {
            let filename = stripped.strip_prefix("config/").unwrap_or(stripped);
            let dest = config_dir.join(filename);
            let action = apply_file(&mut entry, &dest, opts)?;
            record_action(
                &action,
                "config",
                &stripped_str,
                &mut imported,
                &mut skipped,
            );
            continue;
        }

        if stripped_str.starts_with("workspace/") {
            let rel = stripped.strip_prefix("workspace/").unwrap_or(stripped);
            let dest = data_dir.join(rel);
            let action = apply_file(&mut entry, &dest, opts)?;
            record_action(
                &action,
                "workspace",
                &stripped_str,
                &mut imported,
                &mut skipped,
            );
            continue;
        }

        if stripped_str.starts_with("db/") {
            // Buffer database files in memory — they need special merge handling.
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            db_snapshots.push((stripped_str.clone(), buf));
            continue;
        }

        if stripped_str.starts_with("sessions/") {
            let rel = stripped.strip_prefix("sessions/").unwrap_or(stripped);
            let dest = data_dir.join("sessions").join(rel);
            let action = apply_file(&mut entry, &dest, opts)?;
            let category = if stripped_str.contains("media/") {
                "media"
            } else {
                "session"
            };
            record_action(
                &action,
                category,
                &stripped_str,
                &mut imported,
                &mut skipped,
            );
            continue;
        }

        debug!(path = %stripped_str, "ignoring unknown archive entry");
    }

    let manifest = manifest.ok_or_else(|| anyhow::anyhow!("archive missing manifest.json"))?;

    // ── Merge SQLite databases ───────────────────────────────────────
    if !opts.dry_run {
        for (archive_path, data) in &db_snapshots {
            if archive_path == "db/moltis.db" {
                match merge_moltis_db(data_dir, data, opts.conflict).await {
                    Ok(counts) => {
                        for (table, n) in &counts {
                            if *n > 0 {
                                imported.push(ImportedItem {
                                    category: "database".into(),
                                    path: format!("moltis.db/{table}"),
                                    action: format!("merged {n} rows"),
                                });
                            }
                        }
                    },
                    Err(e) => {
                        warnings.push(format!("failed to merge moltis.db: {e}"));
                    },
                }
            } else if archive_path == "db/memory.db" {
                match apply_memory_db(data_dir, data, opts.conflict).await {
                    Ok(action) => {
                        imported.push(ImportedItem {
                            category: "database".into(),
                            path: "memory.db".into(),
                            action,
                        });
                    },
                    Err(e) => {
                        warnings.push(format!("failed to import memory.db: {e}"));
                    },
                }
            }
        }
    } else {
        for (archive_path, _) in &db_snapshots {
            skipped.push(ImportedItem {
                category: "database".into(),
                path: archive_path.clone(),
                action: "dry-run".into(),
            });
        }
    }

    info!(
        imported = imported.len(),
        skipped = skipped.len(),
        warnings = warnings.len(),
        "import complete"
    );

    Ok(ImportResult {
        manifest,
        imported,
        skipped,
        warnings,
    })
}

/// Strip the top-level archive directory prefix.
fn strip_archive_prefix(path: &Path) -> &Path {
    let mut components = path.components();
    // Skip the first component (the dated prefix directory).
    if let Some(Component::Normal(_)) = components.next() {
        components.as_path()
    } else {
        path
    }
}

/// Reject paths that could escape the target directory.
fn is_safe_path(path: &Path) -> bool {
    for component in path.components() {
        match component {
            Component::Normal(_) => {},
            _ => return false,
        }
    }
    true
}

enum FileAction {
    Created,
    Overwritten,
    Skipped,
    DryRun,
}

fn apply_file<R: Read>(
    reader: &mut R,
    dest: &Path,
    opts: &ImportOptions,
) -> anyhow::Result<FileAction> {
    if opts.dry_run {
        return Ok(FileAction::DryRun);
    }

    let exists = dest.exists();
    if exists && opts.conflict == ConflictStrategy::Skip {
        return Ok(FileAction::Skipped);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::File::create(dest)?;
    std::io::copy(reader, &mut file)?;

    if exists {
        Ok(FileAction::Overwritten)
    } else {
        Ok(FileAction::Created)
    }
}

fn record_action(
    action: &FileAction,
    category: &str,
    path: &str,
    imported: &mut Vec<ImportedItem>,
    skipped: &mut Vec<ImportedItem>,
) {
    match action {
        FileAction::Created | FileAction::Overwritten => {
            let action_str = match action {
                FileAction::Created => "created",
                FileAction::Overwritten => "overwritten",
                _ => unreachable!(),
            };
            imported.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: action_str.into(),
            });
        },
        FileAction::Skipped => {
            skipped.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: "skipped (exists)".into(),
            });
        },
        FileAction::DryRun => {
            skipped.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: "dry-run".into(),
            });
        },
    }
}

/// Tables to merge from the imported moltis.db, in dependency order.
/// Auth tables are intentionally excluded.
const MERGE_TABLES: &[&str] = &[
    "projects",
    "sessions",
    "channel_sessions",
    "cron_jobs",
    "cron_runs",
    "env_variables",
    "channels",
    "agents",
    "message_log",
];

/// Merge rows from an imported moltis.db into the live database.
async fn merge_moltis_db(
    data_dir: &Path,
    imported_data: &[u8],
    conflict: ConflictStrategy,
) -> anyhow::Result<Vec<(String, u64)>> {
    let live_db = data_dir.join("moltis.db");
    if !live_db.exists() {
        // No live DB — just write the imported one directly.
        std::fs::write(&live_db, imported_data)?;
        return Ok(vec![("(full copy)".into(), 1)]);
    }

    // Write imported data to a temporary file, with a drop guard so it is
    // cleaned up even if an error occurs during the merge sequence.
    let import_path = data_dir.join("moltis.db.import-tmp");
    std::fs::write(&import_path, imported_data)?;
    let _guard = TempFileGuard(import_path.clone());

    let db_url = format!("sqlite:{}?mode=rwc", live_db.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;

    // Attach the imported database.
    let import_path_str = import_path.display().to_string();
    let escaped = import_path_str.replace('\'', "''");
    sqlx::query(&format!("ATTACH DATABASE '{escaped}' AS import_db"))
        .execute(&pool)
        .await?;

    let insert_mode = match conflict {
        ConflictStrategy::Skip => "INSERT OR IGNORE",
        ConflictStrategy::Overwrite => "INSERT OR REPLACE",
    };

    let mut counts = Vec::new();

    for table in MERGE_TABLES {
        // Check if the table exists in the imported DB.
        let exists: bool = sqlx::query_scalar::<_, i32>(
            "SELECT COUNT(*) FROM import_db.sqlite_master WHERE type='table' AND name=?",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false);

        if !exists {
            debug!(table, "table not in imported db, skipping");
            continue;
        }

        let result = sqlx::query(&format!(
            "{insert_mode} INTO main.[{table}] SELECT * FROM import_db.[{table}]"
        ))
        .execute(&pool)
        .await;

        match result {
            Ok(r) => {
                counts.push(((*table).to_owned(), r.rows_affected()));
                if r.rows_affected() > 0 {
                    debug!(table, rows = r.rows_affected(), "merged table");
                }
            },
            Err(e) => {
                // Schema mismatch between versions — warn but continue.
                warn!(table, error = %e, "failed to merge table");
                counts.push(((*table).to_owned(), 0));
            },
        }
    }

    sqlx::query("DETACH DATABASE import_db")
        .execute(&pool)
        .await?;
    pool.close().await;

    // _guard drops here, removing the temp file.
    Ok(counts)
}

/// Import memory.db — copy if missing, merge if exists.
async fn apply_memory_db(
    data_dir: &Path,
    imported_data: &[u8],
    conflict: ConflictStrategy,
) -> anyhow::Result<String> {
    let live_db = data_dir.join("memory.db");
    if !live_db.exists() {
        std::fs::write(&live_db, imported_data)?;
        return Ok("created (no existing memory.db)".into());
    }

    match conflict {
        ConflictStrategy::Skip => Ok("skipped (memory.db exists)".into()),
        ConflictStrategy::Overwrite => {
            std::fs::write(&live_db, imported_data)?;
            Ok("overwritten".into())
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_rejects_traversal() {
        assert!(is_safe_path(Path::new("config/moltis.toml")));
        assert!(is_safe_path(Path::new("db/moltis.db")));
        assert!(!is_safe_path(Path::new("../etc/passwd")));
        assert!(!is_safe_path(Path::new("/absolute/path")));
        assert!(!is_safe_path(Path::new("config/../../etc/passwd")));
    }

    #[test]
    fn strip_prefix_works() {
        let path = Path::new("moltis-backup-20260501-143022/config/moltis.toml");
        let stripped = strip_archive_prefix(path);
        assert_eq!(stripped, Path::new("config/moltis.toml"));
    }

    #[test]
    fn conflict_strategy_serde() {
        let json = serde_json::to_string(&ConflictStrategy::Skip).unwrap();
        assert_eq!(json, "\"skip\"");
        let parsed: ConflictStrategy = serde_json::from_str("\"overwrite\"").unwrap();
        assert_eq!(parsed, ConflictStrategy::Overwrite);
    }
}
