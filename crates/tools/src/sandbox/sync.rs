//! Workspace synchronization for isolated sandbox backends.
//!
//! Isolated backends (Vercel, Daytona, Firecracker) run in their own
//! filesystem — unlike bind-mount backends (Docker, Podman), the host
//! workspace is not directly accessible. This module handles:
//!
//! - **sync-in**: Upload host workspace contents to the sandbox on first run.
//! - **sync-out**: Download workspace changes from the sandbox on cleanup.
//!
//! Uses tar-based transfer: the host workspace is packed into a gzipped
//! tarball, uploaded to the sandbox, and extracted there. The reverse for
//! sync-out.

use std::{
    io::{self, Cursor},
    path::{Component, Path, PathBuf},
};

use tracing::{debug, warn};

use crate::{
    error::{Error, Result},
    exec::ExecOpts,
    sandbox::{
        file_system::SandboxReadResult,
        types::{Sandbox, SandboxId},
    },
};

/// Maximum tarball size for sync read operations (100 MB).
const MAX_SYNC_BYTES: u64 = 100 * 1024 * 1024;

/// Upload host workspace contents to an isolated sandbox.
///
/// Creates a gzipped tarball of the host workspace directory and extracts
/// it in the sandbox's workspace directory. Skips if the host directory
/// doesn't exist or is empty.
pub async fn sync_in(
    backend: &dyn Sandbox,
    id: &SandboxId,
    host_workspace: &Path,
    sandbox_workspace: &str,
) -> Result<()> {
    if !host_workspace.exists() {
        debug!(%id, host = %host_workspace.display(), "sync-in: host workspace does not exist, skipping");
        return Ok(());
    }

    if is_dir_empty(host_workspace) {
        debug!(%id, host = %host_workspace.display(), "sync-in: host workspace is empty, skipping");
        return Ok(());
    }

    let tar_bytes = create_tar_gz(host_workspace).await?;
    if tar_bytes.is_empty() {
        debug!(%id, "sync-in: tar produced empty output, skipping");
        return Ok(());
    }

    debug!(
        %id,
        host = %host_workspace.display(),
        sandbox = sandbox_workspace,
        tar_size = tar_bytes.len(),
        "sync-in: uploading workspace"
    );

    let tar_path = "/tmp/moltis-sync-in.tar.gz";
    backend.write_file(id, tar_path, &tar_bytes).await?;

    let cmd = format!(
        "mkdir -p '{sandbox_workspace}' && tar -xzf {tar_path} -C '{sandbox_workspace}' && rm -f {tar_path}"
    );
    let opts = ExecOpts {
        timeout: std::time::Duration::from_secs(120),
        ..Default::default()
    };
    let result = backend.exec(id, &cmd, &opts).await?;
    if result.exit_code != 0 {
        return Err(Error::message(format!(
            "sync-in: extraction failed (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        )));
    }

    debug!(%id, "sync-in: workspace uploaded successfully");
    Ok(())
}

/// Download workspace changes from an isolated sandbox back to host.
///
/// Creates a gzipped tarball of the sandbox workspace and extracts it
/// to the host directory. Skips if the sandbox workspace is empty.
pub async fn sync_out(
    backend: &dyn Sandbox,
    id: &SandboxId,
    host_workspace: &Path,
    sandbox_workspace: &str,
) -> Result<()> {
    let opts = ExecOpts {
        timeout: std::time::Duration::from_secs(120),
        ..Default::default()
    };

    // Check if sandbox workspace has content.
    let check_cmd = format!(
        "if [ -d '{sandbox_workspace}' ] && [ \"$(ls -A '{sandbox_workspace}' 2>/dev/null)\" ]; then echo non-empty; fi"
    );
    let check = backend.exec(id, &check_cmd, &opts).await?;
    if !check.stdout.contains("non-empty") {
        debug!(%id, "sync-out: sandbox workspace empty, skipping");
        return Ok(());
    }

    debug!(
        %id,
        sandbox = sandbox_workspace,
        host = %host_workspace.display(),
        "sync-out: downloading workspace changes"
    );

    // Create tarball in sandbox.
    let tar_path = "/tmp/moltis-sync-out.tar.gz";
    let tar_cmd = format!("tar -czf {tar_path} -C '{sandbox_workspace}' .");
    let tar_result = backend.exec(id, &tar_cmd, &opts).await?;
    if tar_result.exit_code != 0 {
        return Err(Error::message(format!(
            "sync-out: tar creation failed (exit {}): {}",
            tar_result.exit_code,
            tar_result.stderr.trim()
        )));
    }

    // Read tarball from sandbox.
    let read_result = backend.read_file(id, tar_path, MAX_SYNC_BYTES).await?;
    let tar_bytes = match read_result {
        SandboxReadResult::Ok(bytes) => bytes,
        SandboxReadResult::NotFound => {
            debug!(%id, "sync-out: tarball not found after creation, skipping");
            return Ok(());
        },
        SandboxReadResult::PermissionDenied => {
            return Err(Error::message(
                "sync-out: permission denied reading tarball",
            ));
        },
        SandboxReadResult::TooLarge(size) => {
            warn!(%id, size, "sync-out: workspace tarball exceeds size limit");
            return Err(Error::message(format!(
                "sync-out: workspace too large ({size} bytes exceeds {} byte limit)",
                MAX_SYNC_BYTES
            )));
        },
        SandboxReadResult::NotRegularFile => {
            return Err(Error::message(
                "sync-out: tarball path is not a regular file",
            ));
        },
    };

    if tar_bytes.is_empty() {
        debug!(%id, "sync-out: empty tarball read, skipping");
        return Ok(());
    }

    // Extract on host.
    std::fs::create_dir_all(host_workspace)
        .map_err(|e| Error::message(format!("sync-out: failed to create host dir: {e}")))?;
    extract_tar_gz(host_workspace, &tar_bytes).await?;

    debug!(%id, tar_size = tar_bytes.len(), "sync-out: workspace downloaded successfully");
    Ok(())
}

/// Resolve the host workspace path for sync operations.
///
/// For isolated backends, always returns a path — even when home persistence
/// is disabled — because workspace sync is essential for remote backends to
/// function. Falls back to a dedicated sync directory under `data_dir()`.
pub fn resolve_sync_workspace(
    config: &super::types::SandboxConfig,
    id: &SandboxId,
) -> Option<PathBuf> {
    use super::{
        paths::{detected_container_cli, sandbox_home_persistence_host_dir},
        types::sanitize_path_component,
    };

    let cli = detected_container_cli(config);
    // If home persistence is configured, use that directory.
    if let Some(path) = sandbox_home_persistence_host_dir(config, cli, id) {
        return Some(path);
    }
    // Fallback: dedicated sync directory for isolated backends.
    Some(
        moltis_config::data_dir()
            .join("sandbox")
            .join("sync")
            .join(sanitize_path_component(&id.key)),
    )
}

/// Check if a directory is empty or contains no entries.
fn is_dir_empty(dir: &Path) -> bool {
    dir.read_dir()
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true)
}

/// Create a gzipped tarball of a directory, returning the raw bytes.
/// Uses the host `tar` command (available on Linux and macOS).
async fn create_tar_gz(dir: &Path) -> Result<Vec<u8>> {
    let output = tokio::process::Command::new("tar")
        .args(["-czf", "-", "-C"])
        .arg(dir)
        .arg(".")
        .output()
        .await
        .map_err(|e| Error::message(format!("sync: failed to run tar: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::message(format!(
            "sync: tar creation failed: {stderr}"
        )));
    }
    Ok(output.stdout)
}

async fn extract_tar_gz(dir: &Path, tar_bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::message(format!("sync: failed to create extract dir: {e}")))?;

    let decoder = flate2::read::GzDecoder::new(Cursor::new(tar_bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| Error::message(format!("sync: failed to read tar entries: {e}")))?;

    for entry in entries {
        let mut entry =
            entry.map_err(|e| Error::message(format!("sync: failed to read tar entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| Error::message(format!("sync: invalid tar path: {e}")))?;
        let path = path.into_owned();
        let Some(relative_path) = validate_tar_path(&path)? else {
            continue;
        };

        match entry.header().entry_type() {
            tar::EntryType::Directory => ensure_directory(dir, &relative_path)?,
            tar::EntryType::Regular => {
                ensure_parent_directory(dir, &relative_path)?;
                let target = dir.join(&relative_path);
                reject_existing_symlink(&target)?;
                let mut file = std::fs::File::create(&target).map_err(|e| {
                    Error::message(format!(
                        "sync: failed to create extracted file '{}': {e}",
                        target.display()
                    ))
                })?;
                io::copy(&mut entry, &mut file).map_err(|e| {
                    Error::message(format!(
                        "sync: failed to write extracted file '{}': {e}",
                        target.display()
                    ))
                })?;
            },
            other => {
                return Err(Error::message(format!(
                    "sync: refusing unsupported tar entry type {other:?} for '{}'",
                    path.display()
                )));
            },
        }
    }

    Ok(())
}

fn validate_tar_path(path: &Path) -> Result<Option<PathBuf>> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::message(format!(
                    "sync: refusing unsafe tar path '{}'",
                    path.display()
                )));
            },
        }
    }

    if relative.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(relative))
    }
}

fn ensure_directory(root: &Path, relative_path: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        let Component::Normal(part) = component else {
            return Err(Error::message(format!(
                "sync: refusing unsafe directory path '{}'",
                relative_path.display()
            )));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::message(format!(
                    "sync: refusing to extract through symlink '{}'",
                    current.display()
                )));
            },
            Ok(metadata) if metadata.is_dir() => {},
            Ok(_) => {
                return Err(Error::message(format!(
                    "sync: refusing to replace non-directory '{}'",
                    current.display()
                )));
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|e| {
                    Error::message(format!(
                        "sync: failed to create directory '{}': {e}",
                        current.display()
                    ))
                })?;
            },
            Err(e) => {
                return Err(Error::message(format!(
                    "sync: failed to inspect directory '{}': {e}",
                    current.display()
                )));
            },
        }
    }
    Ok(())
}

fn ensure_parent_directory(root: &Path, relative_path: &Path) -> Result<()> {
    if let Some(parent) = relative_path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_directory(root, parent)?;
    }
    Ok(())
}

fn reject_existing_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::message(format!(
            "sync: refusing to overwrite symlink '{}'",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::message(format!(
            "sync: failed to inspect '{}': {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn tar_gz_with_file(path: &str, content: &[u8]) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, content).unwrap();
        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_raw_file_path(path: &[u8], content: &[u8]) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.as_mut_bytes()[..path.len()].copy_from_slice(path);
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append(&header, content).unwrap();
        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    #[test]
    fn test_is_dir_empty_nonexistent() {
        assert!(is_dir_empty(Path::new("/nonexistent/path/xyz")));
    }

    #[test]
    fn test_is_dir_empty_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_dir_empty(dir.path()));
    }

    #[test]
    fn test_is_dir_empty_with_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        assert!(!is_dir_empty(dir.path()));
    }

    #[tokio::test]
    async fn test_create_tar_gz_with_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        let bytes = create_tar_gz(dir.path()).await.unwrap();
        assert!(!bytes.is_empty());
    }

    #[tokio::test]
    async fn test_create_and_extract_roundtrip() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir(src.path().join("subdir")).unwrap();
        std::fs::write(src.path().join("subdir/nested.txt"), "nested content").unwrap();

        let tar_bytes = create_tar_gz(src.path()).await.unwrap();

        let dst = tempfile::tempdir().unwrap();
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.path().join("hello.txt")).unwrap(),
            "world"
        );
        assert_eq!(
            std::fs::read_to_string(dst.path().join("subdir/nested.txt")).unwrap(),
            "nested content"
        );
    }

    #[tokio::test]
    async fn test_extract_rejects_parent_traversal() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_raw_file_path(b"../escape.txt", b"nope");
        let result = extract_tar_gz(dst.path(), &tar_bytes).await;
        assert!(result.is_err());
        assert!(!dst.path().join("../escape.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_rejects_existing_symlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "original").unwrap();
        std::os::unix::fs::symlink(&outside_file, dst.path().join("link.txt")).unwrap();

        let tar_bytes = tar_gz_with_file("link.txt", b"overwrite");
        let result = extract_tar_gz(dst.path(), &tar_bytes).await;
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "original");
    }
}
