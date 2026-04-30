use serde::{Deserialize, Serialize};

/// Chat configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    /// Automatically generate a session title after the first exchange.
    #[serde(default = "default_auto_title")]
    pub auto_title: bool,
    /// How to handle messages that arrive while an agent run is active.
    #[serde(default = "default_message_queue_mode")]
    pub message_queue_mode: MessageQueueMode,
    /// How `MEMORY.md` is loaded into the prompt for an ongoing session.
    #[serde(default = "default_prompt_memory_mode")]
    pub prompt_memory_mode: PromptMemoryMode,
    /// Maximum characters from each workspace prompt file (`AGENTS.md`, `TOOLS.md`).
    #[serde(default = "default_workspace_file_max_chars")]
    pub workspace_file_max_chars: usize,
    /// Preferred model IDs to show first in selectors (full or raw model IDs).
    pub priority_models: Vec<String>,
    /// Legacy model allowlist. Kept for backward compatibility.
    /// Model visibility is provider-driven (`providers.<name>.models` +
    /// live discovery), so this field is currently ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
    /// Compaction strategy and tuning knobs. See [`CompactionConfig`].
    #[serde(default)]
    pub compaction: CompactionConfig,
}

fn default_auto_title() -> bool {
    true
}

fn default_message_queue_mode() -> MessageQueueMode {
    MessageQueueMode::Followup
}

fn default_prompt_memory_mode() -> PromptMemoryMode {
    PromptMemoryMode::LiveReload
}

fn default_workspace_file_max_chars() -> usize {
    32_000
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            auto_title: default_auto_title(),
            message_queue_mode: default_message_queue_mode(),
            prompt_memory_mode: default_prompt_memory_mode(),
            workspace_file_max_chars: default_workspace_file_max_chars(),
            priority_models: Vec::new(),
            allowed_models: Vec::new(),
            compaction: CompactionConfig::default(),
        }
    }
}

// ── Compaction ────────────────────────────────────────────────────────────

/// Strategy used to shrink a chat session when its context window fills up.
///
/// Each mode trades fidelity against cost. The default is [`CompactionMode::Deterministic`]
/// which is free and offline but lower fidelity than the LLM-backed modes. See
/// [`docs/src/compaction.md`](../../../docs/src/compaction.md) for a full comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionMode {
    /// Replace the entire history with a single extracted summary message.
    ///
    /// Zero LLM calls, zero network I/O, deterministic output. Summarises by
    /// inspecting message structure directly: message counts, tool names,
    /// file-path mentions, recent user requests, keyword-matched pending
    /// work, and a head-3 + tail-5 timeline.
    ///
    /// **Best for:** short chat-bot channels, offline builds, cost-sensitive
    /// deployments, and as a reliable fallback when an LLM isn't available.
    ///
    /// **Weaknesses:** loses reasoning chains, drops middle history entirely,
    /// keyword heuristics miss nuance, recency of tail context is not preserved
    /// verbatim for the retry.
    #[default]
    Deterministic,

    /// Head + middle-prune + tail, with no LLM calls.
    ///
    /// Keeps the first `protect_head` messages verbatim (system prompt +
    /// first exchange), keeps a token-budget tail (default: 20 % of the
    /// context-window threshold) verbatim, and collapses the middle into a
    /// single marker message. Any bulky tool-result content that survives
    /// in the retained slice is replaced with a placeholder. After splicing,
    /// orphaned tool_use / tool_result pairs are repaired so strict
    /// providers accept the retry.
    ///
    /// **Best for:** most agentic coding sessions where recency matters
    /// more than middle history and you want zero token cost.
    ///
    /// **Weaknesses:** cannot merge redundant discussions or preserve
    /// reasoning from the collapsed middle region.
    RecencyPreserving,

    /// Head + LLM-summarised middle + tail using a structured template.
    ///
    /// Head and tail are preserved verbatim (same boundary logic as
    /// [`CompactionMode::RecencyPreserving`]). The middle is summarised
    /// with a single LLM call using the
    /// Goal / Progress / Decisions / Files / Next Steps template used by
    /// hermes-agent and openclaw safeguard. Iterative re-compaction is
    /// automatic: when the first head message is already a compacted
    /// summary, the previous summary body is passed into the prompt so the
    /// model can preserve and update sections instead of re-summarising.
    ///
    /// On LLM failure (error or empty response), automatically falls back
    /// to [`CompactionMode::RecencyPreserving`] so compaction never
    /// silently drops information.
    ///
    /// **Best for:** long agentic coding sessions where losing decisions
    /// and rationale would be expensive, and token budget for a single
    /// summary call is acceptable.
    ///
    /// **Weaknesses:** costs a summary LLM call per compaction; quality
    /// depends on the summary model's instruction-following.
    Structured,

    /// Replace the entire history with a single LLM-generated summary.
    ///
    /// This is the pre-PR-#653 behaviour: stream a plain-text summary of
    /// the conversation from the session's provider, then replace the
    /// history with a single user message containing that summary. No
    /// head/tail preservation.
    ///
    /// **Best for:** maximum token reduction when the session's provider
    /// is cheap and the tail isn't worth preserving (e.g. pure Q&A chat).
    ///
    /// **Weaknesses:** loses recent turns verbatim, lower fidelity than
    /// [`CompactionMode::Structured`] for the same cost.
    LlmReplace,
}

/// Tunable knobs for compaction. Lives under `chat.compaction`.
///
/// Field interpretation depends on the selected [`CompactionMode`]:
/// [`CompactionMode::Deterministic`] only looks at `mode`; the LLM and
/// recency-preserving modes use the full set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    /// Which compaction strategy to use. Default: `deterministic`.
    #[serde(default)]
    pub mode: CompactionMode,

    /// Fraction of the session model's context window at which automatic
    /// compaction fires in `send()`. Also used as the first multiplier for
    /// the verbatim tail budget in `recency_preserving` / `structured`
    /// modes: `tail_tokens = threshold_percent × tail_budget_ratio ×
    /// context_window`.
    ///
    /// Ignored for manual `chat.compact` RPC calls (those always compact
    /// whatever's there).
    ///
    /// The clamp range is `0.1` – `0.95`; out-of-range values log a
    /// validation warning and fall back to the default. Default: `0.95`
    /// to match the pre-PR-#653 hardcoded auto-compact trigger so
    /// upgrades are behaviour-neutral. Users who want earlier
    /// compaction (at the cost of more frequent LLM calls in
    /// `structured` / `llm_replace` modes) should lower this explicitly.
    #[serde(default = "default_compaction_threshold")]
    pub threshold_percent: f32,

    /// Number of head messages preserved verbatim by recency-preserving and
    /// structured modes (system prompt + first exchange). Default: `3`.
    #[serde(default = "default_compaction_protect_head")]
    pub protect_head: u32,

    /// Minimum number of tail messages preserved verbatim as a floor under
    /// the token-budget cut. Default: `20`.
    #[serde(default = "default_compaction_protect_tail_min")]
    pub protect_tail_min: u32,

    /// Size of the tail protection window as a fraction of
    /// `threshold_percent × context_window`. For example, on a 200K model
    /// with the defaults (`threshold_percent = 0.95`,
    /// `tail_budget_ratio = 0.20`) the tail keeps up to 38 000 tokens
    /// verbatim. Default: `0.20`.
    #[serde(default = "default_compaction_tail_ratio")]
    pub tail_budget_ratio: f32,

    /// Tool-result content longer than this many characters is replaced
    /// with a short placeholder when `recency_preserving` / `structured`
    /// modes prune the middle region. Default: `200`.
    #[serde(default = "default_compaction_tool_prune_chars")]
    pub tool_prune_char_threshold: u32,

    /// Provider-qualified model identifier reserved for the auxiliary-
    /// model subsystem (e.g. `"openrouter/google/gemini-2.5-flash"`).
    ///
    /// **Not wired yet** — tracked by beads issue `moltis-8me`. Until
    /// that lands, `structured` and `llm_replace` always use the
    /// session's primary provider regardless of this value. If you set
    /// it today the strategy emits a one-shot WARN naming the field
    /// and the tracking issue so you're not billed for the wrong
    /// model without warning.
    #[serde(default)]
    pub summary_model: Option<String>,

    /// Maximum output tokens reserved for LLM summary calls. Set to `0`
    /// to accept the provider default. Default: `4096`.
    ///
    /// **Not wired yet** — tracked by beads issue `moltis-8me`. Until
    /// that lands, the streaming summary call runs with whatever the
    /// provider's default max-tokens is. Setting this to a non-default
    /// value triggers the same one-shot WARN as `summary_model`.
    #[serde(default = "default_compaction_max_summary_tokens")]
    pub max_summary_tokens: u32,

    /// Whether the "Change `chat.compaction.mode` in moltis.toml…" hint
    /// is included in compaction broadcasts and channel notices. After
    /// you've seen it a few times it tends to become noise; set this to
    /// `false` to strip the hint from future compaction notifications
    /// without disabling the rest of the metadata (mode + token counts
    /// still ship). Default: `true`.
    #[serde(default = "default_compaction_show_settings_hint")]
    pub show_settings_hint: bool,
}

fn default_compaction_threshold() -> f32 {
    // Matches the pre-PR-#653 hardcoded auto-compact trigger of 95 % of
    // the context window so existing deploys see no change in trigger
    // behaviour when they upgrade. Users who want earlier compaction
    // should lower this explicitly in moltis.toml.
    0.95
}

fn default_compaction_protect_head() -> u32 {
    3
}

fn default_compaction_protect_tail_min() -> u32 {
    20
}

fn default_compaction_tail_ratio() -> f32 {
    0.20
}

fn default_compaction_tool_prune_chars() -> u32 {
    200
}

fn default_compaction_max_summary_tokens() -> u32 {
    4_096
}

fn default_compaction_show_settings_hint() -> bool {
    true
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            mode: CompactionMode::default(),
            threshold_percent: default_compaction_threshold(),
            protect_head: default_compaction_protect_head(),
            protect_tail_min: default_compaction_protect_tail_min(),
            tail_budget_ratio: default_compaction_tail_ratio(),
            tool_prune_char_threshold: default_compaction_tool_prune_chars(),
            summary_model: None,
            max_summary_tokens: default_compaction_max_summary_tokens(),
            show_settings_hint: default_compaction_show_settings_hint(),
        }
    }
}

/// Behaviour when `chat.send()` is called during an active run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageQueueMode {
    /// Queue each message; replay them one-by-one after the current run.
    #[default]
    Followup,
    /// Buffer messages; concatenate and process as a single message after the current run.
    Collect,
}

/// How prompt memory is loaded across turns in the same session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptMemoryMode {
    /// Reload `MEMORY.md` from disk before each turn.
    #[default]
    LiveReload,
    /// Freeze the initial `MEMORY.md` content for the lifetime of the session.
    FrozenAtSessionStart,
}

/// How tool schemas are presented to the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolRegistryMode {
    /// All tool schemas are sent to the model on every turn (default).
    #[default]
    Full,
    /// Only `tool_search` is sent; the model discovers and activates tools on demand.
    Lazy,
}

/// Auxiliary model assignments for side tasks.
///
/// Route compression, title generation, and vision to cheaper/faster models
/// while keeping the main session on a more capable model. Falls back to the
/// session's primary provider when a field is `None`.
///
/// ```toml
/// [auxiliary]
/// compaction = "openrouter/google/gemini-2.5-flash"
/// title_generation = "openrouter/google/gemini-2.5-flash"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxiliaryModelsConfig {
    /// Model for context compaction/summarization.
    /// Overrides `chat.compaction.summary_model` when set.
    pub compaction: Option<String>,
    /// Model for session title generation.
    pub title_generation: Option<String>,
    /// Model for vision/image analysis tasks.
    pub vision: Option<String>,
}
