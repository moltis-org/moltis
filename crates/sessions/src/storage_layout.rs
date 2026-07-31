use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use {
    crate::{Error, Result},
    fd_lock::RwLock,
    uuid::Uuid,
};

pub(crate) const STORAGE_VERSION: &str = "v1";
const MIGRATED_DIR: &str = ".migrated";
const MIGRATION_LOCK: &str = ".migration.lock";
const ENCODED_COMPONENT_BYTES: usize = 120;
const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

#[must_use]
pub(crate) fn encode_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len().saturating_mul(2).saturating_add(1));
    encoded.push('k');
    for byte in key.bytes() {
        encoded.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(crate) fn decode_key_path(path: &Path) -> Option<String> {
    let mut components = path.components();
    let first = match components.next()? {
        Component::Normal(component) => component.to_str()?.strip_prefix('k')?.to_string(),
        _ => return None,
    };
    let mut encoded = first;
    for component in components {
        let Component::Normal(component) = component else {
            return None;
        };
        encoded.push_str(component.to_str()?);
    }
    String::from_utf8(hex::decode(encoded).ok()?).ok()
}

#[must_use]
pub(crate) fn history_path(base_dir: &Path, key: &str) -> PathBuf {
    base_dir
        .join(STORAGE_VERSION)
        .join(encoded_key_path(key))
        .join("history.jsonl")
}

#[must_use]
pub(crate) fn media_dir(base_dir: &Path, key: &str) -> PathBuf {
    base_dir
        .join("media")
        .join(STORAGE_VERSION)
        .join(encoded_key_path(key))
        .join("files")
}

pub(crate) fn media_reference(key: &str, filename: &str) -> Result<String> {
    validate_media_filename(filename)?;
    let encoded_path = encoded_key_components(key).join("/");
    Ok(format!(
        "media/{STORAGE_VERSION}/{encoded_path}/files/{filename}"
    ))
}

#[must_use]
pub(crate) fn resolved_media_path(base_dir: &Path, key: &str, filename: &str) -> PathBuf {
    let current = media_dir(base_dir, key).join(filename);
    if current.exists() || migration_marker(base_dir, key).exists() {
        return current;
    }

    legacy_media_dir(base_dir, key)
        .map(|dir| dir.join(filename))
        .filter(|path| is_regular_file(path))
        .unwrap_or(current)
}

pub(crate) fn validate_media_filename(filename: &str) -> Result<()> {
    if filename.is_empty() || filename.contains('\\') {
        return Err(Error::message("invalid session media filename"));
    }
    let mut components = Path::new(filename).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(Error::message("invalid session media filename")),
    }
}

pub(crate) fn ensure_migrated(base_dir: &Path, key: &str) -> Result<()> {
    let Some((legacy_history, legacy_media)) = legacy_paths(base_dir, key) else {
        return Ok(());
    };
    if !legacy_history.exists() && !legacy_media.exists() {
        return Ok(());
    }

    with_migration_lock(base_dir, || {
        let marker = migration_marker(base_dir, key);
        let state = read_migration_state(&marker)?;
        if state == Some(MigrationState::Suppressed) {
            return Ok(());
        }

        let current_history = history_path(base_dir, key);
        let legacy_bytes = if is_regular_file(&legacy_history) {
            fs::metadata(&legacy_history)?.len()
        } else {
            0
        };
        let copied_bytes = match state {
            None if legacy_bytes > 0 && !current_history.exists() => {
                if let Some(parent) = current_history.parent() {
                    create_dir_all_synced(parent)?;
                }
                copy_file_prefix_atomic(&legacy_history, &current_history, legacy_bytes)?;
                write_migration_state(&marker, MigrationState::Copied(legacy_bytes))?;
                legacy_bytes
            },
            None if legacy_bytes > 0 => {
                if !file_ranges_match(&legacy_history, 0, &current_history, 0, legacy_bytes)? {
                    return Err(Error::message(
                        "legacy and current session histories conflict",
                    ));
                }
                write_migration_state(&marker, MigrationState::Copied(legacy_bytes))?;
                legacy_bytes
            },
            None => 0,
            Some(MigrationState::Copied(previous_bytes)) => {
                if legacy_bytes < previous_bytes {
                    return Err(Error::message(
                        "legacy session history changed after migration",
                    ));
                }
                if legacy_bytes > previous_bytes {
                    reconcile_file_range(
                        &marker,
                        &legacy_history,
                        &current_history,
                        previous_bytes,
                        legacy_bytes,
                    )?;
                }
                legacy_bytes
            },
            Some(MigrationState::Suppressed) => return Ok(()),
        };

        if is_directory(&legacy_media) {
            copy_media_dir(&legacy_media, &media_dir(base_dir, key))?;
        }

        if state.is_none() && copied_bytes == 0 {
            write_migration_state(&marker, MigrationState::Copied(0))?;
        }
        Ok(())
    })
}

pub(crate) fn suppress_legacy_fallback(base_dir: &Path, key: &str) -> Result<()> {
    with_migration_lock(base_dir, || {
        write_migration_state(&migration_marker(base_dir, key), MigrationState::Suppressed)
    })
}

pub(crate) fn legacy_history_path(base_dir: &Path, key: &str) -> Option<PathBuf> {
    legacy_component(key).map(|component| base_dir.join(format!("{component}.jsonl")))
}

pub(crate) fn legacy_media_dir(base_dir: &Path, key: &str) -> Option<PathBuf> {
    legacy_component(key).map(|component| base_dir.join("media").join(component))
}

#[must_use]
pub(crate) fn migration_marker(base_dir: &Path, key: &str) -> PathBuf {
    base_dir
        .join(STORAGE_VERSION)
        .join(MIGRATED_DIR)
        .join(encoded_key_path(key))
        .join("state")
}

fn legacy_paths(base_dir: &Path, key: &str) -> Option<(PathBuf, PathBuf)> {
    Some((
        legacy_history_path(base_dir, key)?,
        legacy_media_dir(base_dir, key)?,
    ))
}

fn legacy_component(key: &str) -> Option<String> {
    let component = key.replace(':', "_");
    let mut components = Path::new(&component).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Some(component),
        _ => None,
    }
}

fn encoded_key_path(key: &str) -> PathBuf {
    encoded_key_components(key).into_iter().collect()
}

fn encoded_key_components(key: &str) -> Vec<String> {
    let encoded = hex::encode(key.as_bytes());
    if encoded.is_empty() {
        return vec!["k".to_string()];
    }
    encoded
        .as_bytes()
        .chunks(ENCODED_COMPONENT_BYTES)
        .enumerate()
        .map(|(index, chunk)| {
            let chunk = String::from_utf8_lossy(chunk);
            if index == 0 {
                format!("k{chunk}")
            } else {
                chunk.into_owned()
            }
        })
        .collect()
}

fn with_migration_lock<T>(base_dir: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let version_dir = base_dir.join(STORAGE_VERSION);
    create_dir_all_synced(&version_dir)?;
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(version_dir.join(MIGRATION_LOCK))?;
    let mut lock = RwLock::new(lock_file);
    let _guard = lock
        .write()
        .map_err(|error| Error::lock_failed(error.to_string()))?;
    operation()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationState {
    Copied(u64),
    Suppressed,
}

fn read_migration_state(marker: &Path) -> Result<Option<MigrationState>> {
    let entries = match fs::read_dir(marker) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut copied = None;
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name == "suppressed" {
            return Ok(Some(MigrationState::Suppressed));
        }
        if let Some(bytes) = name
            .strip_prefix("copied-")
            .and_then(|value| value.parse::<u64>().ok())
        {
            copied = Some(copied.map_or(bytes, |current: u64| current.max(bytes)));
        }
    }
    Ok(copied.map(MigrationState::Copied))
}

fn write_migration_state(marker: &Path, state: MigrationState) -> Result<()> {
    create_dir_all_synced(marker)?;
    let filename = match state {
        MigrationState::Copied(bytes) => format!("copied-{bytes}"),
        MigrationState::Suppressed => "suppressed".to_string(),
    };
    let destination = marker.join(filename);
    if destination.exists() {
        return Ok(());
    }
    let temporary = temporary_path(&destination);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.sync_all()?;
    fs::rename(&temporary, &destination)?;
    sync_parent(&destination)?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct PendingRange {
    source_start: u64,
    source_end: u64,
    destination_start: u64,
}

fn pending_range(marker: &Path, source_start: u64) -> Result<Option<PendingRange>> {
    let entries = match fs::read_dir(marker) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let values = name
            .strip_prefix("pending-")
            .map(|value| value.split('-').collect::<Vec<_>>());
        let Some(values) = values.filter(|values| values.len() == 3) else {
            continue;
        };
        let Some(start) = values[0].parse::<u64>().ok() else {
            continue;
        };
        if start != source_start {
            continue;
        }
        let Some(source_end) = values[1].parse::<u64>().ok() else {
            continue;
        };
        let Some(destination_start) = values[2].parse::<u64>().ok() else {
            continue;
        };
        return Ok(Some(PendingRange {
            source_start: start,
            source_end,
            destination_start,
        }));
    }
    Ok(None)
}

fn write_pending_range(marker: &Path, pending: PendingRange) -> Result<()> {
    create_dir_all_synced(marker)?;
    let destination = marker.join(format!(
        "pending-{}-{}-{}",
        pending.source_start, pending.source_end, pending.destination_start
    ));
    if destination.exists() {
        return Ok(());
    }
    let temporary = temporary_path(&destination);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.sync_all()?;
    fs::rename(&temporary, &destination)?;
    sync_parent(&destination)
}

fn copy_media_dir(source: &Path, destination: &Path) -> Result<()> {
    create_dir_all_synced(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let target = destination.join(entry.file_name());
        if !target.exists() {
            copy_file_atomic(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn is_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn copy_file_atomic(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        create_dir_all_synced(parent)?;
    }
    let temporary = temporary_path(destination);
    fs::copy(source, &temporary)?;
    File::open(&temporary)?.sync_all()?;
    if destination.exists() {
        fs::remove_file(&temporary)?;
        return Ok(());
    }
    fs::rename(&temporary, destination)?;
    sync_parent(destination)
}

fn copy_file_prefix_atomic(source: &Path, destination: &Path, bytes: u64) -> Result<()> {
    if let Some(parent) = destination.parent() {
        create_dir_all_synced(parent)?;
    }
    let temporary = temporary_path(destination);
    let source = File::open(source)?;
    let mut source = source.take(bytes);
    let mut target = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let copied = std::io::copy(&mut source, &mut target)?;
    if copied != bytes {
        return Err(Error::message(
            "legacy session history changed during migration",
        ));
    }
    target.sync_all()?;
    if destination.exists() {
        fs::remove_file(&temporary)?;
        return Ok(());
    }
    fs::rename(&temporary, destination)?;
    sync_parent(destination)
}

fn reconcile_file_range(
    marker: &Path,
    source: &Path,
    destination: &Path,
    source_start: u64,
    source_end: u64,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        create_dir_all_synced(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(destination)?;
    let mut lock = RwLock::new(file);
    let mut target = lock
        .write()
        .map_err(|error| Error::lock_failed(error.to_string()))?;
    let pending = if let Some(pending) = pending_range(marker, source_start)? {
        pending
    } else {
        let pending = PendingRange {
            source_start,
            source_end,
            destination_start: target.metadata()?.len(),
        };
        write_pending_range(marker, pending)?;
        pending
    };
    if pending.source_end > source_end {
        return Err(Error::message(
            "legacy session history changed during migration",
        ));
    }

    let bytes = pending.source_end - source_start;
    let expected_end = pending.destination_start.saturating_add(bytes);
    let destination_bytes = target.metadata()?.len();
    if destination_bytes >= expected_end
        && file_ranges_match(
            source,
            source_start,
            destination,
            pending.destination_start,
            bytes,
        )?
    {
        write_migration_state(marker, MigrationState::Copied(pending.source_end))?;
        drop(target);
        if pending.source_end < source_end {
            return reconcile_file_range(
                marker,
                source,
                destination,
                pending.source_end,
                source_end,
            );
        }
        return Ok(());
    }
    if destination_bytes > expected_end {
        return Err(Error::message(
            "current session history changed during migration",
        ));
    }
    target.set_len(pending.destination_start)?;
    target.seek(SeekFrom::Start(pending.destination_start))?;
    let mut source_file = File::open(source)?;
    source_file.seek(SeekFrom::Start(source_start))?;
    let mut source_range = source_file.take(bytes);
    let copied = std::io::copy(&mut source_range, &mut *target)?;
    if copied != bytes {
        return Err(Error::message(
            "legacy session history changed during migration",
        ));
    }
    target.flush()?;
    target.sync_all()?;
    write_migration_state(marker, MigrationState::Copied(pending.source_end))?;
    drop(target);
    if pending.source_end < source_end {
        reconcile_file_range(marker, source, destination, pending.source_end, source_end)?;
    }
    Ok(())
}

fn file_ranges_match(
    left: &Path,
    left_offset: u64,
    right: &Path,
    right_offset: u64,
    bytes: u64,
) -> Result<bool> {
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    left.seek(SeekFrom::Start(left_offset))?;
    right.seek(SeekFrom::Start(right_offset))?;
    let mut left = left.take(bytes);
    let mut right = right.take(bytes);
    let mut left_buffer = [0_u8; 8192];
    let mut right_buffer = [0_u8; 8192];
    let mut compared = 0_u64;
    while compared < bytes {
        let remaining = usize::try_from((bytes - compared).min(left_buffer.len() as u64))
            .unwrap_or(left_buffer.len());
        let left_read = left.read(&mut left_buffer[..remaining])?;
        let right_read = right.read(&mut right_buffer[..remaining])?;
        if left_read != right_read
            || left_buffer[..left_read] != right_buffer[..right_read]
            || left_read == 0
        {
            return Ok(false);
        }
        compared += left_read as u64;
    }
    Ok(true)
}

fn create_dir_all_synced(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir_all_synced(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => sync_parent(path),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session");
    destination.with_file_name(format!(".{filename}.{}.tmp", Uuid::new_v4()))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn codec_is_reversible_and_injective() {
        let keys = [
            "",
            "main",
            "a:b",
            "a_b",
            "A",
            "a",
            "../escape",
            "slash/key",
            "backslash\\key",
            "é",
            "e\u{301}",
        ];
        let encoded = keys.map(encode_key);

        for (key, component) in keys.iter().zip(&encoded) {
            assert_eq!(decode_key_path(Path::new(component)).as_deref(), Some(*key));
            assert!(
                component
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
            );
        }
        for (index, component) in encoded.iter().enumerate() {
            assert!(!encoded[..index].contains(component));
        }
    }

    #[test]
    fn long_keys_are_chunked_and_reversible() {
        let key = "long-key".repeat(100);
        let path = encoded_key_path(&key);
        assert!(path.components().count() > 1);
        assert!(
            path.components()
                .all(|component| component.as_os_str().len() <= ENCODED_COMPONENT_BYTES + 1)
        );
        assert_eq!(decode_key_path(&path).as_deref(), Some(key.as_str()));
    }

    #[test]
    fn unsafe_legacy_keys_cannot_escape_the_store() {
        let base = Path::new("/sessions");
        assert!(legacy_history_path(base, "../escape").is_none());
        assert!(legacy_history_path(base, "/absolute").is_none());
        assert!(legacy_history_path(base, "nested/key").is_none());
    }

    #[test]
    fn pending_legacy_suffix_is_not_duplicated_on_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        let key = "legacy:key";
        let legacy = legacy_history_path(base, key).unwrap();
        let first = b"{\"content\":\"first\"}\n";
        let second = b"{\"content\":\"second\"}\n";
        let third = b"{\"content\":\"third\"}\n";
        fs::write(&legacy, first).unwrap();
        ensure_migrated(base, key).unwrap();

        OpenOptions::new()
            .append(true)
            .open(&legacy)
            .unwrap()
            .write_all(second)
            .unwrap();
        let current = history_path(base, key);
        OpenOptions::new()
            .append(true)
            .open(&current)
            .unwrap()
            .write_all(second)
            .unwrap();
        write_pending_range(&migration_marker(base, key), PendingRange {
            source_start: first.len() as u64,
            source_end: (first.len() + second.len()) as u64,
            destination_start: first.len() as u64,
        })
        .unwrap();
        OpenOptions::new()
            .append(true)
            .open(&legacy)
            .unwrap()
            .write_all(third)
            .unwrap();

        ensure_migrated(base, key).unwrap();
        assert_eq!(
            fs::read(current).unwrap(),
            [first.as_slice(), second.as_slice(), third.as_slice()].concat()
        );
    }
}
