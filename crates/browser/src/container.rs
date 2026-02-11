//! Container management for sandboxed browser instances.
//!
//! Supports both Docker and Apple Container backends, auto-detecting the best
//! available option (prefers Apple Container on macOS when available).

use std::process::Command;

use {
    anyhow::{Context, Result, bail},
    tracing::{debug, info, warn},
};

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
    #[cfg(target_os = "macos")]
    AppleContainer,
}

impl ContainerBackend {
    /// Get the CLI command name for this backend.
    fn cli(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            #[cfg(target_os = "macos")]
            Self::AppleContainer => "container",
        }
    }

    /// Check if this backend is available.
    fn is_available(&self) -> bool {
        is_cli_available(self.cli())
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
    pub fn start(
        image: &str,
        container_prefix: &str,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        let backend = detect_backend()?;
        Self::start_with_backend(
            backend,
            image,
            container_prefix,
            viewport_width,
            viewport_height,
        )
    }

    /// Start a new browser container with a specific backend.
    pub fn start_with_backend(
        backend: ContainerBackend,
        image: &str,
        container_prefix: &str,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self> {
        if !backend.is_available() {
            bail!(
                "{} is not available. Please install it to use sandboxed browser.",
                backend.cli()
            );
        }

        // Find an available port
        let host_port = find_available_port()?;

        info!(
            image,
            host_port,
            backend = backend.cli(),
            "starting browser container"
        );

        let container_id = match backend {
            ContainerBackend::Docker => start_docker_container(
                image,
                container_prefix,
                host_port,
                viewport_width,
                viewport_height,
            )?,
            #[cfg(target_os = "macos")]
            ContainerBackend::AppleContainer => start_apple_container(
                image,
                container_prefix,
                host_port,
                viewport_width,
                viewport_height,
            )?,
        };

        debug!(
            container_id,
            host_port,
            backend = backend.cli(),
            "browser container started"
        );

        // Wait for the container to be ready
        wait_for_ready(host_port)?;

        info!(
            container_id,
            host_port,
            backend = backend.cli(),
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

        let cli = self.backend.cli();
        let result = Command::new(cli)
            .args(["stop", &self.container_id])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                debug!(container_id = %self.container_id, "browser container stopped");
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(
                    container_id = %self.container_id,
                    error = %stderr.trim(),
                    "failed to stop browser container"
                );
            },
            Err(e) => {
                warn!(
                    container_id = %self.container_id,
                    error = %e,
                    "failed to run {} stop",
                    cli
                );
            },
        }

        // For Apple Container, we also need to remove the container
        #[cfg(target_os = "macos")]
        if self.backend == ContainerBackend::AppleContainer {
            let _ = Command::new("container")
                .args(["rm", &self.container_id])
                .output();
        }
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

/// Start a Docker container for the browser.
fn start_docker_container(
    image: &str,
    container_prefix: &str,
    host_port: u16,
    viewport_width: u32,
    viewport_height: u32,
) -> Result<String> {
    let container_name = new_browser_container_name(container_prefix);

    let output = Command::new("docker")
        .args([
            "run",
            "-d",   // Detached
            "--rm", // Auto-remove on stop
            "--name",
            &container_name,
            "-p",
            &format!("{}:3000", host_port), // Map CDP port
            "-e",
            &format!(
                "DEFAULT_LAUNCH_ARGS=[\"--window-size={},{}\"]",
                viewport_width, viewport_height
            ),
            "-e",
            "MAX_CONCURRENT_SESSIONS=1", // One session per container
            "-e",
            "PREBOOT_CHROME=true", // Pre-launch Chrome for faster first connection
            "--shm-size=2gb",      // Chrome needs shared memory
            image,
        ])
        .output()
        .context("failed to run docker command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to start docker container: {}", stderr.trim());
    }

    if container_name.is_empty() {
        bail!("docker container name is empty");
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
) -> Result<String> {
    let container_name = new_browser_container_name(container_prefix);

    // Apple Container uses different syntax for port mapping and env vars
    let output = Command::new("container")
        .args([
            "run",
            "-d",
            "--name",
            &container_name,
            "-p",
            &format!("{}:3000", host_port),
            "-e",
            &format!(
                "DEFAULT_LAUNCH_ARGS=[\"--window-size={},{}\"]",
                viewport_width, viewport_height
            ),
            "-e",
            "MAX_CONCURRENT_SESSIONS=1",
            "-e",
            "PREBOOT_CHROME=true",
            image,
        ])
        .output()
        .context("failed to run container command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to start apple container: {}", stderr.trim());
    }

    Ok(container_name)
}

/// Detect the best available container backend.
///
/// Prefers Apple Container on macOS when available and functional (VM-isolated),
/// falls back to Docker otherwise.
pub fn detect_backend() -> Result<ContainerBackend> {
    #[cfg(target_os = "macos")]
    {
        if is_apple_container_functional() {
            info!("browser sandbox backend: apple-container (VM-isolated)");
            return Ok(ContainerBackend::AppleContainer);
        }
    }

    if is_docker_available() {
        info!("browser sandbox backend: docker");
        return Ok(ContainerBackend::Docker);
    }

    bail!(
        "No container runtime available. Please install Docker \
         to use sandboxed browser mode."
    )
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

    debug!(url, "waiting for browser container to be ready");

    loop {
        if start.elapsed() > timeout {
            bail!(
                "browser container failed to become ready within {}s",
                timeout.as_secs()
            );
        }

        // Try HTTP GET /json/version - this endpoint returns 200 when Chrome is ready
        match probe_http_endpoint(port) {
            Ok(true) => {
                debug!("browser container Chrome endpoint is ready");
                return Ok(());
            },
            Ok(false) => {
                debug!("Chrome endpoint not ready yet, retrying");
            },
            Err(e) => {
                debug!(error = %e, "probe failed, retrying");
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
        .map_err(|e| anyhow::anyhow!("invalid address {addr}: {e}"))?;
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

    // Check for HTTP 200 response
    Ok(status_line.contains("200"))
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
        bail!(
            "docker ps failed while cleaning stale browser containers: {}",
            stderr.trim()
        );
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
        bail!(
            "container list failed while cleaning stale browser containers: {}",
            stderr.trim()
        );
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
        debug!(
            image,
            backend = cli,
            "browser container image already present"
        );
        return Ok(());
    }

    info!(image, backend = cli, "pulling browser container image");

    let output = match backend {
        ContainerBackend::Docker => Command::new(cli).args(["pull", image]).output(),
        #[cfg(target_os = "macos")]
        ContainerBackend::AppleContainer => {
            Command::new(cli).args(["image", "pull", image]).output()
        },
    }
    .context("failed to pull image")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to pull browser image: {}", stderr.trim());
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
}
