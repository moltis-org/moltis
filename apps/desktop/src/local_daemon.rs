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

/// The directory of the running executable. A packaged app ships `moltis`
/// next to it (`Moltis Desktop.app/Contents/MacOS/moltis`).
#[must_use]
pub fn bundled_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// Find an executable `moltis`: the one bundled with the app first, then
/// `path`, then the usual install locations.
#[must_use]
pub fn find_moltis(
    bundled: Option<&Path>,
    path: Option<OsString>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "moltis.exe"
    } else {
        "moltis"
    };
    bundled
        .map(Path::to_path_buf)
        .into_iter()
        .chain(path.iter().flat_map(std::env::split_paths))
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
    run_service_command(moltis, "install")
}

fn run_service_command(moltis: &Path, action: &str) -> anyhow::Result<()> {
    let output = Command::new(moltis).args(["service", action]).output()?;
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
        "moltis service {action} failed ({}): {detail}",
        output.status
    )
}

/// `moltis service restart`: restart the installed user service, for example
/// to pick up a newer bundled binary. Blocking; run it off the UI thread.
pub fn restart_service(moltis: &Path) -> anyhow::Result<()> {
    run_service_command(moltis, "restart")
}

/// The version this app was built as (`MOLTIS_VERSION` at build time), which
/// is also the version of the daemon it bundles. `None` in development builds.
#[must_use]
pub fn bundled_version() -> Option<&'static str> {
    option_env!("MOLTIS_VERSION").filter(|version| !version.is_empty())
}

/// The binary the installed launchd service runs, read from its plist
/// (`~/Library/LaunchAgents/org.moltis.gateway.plist`, written by
/// `moltis service install`). `None` when no service is installed, or on
/// other platforms.
#[must_use]
pub fn installed_service_binary(home: &Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let plist = home.join("Library/LaunchAgents/org.moltis.gateway.plist");
    let text = std::fs::read_to_string(plist).ok()?;
    program_from_plist(&text)
}

/// The first `ProgramArguments` entry of a launchd plist.
fn program_from_plist(plist: &str) -> Option<PathBuf> {
    let after_key = &plist[plist.find("<key>ProgramArguments</key>")?..];
    let start = after_key.find("<string>")? + "<string>".len();
    let end = after_key[start..].find("</string>")? + start;
    let program = after_key[start..end]
        .trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    (!program.is_empty()).then(|| PathBuf::from(program))
}

/// What the app should do about the installed service once connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceCheck {
    /// Nothing to do: no bundled daemon, no service, or someone else's.
    Fine,
    /// The service runs a binary that is gone (the app was moved or deleted)
    /// or another copy of this app: install this app's daemon instead.
    Reinstall,
    /// The service runs this app's daemon, but an older version is still
    /// running (the app was updated in place): it needs a restart.
    Restart { running: String, bundled: String },
}

/// Decide what to do about the service. `installed` is what the service runs,
/// `bundled` this app's daemon, `running` the connected daemon's version.
#[must_use]
pub fn check_service(
    installed: Option<&Path>,
    bundled: Option<&Path>,
    running: Option<&str>,
    bundled_version: Option<&str>,
) -> ServiceCheck {
    let (Some(installed), Some(bundled)) = (installed, bundled) else {
        return ServiceCheck::Fine;
    };
    if installed != bundled {
        // A daemon the user installed some other way (Homebrew, cargo) is
        // left alone; only a missing binary or another app copy is replaced.
        let ours = installed
            .ancestors()
            .any(|dir| dir.extension().is_some_and(|ext| ext == "app"));
        return if ours || !installed.exists() {
            ServiceCheck::Reinstall
        } else {
            ServiceCheck::Fine
        };
    }
    match (running, bundled_version) {
        (Some(running), Some(bundled)) if running != bundled => ServiceCheck::Restart {
            running: running.to_string(),
            bundled: bundled.to_string(),
        },
        _ => ServiceCheck::Fine,
    }
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
        let found = find_moltis(None, Some(dir.path().as_os_str().to_owned()), None).unwrap();
        assert_eq!(found, script);
    }

    #[cfg(unix)]
    #[test]
    fn bundled_binary_wins_over_path() {
        let bundled = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let script = write_script(bundled.path(), "exit 0");
        write_script(on_path.path(), "exit 0");
        let found = find_moltis(
            Some(bundled.path()),
            Some(on_path.path().as_os_str().to_owned()),
            None,
        )
        .unwrap();
        assert_eq!(found, script);
    }

    #[cfg(unix)]
    #[test]
    fn finds_binary_in_home_fallback() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".cargo/bin")).unwrap();
        let script = write_script(&home.path().join(".cargo/bin"), "exit 0");
        let found = find_moltis(None, Some(OsString::new()), Some(home.path()));
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
        let found = find_moltis(None, Some(dir.path().as_os_str().to_owned()), None);
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

    #[test]
    fn reads_program_from_service_plist() {
        let plist = r#"<plist><dict>
  <key>Label</key>
  <string>org.moltis.gateway</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Applications/Moltis Desktop.app/Contents/MacOS/moltis</string>
    <string>--log-level</string>
  </array></dict></plist>"#;
        assert_eq!(
            program_from_plist(plist),
            Some(PathBuf::from(
                "/Applications/Moltis Desktop.app/Contents/MacOS/moltis"
            ))
        );
        assert_eq!(
            program_from_plist(
                "<key>ProgramArguments</key><array><string>/a&amp;b/moltis</string>"
            ),
            Some(PathBuf::from("/a&b/moltis"))
        );
        assert_eq!(program_from_plist("<plist/>"), None);
    }

    #[test]
    fn service_checks() {
        let bundled = Path::new("/Applications/Moltis Desktop.app/Contents/MacOS/moltis");
        let check = |installed: &str, running: Option<&str>, version: Option<&str>| {
            check_service(Some(Path::new(installed)), Some(bundled), running, version)
        };
        // Same binary, same version.
        assert_eq!(
            check(bundled.to_str().unwrap(), Some("1"), Some("1")),
            ServiceCheck::Fine
        );
        // Updated in place: the running daemon is older.
        assert_eq!(
            check(bundled.to_str().unwrap(), Some("1"), Some("2")),
            ServiceCheck::Restart {
                running: "1".into(),
                bundled: "2".into()
            }
        );
        // Development builds have no version to compare.
        assert_eq!(
            check(bundled.to_str().unwrap(), Some("1"), None),
            ServiceCheck::Fine
        );
        // Moved app: the old path is gone, or is another app copy.
        assert_eq!(
            check("/nonexistent/old/moltis", None, None),
            ServiceCheck::Reinstall
        );
        assert_eq!(
            check(
                "/Users/me/Downloads/Moltis Desktop.app/Contents/MacOS/moltis",
                None,
                None
            ),
            ServiceCheck::Reinstall
        );
        // A daemon installed another way is left alone.
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("moltis");
        std::fs::write(&other, "").unwrap();
        assert_eq!(
            check(other.to_str().unwrap(), Some("1"), Some("2")),
            ServiceCheck::Fine
        );
        // Nothing bundled or nothing installed.
        assert_eq!(
            check_service(None, Some(bundled), None, None),
            ServiceCheck::Fine
        );
        assert_eq!(
            check_service(Some(bundled), None, None, None),
            ServiceCheck::Fine
        );
    }
}
