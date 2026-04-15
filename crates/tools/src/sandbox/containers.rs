//! Container image and container lifecycle management.

use std::collections::HashSet;

use {
    serde::Serialize,
    sha2::{Digest, Sha256},
    tracing::warn,
};

#[cfg(any(target_os = "macos", test))]
use super::types::APPLE_CONTAINER_SAFE_WORKDIR;
use {
    super::types::{
        GOGCLI_MODULE_PATH, GOGCLI_VERSION, SANDBOX_HOME_DIR, canonical_sandbox_packages,
    },
    crate::error::{Error, Result},
};

pub(crate) fn sandbox_image_dockerfile(base: &str, packages: &[String]) -> String {
    let pkg_list = canonical_sandbox_packages(packages).join(" ");
    format!(
        "FROM {base}\n\
RUN apt-get update -qq && apt-get install -y -qq {pkg_list} \
    && mkdir -p {SANDBOX_HOME_DIR}\n\
RUN if command -v corepack >/dev/null 2>&1; then corepack enable; fi\n\
RUN if command -v go >/dev/null 2>&1; then \
        GOBIN=/usr/local/bin go install {GOGCLI_MODULE_PATH}@{GOGCLI_VERSION} \
        && ln -sf /usr/local/bin/gog /usr/local/bin/gogcli; \
    fi\n\
RUN curl -fsSL https://mise.jdx.dev/install.sh | sh \
    && echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> /etc/profile.d/mise.sh\n\
ENV HOME={SANDBOX_HOME_DIR}\n\
ENV PATH={SANDBOX_HOME_DIR}/.local/bin:/root/.local/bin:$PATH\n\
WORKDIR {SANDBOX_HOME_DIR}\n"
    )
}

#[cfg(target_os = "macos")]
const APPLE_CONTAINER_FALLBACK_SLEEP_SECONDS: u64 = 2_147_483_647;

#[cfg(any(target_os = "macos", test))]
fn apple_container_wrap_shell_command(shell_command: String) -> String {
    format!("mkdir -p {SANDBOX_HOME_DIR} && {shell_command}")
}

#[cfg(target_os = "macos")]
pub(crate) fn apple_container_bootstrap_command() -> String {
    apple_container_wrap_shell_command(format!(
        "if command -v gnusleep >/dev/null 2>&1; then exec gnusleep infinity; else exec sleep {APPLE_CONTAINER_FALLBACK_SLEEP_SECONDS}; fi"
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn apple_container_run_args(
    name: &str,
    image: &str,
    tz: Option<&str>,
    home_volume: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        name.to_string(),
        "--workdir".to_string(),
        APPLE_CONTAINER_SAFE_WORKDIR.to_string(),
    ];

    if let Some(tz) = tz {
        args.extend(["-e".to_string(), format!("TZ={tz}")]);
    }
    if let Some(volume) = home_volume {
        args.extend(["--volume".to_string(), volume.to_string()]);
    }

    args.push(image.to_string());
    args.extend([
        "sh".to_string(),
        "-c".to_string(),
        apple_container_bootstrap_command(),
    ]);
    args
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn apple_container_exec_args(name: &str, shell_command: String) -> Vec<String> {
    vec![
        "exec".to_string(),
        "--workdir".to_string(),
        APPLE_CONTAINER_SAFE_WORKDIR.to_string(),
        name.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        apple_container_wrap_shell_command(shell_command),
    ]
}

pub(crate) fn container_exec_shell_args(
    cli: &str,
    container_name: &str,
    shell_command: String,
) -> Vec<String> {
    #[cfg(any(target_os = "macos", test))]
    if cli == "container" {
        return apple_container_exec_args(container_name, shell_command);
    }

    #[cfg(not(any(target_os = "macos", test)))]
    let _ = cli;

    vec![
        "exec".to_string(),
        container_name.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        shell_command,
    ]
}

/// Compute the content-hash tag for a pre-built sandbox image.
/// Pure function — independent of any specific container CLI.
pub fn sandbox_image_tag(repo: &str, base: &str, packages: &[String]) -> String {
    let dockerfile = sandbox_image_dockerfile(base, packages);
    let digest = Sha256::digest(dockerfile.as_bytes());
    format!("{repo}:{digest:x}")
}

pub(crate) fn is_sandbox_image_tag(tag: &str) -> bool {
    let Some((repo, _)) = tag.split_once(':') else {
        return false;
    };
    repo.ends_with("-sandbox")
}

/// Return the deterministic image tag for the current sandbox config when the
/// requested image points to a local pre-built sandbox repository.
///
/// This allows recover-on-demand behavior when users delete local pre-built
/// images from the UI while Moltis is still running.
pub(crate) fn rebuildable_sandbox_image_tag(
    requested_image: &str,
    image_repo: &str,
    base_image: &str,
    packages: &[String],
) -> Option<String> {
    if packages.is_empty() {
        return None;
    }
    if !requested_image.starts_with(&format!("{image_repo}:")) {
        return None;
    }
    Some(sandbox_image_tag(image_repo, base_image, packages))
}

/// OCI-compatible CLI binaries that use Docker-format commands
/// (`image ls`, `ps`, `system df`, etc.).
pub(crate) const OCI_COMPATIBLE_CLIS: &[&str] = &["docker", "podman"];

/// Check whether a container image exists locally.
/// `cli` is the container CLI binary (e.g. `"docker"`, `"podman"`, or `"container"`).
pub(crate) async fn sandbox_image_exists(cli: &str, tag: &str) -> bool {
    tokio::process::Command::new(cli)
        .args(["image", "inspect", tag])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Information about a locally cached sandbox image.
#[derive(Debug, Clone)]
pub struct SandboxImage {
    pub tag: String,
    pub size: String,
    pub created: String,
}

/// List all local `<instance>-sandbox:*` images across available container CLIs.
pub async fn list_sandbox_images() -> Result<Vec<SandboxImage>> {
    let mut images = Vec::new();
    let mut seen = HashSet::new();

    // Docker/Podman: both support --format with Go templates.
    for cli in OCI_COMPATIBLE_CLIS {
        if !is_cli_available(cli) {
            continue;
        }
        let output = tokio::process::Command::new(cli)
            .args([
                "image",
                "ls",
                "--format",
                "{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}",
            ])
            .output()
            .await?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() != 3 {
                    continue;
                }
                // Podman prefixes local repos with "localhost/"; normalize.
                let tag = parts[0].strip_prefix("localhost/").unwrap_or(parts[0]);
                if is_sandbox_image_tag(tag) && seen.insert(tag.to_string()) {
                    images.push(SandboxImage {
                        tag: tag.to_string(),
                        size: parts[1].to_string(),
                        created: parts[2].to_string(),
                    });
                }
            }
        }
    }

    // Apple Container: fixed table output (NAME  TAG  DIGEST), no --format.
    // Parse the table, then use `image inspect` JSON for metadata.
    if is_cli_available("container") {
        let output = tokio::process::Command::new("container")
            .args(["image", "ls"])
            .output()
            .await?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                // Columns are whitespace-separated: NAME TAG DIGEST
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 2 && cols[0].ends_with("-sandbox") {
                    let tag = format!("{}:{}", cols[0], cols[1]);
                    if !seen.insert(tag.clone()) {
                        continue;
                    }
                    // Fetch size and created from inspect JSON.
                    let (size, created) = inspect_apple_container_image(&tag).await;
                    images.push(SandboxImage { tag, size, created });
                }
            }
        }
    }

    Ok(images)
}

/// Extract size and created timestamp from Apple Container `image inspect` JSON.
async fn inspect_apple_container_image(tag: &str) -> (String, String) {
    let output = tokio::process::Command::new("container")
        .args(["image", "inspect", tag])
        .output()
        .await;
    let fallback = ("—".to_string(), "—".to_string());
    let Ok(output) = output else {
        return fallback;
    };
    if !output.status.success() {
        return fallback;
    }
    let Ok(json): std::result::Result<serde_json::Value, _> =
        serde_json::from_slice(&output.stdout)
    else {
        return fallback;
    };
    let entry = json.as_array().and_then(|a| a.first());
    let Some(entry) = entry else {
        return fallback;
    };
    let created = entry
        .pointer("/index/annotations/org.opencontainers.image.created")
        .and_then(|v| v.as_str())
        .unwrap_or("—")
        .to_string();
    let size = entry
        .pointer("/variants/0/size")
        .and_then(|v| v.as_u64())
        .map(format_bytes)
        .unwrap_or_else(|| "—".to_string());
    (size, created)
}

/// Format a byte count as a human-readable string (e.g. "361 MB").
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Remove a specific `<instance>-sandbox:*` image.
pub async fn remove_sandbox_image(tag: &str) -> Result<()> {
    if !is_sandbox_image_tag(tag) {
        return Err(Error::message(format!(
            "refusing to remove non-sandbox image: {tag}"
        )));
    }
    // OCI-compatible CLIs (docker, podman) + Apple Container.
    let all_clis: Vec<&str> = OCI_COMPATIBLE_CLIS
        .iter()
        .copied()
        .chain(std::iter::once("container"))
        .collect();
    for cli in &all_clis {
        if !is_cli_available(cli) {
            continue;
        }
        if sandbox_image_exists(cli, tag).await {
            // Apple Container uses `image delete`, Docker/Podman use `image rm`.
            let subcmd = if *cli == "container" {
                "delete"
            } else {
                "rm"
            };
            let output = tokio::process::Command::new(cli)
                .args(["image", subcmd, tag])
                .output()
                .await?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::message(format!(
                    "{cli} image {subcmd} failed for {tag}: {}",
                    stderr.trim()
                )));
            }
        }
    }
    Ok(())
}

/// Remove all local `<instance>-sandbox:*` images.
pub async fn clean_sandbox_images() -> Result<usize> {
    let images = list_sandbox_images().await?;
    let count = images.len();
    for img in &images {
        remove_sandbox_image(&img.tag).await?;
    }
    Ok(count)
}

// ── Running container management ─────────────────────────────────────────────

/// State of a running/stopped container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRunState {
    Running,
    Stopped,
    Exited,
    Unknown,
}

/// Which container backend manages this container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerBackend {
    AppleContainer,
    Docker,
    Podman,
}

/// A container managed by moltis (running, stopped, or exited).
#[derive(Debug, Clone, Serialize)]
pub struct RunningContainer {
    pub name: String,
    pub image: String,
    pub state: ContainerRunState,
    pub backend: ContainerBackend,
    pub cpus: Option<u32>,
    pub memory_mb: Option<u64>,
    pub started: Option<String>,
    pub addr: Option<String>,
}

/// Containers that failed removal but are no longer truly present in the
/// runtime. These ghosts appear in `container list` after a failed
/// `container rm -f` and cannot be deleted until the daemon restarts.
/// Filtering them out of list results gives the UI a consistent view.
static ZOMBIE_CONTAINERS: std::sync::LazyLock<std::sync::RwLock<HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(HashSet::new()));

pub(crate) fn mark_zombie(name: &str) {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.insert(name.to_string());
    }
}

pub(crate) fn unmark_zombie(name: &str) {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.remove(name);
    }
}

pub(crate) fn clear_zombies() {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.clear();
    }
}

pub(crate) fn is_zombie(name: &str) -> bool {
    ZOMBIE_CONTAINERS
        .read()
        .map(|set| set.contains(name))
        .unwrap_or(false)
}

/// List all containers whose name starts with `container_prefix`.
///
/// Queries both Apple Container and Docker backends when available,
/// merging results with the appropriate backend label.
pub async fn list_running_containers(container_prefix: &str) -> Result<Vec<RunningContainer>> {
    let mut containers = Vec::new();
    let mut seen = HashSet::new();

    // Apple Container: `container list --format json` outputs a JSON array.
    // Each element has nested fields: configuration.id, status,
    // configuration.image.reference, configuration.resources, networks[].
    if is_cli_available("container") {
        let output = tokio::process::Command::new("container")
            .args(["list", "--format", "json"])
            .output()
            .await;
        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let entries: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap_or_default();
            for entry in entries {
                let name = entry
                    .pointer("/configuration/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !name.starts_with(container_prefix) || !seen.insert(name.to_string()) {
                    continue;
                }
                let state_str = entry
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let state = match state_str {
                    "running" => ContainerRunState::Running,
                    "stopped" => ContainerRunState::Stopped,
                    "exited" => ContainerRunState::Exited,
                    _ => ContainerRunState::Unknown,
                };
                let image = entry
                    .pointer("/configuration/image/reference")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let cpus = entry
                    .pointer("/configuration/resources/cpus")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let memory_mb = entry
                    .pointer("/configuration/resources/memoryInBytes")
                    .and_then(|v| v.as_u64())
                    .map(|v| v / (1024 * 1024));
                // startedDate is a Core Foundation absolute time (seconds since 2001-01-01).
                // Store as unix timestamp string; the frontend formats for display.
                let started =
                    entry
                        .get("startedDate")
                        .and_then(|v| v.as_f64())
                        .map(|cf_timestamp| {
                            // CF absolute time epoch: 2001-01-01T00:00:00Z = 978307200 unix seconds.
                            let unix_secs = cf_timestamp as i64 + 978_307_200;
                            unix_secs.to_string()
                        });
                let addr = entry
                    .pointer("/networks/0/ipv4Address")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                containers.push(RunningContainer {
                    name: name.to_string(),
                    image,
                    state,
                    backend: ContainerBackend::AppleContainer,
                    cpus,
                    memory_mb,
                    started,
                    addr,
                });
            }
        }
    }

    // Docker/Podman: `<cli> ps -a --filter name=<prefix> --format json` outputs one JSON per line.
    for (cli, backend) in OCI_COMPATIBLE_CLIS
        .iter()
        .zip([ContainerBackend::Docker, ContainerBackend::Podman])
    {
        if !is_cli_available(cli) {
            continue;
        }
        let output = tokio::process::Command::new(cli)
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("name={container_prefix}"),
                "--format",
                "{{json .}}",
            ])
            .output()
            .await;
        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(json): std::result::Result<serde_json::Value, _> =
                    serde_json::from_str(line)
                else {
                    continue;
                };
                let name = json
                    .get("Names")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !name.starts_with(container_prefix) || !seen.insert(name.to_string()) {
                    continue;
                }
                let state_str = json
                    .get("State")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let state = match state_str {
                    "running" => ContainerRunState::Running,
                    "exited" => ContainerRunState::Exited,
                    _ => ContainerRunState::Stopped,
                };
                let image = json
                    .get("Image")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let started = json
                    .get("CreatedAt")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                containers.push(RunningContainer {
                    name: name.to_string(),
                    image,
                    state,
                    backend,
                    cpus: None,
                    memory_mb: None,
                    started,
                    addr: None,
                });
            }
        }
    }

    containers.retain(|c| !is_zombie(&c.name));
    Ok(containers)
}

/// Disk usage summary from the container runtime.
#[derive(Debug, Clone, Serialize)]
pub struct ContainerDiskUsage {
    pub containers_total: u64,
    pub containers_active: u64,
    pub containers_size_bytes: u64,
    pub containers_reclaimable_bytes: u64,
    pub images_total: u64,
    pub images_active: u64,
    pub images_size_bytes: u64,
}

/// Query container runtime disk usage.
///
/// Uses `container system df --format json` for Apple Container,
/// falls back to `docker system df --format json` for Docker.
pub async fn container_disk_usage() -> Result<ContainerDiskUsage> {
    // Try Apple Container first.
    if is_cli_available("container") {
        let output = tokio::process::Command::new("container")
            .args(["system", "df", "--format", "json"])
            .output()
            .await;
        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                let c = json.get("containers").cloned().unwrap_or_default();
                let i = json.get("images").cloned().unwrap_or_default();
                return Ok(ContainerDiskUsage {
                    containers_total: c.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
                    containers_active: c.get("active").and_then(|v| v.as_u64()).unwrap_or(0),
                    containers_size_bytes: c
                        .get("sizeInBytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    containers_reclaimable_bytes: c
                        .get("reclaimable")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    images_total: i.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
                    images_active: i.get("active").and_then(|v| v.as_u64()).unwrap_or(0),
                    images_size_bytes: i.get("sizeInBytes").and_then(|v| v.as_u64()).unwrap_or(0),
                });
            }
        }
    }

    // Fallback: Docker/Podman `<cli> system df --format json` (one JSON per line per type).
    for cli in OCI_COMPATIBLE_CLIS {
        if !is_cli_available(cli) {
            continue;
        }
        let output = tokio::process::Command::new(cli)
            .args(["system", "df", "--format", "{{json .}}"])
            .output()
            .await;
        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut usage = ContainerDiskUsage {
                containers_total: 0,
                containers_active: 0,
                containers_size_bytes: 0,
                containers_reclaimable_bytes: 0,
                images_total: 0,
                images_active: 0,
                images_size_bytes: 0,
            };
            for line in stdout.lines() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                    let rtype = json.get("Type").and_then(|v| v.as_str()).unwrap_or("");
                    let total = json.get("TotalCount").and_then(|v| v.as_u64()).unwrap_or(0);
                    let active = json.get("Active").and_then(|v| v.as_u64()).unwrap_or(0);
                    let size = json.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let reclaimable = json
                        .get("Reclaimable")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    match rtype {
                        "Containers" => {
                            usage.containers_total = total;
                            usage.containers_active = active;
                            usage.containers_size_bytes = size;
                            usage.containers_reclaimable_bytes = reclaimable;
                        },
                        "Images" => {
                            usage.images_total = total;
                            usage.images_active = active;
                            usage.images_size_bytes = size;
                        },
                        _ => {},
                    }
                }
            }
            return Ok(usage);
        }
    }

    Err(Error::message("no container CLI available for disk usage"))
}

/// Remove all containers whose name starts with `container_prefix`.
///
/// Returns the number of containers removed.
pub async fn clean_all_containers(container_prefix: &str) -> Result<usize> {
    let containers = list_running_containers(container_prefix).await?;
    let mut removed = 0;
    for c in &containers {
        // Stop running containers first.
        if c.state == ContainerRunState::Running {
            let _ = stop_container(&c.name).await;
        }
        match remove_container(&c.name).await {
            Ok(()) => removed += 1,
            Err(e) => {
                warn!(name = %c.name, %e, "failed to remove container during clean all");
            },
        }
    }
    Ok(removed)
}

/// Stop a container by name. Detects the backend from the available CLIs.
///
/// Safety: callers must validate that `name` starts with the expected prefix
/// to prevent stopping arbitrary containers.
pub async fn stop_container(name: &str) -> Result<()> {
    // Try Apple Container first, then Docker/Podman.
    if is_cli_available("container") {
        let output = tokio::process::Command::new("container")
            .args(["stop", name])
            .output()
            .await;
        if let Ok(ref o) = output
            && o.status.success()
        {
            return Ok(());
        }
    }
    for cli in OCI_COMPATIBLE_CLIS {
        if !is_cli_available(cli) {
            continue;
        }
        let output = tokio::process::Command::new(cli)
            .args(["stop", name])
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::message(format!(
                "{cli} stop failed for {name}: {}",
                stderr.trim()
            )));
        }
        return Ok(());
    }
    Err(Error::message(format!(
        "no container CLI available to stop {name}"
    )))
}

/// Remove a container by name (force). Detects the backend from available CLIs.
///
/// Safety: callers must validate that `name` starts with the expected prefix
/// to prevent removing arbitrary containers.
pub async fn remove_container(name: &str) -> Result<()> {
    // Try Apple Container first, then Docker.
    if is_cli_available("container") {
        let output = tokio::process::Command::new("container")
            .args(["rm", "-f", name])
            .output()
            .await;
        if let Ok(ref o) = output
            && o.status.success()
        {
            unmark_zombie(name);
            return Ok(());
        }

        // rm failed — inspect to classify the ghost container.
        let inspect = tokio::process::Command::new("container")
            .args(["inspect", name, "--format", "json"])
            .output()
            .await;
        match inspect {
            Ok(ref ins) if ins.status.success() => {
                let stdout = String::from_utf8_lossy(&ins.stdout);
                let status = apple_container_status_from_inspect(&stdout);
                if status == Some("running") {
                    // Container is genuinely running — return the rm error.
                    let stderr = output
                        .as_ref()
                        .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                        .unwrap_or_default();
                    return Err(Error::message(format!(
                        "container rm failed for running container {name}: {}",
                        stderr.trim()
                    )));
                }
                // Stopped/exited/unknown — ghost container, mark as zombie.
                tracing::warn!(
                    name,
                    ?status,
                    "container rm -f failed for stopped container, marking as zombie"
                );
                mark_zombie(name);
                return Ok(());
            },
            _ => {
                // Inspect failed (not found) — container is already gone,
                // mark as zombie so it's filtered from stale list results.
                mark_zombie(name);
                return Ok(());
            },
        }
    }
    for cli in OCI_COMPATIBLE_CLIS {
        if !is_cli_available(cli) {
            continue;
        }
        let output = tokio::process::Command::new(cli)
            .args(["rm", "-f", name])
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::message(format!(
                "{cli} rm failed for {name}: {}",
                stderr.trim()
            )));
        }
        unmark_zombie(name);
        return Ok(());
    }
    Err(Error::message(format!(
        "no container CLI available to remove {name}"
    )))
}

/// Restart the container daemon. For Apple Container this runs
/// `container system stop` followed by `container system start`.
/// For Docker it runs `docker restart` on the Docker daemon socket
/// (equivalent to `systemctl restart docker` but works cross-platform).
///
/// This clears ghost containers and corrupted daemon state.
pub async fn restart_container_daemon() -> Result<()> {
    if is_cli_available("container") {
        let stop = tokio::process::Command::new("container")
            .args(["system", "stop"])
            .output()
            .await?;
        if !stop.status.success() {
            let stderr = String::from_utf8_lossy(&stop.stderr);
            return Err(Error::message(format!(
                "container system stop failed: {}",
                stderr.trim()
            )));
        }
        let start = tokio::process::Command::new("container")
            .args(["system", "start"])
            .output()
            .await?;
        if !start.status.success() {
            let stderr = String::from_utf8_lossy(&start.stderr);
            return Err(Error::message(format!(
                "container system start failed: {}",
                stderr.trim()
            )));
        }
        clear_zombies();
        return Ok(());
    }
    // Docker/Podman: best-effort prune of stopped containers.
    for cli in OCI_COMPATIBLE_CLIS {
        if !is_cli_available(cli) {
            continue;
        }
        let _ = tokio::process::Command::new(cli)
            .args(["container", "prune", "-f"])
            .output()
            .await;
        return Ok(());
    }
    Err(Error::message(
        "no container CLI available to restart daemon",
    ))
}

pub(crate) fn apple_container_status_from_inspect(stdout: &str) -> Option<&'static str> {
    let inspect = stdout.trim();
    if inspect.is_empty() || inspect == "[]" {
        return None;
    }

    if inspect.contains(r#""status":"running""#) {
        return Some("running");
    }

    if inspect.contains(r#""status":"stopped""#) || inspect.contains(r#""status":"exited""#) {
        return Some("stopped");
    }

    None
}

pub(crate) fn is_apple_container_service_error(stderr: &str) -> bool {
    stderr.contains("XPC connection error") || stderr.contains("Connection invalid")
}

pub(crate) fn is_apple_container_exists_error(stderr: &str) -> bool {
    stderr.contains("already exists") || stderr.contains("exists: \"container with id")
}

pub(crate) fn is_apple_container_daemon_stale_error(text: &str) -> bool {
    // Both patterns are required — `NSPOSIXErrorDomain` alone can appear in
    // benign log-fetching errors (Code=2 "No such file or directory") when a
    // container vanishes. The stale-daemon signature is specifically EINVAL:
    // `NSPOSIXErrorDomain Code=22 "Invalid argument"`.
    text.contains("NSPOSIXErrorDomain") && text.contains("Invalid argument")
}

pub(crate) fn is_apple_container_corruption_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    is_apple_container_service_error(stderr)
        || is_apple_container_exists_error(stderr)
        || is_apple_container_daemon_stale_error(stderr)
        || lower.contains("failed to bootstrap container")
        || lower.contains("config.json")
        || lower.contains("vm never booted")
}

pub(crate) fn should_use_docker_backend(
    docker_cli_available: bool,
    docker_daemon_available: bool,
) -> bool {
    docker_cli_available && docker_daemon_available
}

pub(crate) fn is_docker_daemon_available() -> bool {
    std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Check whether a CLI tool is available on PATH.
pub(crate) fn is_cli_available(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Return the name of the container CLI to use for OCI image operations
/// (`docker run`, `docker build`, etc.).
///
/// Prefers `podman` (daemonless) when available, falls back to `docker`.
/// The result is cached after the first call.
pub fn container_cli() -> &'static str {
    static CLI: std::sync::OnceLock<&str> = std::sync::OnceLock::new();
    CLI.get_or_init(|| {
        if is_cli_available("podman") {
            "podman"
        } else {
            "docker"
        }
    })
}
