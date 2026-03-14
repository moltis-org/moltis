//! Path resolution, mount detection, and home persistence directories.

use std::{
    collections::{HashMap, HashSet},
    path::{Path as FsPath, PathBuf},
    sync::{Mutex, OnceLock},
};

use tracing::{debug, warn};

use {
    super::{
        containers::{is_cli_available, is_docker_daemon_available, should_use_docker_backend},
        types::{HomePersistence, SandboxConfig, SandboxId, sanitize_path_component},
    },
    crate::error::Result,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerMount {
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
}

pub(crate) static HOST_DATA_DIR_CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
#[cfg(test)]
pub(crate) static TEST_CONTAINER_MOUNT_OVERRIDES: OnceLock<
    Mutex<HashMap<String, Vec<ContainerMount>>>,
> = OnceLock::new();

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

pub(crate) fn read_trimmed_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|contents| contents.trim().to_string())
        .filter(|contents| !contents.is_empty())
}

pub(crate) fn normalize_cgroup_container_ref(segment: &str) -> Option<String> {
    let mut value = segment.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(stripped) = value.strip_suffix(".scope") {
        value = stripped;
    }
    for prefix in ["docker-", "libpod-", "cri-containerd-"] {
        if let Some(stripped) = value.strip_prefix(prefix) {
            value = stripped;
            break;
        }
    }
    if value.len() < 12 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(value.to_string())
}

pub(crate) fn current_container_references() -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for candidate in [
        std::env::var("HOSTNAME").ok(),
        read_trimmed_file("/etc/hostname"),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(candidate.clone()) {
            refs.push(candidate);
        }
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/self/cgroup") {
        for candidate in cgroup
            .lines()
            .flat_map(|line| line.split(['/', ':']))
            .filter_map(normalize_cgroup_container_ref)
        {
            if seen.insert(candidate.clone()) {
                refs.push(candidate);
            }
        }
    }
    refs
}

pub(crate) fn parse_container_mounts_from_inspect(stdout: &str) -> Vec<ContainerMount> {
    let Ok(json): std::result::Result<serde_json::Value, _> = serde_json::from_str(stdout) else {
        return Vec::new();
    };
    let root = json
        .as_array()
        .and_then(|entries| entries.first())
        .unwrap_or(&json);
    root.get("Mounts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let source = entry.get("Source")?.as_str()?.trim();
            let destination = entry.get("Destination")?.as_str()?.trim();
            if source.is_empty() || destination.is_empty() {
                return None;
            }
            Some(ContainerMount {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
            })
        })
        .collect()
}

pub(crate) fn resolve_host_path_from_mounts(
    guest_path: &FsPath,
    mounts: &[ContainerMount],
) -> Option<PathBuf> {
    mounts
        .iter()
        .filter_map(|mount| {
            let relative = guest_path.strip_prefix(&mount.destination).ok()?;
            Some((
                mount.destination.components().count(),
                if relative.as_os_str().is_empty() {
                    mount.source.clone()
                } else {
                    mount.source.join(relative)
                },
            ))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, resolved)| resolved)
}

#[cfg(test)]
pub(crate) fn test_container_mount_override_key(cli: &str, reference: &str) -> String {
    format!("{cli}:{reference}")
}

pub(crate) fn inspect_current_container_mounts(cli: &str, reference: &str) -> Vec<ContainerMount> {
    #[cfg(test)]
    {
        let overrides = TEST_CONTAINER_MOUNT_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
        let guard = overrides.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(mounts) = guard.get(&test_container_mount_override_key(cli, reference)) {
            return mounts.clone();
        }
        Vec::new()
    }

    #[cfg(not(test))]
    {
        let output = match std::process::Command::new(cli)
            .args(["inspect", reference])
            .output()
        {
            Ok(output) if output.status.success() => output,
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!(
                    cli,
                    reference,
                    stderr = %stderr.trim(),
                    "container inspect failed while auto-detecting host data dir"
                );
                return Vec::new();
            },
            Err(error) => {
                debug!(
                    cli,
                    reference,
                    %error,
                    "could not inspect current container while auto-detecting host data dir"
                );
                return Vec::new();
            },
        };
        parse_container_mounts_from_inspect(&String::from_utf8_lossy(&output.stdout))
    }
}

pub(crate) fn detect_host_data_dir_with_references(
    cli: &str,
    guest_data_dir: &FsPath,
    references: &[String],
) -> Option<PathBuf> {
    references.iter().find_map(|reference| {
        let mounts = inspect_current_container_mounts(cli, reference);
        if mounts.is_empty() {
            return None;
        }
        let resolved = resolve_host_path_from_mounts(guest_data_dir, &mounts)?;
        debug!(
            cli,
            reference,
            guest_path = %guest_data_dir.display(),
            host_path = %resolved.display(),
            "auto-detected host data dir from current container mounts"
        );
        Some(resolved)
    })
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

    let detected =
        detect_host_data_dir_with_references(cli, guest_data_dir, &current_container_references());

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
        return configured;
    }
    if let Some(cli) = cli
        && let Some(detected) = detect_host_data_dir(cli, &guest_data_dir)
    {
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

pub(crate) fn sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
    id: &SandboxId,
) -> Option<PathBuf> {
    let base = sandbox_home_persistence_base_dir(config, cli);
    match config.home_persistence {
        HomePersistence::Off => None,
        HomePersistence::Shared => Some(resolve_shared_home_dir(config, cli)),
        HomePersistence::Session => {
            Some(base.join("session").join(sanitize_path_component(&id.key)))
        },
    }
}

pub(crate) fn guest_visible_sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    id: &SandboxId,
) -> Option<PathBuf> {
    let base = moltis_config::data_dir().join("sandbox").join("home");
    match config.home_persistence {
        HomePersistence::Off => None,
        HomePersistence::Shared => Some(
            config
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
                .unwrap_or_else(|| base.join("shared")),
        ),
        HomePersistence::Session => {
            Some(base.join("session").join(sanitize_path_component(&id.key)))
        },
    }
}

pub(crate) fn ensure_sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    cli: Option<&str>,
    id: &SandboxId,
) -> Result<Option<PathBuf>> {
    let Some(path) = sandbox_home_persistence_host_dir(config, cli, id) else {
        return Ok(None);
    };
    let guest_visible_path = guest_visible_sandbox_home_persistence_host_dir(config, id);
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
