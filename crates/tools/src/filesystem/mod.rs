//! Native filesystem tools.
//!
//! Provides `file_read` and `file_info` as built-in replacements for the
//! MCP filesystem server's read/info tools. Additional tools (write, edit,
//! tree, list, search, create_dir, move) will follow in later phases.

pub mod info;
pub mod read;

use {
    crate::{
        approval::{ApprovalBroadcaster, ApprovalDecision, ApprovalManager},
        error::Error,
    },
    std::{
        path::{Path, PathBuf},
        sync::Arc,
    },
};

/// Validate that a resolved (canonicalized) path falls within the allowed
/// directory list.
///
/// # Rules
/// - If `allowed_dirs` is empty, all paths are permitted (permissive mode).
/// - Otherwise the canonical path must start with one of the canonicalized
///   allowed-dir prefixes.  The prefix comparison is done with a trailing
///   separator so that `/tmp/foo` does *not* match allowed dir `/tmp/fo`.
///
/// # Errors
/// Returns an `Error` with a human-readable message listing the allowed dirs
/// when the path is outside the boundary.
pub(crate) fn check_allowed_dir(
    resolved_path: &Path,
    allowed_dirs: &[String],
) -> crate::Result<()> {
    if allowed_dirs.is_empty() {
        return Ok(());
    }

    let resolved_str = resolved_path.to_string_lossy();

    for dir in allowed_dirs {
        // Canonicalize the allowed dir once per check.  In production the
        // caller typically caches canonicalized prefixes, but since this
        // function is called per-execute the overhead is negligible.
        let canonical_dir = match std::fs::canonicalize(dir) {
            Ok(p) => p,
            Err(_) => continue, // non-existent allowed dir — skip
        };

        let canonical_str = canonical_dir.to_string_lossy();

        // Exact match (the resolved path *is* the allowed dir).
        if resolved_str == canonical_str {
            return Ok(());
        }

        // Prefix match — ensure trailing separator so `/tmp/fo` doesn't
        // match `/tmp/foo/bar`.
        let prefix = if canonical_str.ends_with('/') {
            canonical_str.into_owned()
        } else {
            format!("{canonical_str}/")
        };

        if resolved_str.starts_with(&prefix) {
            return Ok(());
        }
    }

    // Build a helpful message listing the allowed directories.
    let dir_list = allowed_dirs
        .iter()
        .map(|d| format!("  - {d}"))
        .collect::<Vec<_>>()
        .join("\n");

    Err(Error::message(format!(
        "path '{resolved_str}' is outside the allowed directories:\n{dir_list}"
    )))
}

/// Canonicalize a path.  Returns `None` if the path does not exist on disk.
///
/// For allowed-dir enforcement we want to resolve symlinks first.  Returning
/// `None` (rather than falling back to the raw string) prevents crafted paths
/// like `/allowed/sneaky/../etc/passwd` from bypassing containment when the
/// intermediate components don't exist.
pub(crate) fn canonicalize_or_original(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Enforce path containment and approval gating for filesystem tools.
///
/// The approval manager (if present) is consulted **first**, before the
/// `allowed_dirs` containment check.  This ensures that a `Deny` security
/// level or an `Always` approval mode cannot be bypassed by placing a path
/// inside `allowed_dirs`.
///
/// # Precedence
///
/// ```text
/// is_inside = check_allowed_dir(resolved, allowed_dirs).is_ok()
///
/// No approval manager → allowed_dirs is the only gate:
///   inside  = proceed
///   outside = reject
///
/// Approval manager present → ALWAYS consult it:
///   SecurityLevel::Deny     → reject ALL paths
///   SecurityLevel::Full     → proceed ALL paths
///   SecurityLevel::Allowlist:
///     ApprovalMode::Off     → inside = proceed, outside = reject (containment only)
///     ApprovalMode::OnMiss  → inside = proceed, outside = needs_approval
///     ApprovalMode::Always  → needs_approval for ALL paths (even inside allowed_dirs)
/// ```
///
/// # Arguments
/// * `path` — the raw user-supplied path string (used in approval messages)
/// * `resolved` — the canonicalized path (used for containment checks)
/// * `allowed_dirs` — directory allowlist (empty = all allowed)
/// * `approval_manager` — optional approval manager
/// * `broadcaster` — optional approval broadcaster
pub(crate) async fn enforce_approval(
    path: &str,
    resolved: &Path,
    allowed_dirs: &[String],
    approval_manager: Option<&Arc<ApprovalManager>>,
    broadcaster: Option<&Arc<dyn ApprovalBroadcaster>>,
) -> crate::Result<()> {
    let is_inside = check_allowed_dir(resolved, allowed_dirs).is_ok();

    match approval_manager {
        None => {
            // No approval manager — allowed_dirs is the only gate.
            if is_inside {
                Ok(())
            } else {
                check_allowed_dir(resolved, allowed_dirs)
            }
        },
        Some(mgr) => {
            // Always consult the approval manager first.
            match mgr.security_level {
                crate::approval::SecurityLevel::Deny => {
                    return Err(Error::message(format!(
                        "filesystem access denied: security level is 'deny': {path}"
                    )));
                },
                crate::approval::SecurityLevel::Full => return Ok(()),
                crate::approval::SecurityLevel::Allowlist => {},
            }

            // SecurityLevel::Allowlist — consult mode.
            let needs_approval = match mgr.mode {
                crate::approval::ApprovalMode::Off => {
                    // Containment only: inside = proceed, outside = reject.
                    if is_inside {
                        return Ok(());
                    } else {
                        return check_allowed_dir(resolved, allowed_dirs);
                    }
                },
                crate::approval::ApprovalMode::OnMiss => {
                    // Inside = proceed, outside = needs_approval.
                    !is_inside
                },
                crate::approval::ApprovalMode::Always => {
                    // Always prompt, even for inside paths.
                    true
                },
            };

            if needs_approval {
                let display = if is_inside {
                    format!("{path} (path is inside allowed directories)")
                } else {
                    path.to_string()
                };
                request_approval(&display, mgr, broadcaster).await
            } else {
                Ok(())
            }
        },
    }
}

/// Broadcast an approval request and wait for the user's decision.
async fn request_approval(
    display_path: &str,
    approval_manager: &Arc<ApprovalManager>,
    broadcaster: Option<&Arc<dyn ApprovalBroadcaster>>,
) -> crate::Result<()> {
    tracing::info!(path = %display_path, "filesystem access needs approval, waiting...");
    let (req_id, rx) = approval_manager.create_request(display_path).await;

    if let Some(bc) = broadcaster
        && let Err(e) = bc.broadcast_request(&req_id, display_path).await
    {
        tracing::warn!(error = %e, "failed to broadcast approval request");
    }

    match approval_manager.wait_for_decision(rx).await {
        ApprovalDecision::Approved => {
            tracing::info!(path = %display_path, "filesystem access approved");
            Ok(())
        },
        ApprovalDecision::Denied => {
            Err(Error::message(format!(
                "filesystem access denied by user: {display_path}"
            )))
        },
        ApprovalDecision::Timeout => {
            Err(Error::message(format!(
                "approval timed out for filesystem access: {display_path}"
            )))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- check_allowed_dir tests ---

    #[test]
    fn empty_allowed_dirs_allows_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("some_file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();
        assert!(check_allowed_dir(&resolved, &[]).is_ok());
    }

    #[test]
    fn path_inside_allowed_dir_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("subdir").join("file.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();
        assert!(check_allowed_dir(&resolved, &[tmp.path().to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn path_outside_allowed_dir_is_rejected() {
        let allowed = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("secret.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();
        let err = check_allowed_dir(&resolved, &[allowed.path().to_str().unwrap().to_string()])
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("outside the allowed directories"), "{msg}");
        assert!(msg.contains(allowed.path().to_str().unwrap()), "{msg}");
    }

    #[test]
    fn prefix_boundary_prevents_false_match() {
        // Create /tmp/test_prefix_guard and /tmp/test_prefix_guard_extra
        let base = tempfile::tempdir().unwrap();
        let allowed_dir = base.path().join("foo");
        let sneaky_dir = base.path().join("foo_secret");
        std::fs::create_dir_all(&allowed_dir).unwrap();
        std::fs::create_dir_all(&sneaky_dir).unwrap();

        let sneaky_file = sneaky_dir.join("data.txt");
        std::fs::write(&sneaky_file, "sneaky").unwrap();
        let resolved = canonicalize_or_original(&sneaky_file).unwrap();

        let err =
            check_allowed_dir(&resolved, &[allowed_dir.to_str().unwrap().to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("outside the allowed directories"),
            "{}",
            err.to_string()
        );
    }

    #[test]
    fn canonicalizes_allowed_dir_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let file = target.join("file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();

        // Allowed dir is specified via symlink — should still work because
        // both sides canonicalize to the same real path.
        assert!(check_allowed_dir(&resolved, &[link.to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn symlink_escape_is_detected() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let link = allowed.path().join("escape_link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();

        let resolved = canonicalize_or_original(&link).unwrap();
        let err = check_allowed_dir(&resolved, &[allowed.path().to_str().unwrap().to_string()])
            .unwrap_err();
        assert!(
            err.to_string().contains("outside the allowed directories"),
            "{}",
            err.to_string()
        );
    }

    #[test]
    fn exact_allowed_dir_match_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let resolved = canonicalize_or_original(tmp.path()).unwrap();
        assert!(check_allowed_dir(&resolved, &[tmp.path().to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn multiple_allowed_dirs() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let file_in_dir2 = dir2.path().join("file.txt");
        std::fs::write(&file_in_dir2, "data").unwrap();
        let resolved = canonicalize_or_original(&file_in_dir2).unwrap();

        assert!(
            check_allowed_dir(&resolved, &[
                dir1.path().to_str().unwrap().to_string(),
                dir2.path().to_str().unwrap().to_string(),
            ])
            .is_ok()
        );

        let file_outside = outside.path().join("other.txt");
        std::fs::write(&file_outside, "data").unwrap();
        let resolved_out = canonicalize_or_original(&file_outside).unwrap();
        assert!(
            check_allowed_dir(&resolved_out, &[
                dir1.path().to_str().unwrap().to_string(),
                dir2.path().to_str().unwrap().to_string(),
            ])
            .is_err()
        );
    }

    #[test]
    fn non_existent_allowed_dir_is_skipped() {
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();

        // Only allowed dir is non-existent — file should be rejected.
        assert!(check_allowed_dir(&resolved, &["/nonexistent/path/12345".to_string()]).is_err());
    }

    // --- enforce_approval tests ---

    #[tokio::test]
    async fn deny_level_blocks_even_when_inside_allowed_dir() {
        use crate::approval::{ApprovalManager, SecurityLevel};

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("secret.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();

        let mut mgr = ApprovalManager::default();
        mgr.security_level = SecurityLevel::Deny;

        let result = enforce_approval(
            file.to_str().unwrap(),
            &resolved,
            &[tmp.path().to_str().unwrap().to_string()],
            Some(&Arc::new(mgr)),
            None,
        )
        .await;

        assert!(result.is_err(), "Deny level should reject even paths inside allowed_dirs");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("deny"), "error message should mention deny: {msg}");
    }

    #[tokio::test]
    async fn always_mode_prompts_even_when_inside_allowed_dir() {
        use crate::approval::{ApprovalManager, ApprovalMode};

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file).unwrap();

        let mut mgr = ApprovalManager::default();
        mgr.mode = ApprovalMode::Always;
        mgr.timeout = std::time::Duration::from_millis(100);

        let result = enforce_approval(
            file.to_str().unwrap(),
            &resolved,
            &[tmp.path().to_str().unwrap().to_string()],
            Some(&Arc::new(mgr)),
            None,
        )
        .await;

        // No broadcaster → no one to approve → should timeout (which we map to error)
        assert!(
            result.is_err(),
            "Always mode should require approval even for paths inside allowed_dirs"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("timed out") || msg.contains("denied"),
            "expected timeout/denied but got: {msg}"
        );
    }
}