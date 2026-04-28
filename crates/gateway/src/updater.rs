//! In-place update logic for moltis.
//!
//! Detects how moltis was installed and performs the appropriate upgrade:
//! - Binary installs: download tarball, verify checksum, replace binary.
//! - Homebrew: run `brew upgrade moltis`.
//! - Package managers (deb/rpm/arch/snap): return the manual command.
//! - Docker: replace binary in-place + warn about image persistence.

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use sha2::{Digest, Sha256};

use crate::update_check;

/// Global guard to prevent concurrent update attempts.
static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

const GITHUB_REPO: &str = "moltis-org/moltis";
const GPG_KEY_URL: &str = "https://pen.so/gpg.asc";

// ── Types ────────────────────────────────────────────────────

/// How moltis was installed on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Binary,
    Homebrew,
    Deb,
    Rpm,
    Arch,
    Snap,
    Docker,
    Unknown,
}

/// Result of an update attempt.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status")]
pub enum UpdateOutcome {
    /// Binary replaced, restart will apply the new version.
    #[serde(rename = "updated")]
    Updated {
        from: String,
        to: String,
        method: InstallMethod,
    },
    /// Already running the requested (or latest) version.
    #[serde(rename = "already_up_to_date")]
    AlreadyUpToDate { version: String },
    /// Update requires root/manual intervention — here is the command.
    #[serde(rename = "manual_required")]
    ManualRequired {
        method: InstallMethod,
        command: String,
        version: String,
    },
    /// Binary replaced inside a container — warn about persistence.
    #[serde(rename = "docker_updated")]
    DockerUpdated { from: String, to: String },
}

/// Errors specific to the update process.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("an update is already in progress")]
    AlreadyInProgress,
    #[error("cannot update: dev build (no release version)")]
    DevBuild,
    #[error("failed to resolve target version: {0}")]
    VersionResolution(String),
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("checksum verification failed")]
    ChecksumMismatch,
    #[error("extraction failed: {0}")]
    Extraction(String),
    #[error("binary replacement failed: {0}")]
    Replacement(String),
    #[error("homebrew upgrade failed: {0}")]
    Homebrew(String),
}

// ── Install method detection ─────────────────────────────────

/// Detect how moltis was installed by examining the executable path and
/// probing package managers.
pub fn detect_install_method() -> InstallMethod {
    if is_running_in_container() {
        return InstallMethod::Docker;
    }

    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(_) => return InstallMethod::Unknown,
    };
    let exe_str = exe.to_string_lossy();

    // Homebrew: path contains /Cellar/ or /homebrew/
    if exe_str.contains("/Cellar/") || exe_str.contains("/homebrew/") {
        return InstallMethod::Homebrew;
    }

    // Snap
    if exe_str.contains("/snap/") {
        return InstallMethod::Snap;
    }

    // System-installed binary — probe package managers
    if exe_str.starts_with("/usr/bin/") || exe_str.starts_with("/usr/local/bin/") {
        if probe_package_manager("dpkg", &["-S", &exe_str]) {
            return InstallMethod::Deb;
        }
        if probe_package_manager("rpm", &["-qf", &exe_str]) {
            return InstallMethod::Rpm;
        }
        if probe_package_manager("pacman", &["-Qo", &exe_str]) {
            return InstallMethod::Arch;
        }
    }

    // Default: standalone binary (e.g. ~/.local/bin/moltis)
    InstallMethod::Binary
}

fn probe_package_manager(cmd: &str, args: &[&str]) -> bool {
    std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn is_running_in_container() -> bool {
    if Path::new("/.dockerenv").exists() {
        return true;
    }
    if std::env::var_os("container").is_some() {
        return true;
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup")
        && (cgroup.contains("docker")
            || cgroup.contains("kubepods")
            || cgroup.contains("containerd"))
    {
        return true;
    }
    false
}

// ── Target triple ────────────────────────────────────────────

/// Map `std::env::consts::{OS, ARCH}` to the target triple used in release
/// artifact filenames (e.g. `x86_64-unknown-linux-gnu`).
fn target_triple() -> Result<&'static str, UpdateError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        (os, arch) => Err(UpdateError::UnsupportedPlatform(format!("{os}-{arch}"))),
    }
}

/// Build the release tag from a version string.
/// Date-based versions (YYYYMMDD.NN) are bare tags; semver gets a `v` prefix.
fn release_tag(version: &str) -> String {
    if version.len() > 8 && version.as_bytes().iter().take(8).all(u8::is_ascii_digit) {
        version.to_owned()
    } else {
        format!("v{version}")
    }
}

// ── Core update logic ────────────────────────────────────────

/// Perform an in-place update.
///
/// `requested_version` is `None` for "latest" or `Some("20260428.03")` for a
/// specific version.
pub async fn perform_update(
    client: &reqwest::Client,
    releases_url: &str,
    requested_version: Option<&str>,
) -> Result<UpdateOutcome, UpdateError> {
    if moltis_config::version::IS_DEV_BUILD {
        return Err(UpdateError::DevBuild);
    }

    if UPDATE_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(UpdateError::AlreadyInProgress);
    }

    // Drop guard ensures the flag is cleared even on panic.
    struct UpdateGuard;
    impl Drop for UpdateGuard {
        fn drop(&mut self) {
            UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
    }
    let _guard = UpdateGuard;

    perform_update_inner(client, releases_url, requested_version).await
}

async fn perform_update_inner(
    client: &reqwest::Client,
    releases_url: &str,
    requested_version: Option<&str>,
) -> Result<UpdateOutcome, UpdateError> {
    let current = moltis_config::version::VERSION;
    let target = resolve_target_version(client, releases_url, requested_version).await?;

    if target == current {
        return Ok(UpdateOutcome::AlreadyUpToDate {
            version: current.to_owned(),
        });
    }

    let method = detect_install_method();

    match method {
        InstallMethod::Binary | InstallMethod::Docker => {
            do_binary_update(client, current, &target, method).await
        },
        InstallMethod::Homebrew => do_homebrew_update(current, &target),
        InstallMethod::Deb => Ok(manual_command(
            method,
            &target,
            &format!(
                "curl -fsSL https://github.com/{GITHUB_REPO}/releases/download/{tag}/moltis_{ver}_{arch}.deb -o /tmp/moltis.deb && sudo dpkg -i /tmp/moltis.deb",
                tag = release_tag(&target),
                ver = target,
                arch = deb_arch(),
            ),
        )),
        InstallMethod::Rpm => Ok(manual_command(
            method,
            &target,
            &format!(
                "sudo rpm -U https://github.com/{GITHUB_REPO}/releases/download/{tag}/moltis-{ver}-1.{arch}.rpm",
                tag = release_tag(&target),
                ver = target,
                arch = std::env::consts::ARCH,
            ),
        )),
        InstallMethod::Arch => Ok(manual_command(
            method,
            &target,
            &format!(
                "curl -fsSL https://github.com/{GITHUB_REPO}/releases/download/{tag}/moltis-{ver}-1-{arch}.pkg.tar.zst -o /tmp/moltis.pkg.tar.zst && sudo pacman -U /tmp/moltis.pkg.tar.zst",
                tag = release_tag(&target),
                ver = target,
                arch = std::env::consts::ARCH,
            ),
        )),
        InstallMethod::Snap => Ok(manual_command(method, &target, "sudo snap refresh moltis")),
        InstallMethod::Unknown => Err(UpdateError::UnsupportedPlatform(
            "cannot determine install method".into(),
        )),
    }
}

fn manual_command(method: InstallMethod, version: &str, command: &str) -> UpdateOutcome {
    UpdateOutcome::ManualRequired {
        method,
        command: command.to_owned(),
        version: version.to_owned(),
    }
}

fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

// ── Version resolution ───────────────────────────────────────

async fn resolve_target_version(
    client: &reqwest::Client,
    releases_url: &str,
    requested: Option<&str>,
) -> Result<String, UpdateError> {
    if let Some(ver) = requested {
        return Ok(ver.trim().trim_start_matches(['v', 'V']).to_owned());
    }

    // Fetch latest from the releases manifest (same logic as update_check).
    let current = moltis_config::version::VERSION;
    let availability = update_check::fetch_update_availability(client, releases_url, current).await;

    if let Some(ver) = availability.latest_version {
        Ok(ver)
    } else {
        Err(UpdateError::VersionResolution(
            "could not determine latest version from releases manifest".into(),
        ))
    }
}

// ── Binary replacement ───────────────────────────────────────

async fn do_binary_update(
    client: &reqwest::Client,
    current: &str,
    target: &str,
    method: InstallMethod,
) -> Result<UpdateOutcome, UpdateError> {
    let triple = target_triple()?;
    let tag = release_tag(target);
    let tarball_name = format!("moltis-{target}-{triple}.tar.gz");
    let base_url = format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}");
    let tarball_url = format!("{base_url}/{tarball_name}");
    let checksum_url = format!("{tarball_url}.sha256");

    let tmp = tempfile::tempdir()
        .map_err(|e| UpdateError::Download(format!("failed to create temp dir: {e}")))?;

    // Download tarball
    let tarball_path = tmp.path().join(&tarball_name);
    download_file(client, &tarball_url, &tarball_path).await?;

    // Download and verify checksum — hard fail if missing or mismatched.
    let checksum_path = tmp.path().join("checksum.sha256");
    download_file(client, &checksum_url, &checksum_path).await?;
    let checksum_content = std::fs::read_to_string(&checksum_path)
        .map_err(|e| UpdateError::Download(format!("failed to read checksum file: {e}")))?;
    let expected = checksum_content
        .split_whitespace()
        .next()
        .unwrap_or_default();
    verify_sha256(&tarball_path, expected)?;

    // Best-effort GPG signature verification (informational only).
    let sig_url = format!("{tarball_url}.asc");
    let sig_path = tmp.path().join(format!("{tarball_name}.asc"));
    verify_gpg_signature(client, &sig_url, &sig_path, &tarball_path).await;

    // Extract binary from tarball
    let extracted = extract_binary_from_tarball(&tarball_path, tmp.path())?;

    // Replace current binary
    let current_exe = std::env::current_exe()
        .map_err(|e| UpdateError::Replacement(format!("cannot determine current exe: {e}")))?;
    replace_binary(&extracted, &current_exe)?;

    if method == InstallMethod::Docker {
        Ok(UpdateOutcome::DockerUpdated {
            from: current.to_owned(),
            to: target.to_owned(),
        })
    } else {
        Ok(UpdateOutcome::Updated {
            from: current.to_owned(),
            to: target.to_owned(),
            method,
        })
    }
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
) -> Result<(), UpdateError> {
    tracing::info!(url, "downloading update artifact");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| UpdateError::Download(format!("{url}: {e}")))?;

    if !response.status().is_success() {
        return Err(UpdateError::Download(format!(
            "{url}: HTTP {}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| UpdateError::Download(format!("{url}: {e}")))?;

    std::fs::write(dest, &bytes)
        .map_err(|e| UpdateError::Download(format!("write {}: {e}", dest.display())))?;

    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), UpdateError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| UpdateError::Download(format!("open for checksum: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| UpdateError::Download(format!("read for checksum: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        tracing::error!(expected, actual, "SHA-256 checksum mismatch");
        return Err(UpdateError::ChecksumMismatch);
    }
    Ok(())
}

/// Best-effort GPG signature verification.
///
/// Downloads the detached `.asc` signature and the maintainer's public key,
/// imports the key into a temporary keyring, and runs `gpg --verify`.
/// Logs the result but never fails the update — GPG may not be installed,
/// or the release may not have been GPG-signed yet (CI creates Sigstore
/// signatures; GPG signing is a separate manual step).
async fn verify_gpg_signature(
    client: &reqwest::Client,
    sig_url: &str,
    sig_path: &Path,
    artifact_path: &Path,
) {
    // Download .asc signature
    if download_file(client, sig_url, sig_path).await.is_err() {
        tracing::debug!("no GPG signature available for this release");
        return;
    }

    // Check that gpg is available
    if std::process::Command::new("gpg")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        tracing::debug!("gpg not found, skipping signature verification");
        return;
    }

    // Create a temporary GNUPG home to avoid polluting the user's keyring
    let gnupg_home = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return,
    };

    // Import the maintainer's public key
    let key_url = GPG_KEY_URL;
    let key_path = gnupg_home.path().join("maintainer.asc");
    if download_file(client, key_url, &key_path).await.is_err() {
        tracing::debug!("could not fetch maintainer GPG key");
        return;
    }

    let import = std::process::Command::new("gpg")
        .args(["--homedir"])
        .arg(gnupg_home.path())
        .args(["--batch", "--quiet", "--import"])
        .arg(&key_path)
        .output();

    if import.is_err() || !import.is_ok_and(|o| o.status.success()) {
        tracing::debug!("failed to import maintainer GPG key");
        return;
    }

    // Verify the signature
    let verify = std::process::Command::new("gpg")
        .args(["--homedir"])
        .arg(gnupg_home.path())
        .args(["--batch", "--verify"])
        .arg(sig_path)
        .arg(artifact_path)
        .output();

    match verify {
        Ok(output) if output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let signer = stderr
                .lines()
                .find(|l| l.contains("Good signature"))
                .unwrap_or("verified");
            tracing::info!("GPG signature verified: {signer}");
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("GPG signature verification failed: {stderr}");
        },
        Err(e) => {
            tracing::debug!("gpg verify command failed: {e}");
        },
    }
}

fn extract_binary_from_tarball(tarball: &Path, dest_dir: &Path) -> Result<PathBuf, UpdateError> {
    let file = std::fs::File::open(tarball)
        .map_err(|e| UpdateError::Extraction(format!("open tarball: {e}")))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    archive
        .unpack(dest_dir)
        .map_err(|e| UpdateError::Extraction(format!("unpack: {e}")))?;

    // The tarball should contain a `moltis` binary at the top level.
    let binary = dest_dir.join("moltis");
    if binary.exists() {
        return Ok(binary);
    }

    // Some tarballs might nest it in a directory
    for entry in std::fs::read_dir(dest_dir)
        .map_err(|e| UpdateError::Extraction(format!("read dir: {e}")))?
    {
        let entry = entry.map_err(|e| UpdateError::Extraction(format!("dir entry: {e}")))?;
        let candidate = entry.path().join("moltis");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(UpdateError::Extraction(
        "moltis binary not found in tarball".into(),
    ))
}

fn replace_binary(new_binary: &Path, current_exe: &Path) -> Result<(), UpdateError> {
    let parent = current_exe
        .parent()
        .ok_or_else(|| UpdateError::Replacement("no parent directory".into()))?;
    let old_path = parent.join(".moltis.old");

    // Move current binary aside
    if let Err(e) = std::fs::rename(current_exe, &old_path) {
        // On some systems rename fails across filesystems — try copy instead
        tracing::debug!("rename failed ({e}), trying copy");
        std::fs::copy(current_exe, &old_path)
            .map_err(|e| UpdateError::Replacement(format!("backup current binary: {e}")))?;
    }

    // Copy new binary into place
    if let Err(e) = std::fs::copy(new_binary, current_exe) {
        // Attempt to restore old binary
        let _ = std::fs::rename(&old_path, current_exe);
        return Err(UpdateError::Replacement(format!("install new binary: {e}")));
    }

    // Set executable permission
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        let _ = std::fs::set_permissions(current_exe, perms);
    }

    // Clean up old binary (best-effort)
    let _ = std::fs::remove_file(&old_path);

    Ok(())
}

// ── Homebrew ─────────────────────────────────────────────────

fn do_homebrew_update(current: &str, target: &str) -> Result<UpdateOutcome, UpdateError> {
    tracing::info!("running brew upgrade moltis");
    let output = std::process::Command::new("brew")
        .args(["upgrade", "moltis"])
        .output()
        .map_err(|e| UpdateError::Homebrew(format!("failed to run brew: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "already installed" is not an error
        if stderr.contains("already installed") || stderr.contains("already up-to-date") {
            return Ok(UpdateOutcome::AlreadyUpToDate {
                version: current.to_owned(),
            });
        }
        return Err(UpdateError::Homebrew(stderr.into_owned()));
    }

    Ok(UpdateOutcome::Updated {
        from: current.to_owned(),
        to: target.to_owned(),
        method: InstallMethod::Homebrew,
    })
}

// ── Restart helper ───────────────────────────────────────────

/// Re-exec the current process (reuse the restart pattern from tools_routes).
pub fn restart_process() {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            tracing::error!("failed to get current executable path: {e}");
            std::process::exit(1);
        },
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    tracing::info!(exe = %exe.display(), args = ?args, "re-executing after update");

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // exec() replaces the current process — this is intentional for self-update.
        // No user input is involved; exe and args come from the running process itself.
        let err = std::process::Command::new(&exe).args(&args).exec();
        tracing::error!("failed to exec: {err}");
        std::process::exit(1);
    }

    #[cfg(not(unix))]
    {
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                tracing::error!("failed to spawn new process: {e}");
                std::process::exit(1);
            },
        }
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_install_method_returns_something() {
        // On dev machines this will be Binary; in CI it depends on the env.
        let method = detect_install_method();
        // Just assert it doesn't panic and returns a valid variant
        assert!(matches!(
            method,
            InstallMethod::Binary
                | InstallMethod::Homebrew
                | InstallMethod::Deb
                | InstallMethod::Rpm
                | InstallMethod::Arch
                | InstallMethod::Snap
                | InstallMethod::Docker
                | InstallMethod::Unknown
        ));
    }

    #[test]
    fn target_triple_resolves() {
        let triple = target_triple();
        // Should succeed on macOS and Linux CI
        if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
            assert!(triple.is_ok(), "target_triple() failed: {triple:?}");
        }
    }

    #[test]
    fn release_tag_date_version() {
        assert_eq!(release_tag("20260428.03"), "20260428.03");
    }

    #[test]
    fn release_tag_semver() {
        assert_eq!(release_tag("0.10.18"), "v0.10.18");
    }

    #[test]
    fn deb_arch_mapping() {
        let arch = deb_arch();
        assert!(!arch.is_empty());
    }

    #[test]
    fn verify_sha256_correct() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").expect("write test file");
        // SHA-256 of "hello world"
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(&path, expected).is_ok());
    }

    #[test]
    fn verify_sha256_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").expect("write test file");
        assert!(verify_sha256(&path, "0000000000000000").is_err());
    }

    #[test]
    fn manual_command_builds_outcome() {
        let outcome = manual_command(InstallMethod::Deb, "20260428.03", "sudo dpkg -i foo.deb");
        match outcome {
            UpdateOutcome::ManualRequired {
                method,
                command,
                version,
            } => {
                assert_eq!(method, InstallMethod::Deb);
                assert!(command.contains("dpkg"));
                assert_eq!(version, "20260428.03");
            },
            _ => panic!("expected ManualRequired"),
        }
    }

    #[test]
    fn concurrent_guard_prevents_double_update() {
        // In dev builds, perform_update returns DevBuild before checking the guard.
        // Test the guard directly instead.
        assert!(
            UPDATE_IN_PROGRESS
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        );
        // Second attempt should fail
        assert!(
            UPDATE_IN_PROGRESS
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        );
        UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}
