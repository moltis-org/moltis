//! Import a `.tar.gz` archive into a Moltis instance.

use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path},
};

use {
    cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt},
    cap_std::fs::{Dir, OpenOptions},
    flate2::read::GzDecoder,
    serde::{Deserialize, Serialize},
    tar::Archive,
    tracing::{debug, info, warn},
};

use crate::manifest::{
    ExportManifest, FORMAT_VERSION, MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_PATH_METADATA_BYTES,
    MAX_COMPRESSED_ARCHIVE_BYTES, MAX_MANAGED_ENTRIES, MAX_MANAGED_FILE_BYTES,
    MAX_MANAGED_TOTAL_BYTES, MAX_MANAGED_TOTAL_PATH_BYTES, MAX_UNCOMPRESSED_ARCHIVE_BYTES,
};

/// Drop guard that removes a temporary file when it goes out of scope.
struct TempFileGuard(std::path::PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct ManagedTempGuard<'a> {
    parent: &'a Dir,
    name: String,
    armed: bool,
}

impl ManagedTempGuard<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ManagedTempGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.parent.remove_file(&self.name);
        }
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
    let managed_files_dir = data_dir.join("files");
    import_archive_into_dirs(config_dir, data_dir, &managed_files_dir, opts, reader).await
}

pub(crate) async fn import_archive_into_dirs<R: Read>(
    config_dir: &Path,
    data_dir: &Path,
    managed_files_dir: &Path,
    opts: &ImportOptions,
    mut reader: R,
) -> anyhow::Result<ImportResult> {
    // Stage every import, including previews, so compressed-size validation and
    // parsing behavior are identical between dry-run and apply.
    let mut staged_archive = tempfile::tempfile()?;
    let mut limited_reader = reader.by_ref().take(MAX_COMPRESSED_ARCHIVE_BYTES + 1);
    let compressed_bytes = std::io::copy(&mut limited_reader, &mut staged_archive)?;
    if compressed_bytes > MAX_COMPRESSED_ARCHIVE_BYTES {
        anyhow::bail!("archive exceeds the 2 GiB compressed import limit");
    }
    staged_archive.seek(SeekFrom::Start(0))?;
    if opts.dry_run {
        return process_archive_into_dirs(
            config_dir,
            data_dir,
            managed_files_dir,
            opts,
            staged_archive,
        )
        .await;
    }

    // Validate the complete archive before applying any entry. The manifest is
    // intentionally written last by exports, so one-pass extraction could
    // otherwise mutate data before discovering a truncated or future archive.
    let validation_options = ImportOptions {
        conflict: opts.conflict,
        dry_run: true,
    };
    process_archive_into_dirs(
        config_dir,
        data_dir,
        managed_files_dir,
        &validation_options,
        &mut staged_archive,
    )
    .await?;
    staged_archive.seek(SeekFrom::Start(0))?;
    process_archive_into_dirs(
        config_dir,
        data_dir,
        managed_files_dir,
        opts,
        staged_archive,
    )
    .await
}

async fn process_archive_into_dirs<R: Read>(
    config_dir: &Path,
    data_dir: &Path,
    managed_files_dir: &Path,
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
    let mut managed_root: Option<Option<Dir>> = None;
    let mut managed_paths = HashSet::new();
    let mut managed_file_paths = HashSet::new();
    let mut managed_directory_paths = HashSet::new();
    let mut managed_entry_kinds = HashMap::new();
    let mut managed_total_bytes = 0u64;
    let mut managed_total_path_bytes = 0usize;
    let mut uncompressed_bytes = 0u64;
    let mut entry_count = 0usize;
    let mut pending_long_path = None;

    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            anyhow::bail!("archive exceeds the {MAX_ARCHIVE_ENTRIES} entry limit");
        }
        uncompressed_bytes = uncompressed_bytes
            .checked_add(entry.size())
            .ok_or_else(|| anyhow::anyhow!("archive uncompressed size exceeds supported range"))?;
        if uncompressed_bytes > MAX_UNCOMPRESSED_ARCHIVE_BYTES {
            anyhow::bail!("archive exceeds the 16 GiB uncompressed import limit");
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_gnu_longname() {
            if pending_long_path.is_some() {
                anyhow::bail!("archive contains consecutive GNU long path entries");
            }
            if entry.size() > MAX_ARCHIVE_PATH_METADATA_BYTES {
                anyhow::bail!("archive path metadata exceeds the 8 KiB limit");
            }
            let mut path = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut path)?;
            while path.last() == Some(&0) {
                path.pop();
            }
            pending_long_path = Some(Path::new(std::str::from_utf8(&path)?).to_path_buf());
            continue;
        }
        if entry_type.is_gnu_longlink()
            || entry_type.is_pax_global_extensions()
            || entry_type.is_pax_local_extensions()
        {
            anyhow::bail!("archive contains unsupported link or PAX path metadata");
        }
        let raw_path = pending_long_path
            .take()
            .unwrap_or(entry.path()?.to_path_buf());

        // Strip the top-level prefix directory (moltis-backup-YYYYMMDD-HHMMSS/).
        let stripped = strip_archive_prefix(&raw_path);

        // Validate path safety — reject traversal attacks.
        if !is_safe_path(stripped) {
            if targets_managed_files(&raw_path) {
                anyhow::bail!("invalid managed Files archive path: {}", raw_path.display());
            }
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

        if stripped_str.starts_with("files/") {
            let rel = stripped.strip_prefix("files/").unwrap_or(stripped);
            let components = validate_managed_archive_path(rel)?;
            let relative = managed_relative_string(rel)?;
            if !managed_paths.insert(relative.clone()) {
                anyhow::bail!("duplicate managed Files archive entry: {relative}");
            }
            managed_total_path_bytes = managed_total_path_bytes
                .checked_add(relative.len())
                .ok_or_else(|| {
                    anyhow::anyhow!("managed Files path data exceeds supported range")
                })?;
            if managed_total_path_bytes > MAX_MANAGED_TOTAL_PATH_BYTES {
                anyhow::bail!("managed Files archive exceeds the 16 MiB path metadata limit");
            }
            if managed_paths.len() > MAX_MANAGED_ENTRIES {
                anyhow::bail!(
                    "managed Files archive exceeds the {MAX_MANAGED_ENTRIES} entry limit"
                );
            }

            if !entry_type.is_file() && !entry_type.is_dir() {
                anyhow::bail!(
                    "managed Files archive entry is not a regular file or directory: {relative}"
                );
            }
            record_managed_entry_shape(&components, entry_type.is_dir(), &mut managed_entry_kinds)?;
            if entry_type.is_file() {
                let size = entry.size();
                if size > MAX_MANAGED_FILE_BYTES {
                    anyhow::bail!("managed Files entry exceeds the 1 GiB limit: {relative}");
                }
                managed_total_bytes = managed_total_bytes.checked_add(size).ok_or_else(|| {
                    anyhow::anyhow!("managed Files archive size exceeds supported range")
                })?;
                if managed_total_bytes > MAX_MANAGED_TOTAL_BYTES {
                    anyhow::bail!("managed Files archive exceeds the 8 GiB total size limit");
                }
                managed_file_paths.insert(relative.clone());
            } else {
                managed_directory_paths.insert(relative.clone());
            }

            if managed_root.is_none() {
                managed_root = Some(open_managed_root(managed_files_dir, !opts.dry_run)?);
            }
            let root = managed_root.as_ref().and_then(Option::as_ref);
            if entry_type.is_dir() {
                ensure_managed_directory(root, &components, !opts.dry_run)?;
                continue;
            }

            let action = apply_managed_file(&mut entry, root, &components, opts)?;
            record_action(&action, "files", &stripped_str, &mut imported, &mut skipped);
            continue;
        }

        debug!(path = %stripped_str, "ignoring unknown archive entry");
    }

    if pending_long_path.is_some() {
        anyhow::bail!("archive ends with GNU long path metadata without a following entry");
    }

    let manifest = manifest.ok_or_else(|| anyhow::anyhow!("archive missing manifest.json"))?;
    validate_managed_inventory(
        &manifest,
        &managed_file_paths,
        &managed_directory_paths,
        managed_total_bytes,
    )?;

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

fn targets_managed_files(path: &Path) -> bool {
    matches!(
        path.components().nth(1),
        Some(Component::Normal(component)) if component == "files"
    )
}

/// Reject paths that could escape the target directory.
pub(crate) fn is_safe_path(path: &Path) -> bool {
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
    DryRunCreate,
    DryRunSkip,
    DryRunOverwrite,
}

fn apply_file<R: Read>(
    reader: &mut R,
    dest: &Path,
    opts: &ImportOptions,
) -> anyhow::Result<FileAction> {
    if opts.dry_run {
        return Ok(match (dest.exists(), opts.conflict) {
            (true, ConflictStrategy::Skip) => FileAction::DryRunSkip,
            (true, ConflictStrategy::Overwrite) => FileAction::DryRunOverwrite,
            (false, _) => FileAction::DryRunCreate,
        });
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
        FileAction::DryRunCreate => {
            skipped.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: "would create".into(),
            });
        },
        FileAction::DryRunSkip => {
            skipped.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: "would skip (exists)".into(),
            });
        },
        FileAction::DryRunOverwrite => {
            skipped.push(ImportedItem {
                category: category.into(),
                path: path.into(),
                action: "would overwrite".into(),
            });
        },
    }
}

const MAX_MANAGED_PATH_DEPTH: usize = 64;
const MAX_MANAGED_PATH_BYTES: usize = 4096;
const MAX_MANAGED_COMPONENT_BYTES: usize = 255;
const MANAGED_TEMP_PREFIX: &str = ".moltis-upload-";

fn record_managed_entry_shape(
    components: &[String],
    is_directory: bool,
    entry_kinds: &mut HashMap<String, bool>,
) -> anyhow::Result<()> {
    let mut current = String::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(component);
        if entry_kinds.get(&current) == Some(&false) {
            anyhow::bail!("managed Files archive path crosses a file: {current}");
        }
        entry_kinds.entry(current.clone()).or_insert(true);
    }

    let path = components.join("/");
    if !is_directory && entry_kinds.get(&path) == Some(&true) {
        anyhow::bail!("managed Files archive file conflicts with a directory: {path}");
    }
    entry_kinds.insert(path, is_directory);
    Ok(())
}

fn validate_managed_inventory(
    manifest: &ExportManifest,
    actual_files: &HashSet<String>,
    actual_directories: &HashSet<String>,
    actual_total_bytes: u64,
) -> anyhow::Result<()> {
    let inventory = &manifest.inventory.managed_files;
    let expected_files = inventory.files.iter().cloned().collect::<HashSet<_>>();
    let expected_directories = inventory
        .directories
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if expected_files.len() != inventory.files.len()
        || expected_directories.len() != inventory.directories.len()
    {
        anyhow::bail!("managed Files manifest inventory contains duplicate paths");
    }
    if &expected_files != actual_files
        || &expected_directories != actual_directories
        || inventory.total_bytes != actual_total_bytes
    {
        anyhow::bail!("managed Files archive entries do not match the manifest inventory");
    }
    Ok(())
}

fn validate_managed_archive_path(path: &Path) -> anyhow::Result<Vec<String>> {
    if !is_safe_path(path) || path.as_os_str().is_empty() {
        anyhow::bail!("invalid managed Files archive path: {}", path.display());
    }
    let raw = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("managed Files archive path is not valid UTF-8"))?;
    if raw.len() > MAX_MANAGED_PATH_BYTES || raw.contains(['\\', '\0']) {
        anyhow::bail!("invalid managed Files archive path: {raw:?}");
    }
    let components = raw.split('/').map(str::to_owned).collect::<Vec<_>>();
    if components.len() > MAX_MANAGED_PATH_DEPTH
        || components.iter().any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || component.len() > MAX_MANAGED_COMPONENT_BYTES
                || component.starts_with(MANAGED_TEMP_PREFIX)
                || looks_like_windows_prefix(component)
        })
    {
        anyhow::bail!("invalid managed Files archive path: {raw:?}");
    }
    Ok(components)
}

fn managed_relative_string(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| anyhow::anyhow!("managed Files archive path is not valid UTF-8"))
}

fn looks_like_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn open_managed_root(root_path: &Path, create: bool) -> anyhow::Result<Option<Dir>> {
    let parent_path = root_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("managed Files root has no parent"))?;
    let name = root_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("managed Files root has no directory name"))?;
    let parent = match Dir::open_ambient_dir(parent_path, cap_std::ambient_authority()) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            parent.create_dir(name)?;
            parent.symlink_metadata(name)?
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() || !metadata.is_dir() {
        anyhow::bail!("managed Files root must be a directory and cannot be a symlink");
    }
    Ok(Some(parent.open_dir_nofollow(name)?))
}

fn ensure_managed_directory(
    root: Option<&Dir>,
    components: &[String],
    create: bool,
) -> anyhow::Result<Option<Dir>> {
    let Some(root) = root else {
        return Ok(None);
    };
    let mut directory = root.try_clone()?;
    for component in components {
        let metadata = match directory.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                directory.create_dir(component)?;
                directory.symlink_metadata(component)?
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() || !metadata.is_dir() {
            anyhow::bail!(
                "managed Files import path crosses a non-directory or symlink: {component:?}"
            );
        }
        directory = directory.open_dir_nofollow(component)?;
    }
    Ok(Some(directory))
}

fn apply_managed_file<R: Read>(
    reader: &mut R,
    root: Option<&Dir>,
    components: &[String],
    opts: &ImportOptions,
) -> anyhow::Result<FileAction> {
    let (name, parents) = components
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("managed Files import cannot target its root"))?;
    let parent = ensure_managed_directory(root, parents, !opts.dry_run)?;
    let exists = parent
        .as_ref()
        .map(|parent| managed_target_exists(parent, name))
        .transpose()?
        .unwrap_or(false);
    if opts.dry_run {
        return Ok(match (exists, opts.conflict) {
            (true, ConflictStrategy::Skip) => FileAction::DryRunSkip,
            (true, ConflictStrategy::Overwrite) => FileAction::DryRunOverwrite,
            (false, _) => FileAction::DryRunCreate,
        });
    }
    if exists && opts.conflict == ConflictStrategy::Skip {
        return Ok(FileAction::Skipped);
    }

    let parent = parent.ok_or_else(|| anyhow::anyhow!("managed Files parent was not created"))?;
    let (temp_name, mut file) = create_managed_temp_file(&parent)?;
    let mut temp_guard = ManagedTempGuard {
        parent: &parent,
        name: temp_name.clone(),
        armed: true,
    };
    let stage_result = (|| -> anyhow::Result<()> {
        if !file.metadata()?.is_file() {
            anyhow::bail!("managed Files staging entry is not a regular file");
        }
        std::io::copy(reader, &mut file)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    stage_result?;

    let exists_at_commit = managed_target_exists(&parent, name)?;
    if !exists && exists_at_commit && opts.conflict == ConflictStrategy::Skip {
        return Ok(FileAction::Skipped);
    }
    commit_managed_temp(&parent, &temp_name, name, exists_at_commit)?;
    temp_guard.disarm();

    if exists || exists_at_commit {
        Ok(FileAction::Overwritten)
    } else {
        Ok(FileAction::Created)
    }
}

fn managed_target_exists(parent: &Dir, name: &str) -> anyhow::Result<bool> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() || !metadata.is_file() {
        anyhow::bail!(
            "managed Files import target is not a regular file or is a symlink: {name:?}"
        );
    }
    Ok(true)
}

fn create_managed_temp_file(parent: &Dir) -> anyhow::Result<(String, cap_std::fs::File)> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    for _ in 0..8 {
        let name = format!(
            "{MANAGED_TEMP_PREFIX}import-{}.tmp",
            uuid::Uuid::new_v4().simple()
        );
        match parent.open_with(&name, &options) {
            Ok(file) => return Ok((name, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("failed to allocate a unique managed Files staging name")
}

#[cfg(not(windows))]
fn commit_managed_temp(
    parent: &Dir,
    temp_name: &str,
    destination_name: &str,
    destination_exists: bool,
) -> anyhow::Result<()> {
    if !destination_exists {
        parent.hard_link(temp_name, parent, destination_name)?;
        parent.remove_file(temp_name)?;
        return Ok(());
    }
    parent.rename(temp_name, parent, destination_name)?;
    Ok(())
}

#[cfg(windows)]
fn commit_managed_temp(
    parent: &Dir,
    temp_name: &str,
    destination_name: &str,
    destination_exists: bool,
) -> anyhow::Result<()> {
    if !destination_exists {
        parent.hard_link(temp_name, parent, destination_name)?;
        parent.remove_file(temp_name)?;
        return Ok(());
    }

    let backup_name = format!(
        "{MANAGED_TEMP_PREFIX}backup-{}.tmp",
        uuid::Uuid::new_v4().simple()
    );
    parent.rename(destination_name, parent, &backup_name)?;
    if let Err(commit_error) = parent.rename(temp_name, parent, destination_name) {
        if let Err(restore_error) = parent.rename(&backup_name, parent, destination_name) {
            anyhow::bail!(
                "failed to commit managed Files import ({commit_error}) and restore original ({restore_error})"
            );
        }
        return Err(commit_error.into());
    }
    if let Err(error) = parent.remove_file(&backup_name) {
        warn!(%error, "failed to remove managed Files import backup");
    }
    Ok(())
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

    struct FailAfterFirstRead {
        delivered: bool,
    }

    impl Read for FailAfterFirstRead {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.delivered {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "simulated truncated archive entry",
                ));
            }
            self.delivered = true;
            let content = b"partial replacement";
            let length = buffer.len().min(content.len());
            buffer[..length].copy_from_slice(&content[..length]);
            Ok(length)
        }
    }

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

    #[test]
    fn managed_path_rejects_excessive_depth_and_internal_names() {
        let deep_path = std::iter::repeat_n("nested", MAX_MANAGED_PATH_DEPTH + 1)
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_managed_archive_path(Path::new(&deep_path)).is_err());
        assert!(
            validate_managed_archive_path(Path::new("notes/.moltis-upload-pending.tmp")).is_err()
        );
    }

    #[test]
    fn managed_file_read_failure_preserves_original_and_cleans_temp() {
        let data_dir = tempfile::tempdir().unwrap();
        let managed_root = data_dir.path().join("files");
        std::fs::create_dir_all(&managed_root).unwrap();
        let destination = managed_root.join("existing.txt");
        std::fs::write(&destination, "original contents").unwrap();
        let root = open_managed_root(&managed_root, false).unwrap().unwrap();
        let mut reader = FailAfterFirstRead { delivered: false };

        let result = apply_managed_file(
            &mut reader,
            Some(&root),
            &["existing.txt".to_owned()],
            &ImportOptions {
                conflict: ConflictStrategy::Overwrite,
                dry_run: false,
            },
        );

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "original contents"
        );
        assert!(std::fs::read_dir(managed_root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(MANAGED_TEMP_PREFIX)
        }));
    }
}
