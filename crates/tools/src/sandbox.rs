use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

#[cfg(feature = "wasm")]
use crate::wasm_engine::WasmComponentEngine;
use crate::{
    exec::{ExecOpts, ExecResult},
    wasm_limits::WasmToolLimits,
};

fn truncate_output_for_display(output: &mut String, max_output_bytes: usize) {
    if output.len() <= max_output_bytes {
        return;
    }
    output.truncate(output.floor_char_boundary(max_output_bytes));
    output.push_str("\n... [output truncated]");
}

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

/// Check whether the current process is running as root (UID 0).
fn is_running_as_root() -> bool {
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

fn host_package_name_candidates(pkg: &str) -> Vec<String> {
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

/// Persistence mode for `/home/sandbox` in container backends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HomePersistence {
    Off,
    Session,
    #[default]
    Shared,
}

impl std::fmt::Display for HomePersistence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Session => f.write_str("session"),
            Self::Shared => f.write_str("shared"),
        }
    }
}

impl From<&moltis_config::schema::HomePersistenceConfig> for HomePersistence {
    fn from(value: &moltis_config::schema::HomePersistenceConfig) -> Self {
        match value {
            moltis_config::schema::HomePersistenceConfig::Off => Self::Off,
            moltis_config::schema::HomePersistenceConfig::Session => Self::Session,
            moltis_config::schema::HomePersistenceConfig::Shared => Self::Shared,
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
    /// Persistence strategy for `/home/sandbox`.
    pub home_persistence: HomePersistence,
    /// Host directory used for shared `/home/sandbox` persistence.
    /// Relative paths are resolved against `data_dir()`.
    pub shared_home_dir: Option<PathBuf>,
    pub image: Option<String>,
    pub container_prefix: Option<String>,
    pub no_network: bool,
    /// Backend: `"auto"` (default), `"docker"`, `"apple-container"`,
    /// `"restricted-host"`, or `"wasm"`.
    /// `"auto"` prefers Apple Container on macOS, then Docker, then restricted-host.
    pub backend: String,
    pub resource_limits: ResourceLimits,
    /// Packages to install via `apt-get` after container creation.
    /// Set to an empty list to skip provisioning.
    pub packages: Vec<String>,
    /// IANA timezone (e.g. "Europe/Paris") injected as `TZ` env var into containers.
    pub timezone: Option<String>,
    /// Fuel limit for WASM sandbox execution (default: 1 billion instructions).
    pub wasm_fuel_limit: Option<u64>,
    /// Epoch interruption interval in milliseconds for WASM sandbox (default: 100ms).
    pub wasm_epoch_interval_ms: Option<u64>,
    /// Per-tool WASM limits (fuel/memory). Falls back to built-in defaults when absent.
    pub wasm_tool_limits: Option<WasmToolLimits>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::default(),
            scope: SandboxScope::default(),
            workspace_mount: WorkspaceMount::default(),
            home_persistence: HomePersistence::default(),
            shared_home_dir: None,
            image: None,
            container_prefix: None,
            no_network: false,
            backend: "auto".into(),
            resource_limits: ResourceLimits::default(),
            packages: Vec::new(),
            timezone: None,
            wasm_fuel_limit: None,
            wasm_epoch_interval_ms: None,
            wasm_tool_limits: None,
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
            home_persistence: HomePersistence::from(&cfg.home_persistence),
            shared_home_dir: cfg
                .shared_home_dir
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
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
            wasm_fuel_limit: cfg.wasm_fuel_limit,
            wasm_epoch_interval_ms: cfg.wasm_epoch_interval_ms,
            wasm_tool_limits: cfg.wasm_tool_limits.as_ref().map(WasmToolLimits::from),
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

    /// Whether this backend provides actual isolation.
    /// Returns `false` for `NoSandbox` (pass-through to host).
    fn is_real(&self) -> bool {
        true
    }

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

fn canonical_sandbox_packages(packages: &[String]) -> Vec<String> {
    let mut canonical: Vec<String> = packages
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    canonical.sort();
    canonical.dedup();
    canonical
}

const SANDBOX_HOME_DIR: &str = "/home/sandbox";
const GOGCLI_MODULE_PATH: &str = "github.com/steipete/gogcli/cmd/gog";
const GOGCLI_VERSION: &str = "latest";
#[cfg(target_os = "macos")]
const APPLE_CONTAINER_SAFE_WORKDIR: &str = "/tmp";

fn sanitize_path_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

fn sandbox_home_persistence_base_dir() -> PathBuf {
    moltis_config::data_dir().join("sandbox").join("home")
}

fn default_shared_home_dir() -> PathBuf {
    sandbox_home_persistence_base_dir().join("shared")
}

fn resolve_shared_home_dir(config: &SandboxConfig) -> PathBuf {
    let Some(path) = config
        .shared_home_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return default_shared_home_dir();
    };
    if path.is_absolute() {
        return path.clone();
    }
    moltis_config::data_dir().join(path)
}

/// Effective host path used when shared home persistence is enabled.
pub fn shared_home_dir_path(config: &SandboxConfig) -> PathBuf {
    resolve_shared_home_dir(config)
}

fn sandbox_home_persistence_host_dir(config: &SandboxConfig, id: &SandboxId) -> Option<PathBuf> {
    let base = sandbox_home_persistence_base_dir();
    match config.home_persistence {
        HomePersistence::Off => None,
        HomePersistence::Shared => Some(resolve_shared_home_dir(config)),
        HomePersistence::Session => {
            Some(base.join("session").join(sanitize_path_component(&id.key)))
        },
    }
}

fn ensure_sandbox_home_persistence_host_dir(
    config: &SandboxConfig,
    id: &SandboxId,
) -> Result<Option<PathBuf>> {
    let Some(path) = sandbox_home_persistence_host_dir(config, id) else {
        return Ok(None);
    };
    std::fs::create_dir_all(&path)?;
    Ok(Some(path))
}

fn sandbox_image_dockerfile(base: &str, packages: &[String]) -> String {
    let pkg_list = canonical_sandbox_packages(packages).join(" ");
    format!(
        "FROM {base}\n\
RUN apt-get update -qq && apt-get install -y -qq {pkg_list} \
    && mkdir -p {SANDBOX_HOME_DIR}\n\
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
fn apple_container_bootstrap_command() -> String {
    format!("mkdir -p {SANDBOX_HOME_DIR} && exec sleep infinity")
}

#[cfg(target_os = "macos")]
fn apple_container_run_args(
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

#[cfg(target_os = "macos")]
fn apple_container_exec_args(name: &str, shell_command: String) -> Vec<String> {
    vec![
        "exec".to_string(),
        "--workdir".to_string(),
        APPLE_CONTAINER_SAFE_WORKDIR.to_string(),
        name.to_string(),
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

fn is_sandbox_image_tag(tag: &str) -> bool {
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
fn rebuildable_sandbox_image_tag(
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

/// List all local `<instance>-sandbox:*` images across available container CLIs.
pub async fn list_sandbox_images() -> Result<Vec<SandboxImage>> {
    let mut images = Vec::new();
    let mut seen = HashSet::new();

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
                    && is_sandbox_image_tag(parts[0])
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

/// Remove a specific `<instance>-sandbox:*` image.
pub async fn remove_sandbox_image(tag: &str) -> Result<()> {
    anyhow::ensure!(
        is_sandbox_image_tag(tag),
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

fn mark_zombie(name: &str) {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.insert(name.to_string());
    }
}

fn unmark_zombie(name: &str) {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.remove(name);
    }
}

fn clear_zombies() {
    if let Ok(mut set) = ZOMBIE_CONTAINERS.write() {
        set.clear();
    }
}

fn is_zombie(name: &str) -> bool {
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

    // Docker: `docker ps -a --filter name=<prefix> --format json` outputs one JSON per line.
    if is_cli_available("docker") {
        let output = tokio::process::Command::new("docker")
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
                    backend: ContainerBackend::Docker,
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

    // Fallback: Docker `docker system df --format json` (one JSON per line per type).
    if is_cli_available("docker") {
        let output = tokio::process::Command::new("docker")
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

    anyhow::bail!("no container CLI available for disk usage")
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
    // Try Apple Container first, then Docker.
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
    if is_cli_available("docker") {
        let output = tokio::process::Command::new("docker")
            .args(["stop", name])
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker stop failed for {name}: {}", stderr.trim());
        }
        return Ok(());
    }
    anyhow::bail!("no container CLI available to stop {name}")
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
                    anyhow::bail!(
                        "container rm failed for running container {name}: {}",
                        stderr.trim()
                    );
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
    if is_cli_available("docker") {
        let output = tokio::process::Command::new("docker")
            .args(["rm", "-f", name])
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker rm failed for {name}: {}", stderr.trim());
        }
        unmark_zombie(name);
        return Ok(());
    }
    anyhow::bail!("no container CLI available to remove {name}")
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
            anyhow::bail!("container system stop failed: {}", stderr.trim());
        }
        let start = tokio::process::Command::new("container")
            .args(["system", "start"])
            .output()
            .await?;
        if !start.status.success() {
            let stderr = String::from_utf8_lossy(&start.stderr);
            anyhow::bail!("container system start failed: {}", stderr.trim());
        }
        clear_zombies();
        return Ok(());
    }
    if is_cli_available("docker") {
        // Docker Desktop restarts via the CLI are not straightforward;
        // restarting the daemon is platform-specific. Best-effort: just
        // prune stopped containers to clear stale state.
        let _ = tokio::process::Command::new("docker")
            .args(["container", "prune", "-f"])
            .output()
            .await;
        return Ok(());
    }
    anyhow::bail!("no container CLI available to restart daemon")
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

    fn image_repo(&self) -> &str {
        self.container_prefix()
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

    /// Security hardening flags for `docker run`.
    ///
    /// `is_prebuilt` controls whether `--read-only` is applied: prebuilt images
    /// already have packages baked in so the root FS can be read-only, while
    /// non-prebuilt images need a writable root for `apt-get` provisioning.
    fn hardening_args(is_prebuilt: bool) -> Vec<String> {
        let mut args = vec![
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges".to_string(),
            "--tmpfs".to_string(),
            "/tmp:rw,nosuid,size=256m".to_string(),
            "--tmpfs".to_string(),
            "/run:rw,nosuid,size=64m".to_string(),
        ];
        if is_prebuilt {
            args.push("--read-only".to_string());
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

    fn home_persistence_args(&self, id: &SandboxId) -> Result<Vec<String>> {
        let Some(host_dir) = ensure_sandbox_home_persistence_host_dir(&self.config, id)? else {
            return Ok(Vec::new());
        };
        let volume = format!("{}:{SANDBOX_HOME_DIR}:rw", host_dir.display());
        Ok(vec!["-v".to_string(), volume])
    }

    async fn resolve_local_image(&self, requested_image: &str) -> Result<String> {
        if sandbox_image_exists("docker", requested_image).await {
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
                "sandbox image missing locally, rebuilding on demand"
            );
        } else {
            warn!(
                requested = requested_image,
                rebuilt = %rebuild_tag,
                "requested sandbox image missing locally, using deterministic tag from current config"
            );
        }

        let Some(result) = self.build_image(&base_image, &packages).await? else {
            return Ok(requested_image.to_string());
        };
        Ok(result.tag)
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

        // Resolve image first so we know whether it's prebuilt (affects hardening).
        let requested_image = image_override.unwrap_or_else(|| self.image());
        let image = self.resolve_local_image(requested_image).await?;
        let is_prebuilt = image.starts_with(&format!("{}:", self.image_repo()));

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
        args.extend(Self::hardening_args(is_prebuilt));
        args.extend(self.workspace_args());
        args.extend(self.home_persistence_args(id)?);

        args.push(image.clone());
        args.extend(["sleep".to_string(), "infinity".to_string()]);

        let output = tokio::process::Command::new("docker")
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker run failed: {}", stderr.trim());
        }

        // Skip provisioning if the image is a pre-built instance sandbox image
        // (packages are already baked in — including /home/sandbox from the Dockerfile).
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

        let tag = sandbox_image_tag(self.image_repo(), base, packages);

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

        let pkg_list = canonical_sandbox_packages(packages).join(" ");
        let dockerfile = sandbox_image_dockerfile(base, packages);
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

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

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

    fn is_real(&self) -> bool {
        false
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

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

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

/// Restricted host sandbox providing OS-level isolation (env clearing,
/// restricted PATH, rlimits) without containers or WASM. Commands run on the
/// host via `sh -c` with sanitised environment and ulimit wrappers.
pub struct RestrictedHostSandbox {
    config: SandboxConfig,
}

impl RestrictedHostSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    /// Wrap a command with shell `ulimit` calls for resource isolation.
    fn build_ulimit_wrapped_command(&self, command: &str) -> String {
        let limits = &self.config.resource_limits;
        let mut preamble = Vec::new();

        // Max user processes.
        let nproc = limits.pids_max.map(u64::from).unwrap_or(256);
        preamble.push(format!("ulimit -u {nproc} 2>/dev/null"));

        // Max open file descriptors.
        preamble.push("ulimit -n 1024 2>/dev/null".to_string());

        // CPU time in seconds.
        let cpu_secs = limits
            .cpu_quota
            .map(|q| q.ceil() as u64 * 60)
            .unwrap_or(300);
        preamble.push(format!("ulimit -t {cpu_secs} 2>/dev/null"));

        // Virtual memory (in KB for ulimit -v).
        let mem_bytes = limits
            .memory_limit
            .as_deref()
            .and_then(parse_memory_limit)
            .unwrap_or(512 * 1024 * 1024);
        let mem_kb = mem_bytes / 1024;
        preamble.push(format!("ulimit -v {mem_kb} 2>/dev/null"));

        format!("{}; {command}", preamble.join("; "))
    }
}

/// Parse a human-readable memory limit like "512M" or "1G" into bytes.
fn parse_memory_limit(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, multiplier) =
        if let Some(n) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
            (n, 1024 * 1024 * 1024)
        } else if let Some(n) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
            (n, 1024 * 1024)
        } else if let Some(n) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
            (n, 1024)
        } else {
            (s, 1)
        };
    num_part.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

#[async_trait]
impl Sandbox for RestrictedHostSandbox {
    fn backend_name(&self) -> &'static str {
        "restricted-host"
    }

    fn is_real(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        // Wrap the command with shell ulimit calls for resource isolation.
        let wrapped = self.build_ulimit_wrapped_command(command);

        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", &wrapped]);

        // Scrub all inherited env vars for isolation.
        cmd.env_clear();

        // Set minimal safe environment.
        cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");
        cmd.env("HOME", "/tmp");
        cmd.env("LANG", "C.UTF-8");

        // Apply user-specified env vars.
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        if let Some(ref dir) = opts.working_dir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        let child = cmd.spawn()?;
        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => anyhow::bail!("restricted-host sandbox exec failed: {e}"),
            Err(_) => anyhow::bail!(
                "restricted-host sandbox exec timed out after {}s",
                opts.timeout.as_secs()
            ),
        }
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

/// Returns `true` when the WASM sandbox feature is compiled in.
#[cfg(feature = "wasm")]
pub fn is_wasm_sandbox_available() -> bool {
    true
}

#[cfg(not(feature = "wasm"))]
pub fn is_wasm_sandbox_available() -> bool {
    false
}

// ---------------------------------------------------------------------------
// WASM sandbox (real Wasmtime + WASI isolation)
// ---------------------------------------------------------------------------

/// Real WASM sandbox that uses Wasmtime + WASI for isolated execution.
///
/// Two execution tiers:
/// - **Built-in commands** (~20 common coreutils): echo, cat, ls, mkdir, rm,
///   cp, mv, pwd, env, head, tail, wc, sort, touch, which, true, false,
///   test/[, basename, dirname.  These operate on a sandboxed directory tree.
/// - **WASM module execution**: `.wasm` files run via Wasmtime + WASI with
///   preopened dirs, fuel metering, epoch interruption, and captured I/O.
///
/// Unknown commands return exit code 127.
#[cfg(feature = "wasm")]
pub struct WasmSandbox {
    config: SandboxConfig,
    wasm_engine: Arc<WasmComponentEngine>,
}

#[cfg(feature = "wasm")]
impl WasmSandbox {
    pub fn new(config: SandboxConfig) -> Result<Self> {
        let memory_reservation = config
            .resource_limits
            .memory_limit
            .as_deref()
            .and_then(parse_memory_limit);
        let wasm_engine = Arc::new(WasmComponentEngine::new(memory_reservation)?);
        Ok(Self {
            config,
            wasm_engine,
        })
    }

    /// Default fuel limit: 1 billion instructions.
    fn fuel_limit(&self) -> u64 {
        self.config.wasm_fuel_limit.unwrap_or(1_000_000_000)
    }

    /// Default epoch interval: 100ms.
    fn epoch_interval_ms(&self) -> u64 {
        self.config.wasm_epoch_interval_ms.unwrap_or(100)
    }

    /// Root directory for this sandbox instance's isolated filesystem.
    fn sandbox_root(&self, id: &SandboxId) -> PathBuf {
        match self.config.home_persistence {
            HomePersistence::Shared => {
                let base = self.config.shared_home_dir.clone().unwrap_or_else(|| {
                    moltis_config::data_dir()
                        .join("sandbox")
                        .join("home")
                        .join("shared")
                });
                base.join("wasm")
            },
            HomePersistence::Session => moltis_config::data_dir()
                .join("sandbox")
                .join("wasm")
                .join(sanitize_path_component(&id.key)),
            HomePersistence::Off => moltis_config::data_dir()
                .join("sandbox")
                .join("wasm")
                .join(sanitize_path_component(&id.key)),
        }
    }

    /// Guest home directory inside the sandboxed filesystem.
    fn home_dir(&self, id: &SandboxId) -> PathBuf {
        self.sandbox_root(id).join("home")
    }

    /// Guest tmp directory inside the sandboxed filesystem.
    fn tmp_dir(&self, id: &SandboxId) -> PathBuf {
        self.sandbox_root(id).join("tmp")
    }

    /// Execute a `.wasm` module via Wasmtime + WASI with full isolation.
    async fn exec_wasm_module(
        &self,
        wasm_path: &std::path::Path,
        args: &[String],
        id: &SandboxId,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let wasm_engine = Arc::clone(&self.wasm_engine);
        let fuel_limit = self.fuel_limit();
        let epoch_interval_ms = self.epoch_interval_ms();
        let home_dir = self.home_dir(id);
        let tmp_dir = self.tmp_dir(id);
        let wasm_bytes = tokio::fs::read(wasm_path).await?;
        let args = args.to_vec();
        let timeout = opts.timeout;
        let max_output_bytes = opts.max_output_bytes;
        let env_vars: Vec<(String, String)> = opts.env.clone().into_iter().collect();

        let result = tokio::task::spawn_blocking(move || -> Result<ExecResult> {
            use wasmtime_wasi::pipe::MemoryOutputPipe;

            let engine = wasm_engine.engine().clone();
            let stdout_pipe = MemoryOutputPipe::new(max_output_bytes);
            let stderr_pipe = MemoryOutputPipe::new(max_output_bytes);

            let mut wasi_builder = wasmtime_wasi::WasiCtxBuilder::new();
            wasi_builder.stdout(stdout_pipe.clone());
            wasi_builder.stderr(stderr_pipe.clone());
            wasi_builder.args(&args);

            // Minimal safe environment.
            wasi_builder.env("PATH", "/usr/local/bin:/usr/bin:/bin");
            wasi_builder.env("HOME", "/home/sandbox");
            wasi_builder.env("LANG", "C.UTF-8");
            for (k, v) in &env_vars {
                wasi_builder.env(k, v);
            }

            // Preopened directories for filesystem isolation.
            wasi_builder
                .preopened_dir(
                    &home_dir,
                    "/home/sandbox",
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                )
                .map_err(|e| anyhow::anyhow!("failed to preopen /home/sandbox: {e}"))?;
            wasi_builder
                .preopened_dir(
                    &tmp_dir,
                    "/tmp",
                    wasmtime_wasi::DirPerms::all(),
                    wasmtime_wasi::FilePerms::all(),
                )
                .map_err(|e| anyhow::anyhow!("failed to preopen /tmp: {e}"))?;

            // Build preview1-compatible context for core WASM modules.
            let wasi_p1 = wasi_builder.build_p1();

            let mut store = wasmtime::Store::new(&engine, wasi_p1);
            store.set_fuel(fuel_limit)?;
            store.set_epoch_deadline(1);

            // Background epoch ticker for timeout enforcement.
            let engine_clone = engine.clone();
            let epoch_handle = std::thread::spawn(move || {
                let interval = std::time::Duration::from_millis(epoch_interval_ms);
                let deadline = std::time::Instant::now() + timeout;
                while std::time::Instant::now() < deadline {
                    std::thread::sleep(interval);
                    engine_clone.increment_epoch();
                }
            });

            let module = wasm_engine.compile_module(&wasm_bytes)?;
            let mut linker = wasmtime::Linker::new(&engine);
            wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |ctx| ctx)?;

            let instance = linker.instantiate(&mut store, &module)?;
            let func = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .map_err(|e| anyhow::anyhow!("WASM module missing _start: {e}"))?;

            let collect_pipe = |pipe: MemoryOutputPipe| -> String {
                let b: bytes::Bytes = pipe.try_into_inner().unwrap_or_default().into();
                String::from_utf8_lossy(&b).into_owned()
            };

            let exit_code: i32 = match func.call(&mut store, ()) {
                Ok(()) => 0,
                Err(e) => {
                    if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
                        exit.0
                    } else {
                        let msg = format!("{e:#}");
                        let mut stdout_str = collect_pipe(stdout_pipe);
                        let mut stderr_str = collect_pipe(stderr_pipe);
                        truncate_output_for_display(&mut stdout_str, max_output_bytes);

                        if msg.contains("fuel") || msg.contains("epoch") {
                            stderr_str.push_str(&format!("\nWASM execution limit exceeded: {msg}"));
                            truncate_output_for_display(&mut stderr_str, max_output_bytes);
                            drop(epoch_handle);
                            return Ok(ExecResult {
                                stdout: stdout_str,
                                stderr: stderr_str,
                                exit_code: 137,
                            });
                        }

                        stderr_str.push_str(&format!("\nWASM error: {msg}"));
                        truncate_output_for_display(&mut stderr_str, max_output_bytes);
                        drop(epoch_handle);
                        return Ok(ExecResult {
                            stdout: stdout_str,
                            stderr: stderr_str,
                            exit_code: 1,
                        });
                    }
                },
            };

            drop(epoch_handle);
            let mut stdout = collect_pipe(stdout_pipe);
            let mut stderr = collect_pipe(stderr_pipe);
            truncate_output_for_display(&mut stdout, max_output_bytes);
            truncate_output_for_display(&mut stderr, max_output_bytes);

            Ok(ExecResult {
                stdout,
                stderr,
                exit_code,
            })
        })
        .await??;

        Ok(result)
    }
}

#[cfg(feature = "wasm")]
#[async_trait]
impl Sandbox for WasmSandbox {
    fn backend_name(&self) -> &'static str {
        "wasm"
    }

    fn is_real(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        let home = self.home_dir(id);
        let tmp = self.tmp_dir(id);
        tokio::fs::create_dir_all(&home).await?;
        tokio::fs::create_dir_all(&tmp).await?;
        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let sandbox_root = self.sandbox_root(id);
        let env_map: HashMap<String, String> = opts.env.iter().cloned().collect();

        // Parse the command string.
        let segments = WasmBuiltins::parse_command_line(command);

        let mut last_result = ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        };

        for segment in &segments {
            let should_run = match segment.connector {
                CommandConnector::First | CommandConnector::Sequence => true,
                CommandConnector::And => last_result.exit_code == 0,
                CommandConnector::Or => last_result.exit_code != 0,
            };

            if !should_run {
                continue;
            }

            let expanded = WasmBuiltins::expand_vars(&segment.args, &env_map);
            if expanded.is_empty() {
                continue;
            }

            let empty = String::new();
            let (cmd_name, cmd_args) = expanded.split_first().unwrap_or((&empty, &[]));

            // Check for output redirect.
            let (cmd_args, redirect) = WasmBuiltins::extract_redirect(cmd_args);

            // Check if this is a .wasm file reference.
            if cmd_name.ends_with(".wasm") {
                let wasm_path =
                    WasmBuiltins::resolve_guest_path(&sandbox_root, cmd_name, "/home/sandbox");
                if let Some(wasm_path) = wasm_path {
                    last_result = self
                        .exec_wasm_module(&wasm_path, &cmd_args, id, opts)
                        .await?;
                } else {
                    last_result = ExecResult {
                        stdout: String::new(),
                        stderr: format!("{cmd_name}: path outside sandbox or not found\n"),
                        exit_code: 1,
                    };
                }
            } else {
                // Try built-in commands.
                last_result = WasmBuiltins::execute(cmd_name, &cmd_args, &sandbox_root, &env_map);
            }

            // Handle redirects.
            if let Some(ref redir) = redirect {
                let resolved =
                    WasmBuiltins::resolve_guest_path(&sandbox_root, &redir.target, "/home/sandbox");
                if let Some(host_path) = resolved {
                    if let Some(parent) = host_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let write_result = if redir.append {
                        use std::io::Write;
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&host_path)
                            .and_then(|mut f| f.write_all(last_result.stdout.as_bytes()))
                    } else {
                        std::fs::write(&host_path, &last_result.stdout)
                    };
                    if let Err(e) = write_result {
                        last_result.stderr.push_str(&format!("redirect: {e}\n"));
                        last_result.exit_code = 1;
                    } else {
                        last_result.stdout.clear();
                    }
                } else {
                    last_result.stderr.push_str(&format!(
                        "redirect: path outside sandbox: {}\n",
                        redir.target
                    ));
                    last_result.exit_code = 1;
                }
            }
        }

        Ok(last_result)
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if self.config.home_persistence == HomePersistence::Off {
            let root = self.sandbox_root(id);
            if root.exists() {
                tokio::fs::remove_dir_all(&root).await.ok();
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WASM built-in command interpreter
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm")]
#[derive(Debug)]
enum CommandConnector {
    First,
    Sequence, // ;
    And,      // &&
    Or,       // ||
}

#[cfg(feature = "wasm")]
#[derive(Debug)]
struct CommandSegment {
    connector: CommandConnector,
    args: Vec<String>,
}

#[cfg(feature = "wasm")]
struct OutputRedirect {
    target: String,
    append: bool,
}

#[cfg(feature = "wasm")]
struct WasmBuiltins;

#[cfg(feature = "wasm")]
impl WasmBuiltins {
    /// Parse a command line into segments separated by `&&`, `||`, and `;`.
    fn parse_command_line(input: &str) -> Vec<CommandSegment> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut connector = CommandConnector::First;
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '&' && i + 1 < chars.len() && chars[i + 1] == '&' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::And;
                current.clear();
                i += 2;
                continue;
            }
            if chars[i] == '|' && i + 1 < chars.len() && chars[i + 1] == '|' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::Or;
                current.clear();
                i += 2;
                continue;
            }
            if chars[i] == ';' {
                if !current.trim().is_empty()
                    && let Ok(args) = shell_words::split(current.trim())
                {
                    segments.push(CommandSegment { connector, args });
                }
                connector = CommandConnector::Sequence;
                current.clear();
                i += 1;
                continue;
            }
            current.push(chars[i]);
            i += 1;
        }

        if !current.trim().is_empty()
            && let Ok(args) = shell_words::split(current.trim())
        {
            segments.push(CommandSegment { connector, args });
        }

        segments
    }

    /// Expand `$VAR` references in arguments.
    fn expand_vars(args: &[String], env: &HashMap<String, String>) -> Vec<String> {
        args.iter()
            .map(|arg| {
                let mut result = arg.clone();
                for (key, val) in env {
                    result = result.replace(&format!("${key}"), val);
                    result = result.replace(&format!("${{{key}}}"), val);
                }
                // Expand well-known vars.
                result = result.replace("$HOME", "/home/sandbox");
                result = result.replace("${HOME}", "/home/sandbox");
                result
            })
            .collect()
    }

    /// Extract `>` or `>>` redirect from args, returning remaining args + redirect info.
    fn extract_redirect(args: &[String]) -> (Vec<String>, Option<OutputRedirect>) {
        let mut remaining = Vec::new();
        let mut redirect = None;

        let mut i = 0;
        while i < args.len() {
            if args[i] == ">>" && i + 1 < args.len() {
                redirect = Some(OutputRedirect {
                    target: args[i + 1].clone(),
                    append: true,
                });
                i += 2;
            } else if args[i] == ">" && i + 1 < args.len() {
                redirect = Some(OutputRedirect {
                    target: args[i + 1].clone(),
                    append: false,
                });
                i += 2;
            } else if args[i].starts_with(">>") {
                redirect = Some(OutputRedirect {
                    target: args[i][2..].to_string(),
                    append: true,
                });
                i += 1;
            } else if args[i].starts_with('>') && args[i].len() > 1 {
                redirect = Some(OutputRedirect {
                    target: args[i][1..].to_string(),
                    append: false,
                });
                i += 1;
            } else {
                remaining.push(args[i].clone());
                i += 1;
            }
        }

        (remaining, redirect)
    }

    /// Resolve a guest path to a host path within the sandbox root.
    /// Returns `None` if the path escapes the sandbox.
    fn resolve_guest_path(
        sandbox_root: &std::path::Path,
        guest_path: &str,
        guest_cwd: &str,
    ) -> Option<PathBuf> {
        let logical = if guest_path.starts_with('/') {
            PathBuf::from(guest_path)
        } else {
            PathBuf::from(guest_cwd).join(guest_path)
        };

        // Map guest paths to host sandbox paths.
        let host_path = if let Ok(rest) = logical.strip_prefix("/home/sandbox") {
            sandbox_root.join("home").join(rest)
        } else if let Ok(rest) = logical.strip_prefix("/tmp") {
            sandbox_root.join("tmp").join(rest)
        } else {
            // Path outside known sandbox mounts.
            return None;
        };

        // Canonicalize the parent to check for symlink escapes.
        // The file itself may not exist yet (e.g. for write targets).
        let check_path = if host_path.exists() {
            host_path.canonicalize().ok()?
        } else if let Some(parent) = host_path.parent() {
            if parent.exists() {
                let canonical_parent = parent.canonicalize().ok()?;
                canonical_parent.join(host_path.file_name()?)
            } else {
                host_path.clone()
            }
        } else {
            host_path.clone()
        };

        let canonical_root = if sandbox_root.exists() {
            sandbox_root.canonicalize().ok()?
        } else {
            sandbox_root.to_path_buf()
        };

        if check_path.starts_with(&canonical_root) {
            Some(host_path)
        } else {
            None
        }
    }

    /// Execute a built-in command. Returns exit code 127 for unknown commands.
    fn execute(
        name: &str,
        args: &[String],
        sandbox_root: &std::path::Path,
        env: &HashMap<String, String>,
    ) -> ExecResult {
        match name {
            "echo" => Self::cmd_echo(args),
            "cat" => Self::cmd_cat(args, sandbox_root),
            "ls" => Self::cmd_ls(args, sandbox_root),
            "mkdir" => Self::cmd_mkdir(args, sandbox_root),
            "rm" => Self::cmd_rm(args, sandbox_root),
            "cp" => Self::cmd_cp(args, sandbox_root),
            "mv" => Self::cmd_mv(args, sandbox_root),
            "pwd" => ExecResult {
                stdout: "/home/sandbox\n".into(),
                stderr: String::new(),
                exit_code: 0,
            },
            "env" => Self::cmd_env(env),
            "head" => Self::cmd_head(args, sandbox_root),
            "tail" => Self::cmd_tail(args, sandbox_root),
            "wc" => Self::cmd_wc(args, sandbox_root),
            "sort" => Self::cmd_sort(args, sandbox_root),
            "touch" => Self::cmd_touch(args, sandbox_root),
            "which" => Self::cmd_which(args),
            "true" => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            "false" => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 1,
            },
            "test" | "[" => Self::cmd_test(args, sandbox_root),
            "basename" => Self::cmd_basename(args),
            "dirname" => Self::cmd_dirname(args),
            _ => ExecResult {
                stdout: String::new(),
                stderr: format!("{name}: command not found in WASM sandbox\n"),
                exit_code: 127,
            },
        }
    }

    // --- Built-in command implementations ---

    fn cmd_echo(args: &[String]) -> ExecResult {
        // Handle -n flag.
        let (no_newline, text_args) = if args.first().is_some_and(|a| a == "-n") {
            (true, &args[1..])
        } else {
            (false, args)
        };
        let text = text_args.join(" ");
        let stdout = if no_newline {
            text
        } else {
            format!("{text}\n")
        };
        ExecResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_cat(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => stdout.push_str(&content),
                    Err(e) => {
                        stderr.push_str(&format!("cat: {arg}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("cat: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_ls(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut show_long = false;
        let mut show_all = false;
        let mut paths = Vec::new();

        for arg in args {
            if arg.starts_with('-') {
                if arg.contains('l') {
                    show_long = true;
                }
                if arg.contains('a') {
                    show_all = true;
                }
            } else {
                paths.push(arg.as_str());
            }
        }

        if paths.is_empty() {
            paths.push("/home/sandbox");
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for path in &paths {
            match Self::resolve_guest_path(sandbox_root, path, "/home/sandbox") {
                Some(host_path) => {
                    if !host_path.exists() {
                        stderr.push_str(&format!("ls: {path}: No such file or directory\n"));
                        exit_code = 1;
                        continue;
                    }
                    if host_path.is_file() {
                        if let Some(name) = host_path.file_name() {
                            stdout.push_str(&format!("{}\n", name.to_string_lossy()));
                        }
                        continue;
                    }
                    match std::fs::read_dir(&host_path) {
                        Ok(entries) => {
                            let mut names: Vec<String> = entries
                                .filter_map(|e| e.ok())
                                .filter_map(|e| {
                                    let name = e.file_name().to_string_lossy().into_owned();
                                    if !show_all && name.starts_with('.') {
                                        None
                                    } else {
                                        Some(name)
                                    }
                                })
                                .collect();
                            names.sort();
                            if show_long {
                                for name in &names {
                                    let full = host_path.join(name);
                                    let meta = std::fs::metadata(&full);
                                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                                    let kind = if full.is_dir() {
                                        "d"
                                    } else {
                                        "-"
                                    };
                                    stdout.push_str(&format!("{kind}rw-r--r-- {size:>8} {name}\n"));
                                }
                            } else {
                                for name in &names {
                                    stdout.push_str(&format!("{name}\n"));
                                }
                            }
                        },
                        Err(e) => {
                            stderr.push_str(&format!("ls: {path}: {e}\n"));
                            exit_code = 1;
                        },
                    }
                },
                None => {
                    stderr.push_str(&format!("ls: {path}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_mkdir(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;
        let create_parents = args.iter().any(|a| a == "-p");

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    let result = if create_parents {
                        std::fs::create_dir_all(&path)
                    } else {
                        std::fs::create_dir(&path)
                    };
                    if let Err(e) = result {
                        stderr.push_str(&format!("mkdir: {arg}: {e}\n"));
                        exit_code = 1;
                    }
                },
                None => {
                    stderr.push_str(&format!("mkdir: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_rm(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;
        let recursive = args.iter().any(|a| a == "-r" || a == "-rf" || a == "-fr");
        let force = args.iter().any(|a| a.contains('f'));

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    if !path.exists() {
                        if !force {
                            stderr.push_str(&format!("rm: {arg}: No such file or directory\n"));
                            exit_code = 1;
                        }
                        continue;
                    }
                    let result = if path.is_dir() && recursive {
                        std::fs::remove_dir_all(&path)
                    } else if path.is_dir() {
                        stderr.push_str(&format!("rm: {arg}: is a directory\n"));
                        exit_code = 1;
                        continue;
                    } else {
                        std::fs::remove_file(&path)
                    };
                    if let Err(e) = result {
                        stderr.push_str(&format!("rm: {arg}: {e}\n"));
                        exit_code = 1;
                    }
                },
                None => {
                    stderr.push_str(&format!("rm: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_cp(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let non_flag_args: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        if non_flag_args.len() < 2 {
            return ExecResult {
                stdout: String::new(),
                stderr: "cp: missing operand\n".into(),
                exit_code: 1,
            };
        }

        let src_path = non_flag_args[0];
        let dst_path = non_flag_args[1];

        let src = match Self::resolve_guest_path(sandbox_root, src_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("cp: {src_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };
        let dst = match Self::resolve_guest_path(sandbox_root, dst_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("cp: {dst_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };

        let actual_dst = if dst.is_dir() {
            if let Some(name) = src.file_name() {
                dst.join(name)
            } else {
                dst
            }
        } else {
            dst
        };

        match std::fs::copy(&src, &actual_dst) {
            Ok(_) => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            Err(e) => ExecResult {
                stdout: String::new(),
                stderr: format!("cp: {e}\n"),
                exit_code: 1,
            },
        }
    }

    fn cmd_mv(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let non_flag_args: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        if non_flag_args.len() < 2 {
            return ExecResult {
                stdout: String::new(),
                stderr: "mv: missing operand\n".into(),
                exit_code: 1,
            };
        }

        let src_path = non_flag_args[0];
        let dst_path = non_flag_args[1];

        let src = match Self::resolve_guest_path(sandbox_root, src_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("mv: {src_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };
        let dst = match Self::resolve_guest_path(sandbox_root, dst_path, "/home/sandbox") {
            Some(p) => p,
            None => {
                return ExecResult {
                    stdout: String::new(),
                    stderr: format!("mv: {dst_path}: path outside sandbox\n"),
                    exit_code: 1,
                };
            },
        };

        let actual_dst = if dst.is_dir() {
            if let Some(name) = src.file_name() {
                dst.join(name)
            } else {
                dst
            }
        } else {
            dst
        };

        match std::fs::rename(&src, &actual_dst) {
            Ok(()) => ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            },
            Err(e) => ExecResult {
                stdout: String::new(),
                stderr: format!("mv: {e}\n"),
                exit_code: 1,
            },
        }
    }

    fn cmd_env(env: &HashMap<String, String>) -> ExecResult {
        let mut stdout = String::new();
        stdout.push_str("PATH=/usr/local/bin:/usr/bin:/bin\n");
        stdout.push_str("HOME=/home/sandbox\n");
        stdout.push_str("LANG=C.UTF-8\n");
        let mut keys: Vec<&String> = env.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(val) = env.get(key) {
                stdout.push_str(&format!("{key}={val}\n"));
            }
        }
        ExecResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_head(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut lines = 10usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            if args[i] == "-n" && i + 1 < args.len() {
                lines = args[i + 1].parse().unwrap_or(10);
                i += 2;
            } else if args[i].starts_with('-') && args[i][1..].parse::<usize>().is_ok() {
                lines = args[i][1..].parse().unwrap_or(10);
                i += 1;
            } else {
                files.push(&args[i]);
                i += 1;
            }
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        for line in content.lines().take(lines) {
                            stdout.push_str(line);
                            stdout.push('\n');
                        }
                    },
                    Err(e) => {
                        stderr.push_str(&format!("head: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("head: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_tail(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut lines = 10usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            if args[i] == "-n" && i + 1 < args.len() {
                lines = args[i + 1].parse().unwrap_or(10);
                i += 2;
            } else if args[i].starts_with('-') && args[i][1..].parse::<usize>().is_ok() {
                lines = args[i][1..].parse().unwrap_or(10);
                i += 1;
            } else {
                files.push(&args[i]);
                i += 1;
            }
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        for line in &all_lines[start..] {
                            stdout.push_str(line);
                            stdout.push('\n');
                        }
                    },
                    Err(e) => {
                        stderr.push_str(&format!("tail: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("tail: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_wc(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let line_count = content.lines().count();
                        let word_count = content.split_whitespace().count();
                        let byte_count = content.len();
                        stdout.push_str(&format!(
                            "{line_count:>8} {word_count:>8} {byte_count:>8} {file}\n"
                        ));
                    },
                    Err(e) => {
                        stderr.push_str(&format!("wc: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("wc: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_sort(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
        let reverse = args.iter().any(|a| a == "-r");
        let mut all_lines = Vec::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for file in &files {
            match Self::resolve_guest_path(sandbox_root, file, "/home/sandbox") {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        all_lines.extend(content.lines().map(ToOwned::to_owned));
                    },
                    Err(e) => {
                        stderr.push_str(&format!("sort: {file}: {e}\n"));
                        exit_code = 1;
                    },
                },
                None => {
                    stderr.push_str(&format!("sort: {file}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        all_lines.sort();
        if reverse {
            all_lines.reverse();
        }

        let mut stdout = String::new();
        for line in &all_lines {
            stdout.push_str(line);
            stdout.push('\n');
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_touch(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args.iter().filter(|a| !a.starts_with('-')) {
            match Self::resolve_guest_path(sandbox_root, arg, "/home/sandbox") {
                Some(path) => {
                    if !path.exists() {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::write(&path, "") {
                            stderr.push_str(&format!("touch: {arg}: {e}\n"));
                            exit_code = 1;
                        }
                    }
                    // If file exists, we'd update mtime but that's not critical.
                },
                None => {
                    stderr.push_str(&format!("touch: {arg}: path outside sandbox\n"));
                    exit_code = 1;
                },
            }
        }

        ExecResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }

    fn cmd_which(args: &[String]) -> ExecResult {
        let builtins = [
            "echo", "cat", "ls", "mkdir", "rm", "cp", "mv", "pwd", "env", "head", "tail", "wc",
            "sort", "touch", "which", "true", "false", "test", "[", "basename", "dirname",
        ];
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;

        for arg in args {
            if builtins.contains(&arg.as_str()) {
                stdout.push_str(&format!("{arg}: WASM sandbox built-in\n"));
            } else {
                stderr.push_str(&format!("{arg} not found\n"));
                exit_code = 1;
            }
        }

        ExecResult {
            stdout,
            stderr,
            exit_code,
        }
    }

    fn cmd_test(args: &[String], sandbox_root: &std::path::Path) -> ExecResult {
        // Strip trailing ] if present (for [ ... ] syntax).
        let args: Vec<&String> = if args.last().is_some_and(|a| a == "]") {
            args[..args.len() - 1].iter().collect()
        } else {
            args.iter().collect()
        };

        let result = match args.len() {
            0 => false,
            1 => !args[0].is_empty(),
            2 => {
                let op = args[0].as_str();
                let operand = args[1].as_str();
                match op {
                    "-f" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.is_file()),
                    "-d" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.is_dir()),
                    "-e" => Self::resolve_guest_path(sandbox_root, operand, "/home/sandbox")
                        .is_some_and(|p| p.exists()),
                    "-z" => operand.is_empty(),
                    "-n" => !operand.is_empty(),
                    _ => false,
                }
            },
            3 => {
                let left = args[0].as_str();
                let op = args[1].as_str();
                let right = args[2].as_str();
                match op {
                    "=" | "==" => left == right,
                    "!=" => left != right,
                    "-eq" => left.parse::<i64>().ok() == right.parse::<i64>().ok(),
                    "-ne" => left.parse::<i64>().ok() != right.parse::<i64>().ok(),
                    "-lt" => left
                        .parse::<i64>()
                        .ok()
                        .zip(right.parse::<i64>().ok())
                        .is_some_and(|(l, r)| l < r),
                    "-gt" => left
                        .parse::<i64>()
                        .ok()
                        .zip(right.parse::<i64>().ok())
                        .is_some_and(|(l, r)| l > r),
                    _ => false,
                }
            },
            _ => false,
        };

        ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: if result {
                0
            } else {
                1
            },
        }
    }

    fn cmd_basename(args: &[String]) -> ExecResult {
        if args.is_empty() {
            return ExecResult {
                stdout: String::new(),
                stderr: "basename: missing operand\n".into(),
                exit_code: 1,
            };
        }
        let path = std::path::Path::new(&args[0]);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ExecResult {
            stdout: format!("{name}\n"),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn cmd_dirname(args: &[String]) -> ExecResult {
        if args.is_empty() {
            return ExecResult {
                stdout: String::new(),
                stderr: "dirname: missing operand\n".into(),
                exit_code: 1,
            };
        }
        let path = std::path::Path::new(&args[0]);
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".into());
        ExecResult {
            stdout: format!("{parent}\n"),
            stderr: String::new(),
            exit_code: 0,
        }
    }
}

/// Apple Container sandbox using the `container` CLI (macOS 26+, Apple Silicon).
#[cfg(target_os = "macos")]
pub struct AppleContainerSandbox {
    pub config: SandboxConfig,
    name_generations: RwLock<HashMap<String, u32>>,
}

#[cfg(target_os = "macos")]
impl AppleContainerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            name_generations: RwLock::new(HashMap::new()),
        }
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

    async fn container_name(&self, id: &SandboxId) -> String {
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

    async fn bump_container_generation(&self, id: &SandboxId) -> String {
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
        let Some(host_dir) = ensure_sandbox_home_persistence_host_dir(&self.config, id)? else {
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
                            anyhow::bail!("container {name} failed to stay running after startup");
                        },
                        _ => {},
                    }

                    // `container run -d` can return before inspect status flips to
                    // "running". Keep polling briefly before we declare failure.
                    if attempt + 1 < MAX_WAIT_ITERS {
                        tokio::time::sleep(std::time::Duration::from_millis(WAIT_MS)).await;
                        continue;
                    }

                    anyhow::bail!("container {name} did not report running state after startup");
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
                    anyhow::bail!(
                        "container inspect failed for {name} while waiting for running state: {}",
                        stderr.trim()
                    );
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

        anyhow::bail!("container {name} did not become running after startup")
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
        anyhow::bail!(
            "container {name} failed exec readiness probe: {}",
            stderr.trim()
        );
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

        anyhow::bail!("container {name} did not become exec-ready after startup")
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
    ) -> Result<(), CreateError> {
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
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerState {
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
fn ensure_apple_container_service() -> bool {
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

fn is_apple_container_service_error(stderr: &str) -> bool {
    stderr.contains("XPC connection error") || stderr.contains("Connection invalid")
}

fn is_apple_container_exists_error(stderr: &str) -> bool {
    stderr.contains("already exists") || stderr.contains("exists: \"container with id")
}

#[cfg(any(target_os = "macos", test))]
fn is_apple_container_unavailable_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("cannot exec: container is not running")
        || lower.contains("container is not running")
        || (lower.contains("container") && lower.contains("is not running"))
        || lower.contains("container is stopped")
        || lower.contains("no sandbox client exists")
        || lower.contains("notfound")
        || (lower.contains("not found") && lower.contains("container"))
}

#[cfg(target_os = "macos")]
fn should_restart_after_readiness_error(error_text: &str, state: ContainerState) -> bool {
    is_apple_container_unavailable_error(error_text) && state == ContainerState::Stopped
}

fn apple_container_status_from_inspect(stdout: &str) -> Option<&'static str> {
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

/// The Apple Container daemon process is alive but its internal
/// Virtualization.framework state is stale (e.g. after an interrupted
/// macOS restart/sleep). Containers are created but immediately exit
/// with `NSPOSIXErrorDomain Code=22 "Invalid argument"`.
/// Restarting the daemon (`container system stop && container system start`)
/// is the only fix — retrying will never help.
fn is_apple_container_daemon_stale_error(text: &str) -> bool {
    // Both patterns are required — `NSPOSIXErrorDomain` alone can appear in
    // benign log-fetching errors (Code=2 "No such file or directory") when a
    // container vanishes. The stale-daemon signature is specifically EINVAL:
    // `NSPOSIXErrorDomain Code=22 "Invalid argument"`.
    text.contains("NSPOSIXErrorDomain") && text.contains("Invalid argument")
}

/// Returns `true` when a freshly created container stopped immediately and
/// produced no meaningful logs. This indicates the VM never fully booted —
/// a broader symptom than the specific daemon-stale EINVAL signature. It can
/// occur after macOS sleep/wake cycles, resource exhaustion, or
/// Virtualization.framework glitches. The appropriate recovery is a full
/// service restart, same as for daemon-stale errors.
#[cfg(any(target_os = "macos", test))]
fn is_apple_container_boot_failure(logs: Option<&str>) -> bool {
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

fn is_apple_container_corruption_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    is_apple_container_service_error(stderr)
        || is_apple_container_exists_error(stderr)
        || is_apple_container_daemon_stale_error(stderr)
        || lower.contains("failed to bootstrap container")
        || lower.contains("config.json")
        || lower.contains("vm never booted")
}

/// Wrapper sandbox that can fail over from a primary backend to a fallback backend.
///
/// This is used on macOS to fail over from Apple Container to Docker when the
/// Apple runtime enters a corrupted state (stale metadata, missing config.json,
/// service errors, etc.).
pub struct FailoverSandbox {
    primary: Arc<dyn Sandbox>,
    fallback: Arc<dyn Sandbox>,
    primary_name: &'static str,
    fallback_name: &'static str,
    use_fallback: RwLock<bool>,
}

impl FailoverSandbox {
    pub fn new(primary: Arc<dyn Sandbox>, fallback: Arc<dyn Sandbox>) -> Self {
        let primary_name = primary.backend_name();
        let fallback_name = fallback.backend_name();
        Self {
            primary,
            fallback,
            primary_name,
            fallback_name,
            use_fallback: RwLock::new(false),
        }
    }

    async fn fallback_enabled(&self) -> bool {
        *self.use_fallback.read().await
    }

    async fn switch_to_fallback(&self, error: &anyhow::Error) {
        let mut use_fallback = self.use_fallback.write().await;
        if !*use_fallback {
            warn!(
                primary = self.primary_name,
                fallback = self.fallback_name,
                %error,
                "sandbox primary backend failed, switching to fallback backend"
            );
            *use_fallback = true;
        }
    }

    fn should_failover(&self, error: &anyhow::Error) -> bool {
        let message = format!("{error:#}");
        match self.primary_name {
            "apple-container" => is_apple_container_corruption_error(&message),
            "docker" => is_docker_failover_error(&message),
            _ => false,
        }
    }
}

#[async_trait]
impl Sandbox for FailoverSandbox {
    fn backend_name(&self) -> &'static str {
        self.primary_name
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        if self.fallback_enabled().await {
            return self.fallback.ensure_ready(id, image_override).await;
        }

        match self.primary.ensure_ready(id, image_override).await {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                let primary_message = format!("{primary_error:#}");
                self.fallback
                    .ensure_ready(id, image_override)
                    .await
                    .map_err(|fallback_error| {
                        anyhow::anyhow!(
                            "primary sandbox backend ({}) failed: {}; fallback backend ({}) also failed: {}",
                            self.primary_name,
                            primary_message,
                            self.fallback_name,
                            fallback_error
                        )
                    })
            },
        }
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        if self.fallback_enabled().await {
            return self.fallback.exec(id, command, opts).await;
        }

        match self.primary.exec(id, command, opts).await {
            Ok(result) => Ok(result),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                let primary_message = format!("{primary_error:#}");
                self.fallback
                    .ensure_ready(id, None)
                    .await
                    .map_err(|fallback_error| {
                        anyhow::anyhow!(
                            "primary sandbox backend ({}) failed during exec: {}; fallback backend ({}) failed to initialize: {}",
                            self.primary_name,
                            primary_message,
                            self.fallback_name,
                            fallback_error
                        )
                    })?;
                self.fallback.exec(id, command, opts).await
            },
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if self.fallback_enabled().await {
            let result = self.fallback.cleanup(id).await;
            if let Err(error) = self.primary.cleanup(id).await {
                debug!(
                    backend = self.primary_name,
                    %error,
                    "primary sandbox cleanup failed after failover"
                );
            }
            return result;
        }

        self.primary.cleanup(id).await
    }

    async fn build_image(
        &self,
        base: &str,
        packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        if self.fallback_enabled().await {
            return self.fallback.build_image(base, packages).await;
        }

        match self.primary.build_image(base, packages).await {
            Ok(result) => Ok(result),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                self.fallback.build_image(base, packages).await
            },
        }
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
                    anyhow::bail!(
                        "apple container service is not running. \
                         Start it with `container system start` and restart moltis"
                    );
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
                        anyhow::bail!(
                            "apple container daemon has stale Virtualization.framework state \
                             and automatic restart failed (create error: {stderr}). \
                             Restart manually with `container system stop && container system start`"
                        );
                    }
                    if is_last {
                        let diag = Self::diagnose_container_failure(&name).await;
                        anyhow::bail!(
                            "container run failed for {name} (image={image}): {stderr}; diagnostics: {diag}"
                        );
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
                        anyhow::bail!(
                            "apple container daemon has stale Virtualization.framework state \
                             and automatic restart failed (container logs: {log_text}). \
                             Restart manually with `container system stop && container system start`"
                        );
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
                        anyhow::bail!(
                            "apple container {name} did not become exec-ready{boot_note}: \
                             {error:#}; diagnostics: {diag}"
                        );
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
        anyhow::bail!(
            "apple container {name} failed after {MAX_ATTEMPTS} attempts; diagnostics: {diag}"
        );
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let name = self.container_name(id).await;
        info!(name, command, "apple container exec");

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
                anyhow::bail!(
                    "apple container service is not running. \
                     Start it with `container system start` and restart moltis"
                );
            }
            anyhow::bail!("container build failed for {tag}: {}", stderr.trim());
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
        "apple-container" => {
            if !ensure_apple_container_service() {
                tracing::warn!(
                    "apple container service could not be started; \
                     run `container system start` manually, then restart moltis"
                );
            }
            let apple_backend: Arc<dyn Sandbox> =
                Arc::new(AppleContainerSandbox::new(config.clone()));
            maybe_wrap_with_failover(apple_backend, &config)
        },
        "restricted-host" => {
            tracing::info!("sandbox backend: restricted-host (env clearing, rlimits)");
            Arc::new(RestrictedHostSandbox::new(config))
        },
        "wasm" | "wasmtime" => create_wasm_backend(config),
        _ => auto_detect_backend(config),
    }
}

/// Create a WASM sandbox backend, falling back to `RestrictedHostSandbox` if
/// the feature is disabled or initialisation fails.
fn create_wasm_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    #[cfg(feature = "wasm")]
    {
        match WasmSandbox::new(config.clone()) {
            Ok(sandbox) => {
                tracing::info!("sandbox backend: wasm (WASI-isolated execution)");
                Arc::new(sandbox)
            },
            Err(e) => {
                tracing::warn!(%e, "failed to initialize wasmtime engine, falling back to restricted-host");
                Arc::new(RestrictedHostSandbox::new(config))
            },
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        tracing::warn!("wasm sandbox requested but feature not compiled in; using restricted-host");
        Arc::new(RestrictedHostSandbox::new(config))
    }
}

/// Wrap a primary sandbox backend with a failover chain.
///
/// Tries Docker first as fallback, then WASM, returning the primary unwrapped
/// if no fallback runtime is available.
fn maybe_wrap_with_failover(primary: Arc<dyn Sandbox>, config: &SandboxConfig) -> Arc<dyn Sandbox> {
    // Try Docker as fallback first.
    if should_use_docker_backend(is_cli_available("docker"), is_docker_daemon_available()) {
        tracing::info!(
            primary = primary.backend_name(),
            fallback = "docker",
            "sandbox backend failover enabled"
        );
        return Arc::new(FailoverSandbox::new(
            primary,
            Arc::new(DockerSandbox::new(config.clone())),
        ));
    }

    // Use restricted-host as fallback if Docker is unavailable.
    tracing::info!(
        primary = primary.backend_name(),
        fallback = "restricted-host",
        "sandbox backend failover enabled (restricted-host)"
    );
    Arc::new(FailoverSandbox::new(
        primary,
        Arc::new(RestrictedHostSandbox::new(config.clone())),
    ))
}

/// Check whether an error message indicates a Docker daemon connectivity issue.
fn is_docker_failover_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("cannot connect to the docker daemon")
        || lower.contains("is the docker daemon running")
        || lower.contains("error during connect")
        || lower.contains("connection refused")
}

fn auto_detect_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    #[cfg(target_os = "macos")]
    {
        if is_cli_available("container") {
            if ensure_apple_container_service() {
                tracing::info!("sandbox backend: apple-container (VM-isolated, preferred)");
                let apple_backend: Arc<dyn Sandbox> =
                    Arc::new(AppleContainerSandbox::new(config.clone()));
                return maybe_wrap_with_failover(apple_backend, &config);
            }
            tracing::warn!(
                "apple container CLI found but service could not be started; \
                 falling back to docker"
            );
        }
    }

    if should_use_docker_backend(is_cli_available("docker"), is_docker_daemon_available()) {
        tracing::info!("sandbox backend: docker");
        return Arc::new(DockerSandbox::new(config));
    }

    if is_cli_available("docker") {
        tracing::warn!(
            "docker CLI detected but daemon is not accessible; \
             falling back to restricted-host sandbox"
        );
    }

    // Use restricted-host sandbox before falling back to NoSandbox.
    tracing::info!(
        "sandbox backend: restricted-host (env clearing, rlimits; no container runtime available)"
    );
    Arc::new(RestrictedHostSandbox::new(config))
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
    /// First-run container/image setup is about to begin for a session.
    Preparing {
        session_key: String,
        backend: String,
        image: String,
    },
    /// First-run container/image setup completed for a session.
    Prepared {
        session_key: String,
        backend: String,
        image: String,
    },
    /// First-run container/image setup failed for a session.
    PrepareFailed {
        session_key: String,
        backend: String,
        image: String,
        error: String,
    },
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
    /// Event channel for sandbox lifecycle events (prepare/provision/build feedback).
    event_tx: tokio::sync::broadcast::Sender<SandboxEvent>,
    /// Session keys that have already completed sandbox initialization.
    /// Used to avoid repeating first-run preparation banners on every command.
    prepared_sessions: RwLock<HashSet<String>>,
    /// Whether a sandbox image pre-build is currently in progress.
    /// Used by the gateway to show a banner in the UI.
    pub building_flag: std::sync::atomic::AtomicBool,
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
            prepared_sessions: RwLock::new(HashSet::new()),
            building_flag: std::sync::atomic::AtomicBool::new(false),
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
            prepared_sessions: RwLock::new(HashSet::new()),
            building_flag: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Subscribe to sandbox lifecycle events.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<SandboxEvent> {
        self.event_tx.subscribe()
    }

    /// Emit a sandbox event. Silently drops if no subscribers.
    pub fn emit_event(&self, event: SandboxEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Mark a session as preparing for sandbox first-run work.
    /// Returns `true` only the first time for a session key.
    pub async fn mark_preparing_once(&self, session_key: &str) -> bool {
        self.prepared_sessions
            .write()
            .await
            .insert(session_key.to_string())
    }

    /// Clear preparation marker for a session (used on cleanup or prepare failure).
    pub async fn clear_prepared_session(&self, session_key: &str) {
        self.prepared_sessions.write().await.remove(session_key);
    }

    /// Check whether a session should run sandboxed.
    /// Returns `false` when no real container runtime is available, regardless of
    /// config mode or per-session overrides. Otherwise, per-session override takes
    /// priority, then falls back to global mode.
    pub async fn is_sandboxed(&self, session_key: &str) -> bool {
        if !self.backend.is_real() {
            return false;
        }
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
        self.clear_prepared_session(session_key).await;
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct TestSandbox {
        name: &'static str,
        ensure_ready_error: Option<String>,
        exec_error: Option<String>,
        ensure_ready_calls: AtomicUsize,
        exec_calls: AtomicUsize,
        cleanup_calls: AtomicUsize,
    }

    impl TestSandbox {
        fn new(
            name: &'static str,
            ensure_ready_error: Option<&str>,
            exec_error: Option<&str>,
        ) -> Self {
            Self {
                name,
                ensure_ready_error: ensure_ready_error.map(ToOwned::to_owned),
                exec_error: exec_error.map(ToOwned::to_owned),
                ensure_ready_calls: AtomicUsize::new(0),
                exec_calls: AtomicUsize::new(0),
                cleanup_calls: AtomicUsize::new(0),
            }
        }

        fn ensure_ready_calls(&self) -> usize {
            self.ensure_ready_calls.load(Ordering::SeqCst)
        }

        fn exec_calls(&self) -> usize {
            self.exec_calls.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn truncate_output_for_display_handles_multibyte_boundary() {
        let mut output = format!("{}л{}", "a".repeat(1999), "z".repeat(10));
        truncate_output_for_display(&mut output, 2000);
        assert!(output.contains("[output truncated]"));
        assert!(!output.contains('л'));
    }

    #[async_trait::async_trait]
    impl Sandbox for TestSandbox {
        fn backend_name(&self) -> &'static str {
            self.name
        }

        async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
            self.ensure_ready_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(ref error) = self.ensure_ready_error {
                anyhow::bail!("{error}");
            }
            Ok(())
        }

        async fn exec(
            &self,
            _id: &SandboxId,
            _command: &str,
            _opts: &ExecOpts,
        ) -> Result<ExecResult> {
            self.exec_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(ref error) = self.exec_error {
                anyhow::bail!("{error}");
            }
            Ok(ExecResult {
                stdout: "ok".into(),
                stderr: String::new(),
                exit_code: 0,
            })
        }

        async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
            self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

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
    fn test_docker_hardening_args_prebuilt() {
        let args = DockerSandbox::hardening_args(true);
        assert!(args.contains(&"--cap-drop".to_string()));
        assert!(args.contains(&"ALL".to_string()));
        assert!(args.contains(&"--security-opt".to_string()));
        assert!(args.contains(&"no-new-privileges".to_string()));
        assert!(args.contains(&"--read-only".to_string()));
        // Verify tmpfs mounts are present
        assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
        assert!(args.contains(&"/run:rw,nosuid,size=64m".to_string()));
    }

    #[test]
    fn test_docker_hardening_args_not_prebuilt() {
        let args = DockerSandbox::hardening_args(false);
        assert!(args.contains(&"--cap-drop".to_string()));
        assert!(args.contains(&"ALL".to_string()));
        assert!(args.contains(&"--security-opt".to_string()));
        assert!(args.contains(&"no-new-privileges".to_string()));
        // --read-only must NOT be present for non-prebuilt (needs apt-get)
        assert!(!args.contains(&"--read-only".to_string()));
        // tmpfs mounts still present
        assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    }

    #[test]
    fn test_workspace_mount_display() {
        assert_eq!(WorkspaceMount::None.to_string(), "none");
        assert_eq!(WorkspaceMount::Ro.to_string(), "ro");
        assert_eq!(WorkspaceMount::Rw.to_string(), "rw");
    }

    #[test]
    fn test_home_persistence_display() {
        assert_eq!(HomePersistence::Off.to_string(), "off");
        assert_eq!(HomePersistence::Session.to_string(), "session");
        assert_eq!(HomePersistence::Shared.to_string(), "shared");
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
    fn test_docker_home_persistence_args_off() {
        let config = SandboxConfig {
            home_persistence: HomePersistence::Off,
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess-1".into(),
        };
        assert!(docker.home_persistence_args(&id).unwrap().is_empty());
    }

    #[test]
    fn test_docker_home_persistence_args_default_shared() {
        let config = SandboxConfig::default();
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess-1".into(),
        };
        let args = docker.home_persistence_args(&id).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-v");
        let expected_host_dir = moltis_config::data_dir()
            .join("sandbox")
            .join("home")
            .join("shared");
        let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
        assert_eq!(args[1], expected_volume);
    }

    #[test]
    fn test_docker_home_persistence_args_custom_shared_absolute_path() {
        let config = SandboxConfig {
            shared_home_dir: Some(PathBuf::from("/tmp/moltis-shared-home")),
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess-1".into(),
        };
        let args = docker.home_persistence_args(&id).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-v");
        let expected_volume = "/tmp/moltis-shared-home:/home/sandbox:rw".to_string();
        assert_eq!(args[1], expected_volume);
    }

    #[test]
    fn test_docker_home_persistence_args_custom_shared_relative_path() {
        let config = SandboxConfig {
            shared_home_dir: Some(PathBuf::from("sandbox/custom-shared")),
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess-1".into(),
        };
        let args = docker.home_persistence_args(&id).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-v");
        let expected_host_dir = moltis_config::data_dir().join("sandbox/custom-shared");
        let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
        assert_eq!(args[1], expected_volume);
    }

    #[test]
    fn test_docker_home_persistence_args_session() {
        let config = SandboxConfig {
            home_persistence: HomePersistence::Session,
            ..Default::default()
        };
        let docker = DockerSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess:/weird key".into(),
        };
        let args = docker.home_persistence_args(&id).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-v");
        let expected_host_dir = moltis_config::data_dir()
            .join("sandbox")
            .join("home")
            .join("session")
            .join("sess--weird-key");
        let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
        assert_eq!(args[1], expected_volume);
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

    /// Helper: build a `SandboxRouter` with a deterministic backend so tests
    /// don't depend on the host having Docker / Apple Container installed.
    fn router_with_real_backend(config: SandboxConfig) -> SandboxRouter {
        let backend: Arc<dyn Sandbox> = Arc::new(TestSandbox::new("docker", None, None));
        SandboxRouter::with_backend(config, backend)
    }

    #[tokio::test]
    async fn test_sandbox_router_default_all() {
        let config = SandboxConfig::default(); // mode = All
        let router = router_with_real_backend(config);
        assert!(router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_off() {
        let config = SandboxConfig {
            mode: SandboxMode::Off,
            ..Default::default()
        };
        let router = router_with_real_backend(config);
        assert!(!router.is_sandboxed("main").await);
        assert!(!router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_all() {
        let config = SandboxConfig {
            mode: SandboxMode::All,
            ..Default::default()
        };
        let router = router_with_real_backend(config);
        assert!(router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_mode_non_main() {
        let config = SandboxConfig {
            mode: SandboxMode::NonMain,
            ..Default::default()
        };
        let router = router_with_real_backend(config);
        assert!(!router.is_sandboxed("main").await);
        assert!(router.is_sandboxed("session:abc").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_override() {
        let config = SandboxConfig {
            mode: SandboxMode::Off,
            ..Default::default()
        };
        let router = router_with_real_backend(config);
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
        let router = router_with_real_backend(config);
        assert!(router.is_sandboxed("main").await);

        // Override to disable sandbox for main
        router.set_override("main", false).await;
        assert!(!router.is_sandboxed("main").await);
    }

    #[tokio::test]
    async fn test_sandbox_router_no_runtime_returns_false() {
        let backend: Arc<dyn Sandbox> = Arc::new(NoSandbox);
        let config = SandboxConfig {
            mode: SandboxMode::All,
            ..Default::default()
        };
        let router = SandboxRouter::with_backend(config, backend);

        // Even with mode=All, no runtime means not sandboxed
        assert!(!router.is_sandboxed("main").await);
        assert!(!router.is_sandboxed("session:abc").await);

        // Overrides are also ignored when there's no runtime
        router.set_override("main", true).await;
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
        let tag1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
        let tag2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
        assert_eq!(tag1, tag2);
        assert!(tag1.starts_with("moltis-main-sandbox:"));
    }

    #[test]
    fn test_docker_image_tag_order_independent() {
        let p1 = vec!["curl".into(), "git".into()];
        let p2 = vec!["git".into(), "curl".into()];
        assert_eq!(
            sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1),
            sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2),
        );
    }

    #[test]
    fn test_docker_image_tag_normalizes_whitespace_and_duplicates() {
        let p1 = vec!["curl".into(), "git".into(), "curl".into()];
        let p2 = vec![" git ".into(), "curl".into()];
        assert_eq!(
            sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1),
            sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2),
        );
    }

    #[test]
    fn test_sandbox_image_dockerfile_creates_home_in_install_layer() {
        let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into()]);
        assert!(dockerfile.contains(
            "RUN apt-get update -qq && apt-get install -y -qq curl && mkdir -p /home/sandbox"
        ));
        assert!(!dockerfile.contains("RUN mkdir -p /home/sandbox\n"));
    }

    #[test]
    fn test_sandbox_image_dockerfile_installs_gogcli() {
        let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into()]);
        assert!(dockerfile.contains(&format!("go install {GOGCLI_MODULE_PATH}@{GOGCLI_VERSION}")));
        assert!(dockerfile.contains("ln -sf /usr/local/bin/gog /usr/local/bin/gogcli"));
    }

    #[test]
    fn test_docker_image_tag_changes_with_base() {
        let packages = vec!["curl".into()];
        let t1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
        let t2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:24.04", &packages);
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_docker_image_tag_changes_with_packages() {
        let p1 = vec!["curl".into()];
        let p2 = vec!["curl".into(), "git".into()];
        let t1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1);
        let t2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2);
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_rebuildable_sandbox_image_tag_requires_packages() {
        let tag = rebuildable_sandbox_image_tag(
            "moltis-main-sandbox:deadbeef",
            "moltis-main-sandbox",
            "ubuntu:25.10",
            &[],
        );
        assert!(tag.is_none());
    }

    #[test]
    fn test_rebuildable_sandbox_image_tag_requires_local_repo_prefix() {
        let tag = rebuildable_sandbox_image_tag(
            "ubuntu:25.10",
            "moltis-main-sandbox",
            "ubuntu:25.10",
            &["curl".into()],
        );
        assert!(tag.is_none());
    }

    #[test]
    fn test_rebuildable_sandbox_image_tag_returns_deterministic_tag() {
        let packages = vec!["curl".into(), "git".into()];
        let tag = rebuildable_sandbox_image_tag(
            "moltis-main-sandbox:oldtag",
            "moltis-main-sandbox",
            "ubuntu:25.10",
            &packages,
        );
        assert_eq!(
            tag,
            Some(sandbox_image_tag(
                "moltis-main-sandbox",
                "ubuntu:25.10",
                &packages
            ))
        );
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

        assert!(router.mark_preparing_once("main").await);
        assert!(!router.mark_preparing_once("main").await);
        router.clear_prepared_session("main").await;
        assert!(router.mark_preparing_once("main").await);
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

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_apple_container_name_generation_rotation() {
        let sandbox = AppleContainerSandbox::new(SandboxConfig::default());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-abc".into(),
        };

        let first_name = sandbox.container_name(&id).await;
        assert_eq!(first_name, "moltis-sandbox-session-abc");

        let rotated_name = sandbox.bump_container_generation(&id).await;
        assert_eq!(rotated_name, "moltis-sandbox-session-abc-g1");

        let current_name = sandbox.container_name(&id).await;
        assert_eq!(current_name, "moltis-sandbox-session-abc-g1");
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
    fn test_is_apple_container_service_error() {
        assert!(is_apple_container_service_error(
            "Error: internalError: \"XPC connection error\""
        ));
        assert!(is_apple_container_service_error(
            "Error: Connection invalid while contacting service"
        ));
        assert!(!is_apple_container_service_error(
            "Error: something else happened"
        ));
    }

    #[test]
    fn test_is_apple_container_exists_error() {
        assert!(is_apple_container_exists_error(
            "Error: exists: \"container with id moltis-sandbox-main already exists\""
        ));
        assert!(is_apple_container_exists_error(
            "Error: container already exists"
        ));
        assert!(!is_apple_container_exists_error("Error: no such container"));
    }

    #[test]
    fn test_is_apple_container_unavailable_error() {
        assert!(is_apple_container_unavailable_error(
            "cannot exec: container is not running"
        ));
        assert!(is_apple_container_unavailable_error(
            "invalidState: \"container xyz is not running\""
        ));
        assert!(is_apple_container_unavailable_error(
            "invalidState: \"no sandbox client exists: container is stopped\""
        ));
        // notFound errors from get/inspect failures
        assert!(is_apple_container_unavailable_error(
            "Error: notFound: \"get failed: container moltis-sandbox-main not found\""
        ));
        assert!(is_apple_container_unavailable_error(
            "container not found: moltis-sandbox-session-abc"
        ));
        assert!(!is_apple_container_unavailable_error("permission denied"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_should_restart_after_readiness_error() {
        assert!(should_restart_after_readiness_error(
            "cannot exec: container is not running",
            ContainerState::Stopped
        ));
        assert!(!should_restart_after_readiness_error(
            "cannot exec: container is not running",
            ContainerState::Running
        ));
        assert!(!should_restart_after_readiness_error(
            "permission denied",
            ContainerState::Stopped
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_apple_container_run_args_pin_workdir_and_bootstrap_home() {
        let args =
            apple_container_run_args("moltis-sandbox-test", "ubuntu:25.10", Some("UTC"), None);
        let expected = vec![
            "run",
            "-d",
            "--name",
            "moltis-sandbox-test",
            "--workdir",
            "/tmp",
            "-e",
            "TZ=UTC",
            "ubuntu:25.10",
            "sh",
            "-c",
            "mkdir -p /home/sandbox && exec sleep infinity",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(args, expected);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_apple_container_run_args_with_home_volume() {
        let args = apple_container_run_args(
            "moltis-sandbox-test",
            "ubuntu:25.10",
            Some("UTC"),
            Some("/tmp/home:/home/sandbox"),
        );
        let expected = vec![
            "run",
            "-d",
            "--name",
            "moltis-sandbox-test",
            "--workdir",
            "/tmp",
            "-e",
            "TZ=UTC",
            "--volume",
            "/tmp/home:/home/sandbox",
            "ubuntu:25.10",
            "sh",
            "-c",
            "mkdir -p /home/sandbox && exec sleep infinity",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(args, expected);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_apple_container_exec_args_pin_workdir() {
        let args = apple_container_exec_args("moltis-sandbox-test", "true".to_string());
        let expected = vec![
            "exec",
            "--workdir",
            "/tmp",
            "moltis-sandbox-test",
            "sh",
            "-c",
            "true",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(args, expected);
    }

    #[test]
    fn test_apple_container_status_from_inspect() {
        assert_eq!(
            apple_container_status_from_inspect(
                r#"[{"id":"abc","status":"running","configuration":{}}]"#
            ),
            Some("running")
        );
        assert_eq!(
            apple_container_status_from_inspect(r#"[{"id":"abc","status":"stopped"}]"#),
            Some("stopped")
        );
        assert_eq!(apple_container_status_from_inspect("[]"), None);
        assert_eq!(apple_container_status_from_inspect(""), None);
    }

    #[test]
    fn test_is_apple_container_daemon_stale_error() {
        // Full EINVAL pattern from container logs
        assert!(is_apple_container_daemon_stale_error(
            "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\""
        ));
        // Both patterns required — neither alone should match
        assert!(!is_apple_container_daemon_stale_error(
            "NSPOSIXErrorDomain Code=22"
        ));
        assert!(!is_apple_container_daemon_stale_error("Invalid argument"));
        // Log-fetching errors with NSPOSIXErrorDomain Code=2 must NOT match
        assert!(!is_apple_container_daemon_stale_error(
            "Error Domain=NSPOSIXErrorDomain Code=2 \"No such file or directory\""
        ));
        assert!(!is_apple_container_daemon_stale_error(
            "container is not running"
        ));
        assert!(!is_apple_container_daemon_stale_error("permission denied"));
    }

    #[test]
    fn test_is_apple_container_boot_failure() {
        // No logs at all — VM never booted
        assert!(is_apple_container_boot_failure(None));
        // Empty logs
        assert!(is_apple_container_boot_failure(Some("")));
        assert!(is_apple_container_boot_failure(Some("  \n  ")));
        // stdio.log doesn't exist — VM never produced output
        assert!(is_apple_container_boot_failure(Some(
            r#"Error: invalidArgument: "failed to fetch container logs: internalError: "failed to open container logs: Error Domain=NSCocoaErrorDomain Code=4 "The file "stdio.log" doesn't exist."""#
        )));
        // Real logs present — not a boot failure
        assert!(!is_apple_container_boot_failure(Some(
            "sleep: invalid time interval 'infinity'"
        )));
        // Daemon-stale EINVAL is NOT a boot failure (different handler)
        assert!(!is_apple_container_boot_failure(Some(
            "Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\""
        )));
    }

    #[test]
    fn test_is_apple_container_corruption_error() {
        assert!(is_apple_container_corruption_error(
            "failed to bootstrap container because config.json is missing"
        ));
        // Daemon-stale errors should also trigger corruption/failover
        assert!(is_apple_container_corruption_error(
            "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\""
        ));
        assert!(!is_apple_container_corruption_error(
            "cannot exec: container is not running"
        ));
        assert!(!is_apple_container_corruption_error(
            "invalidState: \"no sandbox client exists: container is stopped\""
        ));
        assert!(!is_apple_container_corruption_error("permission denied"));
        // Boot failure "VM never booted" should trigger corruption/failover
        assert!(is_apple_container_corruption_error(
            "apple container test did not become exec-ready (VM never booted): timeout"
        ));
    }

    #[tokio::test]
    async fn test_failover_sandbox_switches_from_apple_to_docker() {
        let primary = Arc::new(TestSandbox::new(
            "apple-container",
            Some("failed to bootstrap container: config.json missing"),
            None,
        ));
        let fallback = Arc::new(TestSandbox::new("docker", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-abc".into(),
        };

        sandbox.ensure_ready(&id, None).await.unwrap();
        sandbox.ensure_ready(&id, None).await.unwrap();

        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 2);
    }

    #[tokio::test]
    async fn test_failover_sandbox_switches_on_boot_failure() {
        let primary = Arc::new(TestSandbox::new(
            "apple-container",
            Some("apple container test did not become exec-ready (VM never booted): timeout"),
            None,
        ));
        let fallback = Arc::new(TestSandbox::new("docker", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-boot".into(),
        };

        sandbox.ensure_ready(&id, None).await.unwrap();

        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 1);
    }

    #[tokio::test]
    async fn test_failover_sandbox_does_not_switch_on_unrelated_error() {
        let primary = Arc::new(TestSandbox::new(
            "apple-container",
            Some("permission denied"),
            None,
        ));
        let fallback = Arc::new(TestSandbox::new("docker", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-abc".into(),
        };

        let error = sandbox.ensure_ready(&id, None).await.unwrap_err();
        assert!(format!("{error:#}").contains("permission denied"));
        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 0);
    }

    #[tokio::test]
    async fn test_failover_sandbox_switches_exec_path() {
        let primary = Arc::new(TestSandbox::new(
            "apple-container",
            None,
            Some("failed to bootstrap container: config.json missing"),
        ));
        let fallback = Arc::new(TestSandbox::new("docker", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-abc".into(),
        };

        let result = sandbox
            .exec(&id, "uname -a", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(primary.exec_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 1);
        assert_eq!(fallback.exec_calls(), 1);
    }

    #[tokio::test]
    async fn test_failover_sandbox_switches_on_daemon_stale_error() {
        let primary = Arc::new(TestSandbox::new(
            "apple-container",
            Some(
                "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\"",
            ),
            None,
        ));
        let fallback = Arc::new(TestSandbox::new("docker", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-abc".into(),
        };

        sandbox.ensure_ready(&id, None).await.unwrap();
        sandbox.ensure_ready(&id, None).await.unwrap();

        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 2);
    }

    #[tokio::test]
    async fn test_failover_sandbox_docker_to_wasm() {
        let primary = Arc::new(TestSandbox::new(
            "docker",
            Some("cannot connect to the docker daemon"),
            None,
        ));
        let fallback = Arc::new(TestSandbox::new("wasm", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-docker-wasm".into(),
        };

        sandbox.ensure_ready(&id, None).await.unwrap();

        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 1);
    }

    #[tokio::test]
    async fn test_failover_docker_does_not_switch_on_unrelated_error() {
        let primary = Arc::new(TestSandbox::new("docker", Some("image not found"), None));
        let fallback = Arc::new(TestSandbox::new("wasm", None, None));
        let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "session-docker-no-failover".into(),
        };

        let error = sandbox.ensure_ready(&id, None).await.unwrap_err();
        assert!(format!("{error:#}").contains("image not found"));
        assert_eq!(primary.ensure_ready_calls(), 1);
        assert_eq!(fallback.ensure_ready_calls(), 0);
    }

    #[test]
    fn test_is_docker_failover_error() {
        assert!(is_docker_failover_error(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock"
        ));
        assert!(is_docker_failover_error("Is the docker daemon running?"));
        assert!(is_docker_failover_error(
            "error during connect: connection refused"
        ));
        assert!(!is_docker_failover_error("image not found"));
        assert!(!is_docker_failover_error("permission denied"));
    }

    #[test]
    fn test_select_backend_wasm() {
        let config = SandboxConfig {
            backend: "wasm".into(),
            ..Default::default()
        };
        let backend = select_backend(config);
        if is_wasm_sandbox_available() {
            assert_eq!(backend.backend_name(), "wasm");
        } else {
            // Falls back to restricted-host when wasm feature is disabled.
            assert_eq!(backend.backend_name(), "restricted-host");
        }
    }

    #[test]
    fn test_select_backend_restricted_host() {
        let config = SandboxConfig {
            backend: "restricted-host".into(),
            ..Default::default()
        };
        let backend = select_backend(config);
        assert_eq!(backend.backend_name(), "restricted-host");
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

    #[test]
    fn test_host_package_name_candidates_t64_to_base() {
        assert_eq!(host_package_name_candidates("libgtk-3-0t64"), vec![
            "libgtk-3-0t64".to_string(),
            "libgtk-3-0".to_string()
        ]);
    }

    #[test]
    fn test_host_package_name_candidates_base_to_t64_for_soname() {
        assert_eq!(host_package_name_candidates("libcups2"), vec![
            "libcups2".to_string(),
            "libcups2t64".to_string()
        ]);
    }

    #[test]
    fn test_host_package_name_candidates_non_library_stays_single() {
        assert_eq!(host_package_name_candidates("curl"), vec![
            "curl".to_string()
        ]);
        assert_eq!(host_package_name_candidates("libreoffice-core"), vec![
            "libreoffice-core".to_string()
        ]);
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
    fn test_is_running_as_root() {
        // In CI and dev, we typically don't run as root.
        let result = is_running_as_root();
        // Just verify it returns a bool without panic.
        let _ = result;
    }

    #[test]
    fn test_should_use_docker_backend() {
        assert!(should_use_docker_backend(true, true));
        assert!(!should_use_docker_backend(true, false));
        assert!(!should_use_docker_backend(false, true));
        assert!(!should_use_docker_backend(false, false));
    }

    #[test]
    fn container_run_state_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(ContainerRunState::Running)
                .unwrap()
                .as_str(),
            Some("running")
        );
        assert_eq!(
            serde_json::to_value(ContainerRunState::Stopped)
                .unwrap()
                .as_str(),
            Some("stopped")
        );
        assert_eq!(
            serde_json::to_value(ContainerRunState::Exited)
                .unwrap()
                .as_str(),
            Some("exited")
        );
        assert_eq!(
            serde_json::to_value(ContainerRunState::Unknown)
                .unwrap()
                .as_str(),
            Some("unknown")
        );
    }

    #[test]
    fn container_backend_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_value(ContainerBackend::AppleContainer)
                .unwrap()
                .as_str(),
            Some("apple-container")
        );
        assert_eq!(
            serde_json::to_value(ContainerBackend::Docker)
                .unwrap()
                .as_str(),
            Some("docker")
        );
    }

    #[test]
    fn running_container_serializes_to_json() {
        let c = RunningContainer {
            name: "moltis-sandbox-sess1".into(),
            image: "ubuntu:25.10".into(),
            state: ContainerRunState::Running,
            backend: ContainerBackend::Docker,
            cpus: Some(2),
            memory_mb: Some(512),
            started: Some("2025-01-01T00:00:00Z".into()),
            addr: None,
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["name"], "moltis-sandbox-sess1");
        assert_eq!(json["state"], "running");
        assert_eq!(json["backend"], "docker");
        assert_eq!(json["cpus"], 2);
        assert_eq!(json["memory_mb"], 512);
        assert!(json["addr"].is_null());
    }

    #[test]
    fn test_zombie_set_lifecycle() {
        // Fresh state: nothing is a zombie.
        assert!(!is_zombie("ghost-1"));

        // Mark as zombie.
        mark_zombie("ghost-1");
        assert!(is_zombie("ghost-1"));

        // Marking again is idempotent.
        mark_zombie("ghost-1");
        assert!(is_zombie("ghost-1"));

        // A different name is not a zombie.
        assert!(!is_zombie("ghost-2"));

        // Unmark clears the zombie.
        unmark_zombie("ghost-1");
        assert!(!is_zombie("ghost-1"));

        // Unmarking a non-zombie is a no-op.
        unmark_zombie("ghost-1");

        // Clear removes all zombies.
        mark_zombie("ghost-a");
        mark_zombie("ghost-b");
        assert!(is_zombie("ghost-a"));
        assert!(is_zombie("ghost-b"));
        clear_zombies();
        assert!(!is_zombie("ghost-a"));
        assert!(!is_zombie("ghost-b"));
    }

    mod restricted_host_tests {
        use super::*;

        #[test]
        fn test_restricted_host_sandbox_backend_name() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            assert_eq!(sandbox.backend_name(), "restricted-host");
        }

        #[test]
        fn test_restricted_host_sandbox_is_real() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            assert!(sandbox.is_real());
        }

        #[tokio::test]
        async fn test_restricted_host_sandbox_ensure_ready_noop() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-rh".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
        }

        #[tokio::test]
        async fn test_restricted_host_sandbox_exec_simple_echo() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-rh-echo".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "echo hello", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "hello");
        }

        #[tokio::test]
        async fn test_restricted_host_sandbox_restricted_env() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-rh-env".into(),
            };
            let result = sandbox
                .exec(&id, "echo $HOME", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "/tmp");
        }

        #[tokio::test]
        async fn test_restricted_host_sandbox_build_image_returns_none() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            let result = sandbox
                .build_image("ubuntu:latest", &["curl".to_string()])
                .await
                .unwrap();
            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_restricted_host_sandbox_cleanup_noop() {
            let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-rh-cleanup".into(),
            };
            sandbox.cleanup(&id).await.unwrap();
        }

        #[test]
        fn test_parse_memory_limit() {
            assert_eq!(parse_memory_limit("512M"), Some(512 * 1024 * 1024));
            assert_eq!(parse_memory_limit("1G"), Some(1024 * 1024 * 1024));
            assert_eq!(parse_memory_limit("256k"), Some(256 * 1024));
            assert_eq!(parse_memory_limit("1024"), Some(1024));
            assert_eq!(parse_memory_limit("invalid"), None);
        }

        #[test]
        fn test_wasm_sandbox_available() {
            assert!(is_wasm_sandbox_available());
        }
    }

    #[cfg(feature = "wasm")]
    mod wasm_sandbox_tests {
        use super::*;

        fn test_config() -> SandboxConfig {
            SandboxConfig {
                home_persistence: HomePersistence::Off,
                ..Default::default()
            }
        }

        #[test]
        fn test_wasm_sandbox_backend_name() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            assert_eq!(sandbox.backend_name(), "wasm");
        }

        #[test]
        fn test_wasm_sandbox_is_real() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            assert!(sandbox.is_real());
        }

        #[test]
        fn test_wasm_sandbox_fuel_limit_default() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            assert_eq!(sandbox.fuel_limit(), 1_000_000_000);
        }

        #[test]
        fn test_wasm_sandbox_fuel_limit_custom() {
            let mut config = test_config();
            config.wasm_fuel_limit = Some(500_000);
            let sandbox = WasmSandbox::new(config).unwrap();
            assert_eq!(sandbox.fuel_limit(), 500_000);
        }

        #[test]
        fn test_wasm_sandbox_epoch_interval_default() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            assert_eq!(sandbox.epoch_interval_ms(), 100);
        }

        #[tokio::test]
        async fn test_wasm_sandbox_ensure_ready_creates_dirs() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-ready".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            assert!(sandbox.home_dir(&id).exists());
            assert!(sandbox.tmp_dir(&id).exists());
            // Cleanup.
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_cleanup_removes_dirs() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-cleanup".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let root = sandbox.sandbox_root(&id);
            assert!(root.exists());
            sandbox.cleanup(&id).await.unwrap();
            assert!(!root.exists());
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_echo() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-echo".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "echo hello world", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "hello world");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_echo_no_newline() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-echo-n".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "echo -n hello", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout, "hello");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_pwd() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-pwd".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "pwd", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "/home/sandbox");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_true_false() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-tf".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            let result = sandbox
                .exec(&id, "true", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);

            let result = sandbox
                .exec(&id, "false", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 1);
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_mkdir_ls() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-mkdir-ls".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            let result = sandbox
                .exec(&id, "mkdir /home/sandbox/testdir", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);

            let result = sandbox
                .exec(&id, "ls /home/sandbox", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("testdir"));
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_touch_cat() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-touch-cat".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            // Write a file using echo with redirect.
            let result = sandbox
                .exec(
                    &id,
                    "echo hello > /home/sandbox/test.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);

            // Read it back.
            let result = sandbox
                .exec(&id, "cat /home/sandbox/test.txt", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "hello");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_rm() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-rm".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            sandbox
                .exec(
                    &id,
                    "echo data > /home/sandbox/to_delete.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();

            let result = sandbox
                .exec(&id, "rm /home/sandbox/to_delete.txt", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);

            let result = sandbox
                .exec(&id, "cat /home/sandbox/to_delete.txt", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 1);
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_unknown_command_127() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-unknown".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "nonexistent_cmd", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 127);
            assert!(result.stderr.contains("command not found"));
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_path_escape_blocked() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-escape".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            // Try to cat a file outside sandbox.
            let result = sandbox
                .exec(&id, "cat /etc/passwd", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 1);
            assert!(result.stderr.contains("outside sandbox"));
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_and_connector() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-and".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "true && echo yes", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "yes");

            let result = sandbox
                .exec(&id, "false && echo no", &ExecOpts::default())
                .await
                .unwrap();
            // The echo shouldn't run, so stdout should be empty.
            assert!(result.stdout.is_empty());
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_or_connector() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-or".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();
            let result = sandbox
                .exec(&id, "false || echo fallback", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert_eq!(result.stdout.trim(), "fallback");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_test_file() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-testcmd".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            sandbox
                .exec(
                    &id,
                    "echo x > /home/sandbox/exists.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();

            let result = sandbox
                .exec(
                    &id,
                    "test -f /home/sandbox/exists.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);

            let result = sandbox
                .exec(&id, "test -f /home/sandbox/nope.txt", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 1);
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_basename_dirname() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-pathops".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            let result = sandbox
                .exec(
                    &id,
                    "basename /home/sandbox/foo/bar.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();
            assert_eq!(result.stdout.trim(), "bar.txt");

            let result = sandbox
                .exec(
                    &id,
                    "dirname /home/sandbox/foo/bar.txt",
                    &ExecOpts::default(),
                )
                .await
                .unwrap();
            assert_eq!(result.stdout.trim(), "/home/sandbox/foo");
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_builtin_which() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let id = SandboxId {
                scope: SandboxScope::Session,
                key: "test-wasm-which".into(),
            };
            sandbox.ensure_ready(&id, None).await.unwrap();

            let result = sandbox
                .exec(&id, "which echo", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 0);
            assert!(result.stdout.contains("built-in"));

            let result = sandbox
                .exec(&id, "which nonexistent", &ExecOpts::default())
                .await
                .unwrap();
            assert_eq!(result.exit_code, 1);
            sandbox.cleanup(&id).await.unwrap();
        }

        #[tokio::test]
        async fn test_wasm_sandbox_build_image_returns_none() {
            let sandbox = WasmSandbox::new(test_config()).unwrap();
            let result = sandbox
                .build_image("ubuntu:latest", &["curl".to_string()])
                .await
                .unwrap();
            assert!(result.is_none());
        }
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
