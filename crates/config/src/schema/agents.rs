use {
    super::*,
    serde::{Deserialize, Deserializer, Serialize},
    std::{
        collections::{HashMap, HashSet},
        path::{Path, PathBuf},
    },
};

const DEFAULT_AGENT_PRESET: &str = "research";

/// Agent presets configure identity, model, and tool policy for agents.
///
/// Each agent persona (including "main") can have a matching preset under
/// `[agents.presets.<agent_id>]`. The preset's `tools.allow`/`tools.deny`
/// applies to **all sessions belonging to that agent** — both the agent's
/// own direct sessions and sub-agents spawned via `spawn_agent`.
///
/// MCP tools appear as `mcp__<server>__<tool>` and can be filtered per-agent
/// via `tools.deny = ["mcp__home-assistant__*"]` on the agent's preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Default preset name used when `spawn_agent.preset` is omitted and
    /// for new sessions when no specific agent is selected. It does NOT
    /// configure tool policy, model, or identity for the main
    /// agent session. For main-session tool allow/deny, use
    /// `[tools.policy]`.
    #[serde(default = "default_preset_name")]
    pub default_preset: Option<String>,
    /// Named spawn presets.
    #[serde(
        default = "default_agent_presets",
        deserialize_with = "deserialize_agent_presets"
    )]
    pub presets: HashMap<String, AgentPreset>,
}

/// Per-request tool choice requested by the agent harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    Any,
    None,
    Tool { name: String },
}

/// Per-agent-run controls for tool visibility and provider tool selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentToolControls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

impl AgentToolControls {
    #[must_use]
    pub fn from_tool_context(tool_context: Option<&serde_json::Value>) -> Self {
        let Some(context) = tool_context else {
            return Self::default();
        };

        let active_tools = context.get("active_tools").and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        });

        let tool_choice =
            context.get("tool_choice").and_then(|value| {
                match serde_json::from_value::<ToolChoice>(value.clone()) {
                    Ok(choice) => Some(choice),
                    Err(error) => {
                        tracing::warn!(%error, "ignoring invalid tool_choice control");
                        None
                    },
                }
            });

        Self {
            active_tools,
            tool_choice,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active_tools.is_none() && self.tool_choice.is_none()
    }
}

impl AgentsConfig {
    /// Return a preset by name.
    pub fn get_preset(&self, name: &str) -> Option<&AgentPreset> {
        self.presets.get(name)
    }

    /// Does this agent's preset force its sandbox on?
    ///
    /// `None` means "whatever the default preset is", the same fallback the
    /// run path takes. The single seat for the question, so every layer that
    /// has to show or enforce it gives the same answer.
    #[must_use]
    pub fn sandbox_forced(&self, agent_id: Option<&str>) -> bool {
        agent_id
            .or(self.default_preset.as_deref())
            .and_then(|id| self.get_preset(id))
            .is_some_and(|preset| preset.sandbox.forces_sandbox())
    }
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            default_preset: default_preset_name(),
            presets: default_agent_presets(),
        }
    }
}

fn default_preset_name() -> Option<String> {
    Some(DEFAULT_AGENT_PRESET.to_string())
}

/// Built-in sub-agent presets available on every install.
///
/// User TOML and markdown definitions with the same key override these
/// defaults during config loading.
#[must_use]
pub fn default_agent_presets() -> HashMap<String, AgentPreset> {
    [
        (
            "research",
            builtin_agent_preset(
                "Researcher",
                "thorough, skeptical, and evidence-oriented",
                "Gather evidence before concluding. Prefer targeted file reads, searches, \
                 web_search, and web_fetch when the answer depends on current or external \
                 facts. Do not edit files unless the task explicitly asks for changes. \
                 Return a concise synthesis with source paths, URLs, commands, and open \
                 questions.",
                Some(16),
                false,
            ),
        ),
        (
            "coder",
            builtin_agent_preset(
                "Coder",
                "pragmatic, idiomatic, and test-focused",
                "Implement scoped code changes. Read the surrounding code first, follow \
                 existing patterns, keep edits small, and remove dead code you directly \
                 replace. Run the smallest relevant verification and report changed files, \
                 validation, and any remaining risk.",
                Some(25),
                false,
            ),
        ),
        (
            "reviewer",
            builtin_agent_preset(
                "Reviewer",
                "precise, skeptical, and security-minded",
                "Review for correctness, regressions, security issues, data loss, and missing \
                 tests. Findings come first, ordered by severity, with concrete file and line \
                 references when available. Do not make edits unless explicitly asked.",
                Some(14),
                false,
            ),
        ),
        (
            "qa",
            builtin_agent_preset(
                "QA",
                "reproducible, evidence-driven, and user-facing",
                "Validate behavior end to end. Reproduce reported bugs, exercise the user \
                 workflow, use browser automation when available, capture useful evidence, \
                 and report exact steps, expected behavior, actual behavior, and pass/fail \
                 status.",
                Some(16),
                false,
            ),
        ),
        (
            "ux",
            builtin_agent_preset(
                "UX Designer",
                "user-centered, accessible, and visually rigorous",
                "Evaluate flows, information architecture, accessibility, visual hierarchy, \
                 copy, responsive behavior, and edge states. Propose concrete changes that \
                 fit the existing design system and call out usability risks without hand-wavy \
                 vibes.",
                Some(14),
                false,
            ),
        ),
        (
            "docs",
            builtin_agent_preset(
                "Docs Writer",
                "clear, accurate, and example-heavy",
                "Update or draft user-facing documentation. Keep docs aligned with behavior, \
                 include runnable examples when useful, verify command names and config keys, \
                 and flag any product behavior that is unclear or undocumented.",
                Some(14),
                false,
            ),
        ),
        (
            "coordinator",
            builtin_agent_preset(
                "Coordinator",
                "structured, concise, and delegation-oriented",
                "Break broad work into independent subtasks, delegate only when useful, track \
                 dependencies, and integrate results into a single answer. Avoid doing \
                 implementation work directly unless coordination is not enough.",
                Some(18),
                true,
            ),
        ),
    ]
    .into_iter()
    .map(|(name, preset)| (name.to_string(), preset))
    .collect()
}

#[must_use]
pub fn is_default_agent_preset(name: &str, preset: &AgentPreset) -> bool {
    default_agent_presets().get(name) == Some(preset)
}

fn deserialize_agent_presets<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, AgentPreset>, D::Error>
where
    D: Deserializer<'de>,
{
    let user_presets = HashMap::<String, AgentPreset>::deserialize(deserializer)?;
    let mut presets = default_agent_presets();
    presets.extend(user_presets);
    Ok(presets)
}

fn builtin_agent_preset(
    display_name: &str,
    theme: &str,
    system_prompt_suffix: &str,
    max_iterations: Option<u64>,
    delegate_only: bool,
) -> AgentPreset {
    AgentPreset {
        identity: AgentIdentity {
            name: Some(display_name.to_string()),
            emoji: None,
            theme: Some(theme.to_string()),
        },
        system_prompt_suffix: Some(system_prompt_suffix.to_string()),
        max_iterations,
        delegate_only,
        ..Default::default()
    }
}

/// Identifies an MCP server by its configuration key.
///
/// Wraps the server name used as the key in `[mcp.servers.<name>]` and
/// in tool names like `mcp__<name>__<tool>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpServerId(String);

impl McpServerId {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Tool-policy deny pattern that blocks all tools from this server.
    #[must_use]
    pub fn to_deny_pattern(&self) -> String {
        format!("mcp__{}__*", self.0)
    }
}

impl std::fmt::Display for McpServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for McpServerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for McpServerId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for McpServerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::borrow::Borrow<str> for McpServerId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Per-agent MCP server access control.
///
/// Controls which MCP servers are visible to this agent. Translates to
/// tool policy deny patterns (`mcp__<server>__*`) at resolution time,
/// so the agent never sees excluded servers' tools in its context.
///
/// ```toml
/// # Allow-list: only these servers are visible
/// [agents.presets.my-agent.mcp]
/// allow_servers = ["github", "memory"]
///
/// # Deny-list: all servers except these
/// [agents.presets.my-agent.mcp]
/// deny_servers = ["home-assistant"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PresetMcpPolicy {
    /// No restrictions — all MCP servers are visible (default).
    #[default]
    All,
    /// Only the listed servers are visible. All others are denied.
    Allow(Vec<McpServerId>),
    /// All servers except the listed ones are visible.
    Deny(Vec<McpServerId>),
}

impl PresetMcpPolicy {
    /// Returns `true` when no MCP restrictions are configured.
    #[must_use]
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

impl Serialize for PresetMcpPolicy {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::All => {
                let map = serializer.serialize_map(Some(0))?;
                map.end()
            },
            Self::Allow(servers) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("allow_servers", servers)?;
                map.end()
            },
            Self::Deny(servers) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("deny_servers", servers)?;
                map.end()
            },
        }
    }
}

impl<'de> Deserialize<'de> for PresetMcpPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use Option to distinguish "field absent" from "field present but empty".
        // `allow_servers = []` means "allow no MCP servers" (deny all),
        // while omitting the field entirely means "no restriction" (All).
        #[derive(Deserialize)]
        struct Raw {
            allow_servers: Option<Vec<McpServerId>>,
            deny_servers: Option<Vec<McpServerId>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        match (raw.allow_servers, raw.deny_servers) {
            (None, None) => Ok(Self::All),
            (Some(servers), None) => Ok(Self::Allow(servers)),
            (None, Some(servers)) => Ok(Self::Deny(servers)),
            (Some(_), Some(_)) => Err(serde::de::Error::custom(
                "mcp: allow_servers and deny_servers are mutually exclusive",
            )),
        }
    }
}

/// Tool policy for an agent preset (allow/deny specific tools).
///
/// Applied as Layer 3 in the 6-layer policy resolution for all sessions
/// belonging to this agent. When both `allow` and `deny` are specified,
/// `allow` acts as a whitelist and `deny` further removes from that list.
/// Glob patterns are supported (e.g. `"mcp__*"` to deny all MCP tools).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetToolPolicy {
    /// Tools to allow (whitelist). If empty, all tools are allowed.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Tools to deny (blacklist). Applied after `allow`.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Scope for per-agent persistent memory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// User-global: `~/.moltis/agent-memory/<preset>/`
    #[default]
    User,
    /// Project-local: `.moltis/agent-memory/<preset>/`
    Project,
    /// Untracked local: `.moltis/agent-memory-local/<preset>/`
    Local,
}

/// Persistent memory configuration for a preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetMemoryConfig {
    /// Memory scope: where the MEMORY.md is stored.
    pub scope: MemoryScope,
    /// Maximum lines to load from MEMORY.md (default: 200).
    pub max_lines: usize,
}

impl Default for PresetMemoryConfig {
    fn default() -> Self {
        Self {
            scope: MemoryScope::default(),
            max_lines: 200,
        }
    }
}

/// Session access policy configuration for a preset.
///
/// Controls which sessions an agent can see and interact with via
/// the `sessions_list`, `sessions_history`, and `sessions_send` tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionAccessPolicyConfig {
    /// Only see sessions with keys matching this prefix.
    pub key_prefix: Option<String>,
    /// Explicit session keys this agent can access (in addition to prefix).
    #[serde(default)]
    pub allowed_keys: Vec<String>,
    /// Whether the agent can send messages to sessions.
    #[serde(default = "default_true")]
    pub can_send: bool,
    /// Whether the agent can access sessions from other agents.
    #[serde(default)]
    pub cross_agent: bool,
}

impl Default for SessionAccessPolicyConfig {
    fn default() -> Self {
        Self {
            key_prefix: None,
            allowed_keys: Vec::new(),
            can_send: true,
            cross_agent: false,
        }
    }
}

/// Per-agent sandbox mode override.
///
/// Only `mode` is enforced at runtime (applied as a per-session override
/// on the `SandboxRouter`). Per-session network/workspace/resource
/// overrides require deeper `SandboxRouter` changes and will be added
/// when the router gains per-session config overlays.
///
/// ```toml
/// [agents.presets.kids.sandbox]
/// mode = "all"
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetSandboxMode {
    /// Disable sandboxing for this agent.
    Off,
    /// Sandbox every session for this agent.
    All,
    /// Inherit the global non-main session sandbox behavior.
    NonMain,
}

impl TryFrom<&str> for PresetSandboxMode {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "off" => Ok(Self::Off),
            "all" => Ok(Self::All),
            "non-main" => Ok(Self::NonMain),
            other => Err(format!("unknown sandbox mode: {other}")),
        }
    }
}

/// Access mode for a per-agent sandbox mount.
///
/// Deliberately a real enum rather than a string: an unrecognised value is a
/// deserialization error, unlike `tools.exec.sandbox.mode`, which maps any
/// unknown string to `off` and so fails open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMountAccess {
    /// Read-only bind mount. The default.
    #[default]
    Ro,
    /// Read-write bind mount.
    Rw,
}

impl SandboxMountAccess {
    /// The wire spelling, as used in the `source:target:access` triple.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ro => "ro",
            Self::Rw => "rw",
        }
    }

    /// Returns `true` when this mount grants write access to the host path.
    #[must_use]
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Rw)
    }
}

impl TryFrom<&str> for SandboxMountAccess {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "ro" => Ok(Self::Ro),
            "rw" => Ok(Self::Rw),
            other => Err(format!(
                "unknown sandbox mount access: {other} (expected \"ro\" or \"rw\")"
            )),
        }
    }
}

/// One extra host path bound into an agent's sandbox container.
///
/// ```toml
/// [[agents.presets.walter.sandbox.mounts]]
/// source = "/home/me/vault"
/// target = "/home/me/vault"
/// access = "rw"
/// ```
///
/// `source` and `target` are required: a missing one is a deserialization
/// error rather than an empty string, because an empty source is exactly the
/// value Docker turns into a silent named volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxMountConfig {
    /// Absolute host path to bind into the sandbox.
    pub source: String,
    /// Absolute path inside the sandbox to bind it at.
    pub target: String,
    /// Access mode; defaults to read-only.
    #[serde(default)]
    pub access: SandboxMountAccess,
}

impl SandboxMountConfig {
    /// Parse one `source:target:access` triple.
    ///
    /// This is the single wire spelling: RPC arrays and markdown frontmatter
    /// both use it, so a value written through one surface reads back through
    /// the other. A wrong field count is an error, never a partial mount.
    pub fn parse_triple(value: &str) -> Result<Self, String> {
        let parts = value.split(':').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(format!(
                "sandbox mount must be \"source:target:access\", got {value:?}"
            ));
        }
        let source = parts[0].trim();
        let target = parts[1].trim();
        if source.is_empty() {
            return Err(format!("sandbox mount {value:?} has an empty source"));
        }
        if target.is_empty() {
            return Err(format!("sandbox mount {value:?} has an empty target"));
        }
        Ok(Self {
            source: source.to_string(),
            target: target.to_string(),
            access: SandboxMountAccess::try_from(parts[2].trim())?,
        })
    }

    /// Render this mount back into its `source:target:access` triple.
    #[must_use]
    pub fn to_triple(&self) -> String {
        format!("{}:{}:{}", self.source, self.target, self.access.as_str())
    }
}

/// Validate a whole set of configured sandbox mounts.
///
/// One entry point so every layer that can see the config shape enforces the
/// same rules. It runs the per-mount shape rules and then the one rule a
/// per-mount check structurally cannot see: two mounts sharing a target.
///
/// Only rules that need no runtime knowledge live here. Collisions with
/// moltis-owned container paths need `moltis-tools` constants and are checked
/// there instead.
///
/// Returns every problem found rather than the first, so `moltis config check`
/// reports them all in one pass.
///
/// # Errors
///
/// Returns the list of human-readable problems when the set is not usable.
pub fn check_mount_set(mounts: &[SandboxMountConfig]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for mount in mounts {
        check_mount_path("source", &mount.source, &mut errors);
        check_mount_path("target", &mount.target, &mut errors);
        check_mount_source(&mount.source, &mut errors);
    }

    // Normalized, so `/srv/a` and `/srv/a/` are the one target they are to the
    // container runtime rather than two that silently shadow each other.
    let mut seen = HashSet::new();
    for mount in mounts {
        if !seen.insert(normalize_mount_path(&mount.target)) {
            errors.push(format!(
                "two sandbox mounts share the target {:?}",
                mount.target
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Normalize an absolute mount path for comparison.
///
/// Purely lexical - these are paths inside a container that does not exist
/// yet, so there is nothing to canonicalize against - but that is enough for
/// the rules below, because it is exactly the resolution the container runtime
/// does before it sees the path. Comparing the raw string instead lets `/.`,
/// `//` and a trailing slash walk straight through a rule that rejects `/` or
/// `/home/sandbox`.
#[must_use]
pub fn normalize_mount_path(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len() + 1);
    normalized.push('/');
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if normalized.len() > 1 {
            normalized.push('/');
        }
        normalized.push_str(segment);
    }
    normalized
}

/// Is `ancestor` a proper prefix directory of `path`? Both must be normalized.
#[must_use]
pub fn is_mount_path_ancestor(ancestor: &str, path: &str) -> bool {
    ancestor != path
        && path.starts_with(ancestor)
        && (ancestor == "/" || path.as_bytes().get(ancestor.len()) == Some(&b'/'))
}

/// A host path a sandbox mount source may not name.
struct DeniedSource {
    /// Normalized absolute path.
    path: String,
    /// What it is, for the error message.
    what: &'static str,
}

/// File names that are a container-runtime socket wherever they sit.
///
/// Matched on the name as well as on the paths below, because the socket is
/// wherever `DOCKER_HOST` or `CONTAINER_HOST` says it is. A container that can
/// talk to one can start a sibling container with any mount at all, which is
/// root on the host by a slightly longer route - and for podman it also walks
/// straight past the `allow_nested_podman` gate, which only decides whether
/// *moltis* passes the socket in.
const DENIED_SOCKET_FILE_NAMES: &[&str] = &["docker.sock", "podman.sock"];

/// The host device tree, denied as a whole but with named exceptions.
const DEV_DIR: &str = "/dev";

/// Device nodes the `/dev` rule makes an exception for.
///
/// Denying the whole subtree left no way to pass a device through and no
/// escape hatch to ask for one, which is not a security property - it just
/// pushes the operator onto a global `[tools.exec.sandbox]` setting or off the
/// sandbox entirely. These four are the ordinary passthroughs: a GPU, a sound
/// card, KVM, and a tun device. Everything else under `/dev` - `/dev/mem`,
/// `/dev/kmem`, the raw block devices - stays denied, because those are the
/// host, not a peripheral.
const ALLOWED_DEVICE_MOUNT_SOURCES: &[&str] = &["/dev/dri", "/dev/snd", "/dev/kvm", "/dev/net/tun"];

/// The fixed half of the mount-source denylist: the same on every host.
///
/// These are the entries that need no host knowledge to be right, because the
/// path is the same inside a container and outside one.
const DENIED_MOUNT_SOURCES: &[(&str, &str)] = &[
    ("/proc", "the host process table"),
    ("/sys", "the host kernel interface"),
    (DEV_DIR, "the host device tree"),
    // The conventional socket paths, so a source *under* one is refused too.
    // Their parent directories need no entry of their own: a source that is an
    // ancestor of a denied path is refused as well, so `/run/podman` and
    // `/var/run` are covered.
    ("/var/run/docker.sock", "the docker socket"),
    ("/run/docker.sock", "the docker socket"),
    ("/run/podman/podman.sock", "the podman socket"),
    ("/var/run/podman/podman.sock", "the podman socket"),
];

/// The host-specific inputs to the mount-source denylist.
///
/// One struct rather than a row of `Option<&Path>` arguments, so adding an
/// entry cannot silently reorder a call site.
#[derive(Debug, Default, Clone, Copy)]
struct DenyDirs<'a> {
    /// The moltis data directory as *this* process sees it.
    data_dir: Option<&'a Path>,
    /// The same directory as the *host* spells it, when that is known.
    ///
    /// A mount source is a host path the container runtime resolves, so when
    /// moltis itself runs in a container the two spellings are different and
    /// only this one can ever match. See [`denied_mount_sources`] for where it
    /// comes from and when it is absent.
    host_data_dir: Option<&'a Path>,
    /// This process's home directory; `.ssh` under it is denied.
    home_dir: Option<&'a Path>,
    /// `$XDG_RUNTIME_DIR`, where a rootless podman puts its socket.
    runtime_dir: Option<&'a Path>,
}

/// Append one derived entry, skipping a relative path and a duplicate.
fn push_denied(
    denied: &mut Vec<DeniedSource>,
    dir: Option<&Path>,
    suffix: Option<&str>,
    what: &'static str,
) {
    let Some(dir) = dir.filter(|dir| dir.is_absolute()) else {
        return;
    };
    let path = suffix.map_or_else(|| dir.to_path_buf(), |suffix| dir.join(suffix));
    let path = normalize_mount_path(&path.to_string_lossy());
    if denied.iter().any(|entry| entry.path == path) {
        return;
    }
    denied.push(DeniedSource { path, what });
}

/// Build the denylist from paths already resolved by the caller.
///
/// Takes the host-specific directories rather than reading them, so the rule
/// is a pure function of its inputs and can be tested without the
/// process-global data-directory override the loader tests move around.
fn denied_mount_sources_for(dirs: DenyDirs<'_>) -> Vec<DeniedSource> {
    let mut denied: Vec<DeniedSource> = DENIED_MOUNT_SOURCES
        .iter()
        .map(|(path, what)| DeniedSource {
            path: (*path).to_string(),
            what,
        })
        .collect();
    // The data directory holds the credential store, and the credential store
    // holds every API key and provider secret moltis knows.
    push_denied(
        &mut denied,
        dirs.data_dir,
        None,
        "the moltis data directory",
    );
    push_denied(
        &mut denied,
        dirs.host_data_dir,
        None,
        "the moltis data directory as the host sees it",
    );
    push_denied(
        &mut denied,
        dirs.home_dir,
        Some(".ssh"),
        "the user's ssh directory",
    );
    push_denied(
        &mut denied,
        dirs.runtime_dir,
        Some("podman/podman.sock"),
        "the rootless podman socket",
    );
    denied
}

/// Host paths a sandbox mount source may never be, live under, or contain.
///
/// Defence in depth, sitting behind the scope fix that stops a sandboxed agent
/// writing its own preset in the first place. It is the second half of the
/// same answer: the first half stops the agent asking for such a mount over
/// RPC, this one refuses the mount even when the ask arrives from somewhere
/// else - a hand-edited TOML, a markdown sidecar, a future write path nobody
/// has thought of yet.
///
/// # What is and is not enforceable from inside a container
///
/// A mount source is a **host** path: the container runtime resolves it on the
/// host, not in moltis's own mount namespace. The fixed entries above are the
/// same path on both sides, so they hold either way. The derived ones are not,
/// and when moltis itself runs in a container they need a host spelling:
///
/// * The data directory is covered on the host whenever the host spelling is
///   known - `[tools.exec.sandbox] host_data_dir` when it is configured, and
///   otherwise whatever the runtime-mount detection worked out, registered
///   through [`crate::set_host_data_dir_hint`]. With neither, only the
///   container-side path is on the list and the host-side one is **not
///   enforced**; `host_data_dir` is the supported way to close that.
/// * `~/.ssh` is derived from this process's own `$HOME`, which inside a
///   container is the container's home. There is nothing to map it onto: the
///   host user is not knowable from in here. So the ssh entry protects a
///   moltis running directly on the host, and is inert in a containerized
///   deployment. It is kept rather than dropped because the host install is
///   the common one, and it is documented as a caveat rather than left to read
///   like protection it cannot give.
/// * `$XDG_RUNTIME_DIR` is read the same way and carries the same caveat. The
///   fixed `/run/podman/...` entries and the socket file-name rule do not, so
///   a rootless socket bound at its conventional path is still refused.
fn denied_mount_sources() -> Vec<DeniedSource> {
    let data_dir = crate::data_dir();
    let host_data_dir = crate::host_data_dir_hint();
    let home_dir = crate::home_dir();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    denied_mount_sources_for(DenyDirs {
        data_dir: Some(&data_dir),
        host_data_dir: host_data_dir.as_deref(),
        home_dir: home_dir.as_deref(),
        runtime_dir: runtime_dir.as_deref(),
    })
}

/// Refuse mount sources that hand the container the host.
///
/// Sources only. A *target* of `/proc` is the container's own `/proc` and is
/// the container runtime's problem, not a host escape.
fn check_mount_source(value: &str, errors: &mut Vec<String>) {
    check_mount_source_in(value, &denied_mount_sources(), errors);
}

/// Run the denylist over one source, lexically and then through a symlink.
///
/// # Symlinks
///
/// The lexical pass is what the container runtime itself does to the string,
/// and it is the only pass that always runs. It cannot see through a link:
/// `/srv/link -> /run/docker.sock` is not `/run/docker.sock` as a string, and
/// the daemon resolves the link when it mounts.
///
/// So the source is also canonicalized and re-checked - but only where this
/// process can see the path. A moltis running directly on the host sees it and
/// the link is caught. A moltis in a container usually does **not**: the host
/// path is not in its mount namespace, `canonicalize` fails, and the check
/// stays purely lexical. That is the honest limit of this rule, and it is why
/// the mount denylist is defence in depth behind the read-only sandbox API key
/// rather than the boundary itself.
fn check_mount_source_in(value: &str, denied: &[DeniedSource], errors: &mut Vec<String>) {
    let source = normalize_mount_path(value);
    if check_denied_source(value, &source, denied, errors) {
        return;
    }
    let Ok(resolved) = std::fs::canonicalize(&source) else {
        return;
    };
    let resolved = normalize_mount_path(&resolved.to_string_lossy());
    if resolved != source {
        check_denied_source(value, &resolved, denied, errors);
    }
}

/// The denylist itself, over one already-normalized path. `true` if refused.
fn check_denied_source(
    value: &str,
    source: &str,
    denied: &[DeniedSource],
    errors: &mut Vec<String>,
) -> bool {
    let before = errors.len();
    if DENIED_SOCKET_FILE_NAMES.contains(&source.rsplit('/').next().unwrap_or_default()) {
        errors.push(format!(
            "sandbox mount source {value:?} is a container runtime socket ({source:?}); a \
             container that can reach one can start another container with any mount it likes"
        ));
        return true;
    }
    for entry in denied {
        if entry.path == DEV_DIR && is_allowed_device_source(source) {
            continue;
        }
        // Three ways to hit, not one. Equality and "the source is under the
        // denied path" are the obvious pair; "the denied path is under the
        // source" is the one that used to be missing, and it is the one that
        // matters most - a source of `/var/run` or `/home/<user>` is not on
        // the list by name and hands over the socket or the ssh keys anyway.
        let hit = source == entry.path
            || is_mount_path_ancestor(source, &entry.path)
            || is_mount_path_ancestor(&entry.path, source);
        if hit {
            errors.push(format!(
                "sandbox mount source {value:?} is or contains {} ({:?}); binding it into a \
                 sandbox gives the sandbox the host",
                entry.what, entry.path
            ));
        }
    }
    errors.len() != before
}

/// Is this source one of the device nodes the `/dev` rule excepts?
fn is_allowed_device_source(source: &str) -> bool {
    ALLOWED_DEVICE_MOUNT_SOURCES
        .iter()
        .any(|allowed| source == *allowed || is_mount_path_ancestor(allowed, source))
}

fn check_mount_path(field: &str, value: &str, errors: &mut Vec<String>) {
    if value.is_empty() {
        errors.push(format!("sandbox mount {field} must not be empty"));
        return;
    }
    if !value.starts_with('/') {
        errors.push(format!(
            "sandbox mount {field} {value:?} must be an absolute path; a relative source makes \
             Docker create a named volume instead of a bind mount"
        ));
    }
    if value.split('/').any(|segment| segment == "..") {
        errors.push(format!(
            "sandbox mount {field} {value:?} must not contain \"..\""
        ));
    }
    // Normalized, not compared verbatim: `/`, `//` and `/.` are the same
    // directory to Docker, and only the first of the three used to be refused.
    if normalize_mount_path(value) == "/" {
        errors.push(format!(
            "sandbox mount {field} {value:?} must not resolve to \"/\""
        ));
    }
    // Neither separator of the wire shape may appear in a path. Every surface
    // but the structured TOML form spells a mount as `source:target:access`,
    // and the markdown sidecar comma separates the list on top of that -
    // neither is escapable, so a path containing one validates here, renders,
    // and then re-parses as something else entirely. Refused at the one seat
    // every write path shares, rather than encoded around: an encoding would
    // have to be got right in two places and would still not survive being
    // read by a human editing the sidecar.
    for (separator, what) in [(',', "a comma"), (':', "a colon")] {
        if value.contains(separator) {
            errors.push(format!(
                "sandbox mount {field} {value:?} must not contain {what}; the \
                 \"source:target:access\" wire shape and the comma-separated markdown agent \
                 definition cannot express one"
            ));
        }
    }
}

/// Parse and validate a `run_as` value into its `(uid, gid)` pair.
///
/// The shape is exactly `uid:gid`: two non-negative integers separated by one
/// colon, neither part empty, and neither the uid nor the gid is `0`.
///
/// This fails closed on purpose and there is no fallback anywhere above it. A
/// `run_as` that silently meant root would be worse than no field at all: it
/// would hand a writable host mount to a root container while the config says
/// the opposite.
///
/// # Errors
///
/// Returns a human-readable problem when the value is not a usable `uid:gid`.
pub fn parse_run_as(value: &str) -> Result<(u32, u32), String> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(format!("sandbox run_as must be \"uid:gid\", got {value:?}"));
    }
    let uid = parse_run_as_id("uid", parts[0], value)?;
    let gid = parse_run_as_id("gid", parts[1], value)?;
    if uid == 0 {
        return Err(format!(
            "sandbox run_as {value:?} has uid 0; running the sandbox as root is refused, because a \
             run_as that silently meant root would be worse than not setting it"
        ));
    }
    if gid == 0 {
        return Err(format!(
            "sandbox run_as {value:?} has gid 0; the root group is refused for the same reason the \
             root user is - it hands group-writable host paths to the container"
        ));
    }
    Ok((uid, gid))
}

fn parse_run_as_id(field: &str, raw: &str, value: &str) -> Result<u32, String> {
    if raw.is_empty() {
        return Err(format!("sandbox run_as {value:?} has an empty {field}"));
    }
    // Parsed by hand rather than through `u32::from_str`, which accepts a
    // leading `+`. Only plain digits are a uid.
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "sandbox run_as {value:?} has a non-numeric {field} {raw:?}"
        ));
    }
    raw.parse::<u32>()
        .map_err(|error| format!("sandbox run_as {value:?} has an out-of-range {field}: {error}"))
}

/// Validate a configured `run_as` value.
///
/// The sibling of [`check_mount_set`], and the single implementation every
/// layer calls, so the RPC write path, the config file path and the runtime
/// mirror cannot disagree about what a valid `run_as` is.
///
/// # Errors
///
/// Returns a human-readable problem when the value is not a usable `uid:gid`.
pub fn check_run_as(value: &str) -> Result<(), String> {
    parse_run_as(value).map(|_| ())
}

/// Per-agent sandbox policy override.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetSandboxPolicy {
    /// Sandbox mode override: "off", "all", "non-main".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<PresetSandboxMode>,
    /// This agent may never run outside a sandbox.
    ///
    /// The one field that forces the sandbox on. It outranks the per-session
    /// toggle, and the gateway refuses a `sessions.patch` that tries to switch
    /// it off. The sibling fields below are deliberately not part of this:
    /// they live here because they *configure* the sandbox, which is a
    /// different statement from requiring one.
    #[serde(default, skip_serializing_if = "crate::schema::is_false")]
    pub force: bool,
    /// Extra host paths bound into this agent's sandbox container.
    ///
    /// Per-agent only: there is deliberately no global list, because the
    /// runtime `SandboxConfig` is cloned into the backend once at router
    /// construction, so anything living there would be every-agent-always.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<SandboxMountConfig>,
    /// Run this agent's sandbox container as `uid:gid`.
    ///
    /// Unset means the container keeps whatever user its image declares, which
    /// for the sandbox images is root. There is deliberately no way to spell
    /// "root" here: uid `0` is refused at every layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_as: Option<String>,
}

impl PresetSandboxPolicy {
    /// Returns `true` when no overrides are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && !self.force && self.mounts.is_empty() && self.run_as.is_none()
    }

    /// Returns `true` when this agent must always run sandboxed.
    #[must_use]
    pub fn forces_sandbox(&self) -> bool {
        self.force
    }
}

/// Per-agent skill access control.
///
/// ```toml
/// # Only allow specific skills
/// [agents.presets.kids.skills]
/// allow = ["web_search"]
///
/// # Deny specific skills
/// [agents.presets.admin.skills]
/// deny = ["gaming", "social-media"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetSkillPolicy {
    /// When `Some`, only these skills (by name or category) are available.
    /// `Some(vec![])` means "no skills allowed" (deny all).
    /// `None` (absent from config) means "no restriction".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    /// Skills (by name or category) to deny from this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
}

impl PresetSkillPolicy {
    /// Returns `true` when no skill filtering is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.allow.is_none() && self.deny.is_none()
    }
}

/// Agent preset configuration.
///
/// Presets define identity, model, tool policies, and system prompt for an
/// agent. When an agent persona has a matching preset (same ID), the preset's
/// `tools.allow`/`tools.deny` filters tools for **all** sessions belonging
/// to that agent — direct chat, channel messages, and spawned sub-agents.
///
/// The global `[tools.policy]` (Layer 1) always applies first; the preset's
/// tool policy (Layer 3) narrows further. MCP tools can be filtered using
/// `tools.deny = ["mcp__<server>__*"]` patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentPreset {
    /// Agent identity overrides.
    pub identity: AgentIdentity,
    /// Optional model override for this preset.
    pub model: Option<String>,
    /// Tool policy for this preset (allow/deny specific tools).
    pub tools: PresetToolPolicy,
    /// Restrict sub-agent to delegation/session/task tools only.
    #[serde(default)]
    pub delegate_only: bool,
    /// Per-turn tool visibility and provider tool-choice controls.
    #[serde(default, skip_serializing_if = "AgentToolControls::is_empty")]
    pub tool_controls: AgentToolControls,
    /// Optional extra instructions appended to sub-agent system prompt.
    pub system_prompt_suffix: Option<String>,
    /// Maximum iterations for agent loop.
    pub max_iterations: Option<u64>,
    /// Timeout in seconds for the sub-agent.
    pub timeout_secs: Option<u64>,
    /// Session access policy for inter-agent communication.
    pub sessions: Option<SessionAccessPolicyConfig>,
    /// Persistent per-agent memory configuration.
    pub memory: Option<PresetMemoryConfig>,
    /// Reasoning/thinking effort level for models that support extended thinking.
    ///
    /// Controls extended thinking for models that support it (e.g. Claude Opus,
    /// OpenAI o-series). Higher values enable deeper reasoning but increase
    /// latency and token usage.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Per-agent MCP server access control.
    ///
    /// Controls which MCP servers are visible to this agent:
    /// - `All` (default) — no restrictions, all MCP servers visible.
    /// - `Allow(servers)` — only listed servers visible; others denied.
    /// - `Deny(servers)` — all servers visible except listed ones.
    #[serde(default, skip_serializing_if = "PresetMcpPolicy::is_all")]
    pub mcp: PresetMcpPolicy,
    /// Per-agent sandbox policy overrides.
    ///
    /// Each set field overrides the global `[tools.exec.sandbox]` value.
    /// Unset fields inherit the global config.
    #[serde(default, skip_serializing_if = "PresetSandboxPolicy::is_empty")]
    pub sandbox: PresetSandboxPolicy,
    /// Per-agent skill access control.
    ///
    /// Controls which skills are visible to this agent. When `allow` is
    /// non-empty, only listed skills are available. `deny` removes skills
    /// by name or category.
    #[serde(default, skip_serializing_if = "PresetSkillPolicy::is_empty")]
    pub skills: PresetSkillPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_controls_parse_from_tool_context() {
        let context = serde_json::json!({
            "active_tools": ["classify_destination", "send_document"],
            "tool_choice": { "type": "tool", "name": "classify_destination" }
        });

        let controls = AgentToolControls::from_tool_context(Some(&context));

        assert_eq!(
            controls.active_tools,
            Some(vec![
                "classify_destination".to_string(),
                "send_document".to_string(),
            ])
        );
        assert_eq!(
            controls.tool_choice,
            Some(ToolChoice::Tool {
                name: "classify_destination".to_string(),
            })
        );
    }

    #[test]
    fn tool_controls_parse_any_variant() {
        let context = serde_json::json!({
            "tool_choice": { "type": "any" }
        });
        let controls = AgentToolControls::from_tool_context(Some(&context));
        assert_eq!(controls.tool_choice, Some(ToolChoice::Any));
        assert!(controls.active_tools.is_none());
    }

    fn mount(source: &str) -> SandboxMountConfig {
        SandboxMountConfig {
            source: source.to_string(),
            target: "/mnt/x".to_string(),
            access: SandboxMountAccess::Ro,
        }
    }

    fn mount_errors(source: &str) -> Vec<String> {
        check_mount_set(&[mount(source)]).err().unwrap_or_default()
    }

    #[test]
    fn denied_mount_sources_are_refused() {
        for source in [
            "/proc",
            "/proc/self",
            "/sys",
            "/sys/fs/cgroup",
            "/dev",
            "/dev/mem",
            "/var/run/docker.sock",
            "/run/docker.sock",
            "/home/someone/.docker/run/docker.sock",
        ] {
            assert!(
                !mount_errors(source).is_empty(),
                "expected {source:?} to be refused as a mount source"
            );
        }
    }

    #[test]
    fn denied_mount_sources_are_matched_after_normalization() {
        // The whole point of going through `normalize_mount_path` rather than
        // comparing strings: these are all `/proc` to the container runtime.
        for source in ["/proc/", "//proc", "/./proc", "/proc/./self"] {
            assert!(
                !mount_errors(source).is_empty(),
                "expected {source:?} to normalize onto the denylist"
            );
        }
    }

    /// The denylist every host-directory test shares, from fixed inputs.
    ///
    /// Fixed rather than read from `crate::data_dir()`, because the loader
    /// tests move the process-global data-directory override around while this
    /// runs, and a rule that reads it mid-assertion races them.
    fn test_deny_dirs() -> DenyDirs<'static> {
        DenyDirs {
            data_dir: Some(Path::new("/var/lib/moltis")),
            host_data_dir: Some(Path::new("/srv/host/moltis-data")),
            home_dir: Some(Path::new("/home/tester")),
            runtime_dir: Some(Path::new("/run/user/1000")),
        }
    }

    /// Errors for one source against a denylist built from fixed directories.
    fn host_dir_mount_errors(source: &str) -> Vec<String> {
        let denied = denied_mount_sources_for(test_deny_dirs());
        let mut errors = Vec::new();
        check_mount_source_in(source, &denied, &mut errors);
        errors
    }

    #[test]
    fn the_data_directory_and_ssh_directory_are_refused() {
        for source in [
            "/var/lib/moltis",
            "/var/lib/moltis/auth.db",
            "/home/tester/.ssh",
            "/home/tester/.ssh/id_ed25519",
        ] {
            assert!(
                !host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to be refused: the data directory holds the credential \
                 store and the ssh directory holds host keys"
            );
        }
        // A sibling under the home directory is not under `.ssh`.
        assert!(host_dir_mount_errors("/home/tester/media").is_empty());
        // And a relative data directory contributes no rule rather than a
        // rule that matches everything.
        assert!(
            denied_mount_sources_for(DenyDirs {
                data_dir: Some(Path::new(".moltis")),
                ..DenyDirs::default()
            })
            .len()
                == DENIED_MOUNT_SOURCES.len()
        );
    }

    #[test]
    fn the_host_side_data_directory_is_refused() {
        // The entry that makes the data-directory rule mean anything when
        // moltis itself runs in a container: a mount source is a host path, so
        // the container-side `/var/lib/moltis` above can never be written by
        // an operator configuring a sibling container.
        for source in ["/srv/host/moltis-data", "/srv/host/moltis-data/auth.db"] {
            assert!(
                !host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to be refused: it is the data directory as the host \
                 spells it"
            );
        }
    }

    #[test]
    fn podman_sockets_are_refused() {
        // Podman is the preferred auto-detected backend, and its socket
        // bypasses `allow_nested_podman` exactly the way the docker one
        // bypasses everything.
        for source in [
            "/run/podman/podman.sock",
            "/var/run/podman/podman.sock",
            "/run/user/1000/podman/podman.sock",
            "/home/tester/.local/share/containers/podman.sock",
        ] {
            assert!(
                !host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to be refused as a podman socket"
            );
        }
    }

    #[test]
    fn a_source_that_contains_a_denied_path_is_refused() {
        // The direction that used to be missing. None of these is on the list
        // by name, and each hands over what is under it.
        for source in [
            "/var/run",
            "/run",
            "/run/podman",
            "/home/tester",
            "/var/lib",
            "/srv/host",
        ] {
            assert!(
                !host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to be refused: it contains a denied path"
            );
        }
    }

    #[test]
    fn device_nodes_stay_mountable_while_dev_does_not() {
        for source in [
            "/dev/dri",
            "/dev/dri/card0",
            "/dev/snd",
            "/dev/kvm",
            "/dev/net/tun",
        ] {
            assert!(
                host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to stay mountable: it is a device passthrough, got {:?}",
                host_dir_mount_errors(source)
            );
        }
        for source in ["/dev", "/dev/mem", "/dev/sda"] {
            assert!(
                !host_dir_mount_errors(source).is_empty(),
                "expected {source:?} to stay refused: it is the host, not a peripheral"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_source_is_refused_where_the_link_is_visible() {
        // The lexical pass cannot see this: the string is not `/proc`, and the
        // container runtime resolves the link when it mounts.
        let Ok(dir) = tempfile::tempdir() else {
            return;
        };
        let link = dir.path().join("link");
        assert!(std::os::unix::fs::symlink("/proc", &link).is_ok());
        let source = link.to_string_lossy().to_string();
        assert!(
            !host_dir_mount_errors(&source).is_empty(),
            "expected the symlink {source:?} to be refused through its target"
        );
    }

    #[test]
    fn ordinary_mount_sources_still_pass() {
        for source in ["/srv/media", "/mnt/data", "/opt/shared"] {
            assert!(
                mount_errors(source).is_empty(),
                "expected {source:?} to stay usable, got {:?}",
                mount_errors(source)
            );
        }
        // Neighbours of a denied prefix are not under it.
        assert!(mount_errors("/system").is_empty());
        assert!(mount_errors("/development").is_empty());
    }

    #[test]
    fn tool_controls_none_context_returns_default() {
        let controls = AgentToolControls::from_tool_context(None);
        assert!(controls.is_empty());
    }
}
