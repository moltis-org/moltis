//! Starting the moltis daemon on this machine when it is not running.
//!
//! The desktop app never runs the daemon inside itself: it installs it as a
//! user service (launchd / systemd) through `moltis service install`, so the
//! daemon keeps running after the app quits and starts again at login.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

/// Where `moltis` is usually installed, beyond `PATH` (apps launched from
/// the Finder get a minimal `PATH`).
fn extra_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = home {
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".local/bin"));
    }
    dirs
}

/// Find an executable `moltis` in `path` or the usual install locations.
#[must_use]
pub fn find_moltis(path: Option<OsString>, home: Option<&Path>) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "moltis.exe"
    } else {
        "moltis"
    };
    path.iter()
        .flat_map(std::env::split_paths)
        .chain(extra_dirs(home))
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// `moltis service install` — installs and starts the user service.
/// Blocking; run it off the UI thread.
pub fn install_service(moltis: &Path) -> anyhow::Result<()> {
    let output = Command::new(moltis).args(["service", "install"]).output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("no output");
    anyhow::bail!(
        "moltis service install failed ({}): {detail}",
        output.status
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[cfg(unix)]
    fn write_script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("moltis");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn finds_binary_on_path_before_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(dir.path(), "exit 0");
        let found = find_moltis(Some(dir.path().as_os_str().to_owned()), None).unwrap();
        assert_eq!(found, script);
    }

    #[cfg(unix)]
    #[test]
    fn finds_binary_in_home_fallback() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".cargo/bin")).unwrap();
        let script = write_script(&home.path().join(".cargo/bin"), "exit 0");
        let found = find_moltis(Some(OsString::new()), Some(home.path()));
        // A system-wide moltis would win; only check ours is found otherwise.
        if !Path::new("/opt/homebrew/bin/moltis").exists()
            && !Path::new("/usr/local/bin/moltis").exists()
            && !Path::new("/usr/bin/moltis").exists()
        {
            assert_eq!(found.unwrap(), script);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("moltis"), "data").unwrap();
        let found = find_moltis(Some(dir.path().as_os_str().to_owned()), None);
        assert_ne!(found.as_deref(), Some(dir.path().join("moltis").as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn install_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let ok = write_script(dir.path(), "exit 0");
        install_service(&ok).unwrap();
        let failing = write_script(dir.path(), "echo 'launchctl said no' >&2; exit 3");
        let error = install_service(&failing).unwrap_err().to_string();
        assert!(error.contains("launchctl said no"), "{error}");
    }
}
