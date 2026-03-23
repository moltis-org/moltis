//! Core types, enums, traits, and constants for the sandbox subsystem.

use std::path::PathBuf;

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
};

use crate::{
    error::Result,
    exec::{ExecOpts, ExecResult},
    wasm_limits::WasmToolLimits,
};

pub(crate) fn truncate_output_for_display(output: &mut String, max_output_bytes: usize) {
    if output.len() <= max_output_bytes {
        return;
    }
    output.truncate(output.floor_char_boundary(max_output_bytes));
    output.push_str("\n... [output truncated]");
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

pub use moltis_network_filter::NetworkPolicy;

/// Configuration for sandbox behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: SandboxMode,
    pub scope: SandboxScope,
    pub workspace_mount: WorkspaceMount,
    /// Host-visible path for Moltis `data_dir()` when running container-backed
    /// sandboxes from inside another container.
    pub host_data_dir: Option<PathBuf>,
    /// Persistence strategy for `/home/sandbox`.
    pub home_persistence: HomePersistence,
    /// Host directory used for shared `/home/sandbox` persistence.
    /// Relative paths are resolved against `data_dir()`.
    pub shared_home_dir: Option<PathBuf>,
    pub image: Option<String>,
    pub container_prefix: Option<String>,
    pub no_network: bool,
    /// Network policy: `Blocked` (no network), `Trusted` (proxy-filtered), `Open` (unrestricted).
    pub network: NetworkPolicy,
    /// Domains allowed through the proxy in `Trusted` mode.
    pub trusted_domains: Vec<String>,
    /// Backend: `"auto"` (default), `"docker"`, `"podman"`, `"apple-container"`,
    /// `"restricted-host"`, or `"wasm"`.
    /// `"auto"` prefers Apple Container on macOS, then Podman, then Docker, then restricted-host.
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
            host_data_dir: None,
            home_persistence: HomePersistence::default(),
            shared_home_dir: None,
            image: None,
            container_prefix: None,
            no_network: false,
            network: NetworkPolicy::default(),
            trusted_domains: Vec::new(),
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
            host_data_dir: cfg
                .host_data_dir
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
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
            network: match cfg.network.as_str() {
                "trusted" => NetworkPolicy::Trusted,
                "bypass" => NetworkPolicy::Bypass,
                // Explicit "blocked" always means Blocked.
                "blocked" => NetworkPolicy::Blocked,
                // Empty/unset: fall back to legacy `no_network` flag.
                _ if cfg.no_network => NetworkPolicy::Blocked,
                _ => NetworkPolicy::Trusted,
            },
            trusted_domains: cfg.trusted_domains.clone(),
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
    /// Human-readable backend name (e.g. "docker", "podman", "apple-container", "cgroup", "none").
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

pub(crate) fn canonical_sandbox_packages(packages: &[String]) -> Vec<String> {
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

pub(crate) const SANDBOX_HOME_DIR: &str = "/home/sandbox";
pub(crate) const GOGCLI_MODULE_PATH: &str = "github.com/steipete/gogcli/cmd/gog";
pub(crate) const GOGCLI_VERSION: &str = "latest";
#[cfg(any(target_os = "macos", test))]
pub(crate) const APPLE_CONTAINER_SAFE_WORKDIR: &str = "/tmp";

pub(crate) fn sanitize_path_component(input: &str) -> String {
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
