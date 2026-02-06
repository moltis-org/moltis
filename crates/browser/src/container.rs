//! Container management for sandboxed browser instances.
//!
//! Manages browserless/chrome containers for isolated browser execution.

use std::process::Command;

use {
    anyhow::{Context, Result, bail},
    tracing::{debug, info, warn},
};

/// A running browser container instance.
pub struct BrowserContainer {
    /// Container ID.
    container_id: String,
    /// Host port mapped to the container's CDP port.
    host_port: u16,
    /// The image used.
    #[allow(dead_code)]
    image: String,
}

impl BrowserContainer {
    /// Start a new browserless container.
    ///
    /// Returns a container instance with the host port for CDP connections.
    pub fn start(image: &str, viewport_width: u32, viewport_height: u32) -> Result<Self> {
        // Find an available port
        let host_port = find_available_port()?;

        info!(image, host_port, "starting browser container");

        // Run the container
        // browserless/chrome exposes CDP on port 3000
        let output = Command::new("docker")
            .args([
                "run",
                "-d",   // Detached
                "--rm", // Auto-remove on stop
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
            bail!("failed to start browser container: {}", stderr.trim());
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if container_id.is_empty() {
            bail!("docker returned empty container ID");
        }

        debug!(container_id, host_port, "browser container started");

        // Wait for the container to be ready
        wait_for_ready(host_port)?;

        info!(container_id, host_port, "browser container ready");

        Ok(Self {
            container_id,
            host_port,
            image: image.to_string(),
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
        info!(container_id = %self.container_id, "stopping browser container");

        let result = Command::new("docker")
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
                    "failed to run docker stop"
                );
            },
        }
    }

    /// Get the container ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.container_id
    }
}

impl Drop for BrowserContainer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Find an available TCP port.
fn find_available_port() -> Result<u16> {
    // Bind to port 0 to get a random available port
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("failed to bind to ephemeral port")?;

    let port = listener
        .local_addr()
        .context("failed to get local address")?
        .port();

    // Drop listener to free the port
    drop(listener);

    Ok(port)
}

/// Wait for the container to be ready by attempting TCP connection.
fn wait_for_ready(port: u16) -> Result<()> {
    use std::{
        net::TcpStream,
        time::{Duration, Instant},
    };

    let addr = format!("127.0.0.1:{}", port);
    let timeout = Duration::from_secs(30);
    let start = Instant::now();

    debug!(addr, "waiting for browser container to be ready");

    loop {
        if start.elapsed() > timeout {
            bail!(
                "browser container failed to become ready within {}s",
                timeout.as_secs()
            );
        }

        // Try to connect via TCP
        match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500)) {
            Ok(_) => {
                debug!("browser container TCP connection succeeded");
                // Give it a moment more to fully initialize
                std::thread::sleep(Duration::from_millis(500));
                return Ok(());
            },
            Err(e) => {
                debug!(error = %e, "TCP connection failed, retrying");
            },
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Check if Docker is available.
#[must_use]
pub fn is_docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pull the browser container image if not present.
pub fn ensure_image(image: &str) -> Result<()> {
    // Check if image exists locally
    let output = Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to check for image")?;

    if output.success() {
        debug!(image, "browser container image already present");
        return Ok(());
    }

    info!(image, "pulling browser container image");

    let output = Command::new("docker")
        .args(["pull", image])
        .output()
        .context("failed to pull image")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to pull browser image: {}", stderr.trim());
    }

    info!(image, "browser container image pulled successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_available_port() {
        let port = find_available_port().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn test_is_docker_available() {
        // Just ensure it doesn't panic
        let _ = is_docker_available();
    }
}
