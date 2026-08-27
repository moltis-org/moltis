use std::{
    fs::File,
    path::Path,
    process::Command,
    sync::{Mutex, OnceLock},
};

use tracing::{debug, warn};

use super::ContainerBackend;

pub(super) fn cleanup_was_confirmed(stop_succeeded: bool, remove_succeeded: bool) -> bool {
    stop_succeeded || remove_succeeded
}

pub(super) fn profile_lock_to_quarantine(
    profile_lock: Option<File>,
    cleanup_confirmed: bool,
) -> Option<File> {
    if cleanup_confirmed {
        None
    } else {
        profile_lock
    }
}

pub(super) fn quarantine_profile_lock(
    profile_lock: File,
    backend: ContainerBackend,
    container_id: &str,
    profile_dir: Option<&Path>,
) {
    static QUARANTINED_PROFILE_LOCKS: OnceLock<Mutex<Vec<File>>> = OnceLock::new();

    warn!(
        container_id,
        backend = backend.cli(),
        profile_dir = profile_dir.map(|path| path.display().to_string()),
        "browser container cleanup could not be confirmed; retaining profile lock until Moltis exits"
    );
    QUARANTINED_PROFILE_LOCKS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(profile_lock);
}

/// Stop and remove a browser container.
///
/// Returns `true` only when at least one backend command confirms the
/// container is no longer running. A successful `stop` is sufficient because
/// browser containers have no restart policy; a successful `rm` confirms the
/// container no longer exists. If both commands fail, callers must assume the
/// container may still be using its mounted browser profile.
pub(super) fn stop_container_by_id(backend: ContainerBackend, container_id: &str) -> bool {
    let cli = backend.cli();
    let result = Command::new(cli).args(["stop", container_id]).output();

    let stop_succeeded = match result {
        Ok(output) if output.status.success() => {
            debug!(container_id, backend = cli, "browser container stopped");
            true
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                container_id,
                backend = cli,
                error = %stderr.trim(),
                "failed to stop browser container"
            );
            false
        },
        Err(e) => {
            warn!(
                container_id,
                backend = cli,
                error = %e,
                "failed to run {} stop",
                cli
            );
            false
        },
    };

    // Containers are started without --rm so that logs and status remain
    // available for diagnostics after a crash. Explicitly remove the
    // container after stopping it.
    let remove_succeeded = match Command::new(cli).args(["rm", container_id]).output() {
        Ok(output) if output.status.success() => {
            debug!(container_id, backend = cli, "browser container removed");
            true
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                container_id,
                backend = cli,
                error = %stderr.trim(),
                "failed to remove browser container"
            );
            false
        },
        Err(e) => {
            warn!(
                container_id,
                backend = cli,
                error = %e,
                "failed to run {} rm",
                cli
            );
            false
        },
    };

    cleanup_was_confirmed(stop_succeeded, remove_succeeded)
}
