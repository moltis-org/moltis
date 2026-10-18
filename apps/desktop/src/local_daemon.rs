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
    run_service_command(&service_binary(moltis)?, "install")
}

fn run_service_command(moltis: &Path, action: &str) -> anyhow::Result<()> {
    run_checked(
        Command::new(moltis).args(["service", action]),
        &format!("moltis service {action}"),
    )
}

/// Run `command`; a failure carries its output.
fn run_checked(command: &mut Command, what: &str) -> anyhow::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("no output");
    anyhow::bail!("{what} failed ({}): {detail}", output.status)
}

/// Start an installed service that is not running with the binary it was
/// installed with (`moltis service restart` rewrites nothing). Blocking; run
/// it off the UI thread.
pub fn start_installed_service(installed: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(home) = moltis_config::home_dir() {
        load_launch_agent(&home.join(LAUNCH_AGENT))?;
    }
    run_service_command(installed, "restart")
}

/// The launchd agent `moltis service install` writes, relative to home.
const LAUNCH_AGENT: &str = "Library/LaunchAgents/org.moltis.gateway.plist";

/// Load the saved launch agent when launchd has unloaded it (`launchctl
/// bootout`, or a failed launch): `service restart` only kickstarts a loaded
/// service, and the installed binary may predate any fix for that. The plist
/// itself is left as installed.
#[cfg(target_os = "macos")]
fn load_launch_agent(plist: &Path) -> anyhow::Result<()> {
    let uid = Command::new("id").arg("-u").output()?;
    let domain = format!("gui/{}", String::from_utf8_lossy(&uid.stdout).trim());
    let loaded = Command::new("launchctl")
        .args(["print", &format!("{domain}/org.moltis.gateway")])
        .output()?
        .status
        .success();
    if loaded {
        return Ok(());
    }
    run_checked(
        Command::new("launchctl")
            .arg("bootstrap")
            .arg(&domain)
            .arg(plist),
        "launchctl bootstrap",
    )
}

/// `moltis service restart`: restart the installed user service, for example
/// to pick up a newer bundled binary. Blocking; run it off the UI thread.
pub fn restart_service(moltis: &Path) -> anyhow::Result<()> {
    run_service_command(&service_binary(moltis)?, "restart")
}

/// The daemon bundled with this app, if it ships one.
#[must_use]
pub fn bundled_daemon() -> Option<PathBuf> {
    bundled_dir()
        .map(|dir| {
            dir.join(if cfg!(windows) {
                "moltis.exe"
            } else {
                "moltis"
            })
        })
        .filter(|path| path.is_file())
}

/// Where the service will run `moltis` from. An AppImage is mounted at a new
/// temporary path on every launch and unmounted when the app quits, so a
/// daemon inside it is copied to `<data_dir>/bin/moltis` for the service to
/// outlive the app. Anything else runs where it is.
#[must_use]
pub fn service_path(moltis: &Path) -> PathBuf {
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    stable_path(moltis, appdir.as_deref(), &moltis_config::data_dir())
        .unwrap_or_else(|| moltis.to_path_buf())
}

fn stable_path(moltis: &Path, appdir: Option<&Path>, data_dir: &Path) -> Option<PathBuf> {
    let appdir = appdir?;
    moltis
        .starts_with(appdir)
        .then(|| data_dir.join("bin").join("moltis"))
}

/// [`service_path`], copying the daemon there first when it differs. The copy
/// is renamed into place, so a daemon still running the old file is
/// unaffected until it restarts.
fn service_binary(moltis: &Path) -> anyhow::Result<PathBuf> {
    let target = service_path(moltis);
    if target != moltis {
        copy_executable(moltis, &target)?;
    }
    Ok(target)
}

fn copy_executable(from: &Path, to: &Path) -> anyhow::Result<()> {
    let dir = to
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", to.display()))?;
    std::fs::create_dir_all(dir)?;
    let partial = to.with_extension("partial");
    std::fs::copy(from, &partial)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&partial, to)?;
    Ok(())
}

/// The version this app was built as (`MOLTIS_VERSION` at build time), which
/// is also the version of the daemon it bundles. `None` in development builds.
#[must_use]
pub fn bundled_version() -> Option<&'static str> {
    option_env!("MOLTIS_VERSION").filter(|version| !version.is_empty())
}

/// The binary the installed user service runs, as `moltis service install`
/// wrote it: the launchd plist on macOS
/// (`~/Library/LaunchAgents/org.moltis.gateway.plist`); on Linux the systemd
/// unit (`~/.config/systemd/user/moltis.service`) or, without systemd, the
/// supervisor script in the data directory. `None` when no service is
/// installed.
#[must_use]
pub fn installed_service_binary(home: &Path, data_dir: &Path) -> Option<PathBuf> {
    let read = |path: PathBuf| std::fs::read_to_string(path).ok();
    if cfg!(target_os = "macos") {
        return read(home.join(LAUNCH_AGENT)).and_then(|plist| program_from_plist(&plist));
    }
    if cfg!(target_os = "linux") {
        return read(home.join(".config/systemd/user/moltis.service"))
            .and_then(|unit| program_from_systemd_unit(&unit))
            .or_else(|| {
                read(data_dir.join("moltis-service-supervisor.sh"))
                    .and_then(|script| program_from_supervisor(&script))
            });
    }
    None
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

/// The program of a systemd unit's `ExecStart=` line.
fn program_from_systemd_unit(unit: &str) -> Option<PathBuf> {
    let command = unit
        .lines()
        .find_map(|line| line.trim().strip_prefix("ExecStart="))?
        .trim();
    let program = match command.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    (!program.is_empty()).then(|| PathBuf::from(program))
}

/// The `MOLTIS_BIN='...'` assignment of the portable supervisor script.
fn program_from_supervisor(script: &str) -> Option<PathBuf> {
    let value = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("MOLTIS_BIN="))?
        .trim();
    let program = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .map_or_else(|| value.to_string(), |inner| inner.replace("'\\''", "'"));
    (!program.is_empty()).then(|| PathBuf::from(program))
}

/// How to bring back a local service that is installed but not running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoStart {
    /// The service is this app's but its binary is gone or another app copy:
    /// install this app's daemon.
    Install,
    /// Start the service as installed, keeping its binary and options (it may
    /// be a Homebrew or cargo install).
    Start(PathBuf),
}

/// Decide how to start a stopped service without asking. `None` means no
/// service is installed (the user has not chosen to run moltis here) or
/// nothing can start it.
#[must_use]
pub fn auto_start(installed: Option<&Path>, bundled: Option<&Path>) -> Option<AutoStart> {
    let installed = installed?;
    match check_service(Some(installed), bundled, None, None) {
        ServiceCheck::Reinstall => Some(AutoStart::Install),
        _ if installed.exists() => Some(AutoStart::Start(installed.to_path_buf())),
        _ => None,
    }
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
    fn reads_program_from_systemd_unit() {
        let unit =
            "[Service]\nType=simple\nExecStart=/home/me/.moltis/bin/moltis --log-level info\n";
        assert_eq!(
            program_from_systemd_unit(unit),
            Some(PathBuf::from("/home/me/.moltis/bin/moltis"))
        );
        assert_eq!(
            program_from_systemd_unit("ExecStart=\"/opt/my apps/moltis\" --log-level info"),
            Some(PathBuf::from("/opt/my apps/moltis"))
        );
        assert_eq!(program_from_systemd_unit("[Service]\n"), None);
    }

    #[test]
    fn reads_program_from_supervisor_script() {
        let script =
            "#!/bin/sh\nset -u\n\nMOLTIS_BIN='/home/me/.moltis/bin/moltis'\nDATA_DIR='/x'\n";
        assert_eq!(
            program_from_supervisor(script),
            Some(PathBuf::from("/home/me/.moltis/bin/moltis"))
        );
        assert_eq!(
            program_from_supervisor("MOLTIS_BIN='/it'\\''s/moltis'"),
            Some(PathBuf::from("/it's/moltis"))
        );
        assert_eq!(program_from_supervisor("#!/bin/sh\n"), None);
    }

    #[test]
    fn appimage_daemons_move_to_the_data_dir() {
        let appdir = Path::new("/tmp/.mount_MoltisAbc");
        let data = Path::new("/home/me/.moltis");
        assert_eq!(
            stable_path(&appdir.join("usr/bin/moltis"), Some(appdir), data),
            Some(data.join("bin/moltis"))
        );
        // Outside an AppImage, or a daemon not inside it, runs where it is.
        assert_eq!(
            stable_path(Path::new("/usr/bin/moltis"), Some(appdir), data),
            None
        );
        assert_eq!(
            stable_path(&appdir.join("usr/bin/moltis"), None, data),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn copies_executable_over_an_existing_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let from = write_script(dir.path(), "exit 0");
        let to = dir.path().join("bin/moltis");
        copy_executable(&from, &to).unwrap();
        std::fs::write(&from, "#!/bin/sh\nexit 1\n").unwrap();
        copy_executable(&from, &to).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "#!/bin/sh\nexit 1\n");
        assert_ne!(
            std::fs::metadata(&to).unwrap().permissions().mode() & 0o111,
            0
        );
        assert!(!dir.path().join("bin/moltis.partial").exists());
    }

    #[test]
    fn auto_start_keeps_other_installs() {
        let bundled = Path::new("/Applications/Moltis Desktop.app/Contents/MacOS/moltis");
        let dir = tempfile::tempdir().unwrap();
        let homebrew = dir.path().join("moltis");
        std::fs::write(&homebrew, "").unwrap();
        // A Homebrew or cargo service is started as installed, never replaced.
        assert_eq!(
            auto_start(Some(&homebrew), Some(bundled)),
            Some(AutoStart::Start(homebrew.clone()))
        );
        assert_eq!(
            auto_start(Some(&homebrew), None),
            Some(AutoStart::Start(homebrew))
        );
        // This app's service whose binary moved is reinstalled.
        assert_eq!(
            auto_start(Some(Path::new("/nonexistent/old/moltis")), Some(bundled)),
            Some(AutoStart::Install)
        );
        // Nothing installed, or nothing left to run.
        assert_eq!(auto_start(None, Some(bundled)), None);
        assert_eq!(
            auto_start(Some(Path::new("/nonexistent/moltis")), None),
            None
        );
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
