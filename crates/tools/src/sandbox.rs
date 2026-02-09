use std::{collections::HashMap, sync::Arc};

use {
    anyhow::Result,
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

use crate::exec::{ExecOpts, ExecResult};

/// Install configured packages inside a container via `apt-get`.
///
/// `cli` is the container CLI binary name (e.g. `"docker"` or `"container"`).
async fn provision_packages(cli: &str, container_name: &str, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let pkg_list = packages.join(" ");
    info!(container = container_name, packages = %pkg_list, "provisioning sandbox packages");
    let output = tokio::process::Command::new(cli)
        .args([
            "exec",
            container_name,
            "sh",
            "-c",
            &format!("apt-get update -qq && apt-get install -y -qq {pkg_list} 2>&1 | tail -5"),
        ])
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

/// Check whether the current host is Debian/Ubuntu (has `/etc/debian_version`
/// and `apt-get` on PATH).
pub fn is_debian_host() -> bool {
    std::path::Path::new("/etc/debian_version").exists() && is_cli_available("apt-get")
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
pub async fn provision_host_packages(packages: &[String]) -> Result<Option<HostProvisionResult>> {
    if packages.is_empty() || !is_debian_host() {
        return Ok(None);
    }

    // Determine which packages are already installed via dpkg-query.
    let mut missing = Vec::new();
    let mut skipped = Vec::new();

    for pkg in packages {
        let output = tokio::process::Command::new("dpkg-query")
            .args(["-W", "-f=${Status}", pkg])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await;
        let installed = output.as_ref().is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).contains("install ok installed")
        });
        if installed {
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

    let pkg_list = missing.join(" ");
    let apt_update = if has_sudo {
        "sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq".to_string()
    } else {
        "DEBIAN_FRONTEND=noninteractive apt-get update -qq".to_string()
    };
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
                installed = missing.len(),
                skipped = skipped.len(),
                "host packages provisioned"
            );
            Ok(Some(HostProvisionResult {
                installed: missing,
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

/// Default container image used when none is configured.
pub const DEFAULT_SANDBOX_IMAGE: &str = "ubuntu:25.10";

/// Sandbox mode controlling when sandboxing is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SandboxMode {
    Off,
    NonMain,
    #[default]
    All,
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::NonMain => f.write_str("non-main"),
            Self::All => f.write_str("all"),
        }
    }
}

/// Scope determines container lifecycle boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SandboxScope {
    #[default]
    Session,
    Agent,
    Shared,
}

impl std::fmt::Display for SandboxScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session => f.write_str("session"),
            Self::Agent => f.write_str("agent"),
            Self::Shared => f.write_str("shared"),
        }
    }
}

/// Workspace mount mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum WorkspaceMount {
    None,
    #[default]
    Ro,
    Rw,
}

impl std::fmt::Display for WorkspaceMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Ro => f.write_str("ro"),
            Self::Rw => f.write_str("rw"),
        }
    }
}

/// Resource limits for sandboxed execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimits {
    /// Memory limit (e.g. "512M", "1G").
    pub memory_limit: Option<String>,
    /// CPU quota as a fraction (e.g. 0.5 = half a core, 2.0 = two cores).
    pub cpu_quota: Option<f64>,
    /// Maximum number of PIDs.
    pub pids_max: Option<u32>,
}

/// Configuration for sandbox behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: SandboxMode,
    pub scope: SandboxScope,
    pub workspace_mount: WorkspaceMount,
    pub image: Option<String>,
    pub container_prefix: Option<String>,
    pub no_network: bool,
    /// Backend: `"auto"` (default), `"docker"`, or `"apple-container"`.
    /// `"auto"` prefers Apple Container on macOS when available.
    pub backend: String,
    pub resource_limits: ResourceLimits,
    /// Packages to install via `apt-get` after container creation.
    /// Set to an empty list to skip provisioning.
    pub packages: Vec<String>,
    /// IANA timezone (e.g. "Europe/Paris") injected as `TZ` env var into containers.
    pub timezone: Option<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::default(),
            scope: SandboxScope::default(),
            workspace_mount: WorkspaceMount::default(),
            image: None,
            container_prefix: None,
            no_network: false,
            backend: "auto".into(),
            resource_limits: ResourceLimits::default(),
            packages: Vec::new(),
            timezone: None,
        }
    }
}

impl From<&moltis_config::schema::SandboxConfig> for SandboxConfig {
    fn from(cfg: &moltis_config::schema::SandboxConfig) -> Self {
        Self {
            mode: match cfg.mode.as_str() {
                "all" => SandboxMode::All,
                "non-main" | "nonmain" => SandboxMode::NonMain,
                _ => SandboxMode::Off,
            },
            scope: match cfg.scope.as_str() {
                "agent" => SandboxScope::Agent,
                "shared" => SandboxScope::Shared,
                _ => SandboxScope::Session,
            },
            workspace_mount: match cfg.workspace_mount.as_str() {
                "rw" => WorkspaceMount::Rw,
                "none" => WorkspaceMount::None,
                _ => WorkspaceMount::Ro,
            },
            image: cfg.image.clone(),
            container_prefix: cfg.container_prefix.clone(),
            no_network: cfg.no_network,
            backend: cfg.backend.clone(),
            resource_limits: ResourceLimits {
                memory_limit: cfg.resource_limits.memory_limit.clone(),
                cpu_quota: cfg.resource_limits.cpu_quota,
                pids_max: cfg.resource_limits.pids_max,
            },
            packages: cfg.packages.clone(),
            timezone: None, // Set by gateway from user profile
        }
    }
}

/// Sandbox identifier — session or agent scoped.
#[derive(Debug, Clone)]
pub struct SandboxId {
    pub scope: SandboxScope,
    pub key: String,
}

impl std::fmt::Display for SandboxId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}/{}", self.scope, self.key)
    }
}

/// Result of a `build_image` call.
#[derive(Debug, Clone)]
pub struct BuildImageResult {
    /// The full image tag (e.g. `moltis-sandbox:abc123`).
    pub tag: String,
    /// Whether the build was actually performed (false = image already existed).
    pub built: bool,
}

/// Trait for sandbox implementations (Docker, cgroups, Apple Container, etc.).
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Human-readable backend name (e.g. "docker", "apple-container", "cgroup", "none").
    fn backend_name(&self) -> &'static str;

    /// Ensure the sandbox environment is ready (e.g., container started).
    /// If `image_override` is provided, use that image instead of the configured default.
    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()>;

    /// Execute a command inside the sandbox.
    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult>;

    /// Clean up sandbox resources.
    async fn cleanup(&self, id: &SandboxId) -> Result<()>;

    /// Pre-build a container image with packages baked in.
    /// Returns `None` for backends that don't support image building.
    async fn build_image(
        &self,
        _base: &str,
        _packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        Ok(None)
    }
}

/// Compute the content-hash tag for a pre-built sandbox image.
/// Pure function — independent of any specific container CLI.
pub fn sandbox_image_tag(base: &str, packages: &[String]) -> String {
    use std::hash::Hasher;
    let mut h = std::hash::DefaultHasher::new();
    // Bump this when the Dockerfile template changes to force a rebuild.
    h.write(b"v4");
    h.write(base.as_bytes());
    let mut sorted: Vec<&String> = packages.iter().collect();
    sorted.sort();
    for p in &sorted {
        h.write(p.as_bytes());
    }
    format!("moltis-sandbox:{:016x}", h.finish())
}

/// Check whether a container image exists locally.
/// `cli` is the container CLI binary (e.g. `"docker"` or `"container"`).
async fn sandbox_image_exists(cli: &str, tag: &str) -> bool {
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

/// List all local `moltis-sandbox:*` images across available container CLIs.
pub async fn list_sandbox_images() -> Result<Vec<SandboxImage>> {
    let mut images = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Docker: supports --format with Go templates.
    if is_cli_available("docker") {
        let output = tokio::process::Command::new("docker")
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
                if parts.len() == 3
                    && parts[0].starts_with("moltis-sandbox:")
                    && seen.insert(parts[0].to_string())
                {
                    images.push(SandboxImage {
                        tag: parts[0].to_string(),
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
                if cols.len() >= 2 && cols[0] == "moltis-sandbox" {
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
fn format_bytes(bytes: u64) -> String {
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

/// Remove a specific `moltis-sandbox:*` image.
pub async fn remove_sandbox_image(tag: &str) -> Result<()> {
    anyhow::ensure!(
        tag.starts_with("moltis-sandbox:"),
        "refusing to remove non-sandbox image: {tag}"
    );
    for cli in &["docker", "container"] {
        if !is_cli_available(cli) {
            continue;
        }
        if sandbox_image_exists(cli, tag).await {
            // Apple Container uses `image delete`, Docker uses `image rm`.
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
                anyhow::bail!("{cli} image {subcmd} failed for {tag}: {}", stderr.trim());
            }
        }
    }
    Ok(())
}

/// Remove all local `moltis-sandbox:*` images.
pub async fn clean_sandbox_images() -> Result<usize> {
    let images = list_sandbox_images().await?;
    let count = images.len();
    for img in &images {
        remove_sandbox_image(&img.tag).await?;
    }
    Ok(count)
}

/// Docker-based sandbox implementation.
pub struct DockerSandbox {
    pub config: SandboxConfig,
}

impl DockerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
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

    fn container_name(&self, id: &SandboxId) -> String {
        format!("{}-{}", self.container_prefix(), id.key)
    }

    fn resource_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let limits = &self.config.resource_limits;
        if let Some(ref mem) = limits.memory_limit {
            args.extend(["--memory".to_string(), mem.clone()]);
        }
        if let Some(cpu) = limits.cpu_quota {
            args.extend(["--cpus".to_string(), cpu.to_string()]);
        }
        if let Some(pids) = limits.pids_max {
            args.extend(["--pids-limit".to_string(), pids.to_string()]);
        }
        args
    }

    fn workspace_args(&self) -> Vec<String> {
        let workspace_dir = moltis_config::data_dir();
        let workspace_dir_str = workspace_dir.display().to_string();
        match self.config.workspace_mount {
            WorkspaceMount::Ro => vec![
                "-v".to_string(),
                format!("{workspace_dir_str}:{workspace_dir_str}:ro"),
            ],
            WorkspaceMount::Rw => vec![
                "-v".to_string(),
                format!("{workspace_dir_str}:{workspace_dir_str}:rw"),
            ],
            WorkspaceMount::None => Vec::new(),
        }
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn backend_name(&self) -> &'static str {
        "docker"
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        let name = self.container_name(id);

        // Check if container already running.
        let check = tokio::process::Command::new("docker")
            .args(["inspect", "--format", "{{.State.Running}}", &name])
            .output()
            .await;

        if let Ok(output) = check {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim() == "true" {
                return Ok(());
            }
        }

        // Start a new container.
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            name.clone(),
        ];

        if self.config.no_network {
            args.push("--network=none".to_string());
        }

        if let Some(ref tz) = self.config.timezone {
            args.extend(["-e".to_string(), format!("TZ={tz}")]);
        }

        args.extend(self.resource_args());
        args.extend(self.workspace_args());

        let image = image_override.unwrap_or_else(|| self.image());
        args.push(image.to_string());
        args.extend(["sleep".to_string(), "infinity".to_string()]);

        let output = tokio::process::Command::new("docker")
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker run failed: {}", stderr.trim());
        }

        // Skip provisioning if the image is a pre-built moltis-sandbox image
        // (packages are already baked in — including /home/sandbox from the Dockerfile).
        let is_prebuilt = image.starts_with("moltis-sandbox:");
        if !is_prebuilt {
            provision_packages("docker", &name, &self.config.packages).await?;
        }

        Ok(())
    }

    async fn build_image(
        &self,
        base: &str,
        packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        if packages.is_empty() {
            return Ok(None);
        }

        let tag = sandbox_image_tag(base, packages);

        // Check if image already exists.
        if sandbox_image_exists("docker", &tag).await {
            info!(
                tag,
                "pre-built sandbox image already exists, skipping build"
            );
            return Ok(Some(BuildImageResult { tag, built: false }));
        }

        // Generate Dockerfile in a temp dir.
        let tmp_dir =
            std::env::temp_dir().join(format!("moltis-sandbox-build-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir)?;

        let pkg_list = packages.join(" ");
        let dockerfile = format!(
            "FROM {base}\n\
RUN apt-get update -qq && apt-get install -y -qq {pkg_list}\n\
RUN curl -fsSL https://mise.jdx.dev/install.sh | sh \
    && echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> /etc/profile.d/mise.sh\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
ENV PATH=/home/sandbox/.local/bin:/root/.local/bin:$PATH\n\
WORKDIR /home/sandbox\n"
        );
        let dockerfile_path = tmp_dir.join("Dockerfile");
        std::fs::write(&dockerfile_path, &dockerfile)?;

        info!(tag, packages = %pkg_list, "building pre-built sandbox image");

        let output = tokio::process::Command::new("docker")
            .args(["build", "-t", &tag, "-f"])
            .arg(&dockerfile_path)
            .arg(&tmp_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        // Clean up temp dir regardless of result.
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let output = output?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker build failed for {tag}: {}", stderr.trim());
        }

        info!(tag, "pre-built sandbox image ready");
        Ok(Some(BuildImageResult { tag, built: true }))
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let name = self.container_name(id);

        let mut args = vec!["exec".to_string()];

        if let Some(ref dir) = opts.working_dir {
            args.extend(["-w".to_string(), dir.display().to_string()]);
        }

        for (k, v) in &opts.env {
            args.extend(["-e".to_string(), format!("{}={}", k, v)]);
        }

        args.push(name);
        args.extend(["sh".to_string(), "-c".to_string(), command.to_string()]);

        let child = tokio::process::Command::new("docker")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                if stdout.len() > opts.max_output_bytes {
                    stdout.truncate(opts.max_output_bytes);
                    stdout.push_str("\n... [output truncated]");
                }
                if stderr.len() > opts.max_output_bytes {
                    stderr.truncate(opts.max_output_bytes);
                    stderr.push_str("\n... [output truncated]");
                }

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => anyhow::bail!("docker exec failed: {e}"),
            Err(_) => anyhow::bail!("docker exec timed out after {}s", opts.timeout.as_secs()),
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let name = self.container_name(id);
        let _ = tokio::process::Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .await;
        Ok(())
    }
}

/// No-op sandbox that passes through to direct execution.
pub struct NoSandbox;

#[async_trait]
impl Sandbox for NoSandbox {
    fn backend_name(&self) -> &'static str {
        "none"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        crate::exec::exec_command(command, opts).await
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

/// Cgroup v2 sandbox using `systemd-run --user --scope` (Linux only, no root required).
#[cfg(target_os = "linux")]
pub struct CgroupSandbox {
    pub config: SandboxConfig,
}

#[cfg(target_os = "linux")]
impl CgroupSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    fn scope_name(&self, id: &SandboxId) -> String {
        let prefix = self
            .config
            .container_prefix
            .as_deref()
            .unwrap_or("moltis-sandbox");
        format!("{}-{}", prefix, id.key)
    }

    fn property_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let limits = &self.config.resource_limits;
        if let Some(ref mem) = limits.memory_limit {
            args.extend(["--property".to_string(), format!("MemoryMax={mem}")]);
        }
        if let Some(cpu) = limits.cpu_quota {
            let pct = (cpu * 100.0) as u64;
            args.extend(["--property".to_string(), format!("CPUQuota={pct}%")]);
        }
        if let Some(pids) = limits.pids_max {
            args.extend(["--property".to_string(), format!("TasksMax={pids}")]);
        }
        args
    }
}

#[cfg(target_os = "linux")]
#[async_trait]
impl Sandbox for CgroupSandbox {
    fn backend_name(&self) -> &'static str {
        "cgroup"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        let output = tokio::process::Command::new("systemd-run")
            .arg("--version")
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                debug!("systemd-run available");
                Ok(())
            },
            _ => anyhow::bail!("systemd-run not found; cgroup sandbox requires systemd"),
        }
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let scope = self.scope_name(id);

        let mut args = vec![
            "--user".to_string(),
            "--scope".to_string(),
            "--unit".to_string(),
            scope,
        ];
        args.extend(self.property_args());
        args.extend(["sh".to_string(), "-c".to_string(), command.to_string()]);

        let mut cmd = tokio::process::Command::new("systemd-run");
        cmd.args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        if let Some(ref dir) = opts.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let child = cmd.spawn()?;
        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                if stdout.len() > opts.max_output_bytes {
                    stdout.truncate(opts.max_output_bytes);
                    stdout.push_str("\n... [output truncated]");
                }
                if stderr.len() > opts.max_output_bytes {
                    stderr.truncate(opts.max_output_bytes);
                    stderr.push_str("\n... [output truncated]");
                }

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => anyhow::bail!("systemd-run exec failed: {e}"),
            Err(_) => anyhow::bail!(
                "systemd-run exec timed out after {}s",
                opts.timeout.as_secs()
            ),
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let scope = self.scope_name(id);
        let _ = tokio::process::Command::new("systemctl")
            .args(["--user", "stop", &format!("{scope}.scope")])
            .output()
            .await;
        Ok(())
    }
}

/// Apple Container sandbox using the `container` CLI (macOS 26+, Apple Silicon).
#[cfg(target_os = "macos")]
pub struct AppleContainerSandbox {
    pub config: SandboxConfig,
}

#[cfg(target_os = "macos")]
impl AppleContainerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
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

    fn container_name(&self, id: &SandboxId) -> String {
        format!("{}-{}", self.container_prefix(), id.key)
    }

    /// Check whether the `container` CLI is available.
    pub async fn is_available() -> bool {
        tokio::process::Command::new("container")
            .arg("--version")
            .output()
            .await
            .is_ok_and(|o| o.status.success())
    }
}

#[cfg(target_os = "macos")]
#[async_trait]
impl Sandbox for AppleContainerSandbox {
    fn backend_name(&self) -> &'static str {
        "apple-container"
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        let name = self.container_name(id);
        let image = image_override.unwrap_or_else(|| self.image());

        // Check if container exists and parse its state.
        // Note: `container inspect` returns exit 0 with empty `[]` for nonexistent
        // containers, so we must also check the output content.
        let check = tokio::process::Command::new("container")
            .args(["inspect", &name])
            .output()
            .await;

        if let Ok(output) = check
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Empty array means container doesn't exist — fall through to create.
            if stdout.trim() == "[]" || stdout.trim().is_empty() {
                info!(
                    name,
                    "apple container not found (inspect returned empty), creating"
                );
            } else if stdout.contains("\"running\"") {
                info!(name, "apple container already running");
                return Ok(());
            } else if stdout.contains("stopped") || stdout.contains("exited") {
                info!(name, "apple container stopped, restarting");
                let start = tokio::process::Command::new("container")
                    .args(["start", &name])
                    .output()
                    .await?;
                if !start.status.success() {
                    let stderr = String::from_utf8_lossy(&start.stderr);
                    warn!(name, %stderr, "container start failed, removing and recreating");
                    let _ = tokio::process::Command::new("container")
                        .args(["rm", &name])
                        .output()
                        .await;
                } else {
                    info!(name, "apple container restarted");
                    return Ok(());
                }
            } else {
                // Unknown state — log and recreate.
                info!(name, state = %stdout.chars().take(200).collect::<String>(), "apple container in unknown state, removing and recreating");
                let _ = tokio::process::Command::new("container")
                    .args(["rm", "-f", &name])
                    .output()
                    .await;
            }
        } else {
            info!(name, "apple container not found, creating");
        }

        // Container doesn't exist — create it.
        // Must pass `sleep infinity` so the container stays alive for subsequent
        // exec calls (the default entrypoint /bin/bash exits immediately without a TTY).
        info!(name, image, "creating apple container");
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            name.clone(),
        ];

        if let Some(ref tz) = self.config.timezone {
            args.extend(["-e".to_string(), format!("TZ={tz}")]);
        }

        args.extend([
            image.to_string(),
            "sleep".to_string(),
            "infinity".to_string(),
        ]);

        let output = tokio::process::Command::new("container")
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "container run failed for {name} (image={image}): {}",
                stderr.trim()
            );
        }

        info!(name, image, "apple container created and running");

        // Skip provisioning if the image is a pre-built moltis-sandbox image
        // (packages are already baked in — including /home/sandbox from the Dockerfile).
        let is_prebuilt = image.starts_with("moltis-sandbox:");
        if !is_prebuilt {
            provision_packages("container", &name, &self.config.packages).await?;
        }

        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let name = self.container_name(id);
        info!(name, command, "apple container exec");

        let mut args = vec!["exec".to_string(), name.clone()];

        // Apple Container CLI doesn't support -e flags, so prepend export
        // statements to inject env vars into the shell.
        let mut prefix = String::new();
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

        args.extend(["sh".to_string(), "-c".to_string(), full_command]);

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

                if stdout.len() > opts.max_output_bytes {
                    stdout.truncate(opts.max_output_bytes);
                    stdout.push_str("\n... [output truncated]");
                }
                if stderr.len() > opts.max_output_bytes {
                    stderr.truncate(opts.max_output_bytes);
                    stderr.push_str("\n... [output truncated]");
                }

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
                anyhow::bail!("container exec failed for {name}: {e}")
            },
            Err(_) => {
                warn!(
                    name,
                    timeout_secs = opts.timeout.as_secs(),
                    "apple container exec timed out"
                );
                anyhow::bail!(
                    "container exec timed out for {name} after {}s",
                    opts.timeout.as_secs()
                )
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

        let tag = sandbox_image_tag(base, packages);

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

        let pkg_list = packages.join(" ");
        let dockerfile = format!(
            "FROM {base}\n\
RUN apt-get update -qq && apt-get install -y -qq {pkg_list}\n\
RUN curl -fsSL https://mise.jdx.dev/install.sh | sh \
    && echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> /etc/profile.d/mise.sh\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
ENV PATH=/home/sandbox/.local/bin:/root/.local/bin:$PATH\n\
WORKDIR /home/sandbox\n"
        );
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
            anyhow::bail!("container build failed for {tag}: {}", stderr.trim());
        }

        info!(tag, "pre-built sandbox image ready (apple container)");
        Ok(Some(BuildImageResult { tag, built: true }))
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let name = self.container_name(id);
        info!(name, "cleaning up apple container");
        let _ = tokio::process::Command::new("container")
            .args(["stop", &name])
            .output()
            .await;
        let _ = tokio::process::Command::new("container")
            .args(["rm", &name])
            .output()
            .await;
        Ok(())
    }
}

/// Create the appropriate sandbox backend based on config and platform.
pub fn create_sandbox(config: SandboxConfig) -> Arc<dyn Sandbox> {
    if config.mode == SandboxMode::Off {
        return Arc::new(NoSandbox);
    }

    select_backend(config)
}

/// Create a real sandbox backend regardless of mode (for use by SandboxRouter,
/// which may need a real backend even when global mode is Off because per-session
/// overrides can enable sandboxing dynamically).
fn create_sandbox_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    select_backend(config)
}

/// Select the sandbox backend based on config and platform availability.
///
/// When `backend` is `"auto"` (the default):
/// - On macOS, prefer Apple Container if the `container` CLI is installed
///   (each sandbox runs in a lightweight VM — stronger isolation than Docker).
/// - Fall back to Docker otherwise.
fn select_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    match config.backend.as_str() {
        "docker" => Arc::new(DockerSandbox::new(config)),
        #[cfg(target_os = "macos")]
        "apple-container" => Arc::new(AppleContainerSandbox::new(config)),
        _ => auto_detect_backend(config),
    }
}

fn auto_detect_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    #[cfg(target_os = "macos")]
    {
        if is_cli_available("container") {
            tracing::info!("sandbox backend: apple-container (VM-isolated, preferred)");
            return Arc::new(AppleContainerSandbox::new(config));
        }
    }

    if should_use_docker_backend(is_cli_available("docker"), is_docker_daemon_available()) {
        tracing::info!("sandbox backend: docker");
        return Arc::new(DockerSandbox::new(config));
    }

    if is_cli_available("docker") {
        tracing::warn!(
            "docker CLI detected but daemon is not accessible; sandboxed execution will use direct host access"
        );
    }

    tracing::warn!(
        "no usable container runtime found; sandboxed execution will use direct host access"
    );
    Arc::new(NoSandbox)
}

fn should_use_docker_backend(docker_cli_available: bool, docker_daemon_available: bool) -> bool {
    docker_cli_available && docker_daemon_available
}

fn is_docker_daemon_available() -> bool {
    std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Check whether a CLI tool is available on PATH.
fn is_cli_available(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Events emitted by the sandbox subsystem for UI feedback.
#[derive(Debug, Clone)]
pub enum SandboxEvent {
    /// Package provisioning started (Apple Container per-container install).
    Provisioning {
        container: String,
        packages: Vec<String>,
    },
    /// Package provisioning finished.
    Provisioned { container: String },
    /// Package provisioning failed (non-fatal).
    ProvisionFailed { container: String, error: String },
}

/// Routes sandbox decisions per-session, with per-session overrides on top of global config.
pub struct SandboxRouter {
    config: SandboxConfig,
    backend: Arc<dyn Sandbox>,
    /// Per-session overrides: true = sandboxed, false = direct execution.
    overrides: RwLock<HashMap<String, bool>>,
    /// Per-session image overrides.
    image_overrides: RwLock<HashMap<String, String>>,
    /// Runtime override for the global default image (set via API, persisted externally).
    global_image_override: RwLock<Option<String>>,
    /// Event channel for sandbox events (provision start/done/error).
    event_tx: tokio::sync::broadcast::Sender<SandboxEvent>,
}

impl SandboxRouter {
    pub fn new(config: SandboxConfig) -> Self {
        // Always create a real sandbox backend, even when global mode is Off,
        // because per-session overrides can enable sandboxing dynamically.
        let backend = create_sandbox_backend(config.clone());
        let (event_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            config,
            backend,
            overrides: RwLock::new(HashMap::new()),
            image_overrides: RwLock::new(HashMap::new()),
            global_image_override: RwLock::new(None),
            event_tx,
        }
    }

    /// Create a router with a custom sandbox backend (useful for testing).
    pub fn with_backend(config: SandboxConfig, backend: Arc<dyn Sandbox>) -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            config,
            backend,
            overrides: RwLock::new(HashMap::new()),
            image_overrides: RwLock::new(HashMap::new()),
            global_image_override: RwLock::new(None),
            event_tx,
        }
    }

    /// Subscribe to sandbox events (provision start/done/error).
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<SandboxEvent> {
        self.event_tx.subscribe()
    }

    /// Emit a sandbox event. Silently drops if no subscribers.
    pub fn emit_event(&self, event: SandboxEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Check whether a session should run sandboxed.
    /// Per-session override takes priority, then falls back to global mode.
    pub async fn is_sandboxed(&self, session_key: &str) -> bool {
        if let Some(&override_val) = self.overrides.read().await.get(session_key) {
            return override_val;
        }
        match self.config.mode {
            SandboxMode::Off => false,
            SandboxMode::All => true,
            SandboxMode::NonMain => session_key != "main",
        }
    }

    /// Set a per-session sandbox override.
    pub async fn set_override(&self, session_key: &str, enabled: bool) {
        self.overrides
            .write()
            .await
            .insert(session_key.to_string(), enabled);
    }

    /// Remove a per-session override (revert to global mode).
    pub async fn remove_override(&self, session_key: &str) {
        self.overrides.write().await.remove(session_key);
    }

    /// Derive a SandboxId for a given session key.
    /// The key is sanitized for use as a container name (only alphanumeric, dash, underscore, dot).
    pub fn sandbox_id_for(&self, session_key: &str) -> SandboxId {
        let sanitized: String = session_key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        SandboxId {
            scope: self.config.scope.clone(),
            key: sanitized,
        }
    }

    /// Clean up sandbox resources for a session.
    pub async fn cleanup_session(&self, session_key: &str) -> Result<()> {
        let id = self.sandbox_id_for(session_key);
        self.backend.cleanup(&id).await?;
        self.remove_override(session_key).await;
        Ok(())
    }

    /// Access the sandbox backend.
    pub fn backend(&self) -> &Arc<dyn Sandbox> {
        &self.backend
    }

    /// Access the global sandbox mode.
    pub fn mode(&self) -> &SandboxMode {
        &self.config.mode
    }

    /// Access the global sandbox config.
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Human-readable name of the sandbox backend (e.g. "docker", "apple-container").
    pub fn backend_name(&self) -> &'static str {
        self.backend.backend_name()
    }

    /// Set a per-session image override.
    pub async fn set_image_override(&self, session_key: &str, image: String) {
        self.image_overrides
            .write()
            .await
            .insert(session_key.to_string(), image);
    }

    /// Remove a per-session image override.
    pub async fn remove_image_override(&self, session_key: &str) {
        self.image_overrides.write().await.remove(session_key);
    }

    /// Set a runtime override for the global default image.
    /// Pass `None` to revert to the config/hardcoded default.
    pub async fn set_global_image(&self, image: Option<String>) {
        *self.global_image_override.write().await = image;
    }

    /// Get the current effective default image (runtime override > config > hardcoded).
    pub async fn default_image(&self) -> String {
        if let Some(ref img) = *self.global_image_override.read().await {
            return img.clone();
        }
        self.config
            .image
            .clone()
            .unwrap_or_else(|| DEFAULT_SANDBOX_IMAGE.to_string())
    }

    /// Resolve the container image for a session.
    ///
    /// Priority (highest to lowest):
    /// 1. `skill_image` — from a skill's Dockerfile cache
    /// 2. Per-session override (`session.sandbox_image`)
    /// 3. Runtime global override (`set_global_image`)
    /// 4. Global config (`config.tools.exec.sandbox.image`)
    /// 5. Default constant (`DEFAULT_SANDBOX_IMAGE`)
    pub async fn resolve_image(&self, session_key: &str, skill_image: Option<&str>) -> String {
        if let Some(img) = skill_image {
            return img.to_string();
        }
        if let Some(img) = self.image_overrides.read().await.get(session_key) {
            return img.clone();
        }
        self.default_image().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_mode_display() {
        assert_eq!(SandboxMode::Off.to_string(), "off");
        assert_eq!(SandboxMode::NonMain.to_string(), "non-main");
        assert_eq!(SandboxMode::All.to_string(), "all");
    }

    #[test]
    fn test_sandbox_scope_display() {
        assert_eq!(SandboxScope::Session.to_string(), "session");
        assert_eq!(SandboxScope::Agent.to_string(), "agent");
        assert_eq!(SandboxScope::Shared.to_string(), "shared");
    }

    #[test]
    fn test_workspace_mount_display() {
        assert_eq!(WorkspaceMount::None.to_string(), "none");
        assert_eq!(WorkspaceMount::Ro.to_string(), "ro");
        assert_eq!(WorkspaceMount::Rw.to_string(), "rw");
    }

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert!(limits.memory_limit.is_none());
        assert!(limits.cpu_quota.is_none());
        assert!(limits.pids_max.is_none());
    }

    #[test]
    fn test_resource_limits_serde() {
        let json = r#"{"memory_limit":"512M","cpu_quota":1.5,"pids_max":100}"#;
        let limits: ResourceLimits = serde_json::from_str(json).unwrap();
        assert_eq!(limits.memory_limit.as_deref(), Some("512M"));
        assert_eq!(limits.cpu_quota, Some(1.5));
        assert_eq!(limits.pids_max, Some(100));
    }

    #[test]
    fn test_sandbox_config_serde() {
        let json = r#"{
            "mode": "all",
            "scope": "session",
            "workspace_mount": "rw",
            "no_network": true,
            "resource_limits": {"memory_limit": "1G"}
        }"#;
        let config: SandboxConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mode, SandboxMode::All);
        assert_eq!(config.workspace_mount, WorkspaceMount::Rw);
        assert!(config.no_network);
        assert_eq!(config.resource_limits.memory_limit.as_deref(), Some("1G"));
    }

    #[test]
    fn test_docker_resource_args() {
        let config = SandboxConfig {
            resource_limits: ResourceLimits {
                memory_limit: Some("256M".into()),
                cpu_quota: Some(0.5),
                pids_max: Some(50),
            },
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let args = docker.resource_args();
        assert_eq!(args, vec![
            "--memory",
            "256M",
            "--cpus",
            "0.5",
            "--pids-limit",
            "50"
        ]);
    }

    #[test]
    fn test_docker_workspace_args_ro() {
        let config = SandboxConfig {
            workspace_mount: WorkspaceMount::Ro,
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let args = docker.workspace_args();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-v");
        assert!(args[1].ends_with(":ro"));
    }

    #[test]
    fn test_docker_workspace_args_none() {
        let config = SandboxConfig {
            workspace_mount: WorkspaceMount::None,
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        assert!(docker.workspace_args().is_empty());
    }

    #[test]
    fn test_create_sandbox_off() {
        let config = SandboxConfig::default();
        let sandbox = create_sandbox(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test".into(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            sandbox.ensure_ready(&id, None).await.unwrap();
            sandbox.cleanup(&id).await.unwrap();
        });
    }

    #[tokio::test]
    async fn test_no_sandbox_exec() {
        let sandbox = NoSandbox;
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test".into(),
        };
        let opts = ExecOpts::default();
        let result = sandbox.exec(&id, "echo sandbox-test", &opts).await.unwrap();
        assert_eq!(result.stdout.trim(), "sandbox-test");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_docker_container_name() {
        let config = SandboxConfig {
            container_prefix: Some("my-prefix".into()),
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "abc123".into(),
        };
        assert_eq!(docker.container_name(&id), "my-prefix-abc123");
    }

    #[tokio::test]
    async fn test_sandbox_router_default_all() {
        let config = SandboxConfig::default(); // mode = All
        let router = SandboxRouter::new(config);
        assert!(router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_off() {
        let config = SandboxConfig {
            mode: SandboxMode::Off,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert!(!router.is_sandboxed("main").await);
        assert!(!router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_all() {
        let config = SandboxConfig {
            mode: SandboxMode::All,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert!(router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_non_main() {
        let config = SandboxConfig {
            mode: SandboxMode::NonMain,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert!(!router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_override() {
        let config = SandboxConfig {
            mode: SandboxMode::Off,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert!(!router.is_sandboxed("session:abc").await);

        router.set_override("session:abc", true).await;
        assert!(router.is_sandboxed("session:abc").await);

        router.set_override("session:abc", false).await;
        assert!(!router.is_sandboxed("session:abc").await);

        router.remove_override("session:abc").await;
        assert!(!router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_override_overrides_mode() {
        let config = SandboxConfig {
            mode: SandboxMode::All,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert!(router.is_sandboxed("main").await);

        // Override to disable sandbox for main
        router.set_override("main", false).await;
        assert!(!router.is_sandboxed("main").await);
    }

    #[test]
    fn test_backend_name_docker() {
        let sandbox = DockerSandbox::new(SandboxConfig::default());
        assert_eq!(sandbox.backend_name(), "docker");
    }

    #[test]
    fn test_backend_name_none() {
        let sandbox = NoSandbox;
        assert_eq!(sandbox.backend_name(), "none");
    }

    #[test]
    fn test_sandbox_router_backend_name() {
        // With "auto", the backend depends on what's available on the host.
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        let name = router.backend_name();
        assert!(
            name == "docker" || name == "apple-container" || name == "none",
            "unexpected backend: {name}"
        );
    }

    #[test]
    fn test_sandbox_router_explicit_docker_backend() {
        let config = SandboxConfig {
            backend: "docker".into(),
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert_eq!(router.backend_name(), "docker");
    }

    #[test]
    fn test_sandbox_router_config_accessor() {
        let config = SandboxConfig {
            mode: SandboxMode::NonMain,
            scope: SandboxScope::Agent,
            image: Some("alpine:latest".into()),
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert_eq!(*router.mode(), SandboxMode::NonMain);
        assert_eq!(router.config().scope, SandboxScope::Agent);
        assert_eq!(router.config().image.as_deref(), Some("alpine:latest"));
    }

    #[test]
    fn test_sandbox_router_sandbox_id_for() {
        let config = SandboxConfig {
            scope: SandboxScope::Session,
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        let id = router.sandbox_id_for("session:abc");
        assert_eq!(id.key, "session-abc");
        // Plain alphanumeric keys pass through unchanged.
        let id2 = router.sandbox_id_for("main");
        assert_eq!(id2.key, "main");
    }

    #[tokio::test]
    async fn test_resolve_image_default() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        let img = router.resolve_image("main", None).await;
        assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
    }

    #[tokio::test]
    async fn test_resolve_image_skill_override() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        let img = router
            .resolve_image("main", Some("moltis-cache/my-skill:abc123"))
            .await;
        assert_eq!(img, "moltis-cache/my-skill:abc123");
    }

    #[tokio::test]
    async fn test_resolve_image_session_override() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        router
            .set_image_override("sess1", "custom:latest".into())
            .await;
        let img = router.resolve_image("sess1", None).await;
        assert_eq!(img, "custom:latest");
    }

    #[tokio::test]
    async fn test_resolve_image_skill_beats_session() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        router
            .set_image_override("sess1", "custom:latest".into())
            .await;
        let img = router
            .resolve_image("sess1", Some("moltis-cache/skill:hash"))
            .await;
        assert_eq!(img, "moltis-cache/skill:hash");
    }

    #[tokio::test]
    async fn test_resolve_image_config_override() {
        let config = SandboxConfig {
            image: Some("my-org/image:v1".into()),
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        let img = router.resolve_image("main", None).await;
        assert_eq!(img, "my-org/image:v1");
    }

    #[tokio::test]
    async fn test_remove_image_override() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        router
            .set_image_override("sess1", "custom:latest".into())
            .await;
        router.remove_image_override("sess1").await;
        let img = router.resolve_image("sess1", None).await;
        assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
    }

    #[test]
    fn test_docker_image_tag_deterministic() {
        let packages = vec!["curl".into(), "git".into(), "wget".into()];
        let tag1 = sandbox_image_tag("ubuntu:25.10", &packages);
        let tag2 = sandbox_image_tag("ubuntu:25.10", &packages);
        assert_eq!(tag1, tag2);
        assert!(tag1.starts_with("moltis-sandbox:"));
    }

    #[test]
    fn test_docker_image_tag_order_independent() {
        let p1 = vec!["curl".into(), "git".into()];
        let p2 = vec!["git".into(), "curl".into()];
        assert_eq!(
            sandbox_image_tag("ubuntu:25.10", &p1),
            sandbox_image_tag("ubuntu:25.10", &p2),
        );
    }

    #[test]
    fn test_docker_image_tag_changes_with_base() {
        let packages = vec!["curl".into()];
        let t1 = sandbox_image_tag("ubuntu:25.10", &packages);
        let t2 = sandbox_image_tag("ubuntu:24.04", &packages);
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_docker_image_tag_changes_with_packages() {
        let p1 = vec!["curl".into()];
        let p2 = vec!["curl".into(), "git".into()];
        let t1 = sandbox_image_tag("ubuntu:25.10", &p1);
        let t2 = sandbox_image_tag("ubuntu:25.10", &p2);
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_no_sandbox_build_image_is_noop() {
        let sandbox = NoSandbox;
        let result = sandbox
            .build_image("ubuntu:25.10", &["curl".into()])
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_sandbox_router_events() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);
        let mut rx = router.subscribe_events();

        router.emit_event(SandboxEvent::Provisioning {
            container: "test".into(),
            packages: vec!["curl".into()],
        });

        let event = rx.try_recv().unwrap();
        match event {
            SandboxEvent::Provisioning {
                container,
                packages,
            } => {
                assert_eq!(container, "test");
                assert_eq!(packages, vec!["curl".to_string()]);
            },
            _ => panic!("unexpected event variant"),
        }
    }

    #[tokio::test]
    async fn test_sandbox_router_global_image_override() {
        let config = SandboxConfig::default();
        let router = SandboxRouter::new(config);

        // Default
        let img = router.default_image().await;
        assert_eq!(img, DEFAULT_SANDBOX_IMAGE);

        // Set global override
        router
            .set_global_image(Some("moltis-sandbox:abc123".into()))
            .await;
        let img = router.default_image().await;
        assert_eq!(img, "moltis-sandbox:abc123");

        // Global override flows through resolve_image
        let img = router.resolve_image("main", None).await;
        assert_eq!(img, "moltis-sandbox:abc123");

        // Session override still wins
        router.set_image_override("main", "custom:v1".into()).await;
        let img = router.resolve_image("main", None).await;
        assert_eq!(img, "custom:v1");

        // Clear and revert
        router.set_global_image(None).await;
        router.remove_image_override("main").await;
        let img = router.default_image().await;
        assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_backend_name_apple_container() {
        let sandbox = AppleContainerSandbox::new(SandboxConfig::default());
        assert_eq!(sandbox.backend_name(), "apple-container");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_sandbox_router_explicit_apple_container_backend() {
        let config = SandboxConfig {
            backend: "apple-container".into(),
            ..Default::default()
        };
        let router = SandboxRouter::new(config);
        assert_eq!(router.backend_name(), "apple-container");
    }

    /// When both Docker and Apple Container are available, test that we can
    /// explicitly select each one.
    #[test]
    fn test_select_backend_explicit_choices() {
        // Docker backend
        if is_cli_available("docker") {
            let config = SandboxConfig {
                backend: "docker".into(),
                ..Default::default()
            };
            let backend = select_backend(config);
            assert_eq!(backend.backend_name(), "docker");
        }

        // Apple Container backend (macOS only)
        #[cfg(target_os = "macos")]
        if is_cli_available("container") {
            let config = SandboxConfig {
                backend: "apple-container".into(),
                ..Default::default()
            };
            let backend = select_backend(config);
            assert_eq!(backend.backend_name(), "apple-container");
        }
    }

    #[test]
    fn test_is_debian_host() {
        let result = is_debian_host();
        // On macOS/Windows this should be false; on Debian/Ubuntu it should be true.
        if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
            assert!(!result);
        }
        // On Linux, it depends on the distro — just verify it returns a bool without panic.
        let _ = result;
    }

    #[tokio::test]
    async fn test_provision_host_packages_empty() {
        let result = provision_host_packages(&[]).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_provision_host_packages_non_debian() {
        if is_debian_host() {
            // Can't test the non-debian path on a Debian host.
            return;
        }
        let result = provision_host_packages(&["curl".into()]).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_should_use_docker_backend() {
        assert!(should_use_docker_backend(true, true));
        assert!(!should_use_docker_backend(true, false));
        assert!(!should_use_docker_backend(false, true));
        assert!(!should_use_docker_backend(false, false));
    }

    #[cfg(target_os = "linux")]
    mod linux_tests {
        use super::*;

        #[test]
        fn test_cgroup_scope_name() {
            let config = SandboxConfig::default();
            let cgroup = CgroupSandbox::new(config);
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "sess1".into(),
            };
            assert_eq!(cgroup.scope_name(&id), "moltis-sandbox-sess1");
        }

        #[test]
        fn test_cgroup_property_args() {
            let config = SandboxConfig {
                resource_limits: ResourceLimits {
                    memory_limit: Some("1G".into()),
                    cpu_quota: Some(2.0),
                    pids_max: Some(200),
                },
                ..Default::default()
            };
            let cgroup = CgroupSandbox::new(config);
            let args = cgroup.property_args();
            assert!(args.contains(&"MemoryMax=1G".to_string()));
            assert!(args.contains(&"CPUQuota=200%".to_string()));
            assert!(args.contains(&"TasksMax=200".to_string()));
        }
    }
}
