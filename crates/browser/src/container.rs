//! Container management for sandboxed browser instances.
//!
//! Supports Docker, Podman, and Apple Container backends, auto-detecting the
//! best available option (Apple Container on macOS → Podman → Docker).

use std::{fmt::Display, process::Command};

use {
    crate::error::Error,
    tracing::{debug, info, warn},
};

type Result<T> = std::result::Result<T, Error>;

trait ContextExt<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C;
}

impl<T, E> ContextExt<T> for std::result::Result<T, E>
where
    E: Display,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        let context = context.into();
        self.map_err(|source| Error::LaunchFailed(format!("{context}: {source}")))
    }

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C,
    {
        let context = f().into();
        self.map_err(|source| Error::LaunchFailed(format!("{context}: {source}")))
    }
}

impl<T> ContextExt<T> for Option<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.ok_or_else(|| Error::LaunchFailed(context.into()))
    }

    fn with_context<C, F>(self, f: F) -> Result<T>
    where
        C: Into<String>,
        F: FnOnce() -> C,
    {
        self.ok_or_else(|| Error::LaunchFailed(f().into()))
    }
}

fn browser_container_name_prefix(container_prefix: &str) -> String {
    format!("{container_prefix}-")
}

fn new_browser_container_name(container_prefix: &str) -> String {
    format!(
        "{}{}",
        browser_container_name_prefix(container_prefix),
        uuid::Uuid::new_v4().as_simple()
    )
}

/// Container backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerBackend {
    Docker,
    Podman,
    #[cfg(target_os = "macos")]
    AppleContainer,
}

impl ContainerBackend {
    /// Get the CLI command name for this backend.
    fn cli(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            #[cfg(target_os = "macos")]
            Self::AppleContainer => "container",
        }
    }

    /// Check if this backend is available.
    fn is_available(&self) -> bool {
        is_cli_available(self.cli())
    }
}

fn stop_container_by_id(backend: ContainerBackend, container_id: &str) {
    let cli = backend.cli();
    let result = Command::new(cli).args(["stop", container_id]).output();

    match result {
        Ok(output) if output.status.success() => {
            debug!(container_id, backend = cli, "browser container stopped");
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                container_id,
                backend = cli,
                error = %stderr.trim(),
                "failed to stop browser container"
            );
        },
        Err(e) => {
            warn!(
                container_id,
                backend = cli,
                error = %e,
                "failed to run {} stop",
                cli
            );
        },
    }

    // Apple Container requires explicit deletion after stop.
    #[cfg(target_os = "macos")]
    if backend == ContainerBackend::AppleContainer {
        match Command::new("container")
            .args(["rm", container_id])
            .output()
        {
            Ok(output) if output.status.success() => {
                debug!(container_id, "browser container removed");
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(
                    container_id,
                    error = %stderr.trim(),
                    "failed to remove browser container"
                );
            },
            Err(e) => {
                warn!(container_id, error = %e, "failed to run container rm");
            },
        }
    }
}

/// A running browser container instance.
pub struct BrowserContainer {
    /// Container ID or name.
    container_id: String,
    /// Host port mapped to the container's CDP port.
    host_port: u16,
    /// The image used.
    #[allow(dead_code)]
    image: String,
    /// The container backend being used.
    backend: ContainerBackend,
}

impl BrowserContainer {
    /// Start a new browser container using the auto-detected backend.
    ///
    /// Returns a container instance with the host port for CDP connections.
    /// When `profile_dir` is `Some`, the host directory is mounted into the
    /// container so that browser profile data persists across sessions.
    pub fn start(
        image: &str,
        container_prefix: &str,
        viewport_width: u32,
        viewport_height: u32,
        low_memory_threshold_mb: u64,
        profile_dir: Option<&std::path::Path>,
    ) -> Result<Self> {
        let backend = detect_backend()?;
        Self::start_with_backend(
            backend,
            image,
            container_prefix,
            viewport_width,
            viewport_height,
            low_memory_threshold_mb,
            profile_dir,
        )
    }

    /// Start a new browser container with a specific backend.
    pub fn start_with_backend(
        backend: ContainerBackend,
        image: &str,
        container_prefix: &str,
        viewport_width: u32,
        viewport_height: u32,
        low_memory_threshold_mb: u64,
        profile_dir: Option<&std::path::Path>,
    ) -> Result<Self> {
        use std::time::Instant;

        if !backend.is_available() {
            return Err(Error::LaunchFailed(format!(
                "{} is not available. Please install it to use sandboxed browser.",
                backend.cli()
            )));
        }

        // Find an available port
        let host_port = find_available_port()?;

        info!(
            image,
            host_port,
            backend = backend.cli(),
            "starting browser container"
        );

        let t0 = Instant::now();
        let container_id = match backend {
            ContainerBackend::Docker | ContainerBackend::Podman => start_oci_container(
                backend,
                image,
                container_prefix,
                host_port,
                viewport_width,
                viewport_height,
                low_memory_threshold_mb,
                profile_dir,
            )?,
            #[cfg(target_os = "macos")]
            ContainerBackend::AppleContainer => start_apple_container(
                image,
                container_prefix,
                host_port,
                viewport_width,
                viewport_height,
                low_memory_threshold_mb,
                profile_dir,
            )?,
        };

        info!(
            container_id,
            host_port,
            backend = backend.cli(),
            elapsed_ms = t0.elapsed().as_millis() as u64,
            "browser container process started, waiting for Chrome readiness"
        );

        // Wait for the container to be ready
        if let Err(error) = wait_for_ready(host_port) {
            warn!(
                container_id,
                host_port,
                backend = backend.cli(),
                error = %error,
                "browser container failed readiness check, cleaning up"
            );
            stop_container_by_id(backend, &container_id);
            return Err(error);
        }

        info!(
            container_id,
            host_port,
            backend = backend.cli(),
            total_startup_ms = t0.elapsed().as_millis() as u64,
            "browser container ready"
        );

        Ok(Self {
            container_id,
            host_port,
            image: image.to_string(),
            backend,
        })
    }

    /// Get the WebSocket URL for CDP connection.
    #[must_use]
    pub fn websocket_url(&self) -> String {
        // browserless/chrome provides a direct WebSocket endpoint
        format!("ws://127.0.0.1:{}", self.host_port)
    }

    /// Get the HTTP URL for health checks.
    #[must_use]
    pub fn http_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.host_port)
    }

    /// Stop and remove the container.
    pub fn stop(&self) {
        info!(
            container_id = %self.container_id,
            backend = self.backend.cli(),
            "stopping browser container"
        );
        stop_container_by_id(self.backend, &self.container_id);
    }

    /// Get the container ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.container_id
    }

    /// Get the backend being used.
    #[must_use]
    pub fn backend(&self) -> ContainerBackend {
        self.backend
    }
}

impl Drop for BrowserContainer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Path inside the container where the browser profile is mounted.
const CONTAINER_PROFILE_PATH: &str = "/data/browser-profile";

/// Build the `DEFAULT_LAUNCH_ARGS` env-var value for containerised Chrome.
///
/// Always includes `--window-size`; appends low-memory flags when the host
/// system RAM is below the given threshold. Adds `--user-data-dir` when a
/// container-side profile path is provided.
fn build_container_launch_args(
    viewport_width: u32,
    viewport_height: u32,
    low_memory_threshold_mb: u64,
    container_profile_dir: Option<&str>,
    backend: ContainerBackend,
) -> String {
    use crate::pool::low_memory_chrome_args;

    let mut args = vec![format!("--window-size={viewport_width},{viewport_height}")];

    if let Some(profile_dir) = container_profile_dir {
        args.push(format!("--user-data-dir={profile_dir}"));
    }

    // Apple Container VMs may not provide /dev/shm reliably; tell Chrome to
    // write shared-memory segments to /tmp instead.
    #[cfg(target_os = "macos")]
    if backend == ContainerBackend::AppleContainer {
        args.push("--disable-dev-shm-usage".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = backend;

    if low_memory_threshold_mb > 0 {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total_mb = sys.total_memory() / (1024 * 1024);
        for flag in low_memory_chrome_args(total_mb, low_memory_threshold_mb) {
            args.push((*flag).to_string());
        }
    }

    let joined = args
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("DEFAULT_LAUNCH_ARGS=[{joined}]")
}

/// Start a Docker container for the browser.
fn start_oci_container(
    backend: ContainerBackend,
    image: &str,
    container_prefix: &str,
    host_port: u16,
    viewport_width: u32,
    viewport_height: u32,
    low_memory_threshold_mb: u64,
    profile_dir: Option<&std::path::Path>,
) -> Result<String> {
    let cli = backend.cli();
    let container_name = new_browser_container_name(container_prefix);

    let container_profile_dir = profile_dir.map(|_| CONTAINER_PROFILE_PATH);
    let launch_args = build_container_launch_args(
        viewport_width,
        viewport_height,
        low_memory_threshold_mb,
        container_profile_dir,
        backend,
    );

    let mut run_args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--rm".to_string(),
        "--name".to_string(),
        container_name.clone(),
        "-p".to_string(),
        format!("{}:3000", host_port),
        "-e".to_string(),
        launch_args,
        "-e".to_string(),
        "MAX_CONCURRENT_SESSIONS=1".to_string(),
        "-e".to_string(),
        "PREBOOT_CHROME=true".to_string(),
        "--shm-size=2gb".to_string(),
    ];

    // Mount the profile directory if persistence is enabled
    if let Some(host_path) = profile_dir {
        run_args.push("-v".to_string());
        run_args.push(format!(
            "{}:{}:rw",
            host_path.display(),
            CONTAINER_PROFILE_PATH
        ));
    }

    run_args.push(image.to_string());

    let output = Command::new(cli)
        .args(&run_args)
        .output()
        .with_context(|| format!("failed to run {cli} command"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::LaunchFailed(format!(
            "failed to start {cli} container: {}",
            stderr.trim()
        )));
    }

    if container_name.is_empty() {
        return Err(Error::LaunchFailed(format!(
            "{cli} container name is empty"
        )));
    }

    Ok(container_name)
}

/// Start an Apple Container for the browser.
#[cfg(target_os = "macos")]
fn start_apple_container(
    image: &str,
    container_prefix: &str,
    host_port: u16,
    viewport_width: u32,
    viewport_height: u32,
    low_memory_threshold_mb: u64,
    profile_dir: Option<&std::path::Path>,
) -> Result<String> {
    let container_name = new_browser_container_name(container_prefix);

    let container_profile_dir = profile_dir.map(|_| CONTAINER_PROFILE_PATH);
    let launch_args = build_container_launch_args(
        viewport_width,
        viewport_height,
        low_memory_threshold_mb,
        container_profile_dir,
        ContainerBackend::AppleContainer,
    );

    let mut container_args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        container_name.clone(),
        "-p".to_string(),
        format!("{}:3000", host_port),
        "-e".to_string(),
        launch_args,
        "-e".to_string(),
        "MAX_CONCURRENT_SESSIONS=1".to_string(),
        "-e".to_string(),
        "PREBOOT_CHROME=true".to_string(),
        // Chrome requires shared memory for rendering; Docker uses --shm-size=2gb,
        // Apple Container doesn't support --shm-size so mount tmpfs at /dev/shm.
        "--tmpfs".to_string(),
        "/dev/shm".to_string(),
    ];

    // Mount the profile directory if persistence is enabled
    if let Some(host_path) = profile_dir {
        container_args.push("-v".to_string());
        container_args.push(format!(
            "{}:{}",
            host_path.display(),
            CONTAINER_PROFILE_PATH
        ));
    }

    container_args.push(image.to_string());

    let output = Command::new("container")
        .args(&container_args)
        .output()
        .context("failed to run container command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::LaunchFailed(format!(
            "failed to start apple container: {}",
            stderr.trim()
        )));
    }

    Ok(container_name)
}

/// Detect the best available container backend.
///
/// Prefers Apple Container on macOS when available and functional (VM-isolated),
/// then Podman (daemonless), then Docker.
pub fn detect_backend() -> Result<ContainerBackend> {
    #[cfg(target_os = "macos")]
    {
        if is_apple_container_functional() {
            info!("browser sandbox backend: apple-container (VM-isolated)");
            return Ok(ContainerBackend::AppleContainer);
        }
    }

    if ContainerBackend::Podman.is_available() {
        info!("browser sandbox backend: podman (daemonless)");
        return Ok(ContainerBackend::Podman);
    }

    if is_docker_available() {
        info!("browser sandbox backend: docker");
        return Ok(ContainerBackend::Docker);
    }

    Err(Error::LaunchFailed(
        "No container runtime available. Please install Docker or Podman to use sandboxed browser mode."
            .to_string(),
    ))
}

/// Check if Apple Container is actually functional (has required plugins).
#[cfg(target_os = "macos")]
fn is_apple_container_functional() -> bool {
    if !is_cli_available("container") {
        return false;
    }
    Command::new("container")
        .args(["image", "pull", "--help"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Check if a CLI tool is available.
fn is_cli_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Find an available TCP port.
fn find_available_port() -> Result<u16> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("failed to bind to ephemeral port")?;

    let port = listener
        .local_addr()
        .context("failed to get local address")?
        .port();

    drop(listener);
    Ok(port)
}

/// Wait for the container to be ready by probing the Chrome DevTools endpoint.
///
/// TCP connectivity alone isn't sufficient - Chrome inside the container may accept
/// connections before it's ready to handle WebSocket requests. We probe `/json/version`
/// which browserless exposes when Chrome is truly ready.
fn wait_for_ready(port: u16) -> Result<()> {
    use std::time::{Duration, Instant};

    let url = format!("http://127.0.0.1:{}/json/version", port);
    let timeout = Duration::from_secs(60);
    let start = Instant::now();
    let mut attempts: u32 = 0;

    info!(
        url,
        timeout_secs = 60,
        "waiting for browser container Chrome readiness"
    );

    loop {
        let elapsed = start.elapsed();
        if elapsed > timeout {
            warn!(
                attempts,
                elapsed_ms = elapsed.as_millis() as u64,
                "browser container failed to become ready within {}s",
                timeout.as_secs()
            );
            return Err(Error::LaunchFailed(format!(
                "browser container failed to become ready within {}s ({} probe attempts)",
                timeout.as_secs(),
                attempts
            )));
        }

        attempts += 1;

        // Try HTTP GET /json/version - this endpoint returns 200 when Chrome is ready
        match probe_http_endpoint(port) {
            Ok(true) => {
                info!(
                    attempts,
                    elapsed_ms = elapsed.as_millis() as u64,
                    "browser container Chrome endpoint is ready"
                );
                return Ok(());
            },
            Ok(false) => {
                // Log progress every 10 attempts (~5 seconds)
                if attempts.is_multiple_of(10) {
                    info!(
                        attempts,
                        elapsed_ms = elapsed.as_millis() as u64,
                        "Chrome endpoint not ready yet, still probing"
                    );
                } else {
                    debug!(attempts, "Chrome endpoint not ready yet, retrying");
                }
            },
            Err(e) => {
                // Log progress every 10 attempts (~5 seconds)
                if attempts.is_multiple_of(10) {
                    info!(
                        attempts,
                        elapsed_ms = elapsed.as_millis() as u64,
                        error = %e,
                        "probe failed, still retrying"
                    );
                } else {
                    debug!(attempts, error = %e, "probe failed, retrying");
                }
            },
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Probe the Chrome /json/version endpoint to check if it's ready.
fn probe_http_endpoint(port: u16) -> Result<bool> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpStream,
        time::Duration,
    };

    let addr = format!("127.0.0.1:{}", port);
    let socket_addr = addr
        .parse()
        .map_err(|e| Error::LaunchFailed(format!("invalid address {addr}: {e}")))?;
    let mut stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    // Send minimal HTTP request
    let request = "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes())?;

    // Read response status line
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;

    let ready = status_line.contains("200");
    debug!(
        port,
        status_line = status_line.trim(),
        ready,
        "probe response"
    );

    Ok(ready)
}

/// Check if Docker is available.
#[must_use]
pub fn is_docker_available() -> bool {
    is_cli_available("docker")
}

/// Check if Apple Container is available and functional.
#[cfg(target_os = "macos")]
#[must_use]
pub fn is_apple_container_available() -> bool {
    is_apple_container_functional()
}

/// Check if any container runtime is available and functional.
#[must_use]
pub fn is_container_available() -> bool {
    #[cfg(target_os = "macos")]
    if is_apple_container_available() {
        return true;
    }
    is_docker_available()
}

fn parse_docker_container_names(output: &[u8], container_prefix: &str) -> Vec<String> {
    let name_prefix = browser_container_name_prefix(container_prefix);
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(&name_prefix))
        .map(str::to_string)
        .collect()
}

#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct AppleContainerListEntry {
    configuration: AppleContainerConfig,
}

#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct AppleContainerConfig {
    id: String,
}

#[cfg(target_os = "macos")]
fn parse_apple_container_names(output: &[u8]) -> Result<Vec<String>> {
    let entries: Vec<AppleContainerListEntry> =
        serde_json::from_slice(output).context("failed to parse apple container list JSON")?;
    Ok(entries
        .into_iter()
        .map(|entry| entry.configuration.id)
        .collect())
}

#[cfg(target_os = "macos")]
fn parse_apple_container_names_for_prefix(
    output: &[u8],
    container_prefix: &str,
) -> Result<Vec<String>> {
    let name_prefix = browser_container_name_prefix(container_prefix);
    Ok(parse_apple_container_names(output)?
        .into_iter()
        .filter(|name| name.starts_with(&name_prefix))
        .collect())
}

fn cleanup_stale_docker_browser_containers(container_prefix: &str) -> Result<usize> {
    if !is_docker_available() {
        return Ok(0);
    }

    let output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .context("failed to list docker containers")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::LaunchFailed(format!(
            "docker ps failed while cleaning stale browser containers: {}",
            stderr.trim()
        )));
    }

    let names = parse_docker_container_names(&output.stdout, container_prefix);
    let mut removed = 0usize;
    for name in names {
        let rm = Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .with_context(|| format!("failed to remove stale docker browser container {name}"))?;
        if rm.status.success() {
            removed += 1;
        } else {
            let stderr = String::from_utf8_lossy(&rm.stderr);
            warn!(
                container_name = %name,
                error = %stderr.trim(),
                "failed to remove stale docker browser container"
            );
        }
    }

    Ok(removed)
}

#[cfg(target_os = "macos")]
fn cleanup_stale_apple_browser_containers(container_prefix: &str) -> Result<usize> {
    if !is_cli_available("container") {
        return Ok(0);
    }

    let output = Command::new("container")
        .args(["list", "--all", "--format", "json"])
        .output()
        .context("failed to list apple containers")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::LaunchFailed(format!(
            "container list failed while cleaning stale browser containers: {}",
            stderr.trim()
        )));
    }

    let names = parse_apple_container_names_for_prefix(&output.stdout, container_prefix)?;
    let mut removed = 0usize;
    for name in names {
        let rm = Command::new("container")
            .args(["delete", "--force", &name])
            .output()
            .with_context(|| format!("failed to remove stale apple browser container {name}"))?;
        if rm.status.success() {
            removed += 1;
        } else {
            let stderr = String::from_utf8_lossy(&rm.stderr);
            warn!(
                container_name = %name,
                error = %stderr.trim(),
                "failed to remove stale apple browser container"
            );
        }
    }

    Ok(removed)
}

#[cfg(target_os = "macos")]
fn cleanup_stale_apple_browser_containers_for_current_platform(
    container_prefix: &str,
) -> Result<usize> {
    cleanup_stale_apple_browser_containers(container_prefix)
}

#[cfg(not(target_os = "macos"))]
fn cleanup_stale_apple_browser_containers_for_current_platform(
    _container_prefix: &str,
) -> Result<usize> {
    Ok(0)
}

/// Remove stale browser containers left behind by previous runs.
///
/// Browser containers are named with an instance-specific prefix so startup can
/// clean up orphaned instances before creating new ones.
pub fn cleanup_stale_browser_containers(container_prefix: &str) -> Result<usize> {
    Ok(cleanup_stale_docker_browser_containers(container_prefix)?
        + cleanup_stale_apple_browser_containers_for_current_platform(container_prefix)?)
}

/// Pull the browser container image if not present.
/// Falls back to Docker if the primary backend fails.
pub fn ensure_image(image: &str) -> Result<()> {
    let backend = detect_backend()?;

    // Try primary backend first
    let result = ensure_image_with_backend(backend, image);

    // On macOS, if Apple Container fails, try Docker as fallback
    #[cfg(target_os = "macos")]
    if result.is_err() && backend == ContainerBackend::AppleContainer && is_docker_available() {
        if let Err(ref e) = result {
            warn!(
                error = %e,
                "Apple Container image pull failed, falling back to Docker"
            );
        }
        return ensure_image_with_backend(ContainerBackend::Docker, image);
    }

    result
}

/// Pull the browser container image using a specific backend.
pub fn ensure_image_with_backend(backend: ContainerBackend, image: &str) -> Result<()> {
    let cli = backend.cli();

    // Check if image exists locally
    let output = Command::new(cli)
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to check for image")?;

    if output.success() {
        info!(
            image,
            backend = cli,
            "browser container image already present"
        );
        return Ok(());
    }

    info!(image, backend = cli, "pulling browser container image");

    let output = match backend {
        ContainerBackend::Docker | ContainerBackend::Podman => {
            Command::new(cli).args(["pull", image]).output()
        },
        #[cfg(target_os = "macos")]
        ContainerBackend::AppleContainer => {
            Command::new(cli).args(["image", "pull", image]).output()
        },
    }
    .context("failed to pull image")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::LaunchFailed(format!(
            "failed to pull browser image: {}",
            stderr.trim()
        )));
    }

    info!(
        image,
        backend = cli,
        "browser container image pulled successfully"
    );
    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_available_port() {
        let port = find_available_port().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn test_new_browser_container_name_prefix() {
        let name = new_browser_container_name("moltis-test-browser");
        assert!(name.starts_with("moltis-test-browser-"));
    }

    #[test]
    fn test_parse_docker_container_names_filters_prefix() {
        let input = b"moltis-test-browser-abc\nother-container\nmoltis-test-browser-def\n";
        let parsed = parse_docker_container_names(input, "moltis-test-browser");
        assert_eq!(parsed, vec![
            "moltis-test-browser-abc".to_string(),
            "moltis-test-browser-def".to_string()
        ]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_parse_apple_container_names_filters_prefix() {
        let json = br#"[
          {"configuration":{"id":"moltis-test-browser-123"}},
          {"configuration":{"id":"not-browser"}},
          {"configuration":{"id":"moltis-test-browser-456"}}
        ]"#;
        let parsed = parse_apple_container_names_for_prefix(json, "moltis-test-browser").unwrap();
        assert_eq!(parsed, vec![
            "moltis-test-browser-123".to_string(),
            "moltis-test-browser-456".to_string()
        ]);
    }

    #[test]
    fn test_is_docker_available() {
        // Just ensure it doesn't panic
        let _ = is_docker_available();
    }

    #[test]
    fn test_is_container_available() {
        // Just ensure it doesn't panic
        let _ = is_container_available();
    }

    #[test]
    fn test_docker_backend_cli() {
        assert_eq!(ContainerBackend::Docker.cli(), "docker");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_apple_container_backend_cli() {
        assert_eq!(ContainerBackend::AppleContainer.cli(), "container");
    }

    #[test]
    fn test_detect_backend_returns_some() {
        // This test will pass if either Docker or Apple Container is available
        // If neither is available, it will error (which is expected)
        let result = detect_backend();
        if is_container_available() {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_build_container_launch_args_without_low_memory() {
        let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
        assert_eq!(args, r#"DEFAULT_LAUNCH_ARGS=["--window-size=1920,1080"]"#);
    }

    #[test]
    fn test_build_container_launch_args_with_profile_dir() {
        let args = build_container_launch_args(
            1920,
            1080,
            0,
            Some("/data/browser-profile"),
            ContainerBackend::Docker,
        );
        assert!(args.contains("--user-data-dir=/data/browser-profile"));
        assert!(args.contains("--window-size=1920,1080"));
    }

    #[test]
    fn test_build_container_launch_args_without_profile_dir() {
        let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
        assert!(!args.contains("--user-data-dir"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_build_container_launch_args_apple_container_has_disable_shm() {
        let args =
            build_container_launch_args(1920, 1080, 0, None, ContainerBackend::AppleContainer);
        assert!(args.contains("--disable-dev-shm-usage"));
        assert!(args.contains("--window-size=1920,1080"));
    }

    #[test]
    fn test_build_container_launch_args_docker_no_disable_shm() {
        let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
        assert!(!args.contains("--disable-dev-shm-usage"));
    }
}
