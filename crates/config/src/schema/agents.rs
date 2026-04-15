use {
    super::*,
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// Agent spawn presets used by tools like `spawn_agent`.
///
/// **IMPORTANT:** Everything under `[agents.presets.*]` — including each
/// preset's `tools.allow`/`tools.deny` — applies ONLY to sub-agents spawned
/// via the `spawn_agent` tool. Preset tool policies have no effect on the
/// main agent session. To filter tools for the main session, configure
/// `[tools.policy]` (see `ToolPolicyConfig`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Default preset name used when `spawn_agent.preset` is omitted.
    ///
    /// Applies ONLY to sub-agents spawned via the `spawn_agent` tool. It
    /// does NOT configure tool policy, model, or identity for the main
    /// agent session. For main-session tool allow/deny, use
    /// `[tools.policy]`.
    pub default_preset: Option<String>,
    /// Named spawn presets.
    #[serde(default)]
    pub presets: HashMap<String, AgentPreset>,
}

impl AgentsConfig {
    /// Return a preset by name.
    pub fn get_preset(&self, name: &str) -> Option<&AgentPreset> {
        self.presets.get(name)
    }
}

/// Tool policy for a preset (allow/deny specific tools).
///
/// When both `allow` and `deny` are specified, `allow` acts as a whitelist
/// and `deny` further removes tools from that list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Spawn policy preset for sub-agents.
///
/// Presets allow defining specialized agent configurations that can be
/// selected when spawning sub-agents. Each preset can override identity,
/// model, tool policies, and system prompt.
///
/// **IMPORTANT:** Presets apply ONLY to sub-agents spawned via the
/// `spawn_agent` tool. The `tools.allow`/`tools.deny` fields on a preset
/// do NOT filter tools for the main agent session — the main session's
/// tool policy is controlled by the top-level `[tools.policy]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}
