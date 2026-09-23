//! Path resolution, mount detection, and home persistence directories.

use std::{
    collections::HashMap,
    path::{Path as FsPath, PathBuf},
    sync::{Mutex, OnceLock},
};

use tracing::warn;

use {
    super::{
        containers::{is_cli_available, is_docker_daemon_available, should_use_docker_backend},
        types::{
            HomePersistence, ManagedFilesMount, SANDBOX_FILES_DIR, SANDBOX_HOME_DIR, SandboxConfig,
            SandboxId, SandboxUser, WorkspaceMount, sanitize_path_component,
        },
    },
    crate::error::{Error, Result},
};

pub(crate) static HOST_DATA_DIR_CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManagedFilesPath {
    Unmanaged,
    Unavailable,
    ReadOnly(PathBuf),
    ReadWrite(PathBuf),
}

pub(crate) fn configured_host_data_dir(config: &SandboxConfig) -> Option<PathBuf> {
    let guest_data_dir = moltis_config::data_dir();
    let path = config
        .host_data_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())?;
    if path.is_absolute() {
        return Some(path.clone());
    }
    Some(guest_data_dir.join(path))
}

pub(crate) fn host_data_dir_cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    HOST_DATA_DIR_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn detect_host_data_dir(cli: &str, guest_data_dir: &FsPath) -> Option<PathBuf> {
    let cache_key = format!("{cli}:{}", guest_data_dir.display());
    {
        let guard = host_data_dir_cache()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(cached) = guard.get(&cache_key) {
            return Some(cached.clone());
        }
    }

    let detected = moltis_config::container_mounts::detect_host_data_dir_with_references(
        cli,
        guest_data_dir,
        &moltis_config::container_mounts::current_container_references(),
    );

    if let Some(path) = detected.clone() {
        let mut guard = host_data_dir_cache()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.insert(cache_key, path);
    }
    detected
}

pub(crate) fn detected_container_cli(config: &SandboxConfig) -> Option<&'static str> {
    match config.backend.as_str() {
        "docker" => Some("docker"),
        "podman" => Some("podman"),
        "auto" => {
            if is_cli_available("podman") {
                Some("podman")
            } else if should_use_docker_backend(
                is_cli_available("docker"),
                is_docker_daemon_available(),
            ) || is_cli_available("docker")
            {
                Some("docker")
            } else {
                None
            }
        },
        _ => None,
    }
}

pub(crate) fn host_visible_data_dir(config: &SandboxConfig, cli: Option<&str>) -> PathBuf {
    let guest_data_dir = moltis_config::data_dir();
    if let Some(configured) = configured_host_data_dir(config) {
        moltis_config::set_host_data_dir_hint(configured.clone());
        return configured;
    }
    if let Some(cli) = cli
        && let Some(detected) = detect_host_data_dir(cli, &guest_data_dir)
    {
        // Publish the detected spelling, so the mount-source denylist in
        // `moltis-config` compares a source against the data directory as the
        // *host* names it rather than against this container's own path. The
        // config loader publishes the configured spelling; this is the other
        // half, and it is why the denylist covers an install that never set
        // `host_data_dir` - from the first container start onwards.
        moltis_config::set_host_data_dir_hint(detected.clone());
        return detected;
    }
    guest_data_dir
}

pub(crate) fn host_visible_path(
    config: &SandboxConfig,
    cli: Option<&str>,
    path: &FsPath,
) -> PathBuf {
    let guest_data_dir = moltis_config::data_dir();
    let Ok(relative_path) = path.strip_prefix(&guest_data_dir) else {
        return path.to_path_buf();
    };
    let host_data_dir = host_visible_data_dir(config, cli);
    if relative_path.as_os_str().is_empty() {
        host_data_dir
    } else {
        host_data_dir.join(relative_path)
    }
}

pub(crate) fn host_visible_managed_files_dir(config: &SandboxConfig, cli: Option<&str>) -> PathBuf {
    host_visible_path(config, cli, &moltis_config::managed_files_dir())
}

pub(crate) fn ensure_managed_files_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
) -> Result<PathBuf> {
    std::fs::create_dir_all(moltis_config::managed_files_dir())?;
    Ok(host_visible_managed_files_dir(config, cli))
}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_managed_files_none_mask_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
) -> Result<PathBuf> {
    let guest_visible = moltis_config::data_dir()
        .join("sandbox")
        .join("masks")
        .join("managed-files-none");
    std::fs::create_dir_all(&guest_visible)?;
    Ok(host_visible_path(config, cli, &guest_visible))
}

fn normalize_guest_path(path: &FsPath) -> PathBuf {
    use std::path::Component;

    path.components()
        .fold(PathBuf::new(), |mut normalized, component| {
            match component {
                Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                Component::RootDir => normalized.push(FsPath::new(std::path::MAIN_SEPARATOR_STR)),
                Component::CurDir => {},
                Component::ParentDir => {
                    normalized.pop();
                },
                Component::Normal(part) => normalized.push(part),
            }
            normalized
        })
}

pub(crate) fn resolve_managed_files_guest_path_on_host(
    config: &SandboxConfig,
    cli: Option<&str>,
    guest_path: &FsPath,
) -> ManagedFilesPath {
    let normalized = normalize_guest_path(guest_path);
    let canonical_guest_dir = FsPath::new(SANDBOX_FILES_DIR);
    let legacy_guest_dir = moltis_config::managed_files_dir();
    let relative = normalized
        .strip_prefix(canonical_guest_dir)
        .or_else(|_| normalized.strip_prefix(&legacy_guest_dir));
    let Ok(relative) = relative else {
        return ManagedFilesPath::Unmanaged;
    };

    if config.managed_files_mount == ManagedFilesMount::None {
        return ManagedFilesPath::Unavailable;
    }

    let host_path = host_visible_managed_files_dir(config, cli).join(relative);
    match config.managed_files_mount {
        ManagedFilesMount::None => ManagedFilesPath::Unavailable,
        ManagedFilesMount::Ro => ManagedFilesPath::ReadOnly(host_path),
        ManagedFilesMount::Rw => ManagedFilesPath::ReadWrite(host_path),
    }
}

pub(crate) fn resolve_workspace_guest_path_on_host(
    config: &SandboxConfig,
    cli: Option<&str>,
    guest_path: &FsPath,
) -> Option<PathBuf> {
    if config.workspace_mount == WorkspaceMount::None {
        return None;
    }
    let guest_workspace_dir = moltis_config::data_dir();
    let relative_path = guest_path.strip_prefix(&guest_workspace_dir).ok()?;
    let host_workspace_dir = host_visible_data_dir(config, cli);
    Some(if relative_path.as_os_str().is_empty() {
        host_workspace_dir
    } else {
        host_workspace_dir.join(relative_path)
    })
}

pub(crate) fn sandbox_home_persistence_base_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
) -> PathBuf {
    host_visible_path(
        config,
        cli,
        &moltis_config::data_dir().join("sandbox").join("home"),
    )
}

pub(crate) fn default_shared_home_dir(config: &SandboxConfig, cli: Option<&str>) -> PathBuf {
    sandbox_home_persistence_base_dir(config, cli).join("shared")
}

pub(crate) fn resolve_shared_home_dir(config: &SandboxConfig, cli: Option<&str>) -> PathBuf {
    let Some(path) = config
        .shared_home_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return default_shared_home_dir(config, cli);
    };
    if path.is_absolute() {
        return host_visible_path(config, cli, path);
    }
    host_visible_path(config, cli, &moltis_config::data_dir().join(path))
}

/// Effective host path used when shared home persistence is enabled.
pub fn shared_home_dir_path(config: &SandboxConfig) -> PathBuf {
    resolve_shared_home_dir(config, detected_container_cli(config))
}

/// Does the operator's config pin an explicit shared home directory?
fn has_operator_shared_home(config: &SandboxConfig) -> bool {
    config
        .shared_home_dir
        .as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty())
}

/// Give a home directory its per-uid leaf when the session runs as a user.
///
/// A `run_as` session never shares a HOME with a root session. Every root
/// session writes into the plain directory and keeps creating root-owned
/// children there, so a `run_as` session pointed at the same path would fail
/// or silently degrade later - and only containers of this uid ever write into
/// the per-uid leaf, so no ownership walk is needed.
fn with_run_as_leaf(path: PathBuf, run_as: Option<&SandboxUser>) -> PathBuf {
    match run_as {
        Some(user) => path.join("user").join(user.uid().to_string()),
        None => path,
    }
}

pub(crate) fn sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
    id: &SandboxId,
    run_as: Option<&SandboxUser>,
) -> Option<PathBuf> {
    let base = sandbox_home_persistence_base_dir(config, cli);
    match config.home_persistence {
        HomePersistence::Off => None,
        HomePersistence::Shared => {
            // With an operator-pinned shared home the per-uid dir hangs under
            // it, because honouring that setting is the point - silently
            // overriding it is exactly the objection that ruled out pinning
            // `home_persistence = "session"`. With no operator setting it is a
            // sibling of `shared` rather than a child, so nothing a root
            // session already wrote there is inherited.
            let root = if has_operator_shared_home(config) {
                resolve_shared_home_dir(config, cli)
            } else if run_as.is_some() {
                base
            } else {
                base.join("shared")
            };
            Some(with_run_as_leaf(root, run_as))
        },
        HomePersistence::Session => Some(with_run_as_leaf(
            base.join("session").join(sanitize_path_component(&id.key)),
            run_as,
        )),
    }
}

pub(crate) fn resolve_home_persistence_guest_path_on_host(
    config: &SandboxConfig,
    cli: Option<&str>,
    id: &SandboxId,
    run_as: Option<&SandboxUser>,
    guest_path: &FsPath,
) -> Option<PathBuf> {
    let guest_home_dir = FsPath::new(SANDBOX_HOME_DIR);
    let relative_path = guest_path.strip_prefix(guest_home_dir).ok()?;
    let host_home_dir = sandbox_home_persistence_host_dir(config, cli, id, run_as)?;
    Some(if relative_path.as_os_str().is_empty() {
        host_home_dir
    } else {
        host_home_dir.join(relative_path)
    })
}

pub(crate) fn guest_visible_sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    id: &SandboxId,
    run_as: Option<&SandboxUser>,
) -> Option<PathBuf> {
    let base = moltis_config::data_dir().join("sandbox").join("home");
    match config.home_persistence {
        HomePersistence::Off => None,
        HomePersistence::Shared => {
            let root = config
                .shared_home_dir
                .as_ref()
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| {
                    if path.is_absolute() {
                        path.clone()
                    } else {
                        moltis_config::data_dir().join(path)
                    }
                })
                .unwrap_or_else(|| {
                    if run_as.is_some() {
                        base.clone()
                    } else {
                        base.join("shared")
                    }
                });
            Some(with_run_as_leaf(root, run_as))
        },
        HomePersistence::Session => Some(with_run_as_leaf(
            base.join("session").join(sanitize_path_component(&id.key)),
            run_as,
        )),
    }
}

/// Is `path` writable by the uid the container will run as?
///
/// Checked from the gateway's own process, which is not that uid, so this reads
/// the mode bits POSIX would apply rather than trying the write: owner bits
/// when the owner matches, group bits when the group matches, other bits
/// otherwise - and never both, which is how POSIX resolves it.
#[cfg(unix)]
fn check_writable_by(path: &FsPath, user: &SandboxUser) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)?;
    let mode = metadata.mode();
    let writable = if metadata.uid() == user.uid() {
        mode & 0o200 != 0
    } else if metadata.gid() == user.gid() {
        mode & 0o020 != 0
    } else {
        mode & 0o002 != 0
    };
    if !writable {
        return Err(Error::message(format!(
            "sandbox home directory {} is not writable by run_as {user} (owner {}:{}, mode {:o}); \
             refusing to start a container that could not write its own HOME",
            path.display(),
            metadata.uid(),
            metadata.gid(),
            mode & 0o7777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_writable_by(path: &FsPath, _user: &SandboxUser) -> Result<()> {
    let metadata = std::fs::metadata(path)?;
    if metadata.permissions().readonly() {
        return Err(Error::message(format!(
            "sandbox home directory {} is read-only; refusing to start a container that could \
             not write its own HOME",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn ensure_sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
    id: &SandboxId,
    run_as: Option<&SandboxUser>,
) -> Result<Option<PathBuf>> {
    let Some(path) = sandbox_home_persistence_host_dir(config, cli, id, run_as) else {
        return Ok(None);
    };
    let guest_visible_path = guest_visible_sandbox_home_persistence_host_dir(config, id, run_as);

    // The guest-visible path is the bind source the container runtime is handed.
    // Creating it here is the whole point: a bind source first created by the
    // docker daemon lands root:root inside a data dir the gateway owns, which
    // is exactly how this deployment's sandbox tree became 0:0. A missing
    // directory is never "unavailable" - it is created.
    if let Some(ref guest_visible) = guest_visible_path {
        std::fs::create_dir_all(guest_visible)?;
        if let Some(user) = run_as {
            check_writable_by(guest_visible, user)?;
        }
    }

    if let Err(error) = std::fs::create_dir_all(&path) {
        if guest_visible_path.as_ref() == Some(&path) {
            return Err(error.into());
        }
        warn!(
            path = %path.display(),
            %error,
            "could not pre-create translated sandbox persistence path; runtime may create it"
        );
    }
    Ok(Some(path))
}
