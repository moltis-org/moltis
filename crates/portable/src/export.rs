//! Export Moltis data into a `.tar.gz` archive.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use {
    cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt},
    cap_std::fs::{Dir, OpenOptions},
    flate2::{Compression, write::GzEncoder},
    sha2::{Digest, Sha256},
    tar::{Builder, Header},
    tracing::{debug, info, warn},
    walkdir::WalkDir,
};

use crate::manifest::{
    ArchiveInventory, ExportManifest, FORMAT_VERSION, MAX_ARCHIVE_ENTRIES,
    MAX_COMPRESSED_ARCHIVE_BYTES, MAX_MANAGED_ENTRIES, MAX_MANAGED_FILE_BYTES,
    MAX_MANAGED_TOTAL_BYTES, MAX_MANAGED_TOTAL_PATH_BYTES, MAX_UNCOMPRESSED_ARCHIVE_BYTES,
};

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
    description: &'static str,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64, description: &'static str) -> Self {
        Self {
            inner,
            remaining: limit,
            description,
        }
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() as u64 > self.remaining {
            return Err(io::Error::other(format!(
                "{} export limit exceeded",
                self.description
            )));
        }
        let written = self.inner.write(buffer)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

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
    /// Include files managed by the Files service. Default: false.
    pub include_files: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_provider_keys: true,
            include_media: false,
            include_files: false,
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
    let managed_files_dir = data_dir.join("files");
    export_archive_from_dirs(config_dir, data_dir, &managed_files_dir, opts, writer).await
}

pub(crate) async fn export_archive_from_dirs<W: Write>(
    config_dir: &Path,
    data_dir: &Path,
    managed_files_dir: &Path,
    opts: &ExportOptions,
    writer: W,
) -> anyhow::Result<ExportManifest> {
    let compressed_writer = LimitedWriter::new(
        writer,
        MAX_COMPRESSED_ARCHIVE_BYTES,
        "2 GiB compressed archive",
    );
    let encoder = GzEncoder::new(compressed_writer, Compression::default());
    let archive_writer = LimitedWriter::new(
        encoder,
        MAX_UNCOMPRESSED_ARCHIVE_BYTES,
        "16 GiB uncompressed archive",
    );
    let mut builder = Builder::new(archive_writer);
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

    // ── Managed Files ────────────────────────────────────────────────
    if opts.include_files {
        add_managed_files(&mut builder, managed_files_dir, &prefix, &mut inventory)?;
    }

    let entries_for = |category: &str, paths: &[String]| {
        paths
            .iter()
            .map(|path| tar_member_count(&format!("{prefix}/{category}/{path}")))
            .sum::<usize>()
    };
    let entry_count = entries_for("config", &inventory.config_files)
        + entries_for("workspace", &inventory.workspace_files)
        + entries_for("sessions", &inventory.session_files)
        + entries_for("sessions", &inventory.media_files)
        + entries_for("files", &inventory.managed_files.files)
        + entries_for("files", &inventory.managed_files.directories)
        + usize::from(inventory.has_moltis_db)
            * tar_member_count(&format!("{prefix}/db/moltis.db"))
        + usize::from(inventory.has_memory_db)
            * tar_member_count(&format!("{prefix}/db/memory.db"))
        + tar_member_count(&format!("{prefix}/manifest.json"));
    if entry_count > MAX_ARCHIVE_ENTRIES {
        anyhow::bail!("export exceeds the {MAX_ARCHIVE_ENTRIES} archive entry limit");
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

    let archive_writer = builder.into_inner()?;
    archive_writer.into_inner().finish()?;
    info!(
        sessions = manifest.inventory.session_count(),
        media = manifest.inventory.media_count(),
        files = manifest.inventory.managed_files.files.len(),
        files_bytes = manifest.inventory.managed_files.total_bytes,
        "export complete"
    );
    Ok(manifest)
}

const MAX_MANAGED_PATH_DEPTH: usize = 64;
const MAX_MANAGED_PATH_BYTES: usize = 4096;
const MAX_MANAGED_COMPONENT_BYTES: usize = 255;
const MANAGED_TEMP_PREFIX: &str = ".moltis-upload-";

fn add_managed_files<W: Write>(
    builder: &mut Builder<W>,
    root_path: &Path,
    prefix: &str,
    inventory: &mut ArchiveInventory,
) -> anyhow::Result<()> {
    let Some(root) = open_managed_root(root_path)? else {
        return Ok(());
    };
    let mut total_path_bytes = 0usize;
    add_managed_directory(
        builder,
        &root,
        Path::new(""),
        prefix,
        inventory,
        &mut total_path_bytes,
        0,
    )
}

fn open_managed_root(root_path: &Path) -> anyhow::Result<Option<Dir>> {
    let parent = root_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("managed Files root has no parent"))?;
    let name = root_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("managed Files root has no directory name"))?;
    let parent = match Dir::open_ambient_dir(parent, cap_std::ambient_authority()) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() || !metadata.is_dir() {
        anyhow::bail!("managed Files root must be a directory and cannot be a symlink");
    }
    Ok(Some(parent.open_dir_nofollow(name)?))
}

fn add_managed_directory<W: Write>(
    builder: &mut Builder<W>,
    directory: &Dir,
    relative: &Path,
    prefix: &str,
    inventory: &mut ArchiveInventory,
    total_path_bytes: &mut usize,
    depth: usize,
) -> anyhow::Result<()> {
    if depth > MAX_MANAGED_PATH_DEPTH {
        anyhow::bail!(
            "managed Files path exceeds maximum depth of {MAX_MANAGED_PATH_DEPTH}: {}",
            relative.display()
        );
    }

    let mut names = directory
        .entries()?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();

    for name in names {
        let name = name
            .into_string()
            .map_err(|_| anyhow::anyhow!("managed Files path is not valid UTF-8"))?;
        if name.starts_with(MANAGED_TEMP_PREFIX) {
            warn!(
                name,
                "skipping internal managed Files staging entry during export"
            );
            continue;
        }
        validate_managed_component(&name)?;
        let child_relative = relative.join(&name);
        validate_managed_relative_path(&child_relative)?;
        let metadata = directory.symlink_metadata(&name)?;
        if metadata.is_symlink() {
            anyhow::bail!(
                "managed Files export does not allow symlinks: {}",
                child_relative.display()
            );
        }

        let relative_string = managed_relative_string(&child_relative)?;
        *total_path_bytes = total_path_bytes
            .checked_add(relative_string.len())
            .ok_or_else(|| anyhow::anyhow!("managed Files path data exceeds supported range"))?;
        if *total_path_bytes > MAX_MANAGED_TOTAL_PATH_BYTES {
            anyhow::bail!("managed Files export exceeds the 16 MiB path metadata limit");
        }
        if inventory.managed_files.files.len() + inventory.managed_files.directories.len()
            >= MAX_MANAGED_ENTRIES
        {
            anyhow::bail!("managed Files export exceeds the {MAX_MANAGED_ENTRIES} entry limit");
        }
        let archive_path = format!("{prefix}/files/{relative_string}");
        if metadata.is_dir() {
            let child = directory.open_dir_nofollow(&name)?;
            add_directory_to_tar(builder, &archive_path)?;
            inventory.managed_files.directories.push(relative_string);
            add_managed_directory(
                builder,
                &child,
                &child_relative,
                prefix,
                inventory,
                total_path_bytes,
                depth + 1,
            )?;
            continue;
        }
        if !metadata.is_file() {
            anyhow::bail!(
                "managed Files export only supports regular files and directories: {}",
                child_relative.display()
            );
        }

        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory.open_with(&name, &options)?;
        let opened_metadata = file.metadata()?;
        if !opened_metadata.is_file() {
            anyhow::bail!(
                "managed Files entry changed while exporting: {}",
                child_relative.display()
            );
        }
        if opened_metadata.len() > MAX_MANAGED_FILE_BYTES {
            anyhow::bail!(
                "managed Files entry exceeds the 1 GiB export limit: {}",
                child_relative.display()
            );
        }
        let next_total = inventory
            .managed_files
            .total_bytes
            .checked_add(opened_metadata.len())
            .ok_or_else(|| anyhow::anyhow!("managed Files size exceeds supported range"))?;
        if next_total > MAX_MANAGED_TOTAL_BYTES {
            anyhow::bail!("managed Files export exceeds the 8 GiB total size limit");
        }
        add_open_file_to_tar(
            builder,
            file.into_std(),
            opened_metadata.len(),
            &archive_path,
        )?;
        inventory.managed_files.files.push(relative_string);
        inventory.managed_files.total_bytes = next_total;
    }
    Ok(())
}

fn validate_managed_component(component: &str) -> anyhow::Result<()> {
    let invalid = component.is_empty()
        || component == "."
        || component == ".."
        || component.contains(['\\', '\0'])
        || component.len() > MAX_MANAGED_COMPONENT_BYTES
        || component.starts_with(MANAGED_TEMP_PREFIX)
        || looks_like_windows_prefix(component);
    if invalid {
        anyhow::bail!("invalid managed Files path component: {component:?}");
    }
    Ok(())
}

fn validate_managed_relative_path(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().len() > MAX_MANAGED_PATH_BYTES
        || path.components().count() > MAX_MANAGED_PATH_DEPTH
        || !super::import::is_safe_path(path)
    {
        anyhow::bail!("invalid managed Files path: {}", path.display());
    }
    Ok(())
}

fn managed_relative_string(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| anyhow::anyhow!("managed Files path is not valid UTF-8"))
}

fn looks_like_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn add_directory_to_tar<W: Write>(
    builder: &mut Builder<W>,
    archive_path: &str,
) -> anyhow::Result<()> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, archive_path, io::empty())?;
    Ok(())
}

fn tar_member_count(path: &str) -> usize {
    1 + usize::from(path.len() >= 100)
}

fn add_open_file_to_tar<W: Write>(
    builder: &mut Builder<W>,
    file: std::fs::File,
    size: u64,
    archive_path: &str,
) -> anyhow::Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, archive_path, file)?;
    Ok(())
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn archive_prefix_format() {
        let prefix = archive_prefix();
        assert!(prefix.starts_with("moltis-backup-"));
        // e.g. moltis-backup-20260501-143022
        assert_eq!(prefix.len(), "moltis-backup-YYYYMMDD-HHMMSS".len());
    }

    #[test]
    fn limited_writer_rejects_a_write_past_its_limit() {
        let mut writer = LimitedWriter::new(Vec::new(), 4, "test");
        writer.write_all(b"four").unwrap();
        let error = writer.write_all(b"!").unwrap_err();
        assert!(error.to_string().contains("test export limit exceeded"));
        assert_eq!(writer.into_inner(), b"four");
    }

    #[test]
    fn long_paths_count_their_gnu_metadata_member() {
        assert_eq!(tar_member_count(&"a".repeat(99)), 1);
        assert_eq!(tar_member_count(&"a".repeat(100)), 2);
    }
}
