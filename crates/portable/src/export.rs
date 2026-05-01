//! Export Moltis data into a `.tar.gz` archive.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use {
    flate2::{Compression, write::GzEncoder},
    sha2::{Digest, Sha256},
    tar::{Builder, Header},
    tracing::{debug, info},
    walkdir::WalkDir,
};

use crate::manifest::{ArchiveInventory, ExportManifest, FORMAT_VERSION};

/// Drop guard that removes a temporary file when it goes out of scope,
/// ensuring cleanup even on early `?` returns.
struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Options controlling what gets included in the export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Include provider API keys (provider_keys.json). Default: true.
    pub include_provider_keys: bool,
    /// Include session media (audio, images). Default: false.
    pub include_media: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_provider_keys: true,
            include_media: false,
        }
    }
}

/// Auth tables to clear from the exported moltis.db snapshot.
const AUTH_TABLES_TO_CLEAR: &[&str] = &[
    "auth_password",
    "auth_sessions",
    "api_keys",
    "passkeys",
    "device_pairing",
    "session_shares",
];

/// Export a `.tar.gz` archive to the given writer.
///
/// Uses `VACUUM INTO` for SQLite snapshots so the live DB is never locked.
pub async fn export_archive<W: Write>(
    config_dir: &Path,
    data_dir: &Path,
    opts: &ExportOptions,
    writer: W,
) -> anyhow::Result<ExportManifest> {
    let encoder = GzEncoder::new(writer, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut inventory = ArchiveInventory::default();

    let prefix = archive_prefix();

    // ── Config files ─────────────────────────────────────────────────
    add_config_file(
        &mut builder,
        config_dir,
        "moltis.toml",
        &prefix,
        &mut inventory,
    )?;
    add_config_file(
        &mut builder,
        config_dir,
        "mcp-servers.json",
        &prefix,
        &mut inventory,
    )?;
    if opts.include_provider_keys {
        add_config_file(
            &mut builder,
            config_dir,
            "provider_keys.json",
            &prefix,
            &mut inventory,
        )?;
    }

    // ── Workspace markdown ───────────────────────────────────────────
    let workspace_files = [
        "SOUL.md",
        "IDENTITY.md",
        "USER.md",
        "BOOT.md",
        "TOOLS.md",
        "MEMORY.md",
        "AGENTS.md",
        "HEARTBEAT.md",
    ];
    for name in workspace_files {
        add_workspace_file(&mut builder, data_dir, name, &prefix, &mut inventory)?;
    }

    // Agent workspace sub-directories.
    let agents_dir = data_dir.join("agents");
    if agents_dir.is_dir() {
        for entry in WalkDir::new(&agents_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let rel = entry.path().strip_prefix(data_dir).unwrap_or(entry.path());
                let archive_path = format!("{prefix}/workspace/{}", rel.display());
                add_file_to_tar(&mut builder, entry.path(), &archive_path)?;
                inventory.workspace_files.push(rel.display().to_string());
            }
        }
    }

    // ── SQLite databases ─────────────────────────────────────────────
    let moltis_db = data_dir.join("moltis.db");
    if moltis_db.exists() {
        let snapshot = vacuum_snapshot(&moltis_db).await?;
        let _guard = TempFileGuard(snapshot.clone());
        strip_auth_tables(&snapshot).await?;
        add_file_to_tar(&mut builder, &snapshot, &format!("{prefix}/db/moltis.db"))?;
        inventory.has_moltis_db = true;
        info!("exported moltis.db snapshot");
    }

    let memory_db = data_dir.join("memory.db");
    if memory_db.exists() {
        let snapshot = vacuum_snapshot(&memory_db).await?;
        let _guard = TempFileGuard(snapshot.clone());
        add_file_to_tar(&mut builder, &snapshot, &format!("{prefix}/db/memory.db"))?;
        inventory.has_memory_db = true;
        info!("exported memory.db snapshot");
    }

    // ── Session JSONL files ──────────────────────────────────────────
    let sessions_dir = data_dir.join("sessions");
    if sessions_dir.is_dir() {
        for entry in WalkDir::new(&sessions_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&sessions_dir)
                .unwrap_or(entry.path());
            let rel_str = rel.display().to_string();

            let is_media = rel_str.starts_with("media/") || rel_str.starts_with("media\\");

            if is_media && !opts.include_media {
                continue;
            }

            let archive_path = format!("{prefix}/sessions/{rel_str}");
            add_file_to_tar(&mut builder, entry.path(), &archive_path)?;

            if is_media {
                inventory.media_files.push(rel_str);
            } else {
                inventory.session_files.push(rel_str);
            }
        }
    }

    // ── Manifest (written last so it has the full inventory) ─────────
    let now = time::OffsetDateTime::now_utc();
    let created_at = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into());

    let moltis_version =
        std::env::var("MOLTIS_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into());

    let manifest = ExportManifest {
        format_version: FORMAT_VERSION,
        moltis_version,
        created_at,
        inventory,
    };

    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    let mut header = Header::new_gnu();
    header.set_size(manifest_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(
        &mut header,
        format!("{prefix}/manifest.json"),
        manifest_json.as_slice(),
    )?;

    builder.into_inner()?.finish()?;
    info!(
        sessions = manifest.inventory.session_count(),
        media = manifest.inventory.media_count(),
        "export complete"
    );
    Ok(manifest)
}

/// Generate a timestamped archive prefix.
fn archive_prefix() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "moltis-backup-{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

/// Add a config-dir file to the archive if it exists.
fn add_config_file<W: Write>(
    builder: &mut Builder<W>,
    config_dir: &Path,
    name: &str,
    prefix: &str,
    inventory: &mut ArchiveInventory,
) -> anyhow::Result<()> {
    let path = config_dir.join(name);
    if path.is_file() {
        add_file_to_tar(builder, &path, &format!("{prefix}/config/{name}"))?;
        inventory.config_files.push(name.to_owned());
        debug!(file = name, "added config file");
    }
    Ok(())
}

/// Add a workspace (data_dir root) file to the archive if it exists.
fn add_workspace_file<W: Write>(
    builder: &mut Builder<W>,
    data_dir: &Path,
    name: &str,
    prefix: &str,
    inventory: &mut ArchiveInventory,
) -> anyhow::Result<()> {
    let path = data_dir.join(name);
    if path.is_file() {
        add_file_to_tar(builder, &path, &format!("{prefix}/workspace/{name}"))?;
        inventory.workspace_files.push(name.to_owned());
        debug!(file = name, "added workspace file");
    }
    Ok(())
}

/// Append a file from disk into the tar archive.
fn add_file_to_tar<W: Write>(
    builder: &mut Builder<W>,
    source: &Path,
    archive_path: &str,
) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(source)?;
    let mut header = Header::new_gnu();
    header.set_size(metadata.len());
    header.set_mode(0o644);
    header.set_cksum();

    let file = std::fs::File::open(source)?;
    builder.append_data(&mut header, archive_path, file)?;
    Ok(())
}

/// Create a consistent snapshot of a SQLite database via `VACUUM INTO`.
async fn vacuum_snapshot(db_path: &Path) -> anyhow::Result<PathBuf> {
    let snapshot_path = db_path.with_extension("db.export-snapshot");

    // Remove stale snapshot if present.
    if snapshot_path.exists() {
        std::fs::remove_file(&snapshot_path)?;
    }

    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;

    let snapshot_str = snapshot_path.display().to_string();
    let escaped = snapshot_str.replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{escaped}'"))
        .execute(&pool)
        .await?;
    pool.close().await;

    debug!(path = %snapshot_path.display(), "created db snapshot");
    Ok(snapshot_path)
}

/// Strip authentication tables from a snapshot so secrets are never exported.
async fn strip_auth_tables(snapshot_path: &Path) -> anyhow::Result<()> {
    let db_url = format!("sqlite:{}?mode=rwc", snapshot_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;

    for table in AUTH_TABLES_TO_CLEAR {
        // Tables may not exist if the user never set up auth.
        let result = sqlx::query(&format!("DELETE FROM [{table}]"))
            .execute(&pool)
            .await;
        match result {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    debug!(table, rows = r.rows_affected(), "cleared auth table");
                }
            },
            Err(e) => {
                // Table doesn't exist — fine.
                debug!(table, error = %e, "skipped auth table (may not exist)");
            },
        }
    }

    pool.close().await;
    Ok(())
}

/// Compute SHA-256 of a file (unused in export itself, useful for manifest extensions).
#[allow(dead_code)]
fn sha256_file(path: &Path) -> anyhow::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_prefix_format() {
        let prefix = archive_prefix();
        assert!(prefix.starts_with("moltis-backup-"));
        // e.g. moltis-backup-20260501-143022
        assert_eq!(prefix.len(), "moltis-backup-YYYYMMDD-HHMMSS".len());
    }
}
