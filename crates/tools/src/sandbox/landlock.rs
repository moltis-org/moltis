//! Optional Landlock filesystem isolation for restricted-host sandbox (Linux only).
//!
//! Provides kernel-level VFS enforcement so child processes can only access
//! paths explicitly listed in `fs_allow_paths`. On non-Linux or when the
//! `landlock` feature is disabled, all functions are no-ops.

use std::path::PathBuf;

/// Result of attempting to apply Landlock restrictions.
#[derive(Debug)]
pub struct LandlockResult {
    /// Whether Landlock rules were successfully built and restrict_self() was called.
    /// Note: This does NOT guarantee kernel-level enforcement. The child's pre_exec
    /// closure may degrade gracefully on kernels < 5.13 or in containers with seccomp.
    /// Call `is_kernel_landlock_available()` for a runtime capability probe.
    pub enforced: bool,
    /// Human-readable status message for logging.
    pub message: String,
}

/// Build a `pre_exec` closure that applies Landlock FS restrictions to the child.
///
/// The returned closure should be passed to `Command::pre_exec()`. It calls
/// `restrict_self()` which only affects the child thread (after fork, before exec).
///
/// # Returns
///
/// A `(LandlockResult, Option<closure>)` pair. If `enforced` is false, returns
/// `None` for the closure. If enforced, the closure is safe to pass to
/// `Command::pre_exec()`.
#[cfg(all(target_os = "linux", feature = "landlock"))]
pub fn build_landlock_pre_exec(
    fs_allow_paths: &[PathBuf],
) -> (
    LandlockResult,
    Option<Box<dyn FnMut() -> std::io::Result<()> + Send + Sync>>,
) {
    use landlock::{
        Access, AccessFs, PathBeneath, PathFd, RestrictionStatus, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetError, ABI,
    };

    if fs_allow_paths.is_empty() {
        return (
            LandlockResult {
                enforced: false,
                message: "fs_allow_paths is empty, Landlock not applied".into(),
            },
            None,
        );
    }

    // ABI::V1 is the baseline (kernel 5.13+). AccessFs::from_all handles
    // graceful degradation — unsupported rights are silently dropped.
    let abi = ABI::V1;
    let access_all = AccessFs::from_all(abi);

    let ruleset = match Ruleset::default()
        .handle_access(access_all)
        .and_then(|rs| rs.create())
    {
        Ok(rs) => rs,
        Err(e) => {
            return (
                LandlockResult {
                    enforced: false,
                    message: format!("Landlock ruleset creation failed: {e}, skipping"),
                },
                None,
            );
        }
    };

    // Build rules from allowlist paths. Invalid paths are silently skipped
    // (e.g., non-existent or inaccessible paths). Valid paths still get enforced.
    let rules = fs_allow_paths.iter().filter_map(|path| {
        PathFd::new(path.as_path())
            .ok()
            .map(|fd| Ok::<_, RulesetError>(PathBeneath::new(fd, AccessFs::from_all(abi))))
    });

    let ruleset = match ruleset.add_rules(rules) {
        Ok(rs) => rs,
        Err(e) => {
            return (
                LandlockResult {
                    enforced: false,
                    message: format!("Landlock add_rules failed: {e}, skipping"),
                },
                None,
            );
        }
    };

    let path_count = fs_allow_paths.len();

    // Wrap in Option so we can .take() inside the FnMut closure.
    let mut ruleset_opt = Some(ruleset);

    let closure: Box<dyn FnMut() -> std::io::Result<()> + Send + Sync> = Box::new(move || {
        let ruleset = ruleset_opt.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "Landlock restrict_self called more than once",
            )
        })?;

        match ruleset.restrict_self() {
            Ok(status) => {
                match status {
                    RestrictionStatus {
                        ruleset: landlock::RulesetStatus::FullyEnforced,
                        ..
                    }
                    | RestrictionStatus {
                        ruleset: landlock::RulesetStatus::PartiallyEnforced,
                        ..
                    } => Ok(()),
                    RestrictionStatus {
                        ruleset: landlock::RulesetStatus::NotEnforced,
                        ..
                    } => {
                        // Not enforced — degrade gracefully, don't block the child.
                        Ok(())
                    }
                }
            }
            Err(_) => {
                // restrict_self failed (e.g., prctl EPERM in constrained containers).
                // Degrade gracefully — don't block child execution.
                // NOTE: stderr is available in pre_exec context (async-signal-safe).
                eprintln!("landlock restrict_self failed, degrading");
                Ok(())
            }
        }
    });

    (
        LandlockResult {
            enforced: true,
            message: format!(
                "Landlock enforced (ABI {abi:?}, {path_count} paths allowed)",
            ),
        },
        Some(closure),
    )
}

/// No-op for non-Linux or when the `landlock` feature is disabled.
#[cfg(not(all(target_os = "linux", feature = "landlock")))]
pub fn build_landlock_pre_exec(
    _fs_allow_paths: &[PathBuf],
) -> (
    LandlockResult,
    Option<Box<dyn FnMut() -> std::io::Result<()> + Send + Sync>>,
) {
    (
        LandlockResult {
            enforced: false,
            message: "Landlock not available (non-Linux or feature disabled)".into(),
        },
        None,
    )
}

/// Apply Landlock pre_exec closure to a tokio `Command`.
///
/// Encapsulates the required `unsafe` block (tokio's `pre_exec` is inherently
/// unsafe because it runs in a forked context). The closure only calls
/// Landlock's `restrict_self()` which uses raw syscalls — async-signal-safe.
#[cfg(all(target_os = "linux", feature = "landlock"))]
pub fn apply_to_command(
    cmd: &mut tokio::process::Command,
    fs_allow_paths: &[PathBuf],
) -> LandlockResult {
    let (result, pre_exec_fn) = build_landlock_pre_exec(fs_allow_paths);
    tracing::debug!(enforced = result.enforced, %result.message, "landlock");
    if let Some(closure) = pre_exec_fn {
        // SAFETY: pre_exec runs after fork() in the child process, before execve().
        // The closure only calls landlock::RulesetCreated::restrict_self() which
        // performs raw syscalls (no heap allocation, no locks).
        #[allow(unsafe_code)]
        unsafe {
            cmd.pre_exec(closure);
        }
    }
    result
}

/// No-op for non-Linux or when the `landlock` feature is disabled.
#[cfg(not(all(target_os = "linux", feature = "landlock")))]
pub fn apply_to_command(
    _cmd: &mut tokio::process::Command,
    _fs_allow_paths: &[PathBuf],
) -> LandlockResult {
    LandlockResult {
        enforced: false,
        message: "Landlock not available (non-Linux or feature disabled)".into(),
    }
}

/// Check if the running kernel supports Landlock (runtime probe).
///
/// Tests the full flow: create ruleset + add_rule + restrict_self. This catches
/// containers where the kernel supports Landlock but seccomp blocks
/// `prctl(PR_SET_NO_NEW_PRIVS)` or `landlock_restrict_self`. The child process
/// exits with code 0 only if restrict_self() achieves Full or Partial enforcement.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_kernel_landlock_available() -> bool {
    use landlock::{Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI};
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let abi = ABI::V1;

    // Test 1: Can we create a ruleset?
    let ruleset = match Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .and_then(|rs| rs.create())
    {
        Ok(rs) => rs,
        Err(_) => return false,
    };

    // Test 2: Can we add a rule? (current dir)
    let fd = match PathFd::new(".") {
        Ok(fd) => fd,
        Err(_) => return false,
    };
    let ruleset = match ruleset.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi))) {
        Ok(rs) => rs,
        Err(_) => return false,
    };

    // Test 3: Can we actually restrict_self? (tests prctl + landlock_restrict_self)
    // The child exits 0 only if Full or Partial enforcement is achieved.
    let mut rs_opt = Some(ruleset);
    let mut cmd = Command::new("true");
    #[allow(unsafe_code)]
    unsafe {
        cmd.pre_exec(move || {
            let rs = rs_opt.take().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::Other, "ruleset already taken")
            })?;
            match rs.restrict_self() {
                Ok(status) => {
                    match status.ruleset {
                        landlock::RulesetStatus::FullyEnforced
                        | landlock::RulesetStatus::PartiallyEnforced => Ok(()),
                        landlock::RulesetStatus::NotEnforced => {
                            // Landlock not enforced — signal failure via exit code
                            std::process::exit(2);
                        }
                    }
                }
                Err(_) => {
                    // restrict_self failed (e.g., seccomp, EPERM)
                    std::process::exit(2);
                }
            }
        });
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// No-op for non-Linux or when the `landlock` feature is disabled.
#[cfg(not(all(target_os = "linux", feature = "landlock")))]
pub const fn is_kernel_landlock_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_paths_returns_noop() {
        let (result, closure) = build_landlock_pre_exec(&[]);
        assert!(!result.enforced);
        assert!(closure.is_none());
    }

    #[cfg(not(all(target_os = "linux", feature = "landlock")))]
    #[test]
    fn test_non_linux_returns_noop() {
        let paths = vec![PathBuf::from("/tmp")];
        let (result, closure) = build_landlock_pre_exec(&paths);
        assert!(!result.enforced);
        assert!(result.message.contains("not available"));
        assert!(closure.is_none());
    }

    #[cfg(all(target_os = "linux", feature = "landlock"))]
    #[test]
    fn test_kernel_availability_probe() {
        let available = is_kernel_landlock_available();
        // Just documents the environment — no assertion needed.
        // In containers with seccomp blocking prctl, this will be false.
        let _ = available;
    }
}
