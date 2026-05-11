//! Core types, enums, traits, and constants for the sandbox subsystem.

use std::path::PathBuf;

use {
    async_trait::async_trait,
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

use crate::{
    error::Result,
    exec::{ExecOpts, ExecResult},
    sandbox::file_system::{
        SandboxGrepOptions, SandboxListFilesResult, SandboxReadResult, command_grep,
        command_list_files, command_read_file, command_write_file,
    },
    wasm_limits::WasmToolLimits,
};

pub(crate) fn truncate_output_for_display(output: &mut String, max_output_bytes: usize) {
    if output.len() <= max_output_bytes {
        return;
    }
    output.truncate(output.floor_char_boundary(max_output_bytes));
    output.push_str("\n... [output truncated]");
}

/// Return the last `n` lines of `text`, or the full text if it has fewer lines.
pub(crate) fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= n {
        return text.to_string();
    }
    format!(
        "... [{} lines truncated]\n{}",
        lines.len() - n,
        lines[lines.len() - n..].join("\n")
    )
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

/// Known sandbox backend identifiers.
///
/// Used in the API/gon layer for type-safe backend references. The config
/// schema uses a plain `String` for flexibility (TOML compatibility), but
/// this enum ensures wire-format consistency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxBackendId {
    Docker,
    Podman,
    AppleContainer,
    Cgroup,
    RestrictedHost,
    Wasm,
    Vercel,
    Daytona,
    Firecracker,
    None,
}

impl SandboxBackendId {
    /// Parse from backend_name() output.
    pub fn from_name(name: &str) -> Self {
        match name {
            "docker" => Self::Docker,
            "podman" => Self::Podman,
            "apple-container" => Self::AppleContainer,
            "cgroup" => Self::Cgroup,
            "restricted-host" => Self::RestrictedHost,
            "wasm" => Self::Wasm,
            "vercel" => Self::Vercel,
            "daytona" => Self::Daytona,
            "firecracker" => Self::Firecracker,
            _ => Self::None,
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
#[derive(Debug, Clone, Deserialize)]
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
    /// GPU device passthrough for Docker/Podman backends (e.g. "all", "device=0").
    pub gpus: Option<String>,
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

    // ── Vercel sandbox configuration ────────────────────────────────────
    /// Vercel API token (`VERCEL_TOKEN` or `VERCEL_OIDC_TOKEN`).
    pub vercel_token: Option<Secret<String>>,
    /// Vercel project ID.
    pub vercel_project_id: Option<String>,
    /// Vercel team ID.
    pub vercel_team_id: Option<String>,
    /// Vercel sandbox runtime (e.g. "node24", "node22", "python3.13").
    pub vercel_runtime: Option<String>,
    /// Vercel sandbox timeout in milliseconds.
    pub vercel_timeout_ms: Option<u64>,
    /// Vercel sandbox vCPU count.
    pub vercel_vcpus: Option<u32>,
    /// Vercel snapshot ID for fast cold starts.
    pub vercel_snapshot_id: Option<String>,

    // ── Daytona sandbox configuration ───────────────────────────────────
    /// Daytona API key (`DAYTONA_API_KEY`).
    pub daytona_api_key: Option<Secret<String>>,
    /// Daytona API URL (default: `https://app.daytona.io/api`).
    pub daytona_api_url: Option<String>,
    /// Daytona target region/environment.
    pub daytona_target: Option<String>,
    /// Custom image for Daytona sandbox creation.
    pub daytona_image: Option<String>,

    // ── Firecracker sandbox configuration (Linux only) ──────────────────
    /// Path to the `firecracker` binary.
    pub firecracker_bin: Option<PathBuf>,
    /// Path to the uncompressed Linux kernel (`vmlinux`).
    pub firecracker_kernel: Option<PathBuf>,
    /// Path to the base ext4 rootfs image.
    pub firecracker_rootfs: Option<PathBuf>,
    /// Path to the SSH private key for VM access.
    pub firecracker_ssh_key: Option<PathBuf>,
    /// Number of vCPUs per Firecracker VM.
    pub firecracker_vcpus: Option<u32>,
    /// Memory in MiB per Firecracker VM.
    pub firecracker_memory_mb: Option<u32>,
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
            gpus: None,
            packages: Vec::new(),
            timezone: None,
            wasm_fuel_limit: None,
            wasm_epoch_interval_ms: None,
            wasm_tool_limits: None,
            vercel_token: None,
            vercel_project_id: None,
            vercel_team_id: None,
            vercel_runtime: None,
            vercel_timeout_ms: None,
            vercel_vcpus: None,
            vercel_snapshot_id: None,
            daytona_api_key: None,
            daytona_api_url: None,
            daytona_target: None,
            daytona_image: None,
            firecracker_bin: None,
            firecracker_kernel: None,
            firecracker_rootfs: None,
            firecracker_ssh_key: None,
            firecracker_vcpus: None,
            firecracker_memory_mb: None,
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
            gpus: cfg.gpus.clone(),
            packages: cfg.packages.clone(),
            timezone: None, // Set by gateway from user profile
            wasm_fuel_limit: cfg.wasm_fuel_limit,
            wasm_epoch_interval_ms: cfg.wasm_epoch_interval_ms,
            wasm_tool_limits: cfg.wasm_tool_limits.as_ref().map(WasmToolLimits::from),
            vercel_token: cfg.vercel_token.clone(),
            vercel_project_id: cfg.vercel_project_id.clone(),
            vercel_team_id: cfg.vercel_team_id.clone(),
            vercel_runtime: cfg.vercel_runtime.clone(),
            vercel_timeout_ms: cfg.vercel_timeout_ms,
            vercel_vcpus: cfg.vercel_vcpus,
            vercel_snapshot_id: cfg.vercel_snapshot_id.clone(),
            daytona_api_key: cfg.daytona_api_key.clone(),
            daytona_api_url: cfg.daytona_api_url.clone(),
            daytona_target: cfg.daytona_target.clone(),
            daytona_image: cfg.daytona_image.clone(),
            firecracker_bin: cfg.firecracker_bin.as_deref().map(PathBuf::from),
            firecracker_kernel: cfg.firecracker_kernel.as_deref().map(PathBuf::from),
            firecracker_rootfs: cfg.firecracker_rootfs.as_deref().map(PathBuf::from),
            firecracker_ssh_key: cfg.firecracker_ssh_key.as_deref().map(PathBuf::from),
            firecracker_vcpus: cfg.firecracker_vcpus,
            firecracker_memory_mb: cfg.firecracker_memory_mb,
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

    /// Read a file inside the sandbox.
    async fn read_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        command_read_file(self, id, file_path, max_bytes).await
    }

    /// Write a file inside the sandbox.
    async fn write_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        command_write_file(self, id, file_path, content).await
    }

    /// List regular files inside the sandbox.
    async fn list_files(&self, id: &SandboxId, root: &str) -> Result<SandboxListFilesResult> {
        command_list_files(self, id, root).await
    }

    /// Run grep inside the sandbox.
    async fn grep(&self, id: &SandboxId, opts: SandboxGrepOptions) -> Result<serde_json::Value> {
        command_grep(self, id, opts).await
    }

    /// Clean up sandbox resources.
    async fn cleanup(&self, id: &SandboxId) -> Result<()>;

    /// Whether this backend provides actual isolation.
    /// Returns `false` for `NoSandbox` (pass-through to host).
    fn is_real(&self) -> bool {
        true
    }

    /// Whether this backend provides filesystem isolation from the host.
    ///
    /// Defaults to `false` (fail-safe): new backends must explicitly opt in
    /// by returning `true`.  Container-based backends (Docker, Podman, Apple
    /// Container, WASM) override this to `true`.  Backends that only provide
    /// resource limits (restricted-host, cgroup) or no isolation (none) keep
    /// the default.
    ///
    /// Used by the exec flow to enforce approval gating and file-path
    /// restrictions when true filesystem isolation is unavailable.
    fn provides_fs_isolation(&self) -> bool {
        false
    }

    /// The default workspace/home directory inside this backend.
    ///
    /// Used by workspace sync to determine where to extract files.
    /// Defaults to `/home/sandbox`. Remote backends override this
    /// (e.g. Vercel returns `/vercel/sandbox`).
    fn workspace_dir(&self) -> &str {
        SANDBOX_HOME_DIR
    }

    /// Workspace directory for a specific prepared session.
    ///
    /// Most backends use a fixed directory and can rely on the default.
    /// Backends whose API returns a per-session project directory override
    /// this so workspace sync uses the same path as command execution.
    async fn workspace_dir_for(&self, _id: &SandboxId) -> String {
        self.workspace_dir().to_string()
    }

    /// Whether this backend manages an isolated filesystem that requires
    /// workspace sync (copy-in on setup, patch extraction on cleanup).
    ///
    /// Defaults to `false`. Local bind-mount backends (Docker, Podman, Apple
    /// Container) mount the host workspace directly. Remote/VM backends
    /// (Vercel, Daytona, Firecracker) return `true` — the workspace must be
    /// synced in via git bundles and changes extracted back via patches.
    fn is_isolated(&self) -> bool {
        false
    }

    /// Install packages inside the sandbox.
    ///
    /// Default implementation uses `apt-get` (Ubuntu/Debian). Backends with
    /// different package managers (e.g. Vercel/Amazon Linux uses `dnf`)
    /// override this method.
    ///
    /// Called once per session after `ensure_ready()` for isolated backends
    /// that don't have packages pre-baked into the image.
    async fn provision_packages(&self, id: &SandboxId, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        let pkg_list = packages.join(" ");
        let cmd = format!(
            "apt-get update -qq && apt-get install -y -qq --no-install-recommends {pkg_list}"
        );
        let opts = ExecOpts {
            timeout: std::time::Duration::from_secs(600),
            ..Default::default()
        };
        let result = self.exec(id, &cmd, &opts).await?;
        if result.exit_code != 0 {
            tracing::warn!(
                %id,
                exit_code = result.exit_code,
                stderr = result.stderr.trim(),
                "package provisioning failed (non-fatal)"
            );
        }
        Ok(())
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

/// Additional Go-based CLI tools installed via `go install` in the sandbox image.
/// Each entry is `(module_path, version, binary_name)`.
///
/// Only tools that work inside a Linux container belong here. macOS-only tools
/// (e.g. wacrawl) are host-only and install via their skill's `requires.install`.
pub(crate) const GO_TOOL_INSTALLS: &[(&str, &str, &str)] = &[
    (
        "github.com/openclaw/discrawl/cmd/discrawl",
        "latest",
        "discrawl",
    ),
    (
        "github.com/vincentkoc/slacrawl/cmd/slacrawl",
        "latest",
        "slacrawl",
    ),
];
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use secrecy::{ExposeSecret, Secret};

    use crate::sandbox::SandboxConfig;

    #[test]
    fn sandbox_config_debug_redacts_remote_backend_credentials() {
        let config = SandboxConfig {
            vercel_token: Some(Secret::new("vercel-secret-value".into())),
            daytona_api_key: Some(Secret::new("daytona-secret-value".into())),
            ..SandboxConfig::default()
        };

        let debug = format!("{config:?}");

        assert!(!debug.contains("vercel-secret-value"));
        assert!(!debug.contains("daytona-secret-value"));
        assert!(debug.contains("vercel_token"));
        assert!(debug.contains("daytona_api_key"));
    }

    #[test]
    fn sandbox_config_deserializes_remote_backend_credentials_as_secrets() {
        let config: SandboxConfig = serde_json::from_str(
            r#"{
                "vercel_token": "vercel-secret-value",
                "daytona_api_key": "daytona-secret-value"
            }"#,
        )
        .unwrap();

        assert_eq!(
            config
                .vercel_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("vercel-secret-value")
        );
        assert_eq!(
            config
                .daytona_api_key
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("daytona-secret-value")
        );
    }
}
