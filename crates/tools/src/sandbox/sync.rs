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

use std::path::Path;

use tracing::{debug, warn};

use crate::{
    error::{Error, Result},
    exec::ExecOpts,
    sandbox::{
        file_system::SandboxReadResult,
        types::{SANDBOX_HOME_DIR, Sandbox, SandboxId},
    },
};

/// Maximum tarball size for sync read operations (100 MB).
const MAX_SYNC_BYTES: u64 = 100 * 1024 * 1024;

/// Default sandbox workspace directory.
pub const DEFAULT_SANDBOX_WORKSPACE: &str = SANDBOX_HOME_DIR;

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
    let check_cmd = format!(
        "if [ -d {sandbox_workspace} ] && [ \"$(ls -A {sandbox_workspace} 2>/dev/null)\" ]; then echo non-empty; fi"
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
    let tar_cmd = format!("tar -czf {tar_path} -C {sandbox_workspace} .");
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
/// Uses the sandbox home persistence directory corresponding to the session.
/// Returns `None` if home persistence is disabled.
pub fn resolve_sync_workspace(
    config: &super::types::SandboxConfig,
    id: &SandboxId,
) -> Option<std::path::PathBuf> {
    use super::paths::{detected_container_cli, sandbox_home_persistence_host_dir};

    let cli = detected_container_cli(config);
    sandbox_home_persistence_host_dir(config, cli, id)
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

/// Extract a gzipped tarball into a directory.
/// Uses the host `tar` command (available on Linux and macOS).
async fn extract_tar_gz(dir: &Path, tar_bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new("tar")
        .args(["-xzf", "-", "-C"])
        .arg(dir)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Error::message(format!("sync: failed to spawn tar: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(tar_bytes)
            .await
            .map_err(|e| Error::message(format!("sync: failed to write tar data: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| Error::message(format!("sync: tar extraction failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::message(format!(
            "sync: tar extraction failed: {stderr}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
