//! Apple Container sandbox backend (macOS 26+, Apple Silicon).

#[cfg(target_os = "macos")]
use std::collections::HashMap;

#[cfg(target_os = "macos")]
use async_trait::async_trait;
use tracing::{debug, info, warn};

#[cfg(target_os = "macos")]
use tokio::sync::RwLock;

#[cfg(target_os = "macos")]
use super::containers::{
    apple_container_exec_args, apple_container_run_args, apple_container_status_from_inspect,
    is_apple_container_daemon_stale_error, is_apple_container_exists_error,
    is_apple_container_service_error, rebuildable_sandbox_image_tag, sandbox_image_dockerfile,
    sandbox_image_exists, sandbox_image_tag, unmark_zombie,
};
#[cfg(target_os = "macos")]
use super::host::provision_packages;
#[cfg(target_os = "macos")]
use super::paths::ensure_sandbox_home_persistence_host_dir;
#[cfg(target_os = "macos")]
use super::types::{
    BuildImageResult, DEFAULT_SANDBOX_IMAGE, NetworkPolicy, SANDBOX_HOME_DIR, Sandbox,
    SandboxConfig, SandboxId, canonical_sandbox_packages, truncate_output_for_display,
};
#[cfg(target_os = "macos")]
use crate::error::{Error, Result};
#[cfg(target_os = "macos")]
use crate::exec::{ExecOpts, ExecResult};

/// Apple Container sandbox using the `container` CLI (macOS 26+, Apple Silicon).
#[cfg(target_os = "macos")]
pub struct AppleContainerSandbox {
    pub config: SandboxConfig,
    name_generations: RwLock<HashMap<String, u32>>,
    /// Cached host gateway IP for proxy routing in Trusted mode.
    host_gateway_cache: RwLock<Option<String>>,
}

#[cfg(target_os = "macos")]
impl AppleContainerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            name_generations: RwLock::new(HashMap::new()),
            host_gateway_cache: RwLock::new(None),
        }
    }

    /// Detect the host gateway IP reachable from inside the container VM.
    /// Caches the result after the first successful probe.
    /// Falls back to `192.168.64.1` (default macOS vmnet gateway).
    async fn detect_host_gateway(&self, container_name: &str) -> String {
        const FALLBACK_GATEWAY: &str = "192.168.64.1";

        // Return cached value if available.
        {
            let cache = self.host_gateway_cache.read().await;
            if let Some(ref gw) = *cache {
                return gw.clone();
            }
        }

        let probe_cmd = "ip route 2>/dev/null | grep default | head -1 | awk '{print $3}'";
        let args = apple_container_exec_args(container_name, probe_cmd.to_string());
        let gateway = match tokio::process::Command::new("container")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if stdout.is_empty() {
                    FALLBACK_GATEWAY.to_string()
                } else {
                    stdout
                }
            },
            _ => FALLBACK_GATEWAY.to_string(),
        };

        // Cache the result.
        {
            let mut cache = self.host_gateway_cache.write().await;
            *cache = Some(gateway.clone());
        }

        gateway
    }

    fn image(&self) -> &str {
        self.config
            .image
            .as_deref()
            .unwrap_or(DEFAULT_SANDBOX_IMAGE)
    }

    fn container_prefix(&self) -> &str {
        self.config
            .container_prefix
            .as_deref()
            .unwrap_or("moltis-sandbox")
    }

    fn base_container_name(&self, id: &SandboxId) -> String {
        format!("{}-{}", self.container_prefix(), id.key)
    }

    pub(crate) async fn container_name(&self, id: &SandboxId) -> String {
        let base = self.base_container_name(id);
        let generation = self
            .name_generations
            .read()
            .await
            .get(&id.key)
            .copied()
            .unwrap_or(0);
        if generation == 0 {
            base
        } else {
            format!("{base}-g{generation}")
        }
    }

    pub(crate) async fn bump_container_generation(&self, id: &SandboxId) -> String {
        let next_generation = {
            let mut generations = self.name_generations.write().await;
            let entry = generations.entry(id.key.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        let base = self.base_container_name(id);
        let next_name = format!("{base}-g{next_generation}");
        warn!(
            session_key = %id.key,
            generation = next_generation,
            name = %next_name,
            "rotating apple container name generation after stale container conflict"
        );
        next_name
    }

    fn image_repo(&self) -> &str {
        self.container_prefix()
    }

    fn home_persistence_volume(&self, id: &SandboxId) -> Result<Option<String>> {
        let Some(host_dir) =
            ensure_sandbox_home_persistence_host_dir(&self.config, Some("container"), id)?
        else {
            return Ok(None);
        };
        Ok(Some(format!("{}:{SANDBOX_HOME_DIR}", host_dir.display())))
    }

    async fn resolve_local_image(&self, requested_image: &str) -> Result<String> {
        if sandbox_image_exists("container", requested_image).await {
            return Ok(requested_image.to_string());
        }

        let base_image = self.image().to_string();
        let packages = self.config.packages.clone();
        let Some(rebuild_tag) = rebuildable_sandbox_image_tag(
            requested_image,
            self.image_repo(),
            &base_image,
            &packages,
        ) else {
            return Ok(requested_image.to_string());
        };

        if requested_image == rebuild_tag {
            info!(
                image = requested_image,
                "apple sandbox image missing locally, rebuilding on demand"
            );
        } else {
            warn!(
                requested = requested_image,
                rebuilt = %rebuild_tag,
                "requested apple sandbox image missing locally, using deterministic tag from current config"
            );
        }

        let Some(result) = self.build_image(&base_image, &packages).await? else {
            return Ok(requested_image.to_string());
        };
        Ok(result.tag)
    }

    /// Check whether the `container` CLI is available.
    pub async fn is_available() -> bool {
        tokio::process::Command::new("container")
            .arg("--version")
            .output()
            .await
            .is_ok_and(|o| o.status.success())
    }

    async fn container_exists(name: &str) -> Result<bool> {
        let output = tokio::process::Command::new("container")
            .args(["inspect", name])
            .output()
            .await?;
        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(!(stdout.trim().is_empty() || stdout.trim() == "[]"))
    }

    async fn remove_container_force(name: &str) {
        let remove = tokio::process::Command::new("container")
            .args(["rm", "-f", name])
            .output()
            .await;

        match remove {
            Ok(output) if output.status.success() => {
                info!(name, "removed stale apple container");
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!(name, %stderr, "failed to remove stale apple container");
            },
            Err(e) => {
                debug!(name, error = %e, "failed to run apple container remove command");
            },
        }
    }

    async fn wait_for_container_absent(name: &str) {
        const MAX_WAIT_ITERS: usize = 20;
        const WAIT_MS: u64 = 100;

        for _ in 0..MAX_WAIT_ITERS {
            match Self::container_exists(name).await {
                Ok(false) => return,
                Ok(true) => tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await,
                Err(e) => {
                    debug!(name, error = %e, "failed while waiting for container removal");
                    return;
                },
            }
        }
    }

    async fn wait_for_container_running(name: &str) -> Result<()> {
        const MAX_WAIT_ITERS: usize = 20;
        const WAIT_MS: u64 = 100;

        for attempt in 0..MAX_WAIT_ITERS {
            let output = tokio::process::Command::new("container")
                .args(["inspect", name])
                .output()
                .await;

            match output {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    match apple_container_status_from_inspect(&stdout) {
                        Some("running") => return Ok(()),
                        Some("stopped") => {
                            return Err(Error::message(format!(
                                "container {name} failed to stay running after startup"
                            )));
                        },
                        _ => {},
                    }

                    // `container run -d` can return before inspect status flips to
                    // "running". Keep polling briefly before we declare failure.
                    if attempt + 1 < MAX_WAIT_ITERS {
                        tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await;
                        continue;
                    }

                    return Err(Error::message(format!(
                        "container {name} did not report running state after startup"
                    )));
                },
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if attempt + 1 < MAX_WAIT_ITERS {
                        debug!(
                            name,
                            attempt,
                            %stderr,
                            "container inspect failed while waiting for running state, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await;
                        continue;
                    }
                    return Err(Error::message(format!(
                        "container inspect failed for {name} while waiting for running state: {}",
                        stderr.trim()
                    )));
                },
                Err(e) => {
                    if attempt + 1 < MAX_WAIT_ITERS {
                        debug!(
                            name,
                            attempt,
                            error = %e,
                            "container inspect command failed while waiting for running state, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await;
                        continue;
                    }
                    return Err(e.into());
                },
            }
        }

        Err(Error::message(format!(
            "container {name} did not become running after startup"
        )))
    }

    async fn probe_container_exec_ready(name: &str) -> Result<()> {
        let args = apple_container_exec_args(name, "true".to_string());
        let output = tokio::process::Command::new("container")
            .args(&args)
            .output()
            .await?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::message(format!(
            "container {name} failed exec readiness probe: {}",
            stderr.trim()
        )))
    }

    async fn wait_for_container_exec_ready(name: &str) -> Result<()> {
        const MAX_WAIT_ITERS: usize = 20;
        const WAIT_MS: u64 = 100;

        Self::wait_for_container_running(name).await?;

        for attempt in 0..MAX_WAIT_ITERS {
            match Self::probe_container_exec_ready(name).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let message = format!("{error:#}");
                    if attempt + 1 < MAX_WAIT_ITERS
                        && is_apple_container_unavailable_error(&message)
                    {
                        debug!(
                            name,
                            attempt,
                            %error,
                            "container exec readiness probe failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await;
                        continue;
                    }
                    return Err(error);
                },
            }
        }

        Err(Error::message(format!(
            "container {name} did not become exec-ready after startup"
        )))
    }

    async fn force_remove_and_wait(name: &str) {
        Self::remove_container_force(name).await;
        Self::wait_for_container_absent(name).await;
    }

    /// Inspect the container and return its current state.
    async fn inspect_container_state(name: &str) -> ContainerState {
        let output = match tokio::process::Command::new("container")
            .args(["inspect", name])
            .output()
            .await
        {
            Ok(o) => o,
            Err(_) => return ContainerState::Unknown,
        };

        if !output.status.success() {
            return ContainerState::NotFound;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() || trimmed == "[]" {
            return ContainerState::NotFound;
        }

        match apple_container_status_from_inspect(&stdout) {
            Some("running") => ContainerState::Running,
            Some("stopped") => ContainerState::Stopped,
            _ => ContainerState::Unknown,
        }
    }

    /// Try to create and start a container. Classifies errors into
    /// `CreateError` variants so the caller can decide the right recovery.
    async fn run_container(
        name: &str,
        image: &str,
        tz: Option<&str>,
        home_volume: Option<&str>,
    ) -> std::result::Result<(), CreateError> {
        let args = apple_container_run_args(name, image, tz, home_volume);

        let output = tokio::process::Command::new("container")
            .args(&args)
            .output()
            .await
            .map_err(|e| CreateError::Other(format!("failed to run container command: {e}")))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if is_apple_container_service_error(&stderr) {
            return Err(CreateError::ServiceDown);
        }
        if is_apple_container_exists_error(&stderr) {
            return Err(CreateError::AlreadyExists);
        }
        Err(CreateError::Other(stderr))
    }

    /// Try to restart a stopped container. Returns `true` if restart succeeded.
    async fn try_restart_container(name: &str) -> bool {
        let start = tokio::process::Command::new("container")
            .args(["start", name])
            .output()
            .await;
        matches!(start, Ok(output) if output.status.success())
    }

    /// Capture the last N lines of container logs (stdout + stderr).
    /// Returns `None` if logs cannot be retrieved.
    async fn capture_container_logs(name: &str, max_lines: usize) -> Option<String> {
        let output = tokio::process::Command::new("container")
            .args(["logs", name])
            .output()
            .await
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut combined = String::new();
        if !stdout.trim().is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.trim().is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }
        if combined.trim().is_empty() {
            return None;
        }

        // Keep only the last N lines.
        let lines: Vec<&str> = combined.lines().collect();
        let tail = if lines.len() > max_lines {
            &lines[lines.len() - max_lines..]
        } else {
            &lines
        };
        Some(tail.join("\n"))
    }

    /// Collect diagnostic information when all recovery attempts have failed.
    async fn diagnose_container_failure(name: &str) -> String {
        let mut diagnostics = Vec::new();

        // Check how many containers are currently running.
        let list_output = tokio::process::Command::new("container")
            .args(["list", "--format", "json"])
            .output()
            .await;
        match list_output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let count = stdout.lines().count();
                diagnostics.push(format!("running containers: {count}"));
            },
            _ => diagnostics.push("running containers: unknown (list failed)".to_string()),
        }

        // Check the state of the target container.
        let state = Self::inspect_container_state(name).await;
        diagnostics.push(format!("container '{name}' state: {state:?}"));

        // Capture container logs — this is the most useful piece: it shows
        // WHY the entrypoint exited (e.g. missing binary, image issues).
        match Self::capture_container_logs(name, 10).await {
            Some(logs) => diagnostics.push(format!("container logs: {logs}")),
            None => diagnostics.push("container logs: (empty or unavailable)".to_string()),
        }

        // Check the service health.
        let service_ok = tokio::process::Command::new("container")
            .args(["system", "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());
        diagnostics.push(format!(
            "container service: {}",
            if service_ok {
                "running"
            } else {
                "not running"
            }
        ));

        diagnostics.join("; ")
    }
}

/// State of an Apple Container as observed via `container inspect`.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerState {
    Running,
    Stopped,
    NotFound,
    Unknown,
}

/// Classification of container creation errors for recovery decisions.
#[cfg(target_os = "macos")]
#[derive(Debug)]
enum CreateError {
    /// The container name is already taken (stale metadata).
    AlreadyExists,
    /// The container service itself is down — retrying won't help.
    ServiceDown,
    /// Any other creation error.
    Other(String),
}

/// Check whether the Apple Container system service is running.
#[cfg(target_os = "macos")]
fn is_apple_container_service_running() -> bool {
    std::process::Command::new("container")
        .args(["system", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Try to start the Apple Container system service.
/// Returns `true` if the service was successfully started.
#[cfg(target_os = "macos")]
fn try_start_apple_container_service() -> bool {
    tracing::info!("apple container service is not running, starting it automatically");
    let result = std::process::Command::new("container")
        .args(["system", "start"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status();
    match result {
        Ok(status) if status.success() => {
            tracing::info!("apple container service started successfully");
            true
        },
        Ok(status) => {
            tracing::warn!(
                exit_code = status.code(),
                "failed to start apple container service; run `container system start` manually"
            );
            false
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to start apple container service; run `container system start` manually"
            );
            false
        },
    }
}

/// Ensure the Apple Container system service is running, starting it if needed.
/// Returns `true` if the service is running (either already or after starting).
#[cfg(target_os = "macos")]
pub fn ensure_apple_container_service() -> bool {
    if is_apple_container_service_running() {
        return true;
    }
    try_start_apple_container_service()
}

/// Restart the Apple Container daemon by stopping then starting it.
/// Used when the daemon is alive but its Virtualization.framework state is stale
/// (e.g. after an interrupted macOS restart/sleep). Returns `true` on success.
#[cfg(target_os = "macos")]
fn restart_apple_container_service() -> bool {
    tracing::warn!("apple container service unhealthy, restarting automatically");

    let stop = std::process::Command::new("container")
        .args(["system", "stop"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status();
    match stop {
        Ok(status) if status.success() => {
            tracing::info!("apple container service stopped");
        },
        Ok(status) => {
            tracing::warn!(
                exit_code = status.code(),
                "failed to stop apple container service"
            );
            // Continue to try start anyway — stop may fail if already stopped.
        },
        Err(e) => {
            tracing::warn!(error = %e, "failed to stop apple container service");
        },
    }

    try_start_apple_container_service()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_apple_container_unavailable_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("cannot exec: container is not running")
        || lower.contains("container is not running")
        || (lower.contains("container") && lower.contains("is not running"))
        || lower.contains("container is stopped")
        || lower.contains("no sandbox client exists")
        || lower.contains("notfound")
        || (lower.contains("not found") && lower.contains("container"))
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn should_restart_after_readiness_error(
    error_text: &str,
    state: ContainerState,
) -> bool {
    is_apple_container_unavailable_error(error_text) && state == ContainerState::Stopped
}

/// Returns `true` when a freshly created container stopped immediately and
/// produced no meaningful logs. This indicates the VM never fully booted —
/// a broader symptom than the specific daemon-stale EINVAL signature. It can
/// occur after macOS sleep/wake cycles, resource exhaustion, or
/// Virtualization.framework glitches. The appropriate recovery is a full
/// service restart, same as for daemon-stale errors.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_apple_container_boot_failure(logs: Option<&str>) -> bool {
    match logs {
        None => true,
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return true;
            }
            // Log-retrieval errors about a missing stdio.log mean
            // the VM never produced any output.
            trimmed.contains("stdio.log") && trimmed.contains("doesn't exist")
        },
    }
}

#[cfg(target_os = "macos")]
#[async_trait]
impl Sandbox for AppleContainerSandbox {
    fn backend_name(&self) -> &'static str {
        "apple-container"
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        let mut name = self.container_name(id).await;
        let requested_image = image_override.unwrap_or_else(|| self.image());
        let image = self.resolve_local_image(requested_image).await?;
        let tz = self.config.timezone.as_deref();
        let home_volume = self.home_persistence_volume(id)?;

        const MAX_ATTEMPTS: usize = 3;
        let mut daemon_restarted = false;

        for attempt in 0..MAX_ATTEMPTS {
            let is_last = attempt + 1 >= MAX_ATTEMPTS;

            // Phase 1: Check existing container and try to reuse it.
            match Self::inspect_container_state(&name).await {
                ContainerState::Running => {
                    info!(name, "apple container already running");
                    match Self::wait_for_container_exec_ready(&name).await {
                        Ok(()) => {
                            unmark_zombie(&name);
                            return Ok(());
                        },
                        Err(error) => {
                            warn!(
                                name,
                                %error,
                                attempt,
                                "apple container running but exec probe failed, removing"
                            );
                            Self::force_remove_and_wait(&name).await;
                        },
                    }
                },
                ContainerState::Stopped => {
                    info!(name, "apple container stopped, restarting");
                    if Self::try_restart_container(&name).await {
                        info!(name, "apple container restarted");
                        match Self::wait_for_container_exec_ready(&name).await {
                            Ok(()) => {
                                unmark_zombie(&name);
                                return Ok(());
                            },
                            Err(error) => {
                                warn!(
                                    name,
                                    %error,
                                    attempt,
                                    "restarted container failed exec probe, removing"
                                );
                                Self::force_remove_and_wait(&name).await;
                            },
                        }
                    } else {
                        warn!(name, attempt, "container restart failed, removing");
                        Self::force_remove_and_wait(&name).await;
                    }
                },
                ContainerState::NotFound => {
                    debug!(name, "apple container not found, will create");
                },
                ContainerState::Unknown => {
                    info!(name, attempt, "apple container in unknown state, removing");
                    Self::force_remove_and_wait(&name).await;
                },
            }

            // Phase 2: Create a new container.
            info!(name, image = %image, attempt, "creating apple container");
            match Self::run_container(&name, &image, tz, home_volume.as_deref()).await {
                Ok(()) => {},
                Err(CreateError::AlreadyExists) => {
                    warn!(
                        name,
                        attempt,
                        "container already exists during create, removing and rotating name"
                    );
                    Self::force_remove_and_wait(&name).await;
                    name = self.bump_container_generation(id).await;
                    continue;
                },
                Err(CreateError::ServiceDown) => {
                    return Err(Error::message(
                        "apple container service is not running. \
                         Start it with `container system start` and restart moltis",
                    ));
                },
                Err(CreateError::Other(stderr)) => {
                    // Daemon-stale errors mean the VM subsystem is broken.
                    // Restart the daemon once and retry; bail if already tried.
                    if is_apple_container_daemon_stale_error(&stderr) {
                        Self::force_remove_and_wait(&name).await;
                        if !daemon_restarted && restart_apple_container_service() {
                            daemon_restarted = true;
                            // Let the daemon fully initialize before retrying.
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            continue;
                        }
                        return Err(Error::message(format!(
                            "apple container daemon has stale Virtualization.framework state \
                             and automatic restart failed (create error: {stderr}). \
                             Restart manually with `container system stop && container system start`"
                        )));
                    }
                    if is_last {
                        let diag = Self::diagnose_container_failure(&name).await;
                        return Err(Error::message(format!(
                            "container run failed for {name} (image={image}): {stderr}; diagnostics: {diag}"
                        )));
                    }
                    warn!(
                        name,
                        %stderr,
                        attempt,
                        "container create failed, retrying"
                    );
                    Self::force_remove_and_wait(&name).await;
                    continue;
                },
            }

            // Phase 3: Wait for exec readiness (do NOT rotate name on failure).
            match Self::wait_for_container_exec_ready(&name).await {
                Ok(()) => {
                    info!(name, image = %image, "apple container created and running");
                    unmark_zombie(&name);

                    // Skip provisioning for pre-built sandbox images.
                    let is_prebuilt = image.starts_with(&format!("{}:", self.image_repo()));
                    if !is_prebuilt {
                        provision_packages("container", &name, &self.config.packages).await?;
                    }

                    return Ok(());
                },
                Err(error) => {
                    let error_message = format!("{error:#}");
                    let state = Self::inspect_container_state(&name).await;
                    if should_restart_after_readiness_error(&error_message, state) {
                        warn!(
                            name,
                            %error,
                            attempt,
                            "apple container stopped during readiness probe, restarting once"
                        );
                        if Self::try_restart_container(&name).await {
                            match Self::wait_for_container_exec_ready(&name).await {
                                Ok(()) => {
                                    info!(
                                        name,
                                        image = %image,
                                        "apple container recovered after readiness restart"
                                    );
                                    unmark_zombie(&name);
                                    let is_prebuilt =
                                        image.starts_with(&format!("{}:", self.image_repo()));
                                    if !is_prebuilt {
                                        provision_packages(
                                            "container",
                                            &name,
                                            &self.config.packages,
                                        )
                                        .await?;
                                    }
                                    return Ok(());
                                },
                                Err(restart_error) => {
                                    warn!(
                                        name,
                                        %restart_error,
                                        attempt,
                                        "apple container restart after readiness failure did not recover"
                                    );
                                },
                            }
                        } else {
                            warn!(
                                name,
                                attempt,
                                "apple container restart after readiness failure was unsuccessful"
                            );
                        }
                    }

                    // Capture logs before removing — this tells us WHY the
                    // entrypoint exited (missing binary, image issue, etc.).
                    let logs = Self::capture_container_logs(&name, 5).await;

                    // Daemon-stale errors (NSPOSIXErrorDomain EINVAL) mean
                    // the VM subsystem is broken. Restart the daemon once and
                    // retry; bail if we already restarted and it still fails.
                    if let Some(ref log_text) = logs
                        && is_apple_container_daemon_stale_error(log_text)
                    {
                        Self::force_remove_and_wait(&name).await;
                        if !daemon_restarted && restart_apple_container_service() {
                            daemon_restarted = true;
                            // Let the daemon fully initialize before retrying.
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            continue;
                        }
                        return Err(Error::message(format!(
                            "apple container daemon has stale Virtualization.framework state \
                             and automatic restart failed (container logs: {log_text}). \
                             Restart manually with `container system stop && container system start`"
                        )));
                    }

                    // Boot failure: container immediately stopped with no output.
                    // The VM likely never booted — try a full service restart
                    // (same recovery as daemon-stale, triggered by absence of
                    // logs rather than a specific error signature).
                    if state == ContainerState::Stopped
                        && !daemon_restarted
                        && is_apple_container_boot_failure(logs.as_deref())
                    {
                        warn!(
                            name,
                            attempt,
                            "apple container immediately stopped with no output, \
                             attempting service restart"
                        );
                        Self::force_remove_and_wait(&name).await;
                        if restart_apple_container_service() {
                            daemon_restarted = true;
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            continue;
                        }
                        warn!(name, "apple container service restart did not help");
                    }

                    if is_last {
                        // Include "VM never booted" when boot failure was
                        // detected so `is_apple_container_corruption_error`
                        // triggers failover to Docker.
                        let boot_note = if is_apple_container_boot_failure(logs.as_deref()) {
                            " (VM never booted)"
                        } else {
                            ""
                        };
                        let diag = Self::diagnose_container_failure(&name).await;
                        return Err(Error::message(format!(
                            "apple container {name} did not become exec-ready{boot_note}: \
                             {error:#}; diagnostics: {diag}"
                        )));
                    }
                    warn!(
                        name,
                        %error,
                        ?logs,
                        attempt,
                        "apple container not exec-ready after create, removing and retrying"
                    );
                    Self::force_remove_and_wait(&name).await;
                },
            }
        }

        // Unreachable: the loop either returns or bails on the last attempt.
        let diag = Self::diagnose_container_failure(&name).await;
        return Err(Error::message(format!(
            "apple container {name} failed after {MAX_ATTEMPTS} attempts; diagnostics: {diag}"
        )));
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let name = self.container_name(id).await;
        info!(name, command, "apple container exec");

        // Apple Container CLI doesn't support -e flags, so prepend export
        // statements to inject env vars into the shell.
        let mut prefix = String::new();

        // Inject proxy env vars so traffic routes through the trusted-network
        // proxy running on the host.
        if self.config.network == NetworkPolicy::Trusted {
            let gateway = self.detect_host_gateway(&name).await;
            let proxy_url = format!(
                "http://{}:{}",
                gateway,
                moltis_network_filter::DEFAULT_PROXY_PORT
            );
            let escaped_proxy = proxy_url.replace('\'', "'\\''");
            for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
                prefix.push_str(&format!("export {key}='{escaped_proxy}'; "));
            }
            let no_proxy = "localhost,127.0.0.1,::1";
            for key in ["NO_PROXY", "no_proxy"] {
                prefix.push_str(&format!("export {key}='{no_proxy}'; "));
            }
        }

        for (k, v) in &opts.env {
            // Shell-escape the value with single quotes.
            let escaped = v.replace('\'', "'\\''");
            prefix.push_str(&format!("export {k}='{escaped}'; "));
        }

        let full_command = if let Some(ref dir) = opts.working_dir {
            format!("{prefix}cd {} && {command}", dir.display())
        } else {
            format!("{prefix}{command}")
        };

        let args = apple_container_exec_args(&name, full_command);

        let child = tokio::process::Command::new("container")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

                debug!(
                    name,
                    exit_code,
                    stdout_len = stdout.len(),
                    stderr_len = stderr.len(),
                    "apple container exec complete"
                );
                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code,
                })
            },
            Ok(Err(e)) => {
                warn!(name, %e, "apple container exec spawn failed");
                return Err(Error::message(format!(
                    "container exec failed for {name}: {e}"
                )));
            },
            Err(_) => {
                warn!(
                    name,
                    timeout_secs = opts.timeout.as_secs(),
                    "apple container exec timed out"
                );
                return Err(Error::message(format!(
                    "container exec timed out for {name} after {}s",
                    opts.timeout.as_secs()
                )));
            },
        }
    }

    async fn build_image(
        &self,
        base: &str,
        packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        if packages.is_empty() {
            return Ok(None);
        }

        let tag = sandbox_image_tag(self.image_repo(), base, packages);

        if sandbox_image_exists("container", &tag).await {
            info!(
                tag,
                "pre-built sandbox image already exists, skipping build"
            );
            return Ok(Some(BuildImageResult { tag, built: false }));
        }

        let tmp_dir =
            std::env::temp_dir().join(format!("moltis-sandbox-build-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir)?;

        let pkg_list = canonical_sandbox_packages(packages).join(" ");
        let dockerfile = sandbox_image_dockerfile(base, packages);
        let dockerfile_path = tmp_dir.join("Dockerfile");
        std::fs::write(&dockerfile_path, &dockerfile)?;

        info!(tag, packages = %pkg_list, "building pre-built sandbox image (apple container)");

        let output = tokio::process::Command::new("container")
            .args(["build", "-t", &tag, "-f"])
            .arg(&dockerfile_path)
            .arg(&tmp_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        let _ = std::fs::remove_dir_all(&tmp_dir);

        let output = output?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("XPC connection error") || stderr.contains("Connection invalid") {
                return Err(Error::message(
                    "apple container service is not running. \
                     Start it with `container system start` and restart moltis",
                ));
            }
            return Err(Error::message(format!(
                "container build failed for {tag}: {}",
                stderr.trim()
            )));
        }

        info!(tag, "pre-built sandbox image ready (apple container)");
        Ok(Some(BuildImageResult { tag, built: true }))
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let base = self.base_container_name(id);
        let max_generation = self
            .name_generations
            .read()
            .await
            .get(&id.key)
            .copied()
            .unwrap_or(0);

        for generation in 0..=max_generation {
            let name = if generation == 0 {
                base.clone()
            } else {
                format!("{base}-g{generation}")
            };
            info!(name, "cleaning up apple container");
            let _ = tokio::process::Command::new("container")
                .args(["stop", &name])
                .output()
                .await;
            let _ = tokio::process::Command::new("container")
                .args(["rm", &name])
                .output()
                .await;
        }
        self.name_generations.write().await.remove(&id.key);
        Ok(())
    }
}
