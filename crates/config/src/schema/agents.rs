use {
    super::*,
    serde::{Deserialize, Deserializer, Serialize},
    std::collections::HashMap,
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

impl AgentsConfig {
    /// Return a preset by name.
    pub fn get_preset(&self, name: &str) -> Option<&AgentPreset> {
        self.presets.get(name)
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

/// Per-agent MCP server access control.
///
/// Excludes specific MCP servers' tools from this agent's sessions.
/// Translates to tool policy deny patterns (`mcp__<server>__*`) at
/// resolution time, so the agent never sees those tools in its context.
///
/// ```toml
/// [agents.presets.my-agent.mcp]
/// deny_servers = ["home-assistant"]  # exclude Home Assistant tools
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetMcpPolicy {
    /// MCP servers to deny. Each entry generates a tool deny pattern
    /// `mcp__<server>__*` that blocks all tools from that server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_servers: Vec<String>,
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
    /// Controls which MCP servers are visible to this agent. When set, this
    /// generates tool policy deny patterns for excluded servers, so the agent
    /// never sees their tools in the prompt context.
    #[serde(default)]
    pub mcp: PresetMcpPolicy,
}
