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

use flate2::{Compression, write::GzEncoder};

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
    let sandbox_workspace = shell_single_quote(sandbox_workspace);
    backend.write_file(id, tar_path, &tar_bytes).await?;

    let cmd = format!(
        "mkdir -p {sandbox_workspace} && tar -xzf {tar_path} -C {sandbox_workspace} && rm -f {tar_path}"
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
    let sandbox_workspace_shell = shell_single_quote(sandbox_workspace);
    let check_cmd = format!(
        "if [ -d {sandbox_workspace_shell} ] && [ \"$(ls -A {sandbox_workspace_shell} 2>/dev/null)\" ]; then echo non-empty; fi"
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
    let tar_cmd = format!("tar -czf {tar_path} -C {sandbox_workspace_shell} .");
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
async fn create_tar_gz(dir: &Path) -> Result<Vec<u8>> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        archive.follow_symlinks(false);
        archive
            .append_dir_all(".", &dir)
            .map_err(|e| Error::message(format!("sync: failed to build tar archive: {e}")))?;
        let encoder = archive
            .into_inner()
            .map_err(|e| Error::message(format!("sync: failed to finish tar archive: {e}")))?;
        encoder
            .finish()
            .map_err(|e| Error::message(format!("sync: failed to gzip tar archive: {e}")))
    })
    .await
    .map_err(|e| Error::message(format!("sync: tar creation task failed: {e}")))?
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
        let relative_path = match validate_tar_path(&path) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "sync: skipping tar entry with unsafe path"
                );
                continue;
            },
        };

        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                if let Err(e) = ensure_directory(dir, &relative_path) {
                    warn!(
                        path = %relative_path.display(),
                        error = %e,
                        "sync: skipping directory entry with unsafe parent path"
                    );
                    continue;
                }
            },
            tar::EntryType::Regular => {
                if let Err(e) = ensure_parent_directory(dir, &relative_path) {
                    warn!(
                        path = %relative_path.display(),
                        error = %e,
                        "sync: skipping regular file with unsafe parent path"
                    );
                    continue;
                }
                let target = dir.join(&relative_path);
                if let Err(e) = reject_existing_symlink(&target) {
                    warn!(
                        path = %relative_path.display(),
                        target = %target.display(),
                        error = %e,
                        "sync: skipping regular file that would overwrite a symlink"
                    );
                    continue;
                }
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
            tar::EntryType::Symlink => {
                let link_target = entry
                    .link_name()
                    .map_err(|e| Error::message(format!("sync: invalid symlink target: {e}")))?
                    .ok_or_else(|| {
                        Error::message(format!(
                            "sync: symlink '{}' is missing a target",
                            path.display()
                        ))
                    })?
                    .into_owned();
                if !is_safe_symlink_target(&relative_path, &link_target) {
                    continue;
                }
                if let Err(e) = ensure_parent_directory(dir, &relative_path) {
                    warn!(
                        path = %relative_path.display(),
                        error = %e,
                        "sync: skipping symlink with unsafe parent path"
                    );
                    continue;
                }
                let target = dir.join(&relative_path);
                replace_existing_symlink_path(&target)?;
                create_symlink(&link_target, &target)?;
            },
            tar::EntryType::Link => {
                let link_target = entry
                    .link_name()
                    .map_err(|e| Error::message(format!("sync: invalid hardlink target: {e}")))?
                    .ok_or_else(|| {
                        Error::message(format!(
                            "sync: hardlink '{}' is missing a target",
                            path.display()
                        ))
                    })?
                    .into_owned();
                extract_hardlink(dir, &relative_path, &link_target)?;
            },
            other => {
                warn!(
                    entry_type = ?other,
                    path = %path.display(),
                    "sync: skipping unsupported tar entry type"
                );
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

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn is_safe_symlink_target(link_path: &Path, link_target: &Path) -> bool {
    let mut resolved = link_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    for component in link_target.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {},
            Component::ParentDir => {
                if !resolved.pop() {
                    warn!(
                        link = %link_path.display(),
                        target = %link_target.display(),
                        "sync: skipping symlink with escaping target"
                    );
                    return false;
                }
            },
            Component::RootDir | Component::Prefix(_) => {
                warn!(
                    link = %link_path.display(),
                    target = %link_target.display(),
                    "sync: skipping symlink with unsafe target"
                );
                return false;
            },
        }
    }
    true
}

fn replace_existing_symlink_path(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            std::fs::remove_file(path).map_err(|e| {
                Error::message(format!(
                    "sync: failed to replace existing path '{}': {e}",
                    path.display()
                ))
            })
        },
        Ok(metadata) if metadata.is_dir() => Err(Error::message(format!(
            "sync: refusing to replace directory '{}' with symlink",
            path.display()
        ))),
        Ok(_) => Err(Error::message(format!(
            "sync: refusing to replace special file '{}' with symlink",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::message(format!(
            "sync: failed to inspect '{}': {e}",
            path.display()
        ))),
    }
}

fn extract_hardlink(root: &Path, relative_path: &Path, link_target: &Path) -> Result<()> {
    let relative_link_target = match validate_tar_path(link_target) {
        Ok(Some(target)) => target,
        Ok(None) => {
            warn!(
                path = %relative_path.display(),
                target = %link_target.display(),
                "sync: skipping hardlink with empty target"
            );
            return Ok(());
        },
        Err(e) => {
            warn!(
                path = %relative_path.display(),
                target = %link_target.display(),
                error = %e,
                "sync: skipping hardlink with unsafe target"
            );
            return Ok(());
        },
    };

    if relative_link_target.as_os_str().is_empty() {
        warn!(
            path = %relative_path.display(),
            target = %link_target.display(),
            "sync: skipping hardlink with empty target"
        );
        return Ok(());
    }

    if let Err(e) = ensure_parent_directory(root, relative_path) {
        warn!(
            path = %relative_path.display(),
            target = %relative_link_target.display(),
            error = %e,
            "sync: skipping hardlink with unsafe parent path"
        );
        return Ok(());
    }
    let source = root.join(&relative_link_target);
    let target = root.join(relative_path);
    if let Err(e) = reject_existing_symlink(&source) {
        warn!(
            path = %relative_path.display(),
            target = %relative_link_target.display(),
            error = %e,
            "sync: skipping hardlink whose source is a symlink"
        );
        return Ok(());
    }
    if let Err(e) = reject_existing_symlink(&target) {
        warn!(
            path = %relative_path.display(),
            target = %relative_link_target.display(),
            error = %e,
            "sync: skipping hardlink that would overwrite a symlink"
        );
        return Ok(());
    }

    match std::fs::symlink_metadata(&source) {
        Ok(metadata) if metadata.is_file() => {
            std::fs::copy(&source, &target).map_err(|e| {
                Error::message(format!(
                    "sync: failed to copy hardlink '{}' from '{}': {e}",
                    target.display(),
                    source.display()
                ))
            })?;
            Ok(())
        },
        Ok(metadata) if metadata.is_dir() => {
            warn!(
                path = %relative_path.display(),
                target = %relative_link_target.display(),
                "sync: skipping hardlink to directory"
            );
            Ok(())
        },
        Ok(_) => {
            warn!(
                path = %relative_path.display(),
                target = %relative_link_target.display(),
                "sync: skipping hardlink to special file"
            );
            Ok(())
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!(
                path = %relative_path.display(),
                target = %relative_link_target.display(),
                "sync: skipping hardlink whose target has not been extracted"
            );
            Ok(())
        },
        Err(e) => Err(Error::message(format!(
            "sync: failed to inspect hardlink target '{}': {e}",
            source.display()
        ))),
    }
}

#[cfg(unix)]
fn create_symlink(link_target: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(link_target, target).map_err(|e| {
        Error::message(format!(
            "sync: failed to create symlink '{}' -> '{}': {e}",
            target.display(),
            link_target.display()
        ))
    })
}

#[cfg(not(unix))]
fn create_symlink(link_target: &Path, target: &Path) -> Result<()> {
    warn!(
        link = %target.display(),
        target = %link_target.display(),
        "sync: skipping symlink extraction on unsupported platform"
    );
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

    fn tar_gz_with_two_files(first_path: &str, second_path: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let first = b"first";
        let mut first_header = tar::Header::new_gnu();
        first_header.set_size(first.len() as u64);
        first_header.set_mode(0o644);
        first_header.set_cksum();
        archive
            .append_data(&mut first_header, first_path, &first[..])
            .unwrap();

        let second = b"second";
        let mut second_header = tar::Header::new_gnu();
        second_header.set_size(second.len() as u64);
        second_header.set_mode(0o644);
        second_header.set_cksum();
        archive
            .append_data(&mut second_header, second_path, &second[..])
            .unwrap();

        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_raw_file_path_and_safe_file(path: &[u8], content: &[u8]) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let mut unsafe_header = tar::Header::new_gnu();
        unsafe_header.as_mut_bytes()[..path.len()].copy_from_slice(path);
        unsafe_header.set_size(content.len() as u64);
        unsafe_header.set_mode(0o644);
        unsafe_header.set_cksum();
        archive.append(&unsafe_header, content).unwrap();

        let mut safe_header = tar::Header::new_gnu();
        safe_header.set_size(4);
        safe_header.set_mode(0o644);
        safe_header.set_cksum();
        archive
            .append_data(&mut safe_header, "safe.txt", &b"safe"[..])
            .unwrap();

        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    #[cfg(unix)]
    fn tar_gz_with_symlink(path: &str, target: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        archive.append_link(&mut header, path, target).unwrap();
        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_hardlink(path: &str, target: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Link);
        header.set_size(0);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_link(&mut header, path, target).unwrap();
        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_directory_and_hardlink(dir_path: &str, hardlink_path: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        archive
            .append_data(&mut dir_header, dir_path, io::empty())
            .unwrap();

        let mut link_header = tar::Header::new_gnu();
        link_header.set_entry_type(tar::EntryType::Link);
        link_header.set_size(0);
        link_header.set_mode(0o644);
        link_header.set_cksum();
        archive
            .append_link(&mut link_header, hardlink_path, dir_path)
            .unwrap();

        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_directory_and_safe_file(dir_path: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        archive
            .append_data(&mut dir_header, dir_path, io::empty())
            .unwrap();

        let mut safe_header = tar::Header::new_gnu();
        safe_header.set_size(4);
        safe_header.set_mode(0o644);
        safe_header.set_cksum();
        archive
            .append_data(&mut safe_header, "safe.txt", &b"safe"[..])
            .unwrap();

        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_file_and_hardlink(file_path: &str, hardlink_path: &str) -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let content = b"shared content";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_size(content.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        archive
            .append_data(&mut file_header, file_path, content.as_slice())
            .unwrap();

        let mut link_header = tar::Header::new_gnu();
        link_header.set_entry_type(tar::EntryType::Link);
        link_header.set_size(0);
        link_header.set_mode(0o644);
        link_header.set_cksum();
        archive
            .append_link(&mut link_header, hardlink_path, file_path)
            .unwrap();

        archive.into_inner().and_then(|enc| enc.finish()).unwrap()
    }

    fn tar_gz_with_unsupported_entry_between_files() -> Vec<u8> {
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut archive = tar::Builder::new(enc);

        let mut first_header = tar::Header::new_gnu();
        first_header.set_size(5);
        first_header.set_mode(0o644);
        first_header.set_cksum();
        archive
            .append_data(&mut first_header, "before.txt", &b"start"[..])
            .unwrap();

        let mut fifo_header = tar::Header::new_gnu();
        fifo_header.set_entry_type(tar::EntryType::Fifo);
        fifo_header.set_path("pipe").unwrap();
        fifo_header.set_size(0);
        fifo_header.set_mode(0o644);
        fifo_header.set_cksum();
        archive.append(&fifo_header, io::empty()).unwrap();

        let mut second_header = tar::Header::new_gnu();
        second_header.set_size(3);
        second_header.set_mode(0o644);
        second_header.set_cksum();
        archive
            .append_data(&mut second_header, "after.txt", &b"end"[..])
            .unwrap();

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

    #[test]
    fn test_shell_single_quote_escapes_embedded_quotes() {
        assert_eq!(
            shell_single_quote("/home/daytona/work' && touch /tmp/pwned && echo '"),
            "'/home/daytona/work'\\'' && touch /tmp/pwned && echo '\\'''"
        );
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
    async fn test_extract_skips_parent_traversal() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_raw_file_path_and_safe_file(b"../escape.txt", b"nope");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("../escape.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dst.path().join("safe.txt")).unwrap(),
            "safe"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_existing_symlink_target_and_continues() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "original").unwrap();
        std::os::unix::fs::symlink(&outside_file, dst.path().join("link.txt")).unwrap();

        let tar_bytes = tar_gz_with_two_files("link.txt", "safe.txt");
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "original");
        assert_eq!(
            std::fs::read_to_string(dst.path().join("safe.txt")).unwrap(),
            "second"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_directory_under_symlink_parent_and_continues() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dst.path().join("lib64")).unwrap();

        let tar_bytes = tar_gz_with_directory_and_safe_file("lib64/python3.11");
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!outside.path().join("python3.11").exists());
        assert_eq!(
            std::fs::read_to_string(dst.path().join("safe.txt")).unwrap(),
            "safe"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_regular_file_under_symlink_parent_and_continues() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dst.path().join("lib64")).unwrap();

        let tar_bytes = tar_gz_with_two_files("lib64/python3.11/foo.py", "safe.txt");
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!outside.path().join("python3.11/foo.py").exists());
        assert_eq!(
            std::fs::read_to_string(dst.path().join("safe.txt")).unwrap(),
            "second"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_preserves_relative_symlink() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_symlink("bin/tool", "../lib/tool");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(
            std::fs::read_link(dst.path().join("bin/tool")).unwrap(),
            PathBuf::from("../lib/tool")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_absolute_symlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_symlink("bin/tool", "/etc/passwd");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("bin/tool").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_escaping_symlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_symlink("bin/tool", "../../escape");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("bin/tool").exists());
    }

    #[tokio::test]
    async fn test_extract_skips_unsupported_tar_entry_type() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_unsupported_entry_between_files();

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.path().join("before.txt")).unwrap(),
            "start"
        );
        assert_eq!(
            std::fs::read_to_string(dst.path().join("after.txt")).unwrap(),
            "end"
        );
    }

    #[tokio::test]
    async fn test_extract_copies_hardlink_to_regular_file() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes =
            tar_gz_with_file_and_hardlink("store/content.txt", "node_modules/pkg/file.txt");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.path().join("node_modules/pkg/file.txt")).unwrap(),
            "shared content"
        );
    }

    #[tokio::test]
    async fn test_extract_skips_missing_hardlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_hardlink("node_modules/pkg/file.txt", "store/missing.txt");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("node_modules/pkg/file.txt").exists());
    }

    #[tokio::test]
    async fn test_extract_skips_hardlink_to_directory() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_directory_and_hardlink("store/dir", "node_modules/pkg/dir");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(dst.path().join("store/dir").is_dir());
        assert!(!dst.path().join("node_modules/pkg/dir").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_hardlink_from_existing_symlink_source() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "original").unwrap();
        std::fs::create_dir_all(dst.path().join("store")).unwrap();
        std::os::unix::fs::symlink(&outside_file, dst.path().join("store/link")).unwrap();

        let tar_bytes = tar_gz_with_hardlink("node_modules/pkg/file.txt", "store/link");
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("node_modules/pkg/file.txt").exists());
        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "original");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_extract_skips_hardlink_to_existing_symlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("target.txt");
        std::fs::write(&outside_file, "original").unwrap();
        std::fs::create_dir_all(dst.path().join("node_modules/pkg")).unwrap();
        std::os::unix::fs::symlink(&outside_file, dst.path().join("node_modules/pkg/file.txt"))
            .unwrap();

        let tar_bytes =
            tar_gz_with_file_and_hardlink("store/content.txt", "node_modules/pkg/file.txt");
        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "original");
        assert_eq!(
            std::fs::read_to_string(dst.path().join("store/content.txt")).unwrap(),
            "shared content"
        );
    }

    #[tokio::test]
    async fn test_extract_skips_escaping_hardlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_hardlink("node_modules/pkg/file.txt", "../escape.txt");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("node_modules/pkg/file.txt").exists());
    }

    #[tokio::test]
    async fn test_extract_skips_absolute_hardlink_target() {
        let dst = tempfile::tempdir().unwrap();
        let tar_bytes = tar_gz_with_hardlink("node_modules/pkg/file.txt", "/etc/passwd");

        extract_tar_gz(dst.path(), &tar_bytes).await.unwrap();

        assert!(!dst.path().join("node_modules/pkg/file.txt").exists());
    }
}
