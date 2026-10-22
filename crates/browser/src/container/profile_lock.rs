//! Ownership of a browser profile's OS file lock.

use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
};

use super::{ContextExt, Error, Result};

/// Explicitly unlock when the owner releases the profile. Closing a plain
/// `File` is insufficient on Unix while another descriptor shares its open
/// file description, such as during a concurrent fork before exec. CLOEXEC
/// closes inherited descriptors at exec, but does not cover that interval.
#[derive(Debug)]
pub(super) struct ProfileLock {
    file: File,
}

impl ProfileLock {
    pub(super) fn acquire(dir: &Path) -> Result<Self> {
        let lock_path = dir.join(".moltis.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open profile lock at {}", lock_path.display()))?;

        file.try_lock().map_err(|error| {
            let message = match error {
                TryLockError::WouldBlock => format!(
                    "browser profile at {} is already in use by another Moltis browser container",
                    dir.display()
                ),
                TryLockError::Error(error) => format!(
                    "failed to lock browser profile at {}: {error}",
                    dir.display()
                ),
            };
            Error::LaunchFailed(message)
        })?;

        // Construct the guard only after successful acquisition: a failed
        // contender must never unlock another owner's lock.
        Ok(Self { file })
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        if let Err(error) = self.file.unlock() {
            tracing::warn!(%error, "failed to unlock browser profile");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        crate::container::{cleanup::profile_lock_to_quarantine, prepare_browserless_v2_profile},
    };

    #[test]
    fn browser_profile_lock_release_does_not_wait_for_duplicated_descriptor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let first_lock = prepare_browserless_v2_profile(temp_dir.path()).unwrap();
        // Model the descriptor retained by a concurrent fork without racing
        // an actual subprocess, sleeps, retries, or unsafe post-fork code.
        let inherited_descriptor = first_lock.file.try_clone().unwrap();
        let singleton = temp_dir.path().join("SingletonLock");
        std::fs::write(&singleton, "live-browser").unwrap();

        assert!(prepare_browserless_v2_profile(temp_dir.path()).is_err());
        assert!(singleton.is_file());
        drop(first_lock);

        let next_lock = prepare_browserless_v2_profile(temp_dir.path()).unwrap();
        assert!(!singleton.exists());
        // Closing the old descriptor must not release the next owner's lock.
        drop(inherited_descriptor);
        assert!(ProfileLock::acquire(temp_dir.path()).is_err());
        drop(next_lock);
        assert!(ProfileLock::acquire(temp_dir.path()).is_ok());
    }

    #[test]
    fn browser_profile_lock_quarantine_retains_ownership_with_duplicate() {
        let temp_dir = tempfile::tempdir().unwrap();
        let lock = prepare_browserless_v2_profile(temp_dir.path()).unwrap();
        let inherited_descriptor = lock.file.try_clone().unwrap();
        let retained = profile_lock_to_quarantine(Some(lock), false);

        assert!(ProfileLock::acquire(temp_dir.path()).is_err());
        drop(inherited_descriptor);
        assert!(ProfileLock::acquire(temp_dir.path()).is_err());
        drop(retained);
        assert!(ProfileLock::acquire(temp_dir.path()).is_ok());
    }

    #[test]
    fn browser_profile_lock_confirmed_cleanup_unlocks_with_duplicate() {
        let temp_dir = tempfile::tempdir().unwrap();
        let lock = prepare_browserless_v2_profile(temp_dir.path()).unwrap();
        let inherited_descriptor = lock.file.try_clone().unwrap();

        assert!(profile_lock_to_quarantine(Some(lock), true).is_none());
        let next_lock = ProfileLock::acquire(temp_dir.path()).unwrap();
        drop(inherited_descriptor);
        assert!(ProfileLock::acquire(temp_dir.path()).is_err());
        drop(next_lock);
    }
}
