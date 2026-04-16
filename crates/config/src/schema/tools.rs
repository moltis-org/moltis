use {
    super::*,
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// Tools configuration (exec, sandbox, policy, web, browser).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub exec: ExecConfig,
    pub policy: ToolPolicyConfig,
    pub web: WebConfig,
    pub maps: MapsConfig,
    pub browser: BrowserConfig,
    /// Native filesystem tools (Read/Write/Edit/MultiEdit/Glob/Grep).
    /// See moltis-org/moltis#657.
    #[serde(default)]
    pub fs: FsToolsConfig,
    /// Maximum wall-clock seconds for an agent run (0 = no timeout). Default 600.
    #[serde(default = "default_agent_timeout_secs")]
    pub agent_timeout_secs: u64,
    /// Maximum number of agent loop iterations before aborting. Default 25.
    #[serde(default = "default_agent_max_iterations")]
    pub agent_max_iterations: usize,
    /// Maximum auto-continue nudges when the model stops mid-task (0 = disabled). Default 2.
    #[serde(default = "default_agent_max_auto_continues")]
    pub agent_max_auto_continues: usize,
    /// Minimum tool calls in the current run before auto-continue can trigger. Default 3.
    #[serde(default = "default_agent_auto_continue_min_tool_calls")]
    pub agent_auto_continue_min_tool_calls: usize,
    /// Maximum bytes for a single tool result before truncation. Default 50KB.
    #[serde(default = "default_max_tool_result_bytes")]
    pub max_tool_result_bytes: usize,
    /// How tool schemas are presented to the model. Default "full".
    #[serde(default)]
    pub registry_mode: ToolRegistryMode,
    /// Window size for the tool-call reflex-loop detector. When this many
    /// consecutive tool calls share the same tool + (args or error), the
    /// runner injects a directive intervention message. Set to 0 to disable.
    /// Default 3.
    #[serde(default = "default_agent_loop_detector_window")]
    pub agent_loop_detector_window: usize,
    /// When the loop detector fires a second time (stage 2), strip the tool
    /// schema list for a single LLM turn so the model is forced to respond
    /// in text. Default true.
    #[serde(default = "default_agent_loop_detector_strip_tools")]
    pub agent_loop_detector_strip_tools_on_second_fire: bool,
    /// Percentage of the provider's context window at which per-iteration
    /// tool-result compaction starts. Oldest results are compacted first.
    /// Set to 0 to disable per-iteration compaction entirely. Default 75.
    #[serde(default = "default_tool_result_compaction_ratio")]
    pub tool_result_compaction_ratio: u32,
    /// Percentage of the provider's context window at which a hard
    /// `ContextWindowExceeded` error fires even after compaction.
    /// Must be greater than `tool_result_compaction_ratio`. Default 90.
    #[serde(default = "default_preemptive_overflow_ratio")]
    pub preemptive_overflow_ratio: u32,
    /// Minimum number of agent loop iterations before per-iteration
    /// tool-result compaction is allowed to fire. Prevents premature
    /// context destruction in short loops. Default 3.
    #[serde(default = "default_compaction_min_iterations")]
    pub compaction_min_iterations: usize,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            exec: ExecConfig::default(),
            policy: ToolPolicyConfig::default(),
            web: WebConfig::default(),
            maps: MapsConfig::default(),
            browser: BrowserConfig::default(),
            fs: FsToolsConfig::default(),
            agent_timeout_secs: default_agent_timeout_secs(),
            agent_max_iterations: default_agent_max_iterations(),
            agent_max_auto_continues: default_agent_max_auto_continues(),
            agent_auto_continue_min_tool_calls: default_agent_auto_continue_min_tool_calls(),
            max_tool_result_bytes: default_max_tool_result_bytes(),
            registry_mode: ToolRegistryMode::default(),
            agent_loop_detector_window: default_agent_loop_detector_window(),
            agent_loop_detector_strip_tools_on_second_fire: default_agent_loop_detector_strip_tools(
            ),
            tool_result_compaction_ratio: default_tool_result_compaction_ratio(),
            preemptive_overflow_ratio: default_preemptive_overflow_ratio(),
            compaction_min_iterations: default_compaction_min_iterations(),
        }
    }
}

/// Configuration for the native filesystem tools
/// (Read / Write / Edit / MultiEdit / Glob / Grep).
///
/// Tracks GH moltis-org/moltis#657. Every field is optional and conservative
/// by default — fs tools work out of the box with no configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FsToolsConfig {
    /// Default search root used by `Glob` and `Grep` when the LLM call
    /// omits the `path` argument. Must be an absolute path. When unset,
    /// calls without an explicit `path` are rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,

    /// Absolute path globs the tools are allowed to access. Empty list
    /// means "all paths allowed". Evaluated after canonicalization, so
    /// symlinks can't be used to escape the allowlist.
    #[serde(default)]
    pub allow_paths: Vec<String>,

    /// Absolute path globs the tools must refuse. Deny wins over allow.
    /// Evaluated after canonicalization.
    #[serde(default)]
    pub deny_paths: Vec<String>,

    /// Whether to track per-session read history (files read, re-read
    /// loop detection). Required for `must_read_before_write`. Default `false`.
    #[serde(default)]
    pub track_reads: bool,

    /// Reject Write/Edit/MultiEdit calls targeting files the session has
    /// not previously Read. Requires `track_reads = true`. Default `false`.
    #[serde(default)]
    pub must_read_before_write: bool,

    /// Whether Write/Edit/MultiEdit must pause for explicit operator
    /// approval before mutating a file. Default `false` for backward
    /// compatibility with existing installs; the generated config
    /// template enables it for new installs.
    #[serde(default)]
    pub require_approval: bool,

    /// Maximum bytes a single `Read` call can return before the file is
    /// rejected with a typed `too_large` payload. Default 10 MB.
    #[serde(default = "default_fs_max_read_bytes")]
    pub max_read_bytes: u64,

    /// What to do with binary files encountered by `Read`.
    #[serde(default)]
    pub binary_policy: FsBinaryPolicy,

    /// Whether `Glob` and `Grep` respect `.gitignore` / `.ignore` files
    /// and `.git/info/exclude` while walking. Default `true`.
    #[serde(default = "default_fs_respect_gitignore")]
    pub respect_gitignore: bool,

    /// When true, Write/Edit/MultiEdit call the existing
    /// `CheckpointManager` to create a per-file backup before mutating,
    /// so the LLM can restore the pre-edit state via
    /// `checkpoint_restore`. Default `false` to avoid unbounded disk
    /// growth on repos with large files.
    #[serde(default)]
    pub checkpoint_before_mutation: bool,

    /// Model context window in tokens. When set, `Read`'s per-call
    /// byte cap scales adaptively so a single Read call can't consume
    /// more than ~20% of the model's working set. Clamped to
    /// `[50 KB, 512 KB]`. When unset, Read uses a fixed 256 KB cap.
    ///
    /// Typical values: 200000 for Claude 3.5/4 Sonnet, 1000000 for
    /// Claude Opus 4.6 1M context, 128000 for GPT-4 Turbo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
}

impl Default for FsToolsConfig {
    fn default() -> Self {
        Self {
            workspace_root: None,
            allow_paths: Vec::new(),
            deny_paths: Vec::new(),
            track_reads: false,
            must_read_before_write: false,
            require_approval: false,
            max_read_bytes: default_fs_max_read_bytes(),
            binary_policy: FsBinaryPolicy::default(),
            respect_gitignore: default_fs_respect_gitignore(),
            checkpoint_before_mutation: false,
            context_window_tokens: None,
        }
    }
}

fn default_fs_max_read_bytes() -> u64 {
    10 * 1024 * 1024
}

const fn default_fs_respect_gitignore() -> bool {
    true
}

/// Strategy for handling binary files when encountered by `Read`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FsBinaryPolicy {
    /// Return a typed `{kind: "binary", bytes: N}` marker without content.
    #[default]
    Reject,
    /// Return `{kind: "binary", bytes: N, base64: "..."}` so the LLM can
    /// access the raw bytes (useful for small images, hashes, etc.).
    /// Still capped by `max_read_bytes`.
    Base64,
}

fn default_agent_timeout_secs() -> u64 {
    600
}

fn default_agent_max_iterations() -> usize {
    25
}

fn default_agent_max_auto_continues() -> usize {
    2
}

fn default_agent_auto_continue_min_tool_calls() -> usize {
    3
}

fn default_max_tool_result_bytes() -> usize {
    50_000
}

fn default_agent_loop_detector_window() -> usize {
    3
}

fn default_agent_loop_detector_strip_tools() -> bool {
    true
}

fn default_tool_result_compaction_ratio() -> u32 {
    75
}

fn default_preemptive_overflow_ratio() -> u32 {
    90
}

fn default_compaction_min_iterations() -> usize {
    3
}

/// Map tools configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MapsConfig {
    /// Preferred map provider used by `show_map`.
    pub provider: MapProvider,
}

/// Map provider selection for map links.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MapProvider {
    #[default]
    #[serde(rename = "google_maps")]
    GoogleMaps,
    #[serde(rename = "apple_maps")]
    AppleMaps,
    #[serde(rename = "openstreetmap")]
    OpenStreetMap,
}

/// Web tools configuration (search, fetch, firecrawl).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub search: WebSearchConfig,
    pub fetch: WebFetchConfig,
    pub firecrawl: FirecrawlConfig,
}

/// Search provider selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    #[default]
    Brave,
    Perplexity,
    Firecrawl,
}

/// Web search tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    pub enabled: bool,
    /// Search provider.
    pub provider: SearchProvider,
    /// Brave Search API key (overrides `BRAVE_API_KEY` env var).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Maximum number of results to return (1-10).
    pub max_results: u8,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// In-memory cache TTL in minutes (0 to disable).
    pub cache_ttl_minutes: u64,
    /// Enable DuckDuckGo HTML fallback when no provider API key is configured.
    /// Disabled by default because it may trigger CAPTCHA challenges.
    pub duckduckgo_fallback: bool,
    /// Perplexity-specific settings.
    pub perplexity: PerplexityConfig,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: SearchProvider::default(),
            api_key: None,
            max_results: 5,
            timeout_seconds: 30,
            cache_ttl_minutes: 15,
            duckduckgo_fallback: false,
            perplexity: PerplexityConfig::default(),
        }
    }
}

/// Perplexity search provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PerplexityConfig {
    /// API key (overrides `PERPLEXITY_API_KEY` / `OPENROUTER_API_KEY` env vars).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Base URL override. Auto-detected from key prefix if empty.
    pub base_url: Option<String>,
    /// Model to use.
    pub model: Option<String>,
}

/// Web fetch tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFetchConfig {
    pub enabled: bool,
    /// Maximum characters to return from fetched content.
    pub max_chars: usize,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// In-memory cache TTL in minutes (0 to disable).
    pub cache_ttl_minutes: u64,
    /// Maximum number of HTTP redirects to follow.
    pub max_redirects: u8,
    /// Use readability extraction for HTML pages.
    pub readability: bool,
    /// CIDR ranges exempt from SSRF blocking (e.g. `["172.22.0.0/16"]`).
    /// Default: empty (all private IPs blocked).
    #[serde(default)]
    pub ssrf_allowlist: Vec<String>,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: 50_000,
            timeout_seconds: 30,
            cache_ttl_minutes: 15,
            max_redirects: 3,
            readability: true,
            ssrf_allowlist: Vec::new(),
        }
    }
}

/// Firecrawl integration configuration.
///
/// Firecrawl provides high-quality markdown extraction from web pages,
/// including JS-heavy and bot-protected sites.  Used as a standalone
/// `firecrawl_scrape` tool, as a `web_search` provider, and as a
/// fallback extractor inside `web_fetch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FirecrawlConfig {
    /// Enable Firecrawl integration.
    pub enabled: bool,
    /// Firecrawl API key (overrides `FIRECRAWL_API_KEY` env var).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Firecrawl API base URL (for self-hosted instances).
    pub base_url: String,
    /// Only extract main content (skip navs, footers, etc.).
    pub only_main_content: bool,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// In-memory cache TTL in minutes (0 to disable).
    pub cache_ttl_minutes: u64,
    /// Use Firecrawl as fallback in `web_fetch` when readability
    /// extraction produces poor results.
    pub web_fetch_fallback: bool,
}

impl Default for FirecrawlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            base_url: "https://api.firecrawl.dev".into(),
            only_main_content: true,
            timeout_seconds: 30,
            cache_ttl_minutes: 15,
            web_fetch_fallback: true,
        }
    }
}

/// Browser automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Whether browser support is enabled.
    pub enabled: bool,
    /// Path to Chrome/Chromium binary (auto-detected if not set).
    pub chrome_path: Option<String>,
    /// Whether to run in headless mode.
    pub headless: bool,
    /// Default viewport width.
    pub viewport_width: u32,
    /// Default viewport height.
    pub viewport_height: u32,
    /// Device scale factor for HiDPI/Retina displays.
    /// 1.0 = standard, 2.0 = Retina/HiDPI, 3.0 = 3x scaling.
    pub device_scale_factor: f64,
    /// Maximum concurrent browser instances (0 = unlimited, limited by memory).
    pub max_instances: usize,
    /// System memory usage threshold (0-100) above which new instances are blocked.
    /// Default is 90 (block new instances when memory > 90% used).
    pub memory_limit_percent: u8,
    /// Instance idle timeout in seconds before closing.
    pub idle_timeout_secs: u64,
    /// Default navigation timeout in milliseconds.
    pub navigation_timeout_ms: u64,
    /// User agent string (uses default if not set).
    pub user_agent: Option<String>,
    /// Additional Chrome arguments.
    #[serde(default)]
    pub chrome_args: Vec<String>,
    /// Docker image to use for sandboxed browser.
    /// Default: "browserless/chrome" - a purpose-built headless Chrome container.
    /// Sandbox mode is controlled per-session via the request, not globally.
    #[serde(default = "default_sandbox_image")]
    pub sandbox_image: String,
    /// Allowed domains for navigation. Empty list means all domains allowed.
    /// When set, the browser will refuse to navigate to non-matching domains.
    /// Supports wildcards: "*.example.com" matches subdomains.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Total system RAM threshold (MB) below which memory-saving Chrome flags
    /// are injected automatically. Set to 0 to disable. Default: 2048.
    #[serde(default = "default_low_memory_threshold_mb")]
    pub low_memory_threshold_mb: u64,
    /// Whether to persist the Chrome user profile across sessions.
    /// When enabled, cookies, auth state, and local storage survive browser restarts.
    /// Profile is stored at `data_dir()/browser/profile/` unless `profile_dir` overrides it.
    #[serde(default = "default_persist_profile")]
    pub persist_profile: bool,
    /// Custom path for the persistent Chrome profile directory.
    /// When set, `persist_profile` is implicitly true.
    /// If not set and `persist_profile` is true, defaults to `data_dir()/browser/profile/`.
    pub profile_dir: Option<String>,
    /// Hostname or IP used to connect to the browser container from the host.
    /// Default: "127.0.0.1" (localhost). When running Moltis itself inside Docker,
    /// set this to "host.docker.internal" or the Docker bridge gateway IP so
    /// Moltis can reach the sibling browser container via the host's port mapping.
    #[serde(default = "default_container_host")]
    pub container_host: String,
    /// Browserless API compatibility mode for websocket endpoints.
    /// - "v1" (default): connect to the base websocket URL.
    /// - "v2": try Browserless v2 paths (`/chrome`, `/chromium`) when needed.
    #[serde(default = "default_browserless_api_version")]
    pub browserless_api_version: BrowserlessApiVersion,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserlessApiVersion {
    #[default]
    V1,
    V2,
}

fn default_sandbox_image() -> String {
    "docker.io/browserless/chrome".to_string()
}

const fn default_low_memory_threshold_mb() -> u64 {
    2048
}

const fn default_persist_profile() -> bool {
    true
}

fn default_container_host() -> String {
    "127.0.0.1".to_string()
}

const fn default_browserless_api_version() -> BrowserlessApiVersion {
    BrowserlessApiVersion::V1
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            chrome_path: None,
            headless: true,
            viewport_width: 2560,
            viewport_height: 1440,
            device_scale_factor: 2.0,
            max_instances: 0, // 0 = unlimited, limited by memory
            memory_limit_percent: 90,
            idle_timeout_secs: 300,
            navigation_timeout_ms: 30000,
            user_agent: None,
            chrome_args: Vec::new(),
            sandbox_image: default_sandbox_image(),
            allowed_domains: Vec::new(),
            low_memory_threshold_mb: default_low_memory_threshold_mb(),
            persist_profile: default_persist_profile(),
            profile_dir: None,
            container_host: default_container_host(),
            browserless_api_version: default_browserless_api_version(),
        }
    }
}

/// Exec tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecConfig {
    pub default_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub approval_mode: String,
    pub security_level: String,
    pub allowlist: Vec<String>,
    pub sandbox: SandboxConfig,
    /// Where to run commands: `"local"` (default), `"node"`, or `"ssh"`.
    pub host: String,
    /// Default node id or display name for remote execution (when `host = "node"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Default SSH target for remote execution (when `host = "ssh"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_target: Option<String>,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            default_timeout_secs: 30,
            max_output_bytes: 200 * 1024,
            approval_mode: "on-miss".into(),
            security_level: "allowlist".into(),
            allowlist: Vec::new(),
            sandbox: SandboxConfig::default(),
            host: "local".into(),
            node: None,
            ssh_target: None,
        }
    }
}

/// Resource limits for sandboxed execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimitsConfig {
    /// Memory limit (e.g. "512M", "1G").
    pub memory_limit: Option<String>,
    /// CPU quota as a fraction (e.g. 0.5 = half a core, 2.0 = two cores).
    pub cpu_quota: Option<f64>,
    /// Maximum number of PIDs.
    pub pids_max: Option<u32>,
}

/// Optional per-tool overrides for WASM fuel and memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolLimitOverrideConfig {
    pub fuel: Option<u64>,
    pub memory: Option<u64>,
}

/// Configurable WASM tool limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WasmToolLimitsConfig {
    pub default_memory: u64,
    pub default_fuel: u64,
    pub tool_overrides: HashMap<String, ToolLimitOverrideConfig>,
}

fn default_wasm_tool_overrides() -> HashMap<String, ToolLimitOverrideConfig> {
    let mb = 1024_u64 * 1024_u64;
    HashMap::from([
        ("calc".to_string(), ToolLimitOverrideConfig {
            fuel: Some(100_000),
            memory: Some(2 * mb),
        }),
        ("web_fetch".to_string(), ToolLimitOverrideConfig {
            fuel: Some(10_000_000),
            memory: Some(32 * mb),
        }),
        ("web_search".to_string(), ToolLimitOverrideConfig {
            fuel: Some(10_000_000),
            memory: Some(32 * mb),
        }),
        ("show_map".to_string(), ToolLimitOverrideConfig {
            fuel: Some(10_000_000),
            memory: Some(64 * mb),
        }),
        ("location".to_string(), ToolLimitOverrideConfig {
            fuel: Some(5_000_000),
            memory: Some(16 * mb),
        }),
    ])
}

impl Default for WasmToolLimitsConfig {
    fn default() -> Self {
        Self {
            default_memory: 16 * 1024 * 1024,
            default_fuel: 1_000_000,
            tool_overrides: default_wasm_tool_overrides(),
        }
    }
}

/// Persistence strategy for `/home/sandbox` in sandbox containers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HomePersistenceConfig {
    Off,
    Session,
    #[default]
    Shared,
}

/// Sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: String,
    pub scope: String,
    pub workspace_mount: String,
    /// Optional host-visible path for Moltis `data_dir()` when creating
    /// sandbox containers from inside another container.
    pub host_data_dir: Option<String>,
    /// Persistence strategy for `/home/sandbox`: off, session, or shared.
    pub home_persistence: HomePersistenceConfig,
    /// Optional host directory for shared `/home/sandbox` persistence.
    /// Relative paths are resolved against `data_dir()`.
    pub shared_home_dir: Option<String>,
    pub image: Option<String>,
    pub container_prefix: Option<String>,
    pub no_network: bool,
    /// Network policy: "blocked" (no network), "trusted" (proxy-filtered), "bypass" (unrestricted, no audit).
    #[serde(default)]
    pub network: String,
    /// Domains allowed through the proxy in `trusted` mode.
    #[serde(default)]
    pub trusted_domains: Vec<String>,
    /// Backend: "auto" (default), "docker", "podman", "apple-container",
    /// "restricted-host", or "wasm".
    /// "auto" prefers Apple Container on macOS, then Podman, then Docker,
    /// then restricted-host. "wasm" uses Wasmtime + WASI for real sandboxed
    /// execution.
    pub backend: String,
    pub resource_limits: ResourceLimitsConfig,
    /// Packages to install via `apt-get` in the sandbox image.
    /// Set to an empty list to skip provisioning.
    #[serde(default = "default_sandbox_packages")]
    pub packages: Vec<String>,
    /// Fuel limit for WASM sandbox execution (default: 1 billion instructions).
    pub wasm_fuel_limit: Option<u64>,
    /// Epoch interruption interval in milliseconds for WASM sandbox (default: 100ms).
    pub wasm_epoch_interval_ms: Option<u64>,
    /// Optional per-tool WASM limits (fuel + memory).
    pub wasm_tool_limits: Option<WasmToolLimitsConfig>,
    /// Optional tool policy overrides applied when running inside this sandbox.
    /// Acts as layer 6 in the policy resolution chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_policy: Option<ToolPolicyConfig>,
}

/// Default packages installed in sandbox containers.
/// Inspired by GitHub Actions runner images — covers commonly needed
/// CLI tools, language runtimes, and utilities for LLM-driven tasks.
fn default_sandbox_packages() -> Vec<String> {
    [
        // Networking & HTTP
        "curl",
        "wget",
        "ca-certificates",
        "dnsutils",
        "netcat-openbsd",
        "openssh-client",
        "iproute2",
        "net-tools",
        // Language runtimes
        "python3",
        "python3-dev",
        "python3-pip",
        "python3-venv",
        "python-is-python3",
        "nodejs",
        "npm",
        "ruby",
        "ruby-dev",
        "golang-go",
        "php-cli",
        "php-mbstring",
        "php-xml",
        "php-curl",
        "default-jdk",
        "maven",
        "perl",
        // Build toolchain & native deps
        "build-essential",
        "clang",
        "libclang-dev",
        "llvm-dev",
        "pkg-config",
        "libssl-dev",
        "libsqlite3-dev",
        "libyaml-dev",
        "liblzma-dev",
        "autoconf",
        "automake",
        "libtool",
        "bison",
        "flex",
        "dpkg-dev",
        "fakeroot",
        "cmake",
        "ninja-build",
        // Compression & archiving
        "zip",
        "unzip",
        "bzip2",
        "xz-utils",
        "p7zip-full",
        "tar",
        "zstd",
        "lz4",
        "pigz",
        // Common CLI utilities (mirrors GitHub runner image)
        "git",
        "gnupg2",
        "jq",
        "rsync",
        "file",
        "tree",
        "sqlite3",
        "sudo",
        "locales",
        "tzdata",
        "shellcheck",
        "patchelf",
        "git-lfs",
        "gettext",
        "lsb-release",
        "software-properties-common",
        "yamllint",
        // Text processing & search
        "ripgrep",
        "fd-find",
        "yq",
        // Terminal multiplexer (useful for capturing ncurses apps)
        "tmux",
        // Browser automation (for browser tool)
        "chromium",
        "libxss1",
        "libnss3",
        "libnspr4",
        "libasound2t64",
        "libatk1.0-0t64",
        "libatk-bridge2.0-0t64",
        "libcups2t64",
        "libdrm2",
        "libgbm1",
        "libgtk-3-0t64",
        "libxcomposite1",
        "libxdamage1",
        "libxfixes3",
        "libxrandr2",
        "libxkbcommon0",
        "fonts-liberation",
        // Image processing (headless)
        "imagemagick",
        "graphicsmagick",
        "libvips-tools",
        "pngquant",
        "optipng",
        "jpegoptim",
        "webp",
        "libimage-exiftool-perl",
        "libheif-dev",
        // Audio / video / media
        "ffmpeg",
        "sox",
        "lame",
        "flac",
        "vorbis-tools",
        "opus-tools",
        "mediainfo",
        // Document & office conversion
        "pandoc",
        "poppler-utils",
        "ghostscript",
        "texlive-latex-base",
        "texlive-latex-extra",
        "texlive-fonts-recommended",
        "antiword",
        "catdoc",
        "unrtf",
        "libreoffice-core",
        "libreoffice-writer",
        // Data processing & conversion
        "csvtool",
        "xmlstarlet",
        "html2text",
        "dos2unix",
        "miller",
        "datamash",
        // Database clients
        "postgresql-client",
        "default-mysql-client",
        // DevOps
        "ansible",
        // GIS / OpenStreetMap / map generation
        "gdal-bin",
        "mapnik-utils",
        "osm2pgsql",
        "osmium-tool",
        "osmctools",
        "python3-mapnik",
        "libgdal-dev",
        // CalDAV / CardDAV
        "vdirsyncer",
        "khal",
        "python3-caldav",
        // Email (IMAP sync, indexing, CLI clients)
        "isync",
        "offlineimap3",
        "notmuch",
        "notmuch-mutt",
        "aerc",
        "mutt",
        "neomutt",
        // Newsgroups (NNTP)
        "tin",
        "slrn",
        // Messaging APIs
        "python3-discord",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: "all".into(),
            scope: "session".into(),
            workspace_mount: "ro".into(),
            host_data_dir: None,
            home_persistence: HomePersistenceConfig::default(),
            shared_home_dir: None,
            image: None,
            container_prefix: None,
            no_network: false,
            network: "trusted".into(),
            trusted_domains: Vec::new(),
            backend: "auto".into(),
            resource_limits: ResourceLimitsConfig::default(),
            packages: default_sandbox_packages(),
            wasm_fuel_limit: None,
            wasm_epoch_interval_ms: None,
            wasm_tool_limits: None,
            tools_policy: None,
        }
    }
}

/// Tool policy configuration (allow/deny lists).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicyConfig {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub profile: Option<String>,
}
