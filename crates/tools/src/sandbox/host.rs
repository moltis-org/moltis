//! Host package provisioning (Debian/Ubuntu apt-get).

use tracing::{info, warn};

use {
    super::containers::{container_exec_shell_args, is_cli_available},
    crate::error::Result,
};

/// Install configured packages inside a container via `apt-get`.
///
/// `cli` is the container CLI binary name (e.g. `"docker"` or `"container"`).
pub(crate) async fn provision_packages(
    cli: &str,
    container_name: &str,
    packages: &[String],
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let pkg_list = packages.join(" ");
    info!(container = container_name, packages = %pkg_list, "provisioning sandbox packages");
    let output = tokio::process::Command::new(cli)
        .args(container_exec_shell_args(
            cli,
            container_name,
            format!("apt-get update -qq && apt-get install -y -qq {pkg_list} 2>&1 | tail -5"),
        ))
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(
            container = container_name,
            %stderr,
            "package provisioning failed (non-fatal)"
        );
    }
    Ok(())
}

/// Check whether the current process is running as root (UID 0).
pub(crate) fn is_running_as_root() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("id")
            .args(["-u"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .is_some_and(|uid| uid.trim() == "0")
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Check whether the current host is Debian/Ubuntu (has `/etc/debian_version`
/// and `apt-get` on PATH).
pub fn is_debian_host() -> bool {
    std::path::Path::new("/etc/debian_version").exists() && is_cli_available("apt-get")
}

pub(crate) fn host_package_name_candidates(pkg: &str) -> Vec<String> {
    let mut candidates = vec![pkg.to_string()];

    if let Some(base) = pkg.strip_suffix("t64") {
        candidates.push(base.to_string());
        return candidates;
    }

    let looks_like_soname_package =
        pkg.starts_with("lib") && pkg.chars().last().is_some_and(|c| c.is_ascii_digit());
    if looks_like_soname_package {
        candidates.push(format!("{pkg}t64"));
    }

    candidates
}

async fn is_installed_dpkg_package(pkg: &str) -> bool {
    tokio::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", pkg])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).contains("install ok installed")
        })
}

async fn resolve_installed_host_package(pkg: &str) -> Option<String> {
    for candidate in host_package_name_candidates(pkg) {
        if is_installed_dpkg_package(&candidate).await {
            return Some(candidate);
        }
    }
    None
}

async fn is_apt_package_available(pkg: &str) -> bool {
    tokio::process::Command::new("apt-cache")
        .args(["show", pkg])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn resolve_installable_host_package(pkg: &str) -> Option<String> {
    for candidate in host_package_name_candidates(pkg) {
        if is_apt_package_available(&candidate).await {
            return Some(candidate);
        }
    }
    None
}

/// Result of host package provisioning.
#[derive(Debug, Clone)]
pub struct HostProvisionResult {
    /// Packages that were actually installed.
    pub installed: Vec<String>,
    /// Packages that were already present.
    pub skipped: Vec<String>,
    /// Whether sudo was used for installation.
    pub used_sudo: bool,
}

/// Install configured packages directly on the host via `apt-get`.
///
/// Used when the sandbox backend is `"none"` (no container runtime) and the
/// host is Debian/Ubuntu. Returns `None` if packages are empty or the host
/// is not Debian-based.
///
/// This is **non-fatal**: failures are logged as warnings and do not block
/// startup.
#[tracing::instrument(skip(packages), fields(package_count = packages.len()))]
pub async fn provision_host_packages(packages: &[String]) -> Result<Option<HostProvisionResult>> {
    if packages.is_empty() || !is_debian_host() {
        return Ok(None);
    }

    // Determine which packages are already installed via dpkg-query.
    let mut missing = Vec::new();
    let mut skipped = Vec::new();

    for pkg in packages {
        if resolve_installed_host_package(pkg).await.is_some() {
            skipped.push(pkg.clone());
        } else {
            missing.push(pkg.clone());
        }
    }

    if missing.is_empty() {
        info!(
            skipped = skipped.len(),
            "all host packages already installed"
        );
        return Ok(Some(HostProvisionResult {
            installed: Vec::new(),
            skipped,
            used_sudo: false,
        }));
    }

    // Check if we can use sudo without a password prompt.
    let has_sudo = tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success());

    let is_root = is_running_as_root();

    if !has_sudo && !is_root {
        info!(
            missing = missing.len(),
            "not running as root and passwordless sudo unavailable; \
             skipping host package provisioning (install packages in the container image instead)"
        );
        return Ok(Some(HostProvisionResult {
            installed: Vec::new(),
            skipped: missing,
            used_sudo: false,
        }));
    }

    let apt_update = if has_sudo {
        "sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq".to_string()
    } else {
        "DEBIAN_FRONTEND=noninteractive apt-get update -qq".to_string()
    };

    // Run apt-get update.
    let update_out = tokio::process::Command::new("sh")
        .args(["-c", &apt_update])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;
    if let Ok(ref out) = update_out
        && !out.status.success()
    {
        let stderr = String::from_utf8_lossy(&out.stderr);
        warn!(%stderr, "apt-get update failed (non-fatal)");
    }

    // Resolve distro-specific package aliases after apt metadata is refreshed.
    let mut installable = Vec::new();
    let mut remapped = Vec::new();
    let mut unavailable = Vec::new();
    for pkg in &missing {
        match resolve_installable_host_package(pkg).await {
            Some(host_pkg) => {
                if host_pkg != *pkg {
                    remapped.push(format!("{pkg}->{host_pkg}"));
                }
                installable.push(host_pkg);
            },
            None => unavailable.push(pkg.clone()),
        }
    }
    installable.sort_unstable();
    installable.dedup();

    if !remapped.is_empty() {
        info!(
            count = remapped.len(),
            remapped = %remapped.join(", "),
            "resolved distro-specific package aliases for host provisioning"
        );
    }
    if !unavailable.is_empty() {
        warn!(
            packages = %unavailable.join(" "),
            "host package(s) unavailable on this distro; skipping"
        );
        skipped.extend(unavailable);
    }
    if installable.is_empty() {
        info!(
            skipped = skipped.len(),
            "no installable host packages after distro compatibility resolution"
        );
        return Ok(Some(HostProvisionResult {
            installed: Vec::new(),
            skipped,
            used_sudo: has_sudo,
        }));
    }

    let pkg_list = installable.join(" ");
    let apt_install = if has_sudo {
        format!("sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {pkg_list}")
    } else {
        format!("DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {pkg_list}")
    };

    info!(
        packages = %pkg_list,
        sudo = has_sudo,
        "provisioning host packages"
    );

    // Run apt-get install.
    let install_out = tokio::process::Command::new("sh")
        .args(["-c", &apt_install])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match install_out {
        Ok(out) if out.status.success() => {
            info!(
                installed = installable.len(),
                skipped = skipped.len(),
                "host packages provisioned"
            );
            Ok(Some(HostProvisionResult {
                installed: installable,
                skipped,
                used_sudo: has_sudo,
            }))
        },
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                %stderr,
                "apt-get install failed (non-fatal)"
            );
            Ok(Some(HostProvisionResult {
                installed: Vec::new(),
                skipped,
                used_sudo: has_sudo,
            }))
        },
        Err(e) => {
            warn!(%e, "failed to run apt-get install (non-fatal)");
            Ok(Some(HostProvisionResult {
                installed: Vec::new(),
                skipped,
                used_sudo: has_sudo,
            }))
        },
    }
}
