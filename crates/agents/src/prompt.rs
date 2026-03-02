use {
    crate::tool_registry::ToolRegistry,
    moltis_config::{
        AgentIdentity, DEFAULT_SOUL, MemoryBootstrapSectionOptions, PromptProfileConfig,
        PromptSectionId, PromptSectionOptions, RuntimeDatetimeTailMode, RuntimeSectionOptions,
        UserDetailsMode, UserDetailsSectionOptions, UserProfile,
    },
    moltis_skills::types::SkillMetadata,
    std::collections::{HashMap, HashSet},
};

// ── Model family detection ──────────────────────────────────────────────────

/// Broad model family classification, used to tune text-based tool prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    Llama,
    Qwen,
    Mistral,
    DeepSeek,
    Gemma,
    Phi,
    Unknown,
}

impl ModelFamily {
    /// Detect the model family from a model identifier string.
    #[must_use]
    pub fn from_model_id(id: &str) -> Self {
        let lower = id.to_ascii_lowercase();
        if lower.contains("llama") {
            Self::Llama
        } else if lower.contains("qwen") {
            Self::Qwen
        } else if lower.contains("mistral") || lower.contains("mixtral") {
            Self::Mistral
        } else if lower.contains("deepseek") {
            Self::DeepSeek
        } else if lower.contains("gemma") {
            Self::Gemma
        } else if lower.contains("phi") {
            Self::Phi
        } else {
            Self::Unknown
        }
    }
}

/// Runtime context for the host process running the current agent turn.
#[derive(Debug, Clone, Default)]
pub struct PromptHostRuntimeContext {
    pub host: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub shell: Option<String>,
    /// Current datetime string for prompt context, localized when timezone is known.
    pub time: Option<String>,
    /// Current date string (`YYYY-MM-DD`) for prompt context.
    pub today: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_key: Option<String>,
    /// Runtime surface the assistant is currently operating in
    /// (for example: "web", "telegram", "discord", "cron", "heartbeat").
    pub surface: Option<String>,
    /// High-level session kind (`web`, `channel`, `cron`).
    pub session_kind: Option<String>,
    /// Active channel type when running in a channel-bound session.
    pub channel_type: Option<String>,
    /// Active channel account identifier when running in a channel-bound session.
    pub channel_account_id: Option<String>,
    /// Active channel chat/recipient ID when running in a channel-bound session.
    pub channel_chat_id: Option<String>,
    /// Best-effort channel chat type (for example `private`, `group`, `channel`).
    pub channel_chat_type: Option<String>,
    /// Persistent Moltis workspace root (`data_dir`), e.g. `~/.moltis`
    /// or `/home/moltis/.moltis` in containerized deploys.
    pub data_dir: Option<String>,
    pub sudo_non_interactive: Option<bool>,
    pub sudo_status: Option<String>,
    pub timezone: Option<String>,
    pub accept_language: Option<String>,
    pub remote_ip: Option<String>,
    /// `"lat,lon"` (e.g. `"48.8566,2.3522"`) from browser geolocation or `USER.md`.
    pub location: Option<String>,
}

/// Runtime context for sandbox execution routing used by the `exec` tool.
#[derive(Debug, Clone, Default)]
pub struct PromptSandboxRuntimeContext {
    pub exec_sandboxed: bool,
    pub mode: Option<String>,
    pub backend: Option<String>,
    pub scope: Option<String>,
    pub image: Option<String>,
    /// Sandbox HOME directory used for `~` and relative paths in `exec`.
    pub home: Option<String>,
    pub workspace_mount: Option<String>,
    /// Mounted workspace/data path inside sandbox when available.
    pub workspace_path: Option<String>,
    pub no_network: Option<bool>,
    /// Per-session override for sandbox enablement.
    pub session_override: Option<bool>,
}

/// Combined runtime context injected into the system prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptRuntimeContext {
    pub host: PromptHostRuntimeContext,
    pub sandbox: Option<PromptSandboxRuntimeContext>,
}

/// Suffix appended to the system prompt when the user's reply medium is voice.
///
/// Instructs the LLM to produce speech-friendly output: no raw URLs, no markdown
/// formatting, concise conversational prose. This is Layer 1 of the voice-friendly
/// response pipeline; Layer 2 (`sanitize_text_for_tts`) catches anything the model
/// misses.
pub const VOICE_REPLY_SUFFIX: &str = "\n\n\
## Voice Reply Mode\n\n\
The user is speaking to you via voice messages. Their messages are transcribed from \
speech-to-text, so treat this as a spoken conversation. You will hear their words as \
text, and your response will be converted to spoken audio for them.\n\n\
Write for speech, not for reading:\n\
- Use natural, conversational sentences. No bullet lists, numbered lists, or headings.\n\
- NEVER include raw URLs. Instead describe the resource by name \
(e.g. \"the Rust documentation website\" instead of \"https://doc.rust-lang.org\").\n\
- No markdown formatting: no bold, italic, headers, code fences, or inline backticks.\n\
- Spell out abbreviations that a text-to-speech engine might mispronounce \
(e.g. \"API\" → \"A-P-I\", \"CLI\" → \"C-L-I\").\n\
- Keep responses concise — two to three short paragraphs at most.\n\
- Use complete sentences and natural transitions between ideas.\n";

/// Build the system prompt for an agent run, including available tools.
///
/// When `native_tools` is true, tool schemas are sent via the API's native
/// tool-calling mechanism (e.g. OpenAI function calling, Anthropic tool_use).
/// When false, tools are described in the prompt itself and the LLM is
/// instructed to emit tool calls as JSON blocks that the runner can parse.
pub fn build_system_prompt(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
) -> String {
    build_system_prompt_with_session_runtime(
        tools,
        native_tools,
        project_context,
        &[],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
}

/// Build the system prompt with explicit runtime context.
pub fn build_system_prompt_with_session_runtime(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
) -> String {
    build_system_prompt_with_profile(
        tools,
        native_tools,
        project_context,
        skills,
        identity,
        user,
        soul_text,
        agents_text,
        tools_text,
        runtime_context,
        memory_text,
        None,
        false,
    )
}

/// Build the system prompt with explicit runtime context and profile overrides.
pub fn build_system_prompt_with_profile(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    profile: Option<&PromptProfileConfig>,
    voice_reply_mode: bool,
) -> String {
    build_system_prompt_full(
        tools,
        native_tools,
        project_context,
        skills,
        identity,
        user,
        soul_text,
        agents_text,
        tools_text,
        runtime_context,
        true, // include_tools
        memory_text,
        profile,
        voice_reply_mode,
    )
}

/// Build a minimal system prompt with explicit runtime context.
pub fn build_system_prompt_minimal_runtime(
    project_context: Option<&str>,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
) -> String {
    build_system_prompt_minimal_with_profile(
        project_context,
        identity,
        user,
        soul_text,
        agents_text,
        tools_text,
        runtime_context,
        memory_text,
        None,
        false,
    )
}

/// Build a minimal prompt with explicit runtime context and profile overrides.
pub fn build_system_prompt_minimal_with_profile(
    project_context: Option<&str>,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    profile: Option<&PromptProfileConfig>,
    voice_reply_mode: bool,
) -> String {
    build_system_prompt_full(
        &ToolRegistry::new(),
        true,
        project_context,
        &[],
        identity,
        user,
        soul_text,
        agents_text,
        tools_text,
        runtime_context,
        false, // include_tools
        memory_text,
        profile,
        voice_reply_mode,
    )
}

/// Returns the default prompt template string used for copy/share workflows.
#[must_use]
pub fn default_prompt_template() -> &'static str {
    DEFAULT_PROMPT_TEMPLATE
}

/// Returns the catalog of supported prompt template variables.
#[must_use]
pub fn prompt_template_variables() -> &'static [PromptTemplateVariable] {
    &PROMPT_TEMPLATE_VARIABLES
}

/// Maximum number of characters from `MEMORY.md` injected into the system
/// prompt to keep the context window manageable.
const MEMORY_BOOTSTRAP_MAX_CHARS: usize = 8_000;
/// Maximum number of characters from project context files (`CLAUDE.md`,
/// project docs, etc.) injected into the prompt.
const PROJECT_CONTEXT_MAX_CHARS: usize = 8_000;
/// Maximum number of characters from each workspace file (`AGENTS.md`,
/// `TOOLS.md`) injected into the prompt.
const WORKSPACE_FILE_MAX_CHARS: usize = 6_000;
const EXEC_ROUTING_GUIDANCE: &str = "Execution routing:\n\
- `exec` runs inside sandbox when `Sandbox(exec): enabled=true`.\n\
- When sandbox is disabled, `exec` runs on the host and may require approval.\n\
- In sandbox mode, `~` and relative paths resolve under `Sandbox(exec): home=...` (usually `/home/sandbox`).\n\
- Persistent workspace files live under `Host: data_dir=...`; when mounted, the same path appears as `Sandbox(exec): workspace_path=...`.\n\
- `Host: sudo_non_interactive=true` means non-interactive sudo is available.\n\
- Sandbox/host routing changes are expected runtime behavior. Do not frame them as surprising or anomalous.\n\n";
/// Build model-family-aware tool call guidance for text-based tool mode.
fn tool_call_guidance(model_id: Option<&str>) -> String {
    let _family = model_id
        .map(ModelFamily::from_model_id)
        .unwrap_or(ModelFamily::Unknown);

    let mut g = String::with_capacity(800);
    g.push_str("## How to call tools\n\n");
    g.push_str("When you need to use a tool, output EXACTLY this fenced block:\n\n");
    g.push_str("```tool_call\n");
    g.push_str("{\"tool\": \"<tool_name>\", \"arguments\": {<arguments>}}\n");
    g.push_str("```\n\n");
    g.push_str("**Rules:**\n");
    g.push_str("- The JSON must be valid. No comments, no trailing commas.\n");
    g.push_str("- One tool call per fenced block. You may include multiple blocks.\n");
    g.push_str("- Wait for the tool result before continuing.\n");
    g.push_str("- You may include brief reasoning text before the block.\n\n");

    // Few-shot example
    g.push_str("**Example:**\n");
    g.push_str("User: What files are in the current directory?\n");
    g.push_str("Assistant: I'll list the files for you.\n");
    g.push_str("```tool_call\n");
    g.push_str("{\"tool\": \"exec\", \"arguments\": {\"command\": \"ls -la\"}}\n");
    g.push_str("```\n\n");

    g
}

/// Format a tool schema in compact human-readable form for text-mode prompts.
///
/// Output: `### tool_name\ndescription\nParams: param1 (type, required), param2 (type)\n`
///
/// This is much shorter than dumping full JSON schema, saving ~60% context tokens.
fn format_compact_tool_schema(schema: &serde_json::Value) -> String {
    let name = schema["name"].as_str().unwrap_or("unknown");
    let desc = schema["description"].as_str().unwrap_or("");
    let params = &schema["parameters"];

    let mut out = format!("### {name}\n{desc}\n");

    if let Some(properties) = params.get("properties").and_then(|v| v.as_object()) {
        let required: Vec<&str> = params
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut param_parts: Vec<String> = Vec::with_capacity(properties.len());
        for (param_name, param_schema) in properties {
            let type_str = param_schema
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            if required.contains(&param_name.as_str()) {
                param_parts.push(format!("{param_name} ({type_str}, required)"));
            } else {
                param_parts.push(format!("{param_name} ({type_str})"));
            }
        }

        if !param_parts.is_empty() {
            out.push_str("Params: ");
            out.push_str(&param_parts.join(", "));
            out.push('\n');
        }
    }
    out.push('\n');
    out
}
const TOOL_GUIDELINES: &str = concat!(
    "## Guidelines\n\n",
    "- Start with a normal conversational response. Do not call tools for greetings, small talk, ",
    "or questions you can answer directly.\n",
    "- Use the calc tool for arithmetic and expressions.\n",
    "- Use the exec tool for shell/system tasks.\n",
    "- If the user starts a message with `/sh `, run it with `exec` exactly as written.\n",
    "- Use the browser tool when the user asks to visit/read/interact with web pages.\n",
    "- Before tool calls, briefly state what you are about to do.\n",
    "- For multi-step tasks, execute one step at a time and check results before proceeding.\n",
    "- Be careful with destructive operations, confirm with the user first.\n",
    "- Do not express surprise about sandbox vs host execution. Route changes are normal.\n",
    "- Do not suggest disabling sandbox unless the user explicitly asks for host execution or ",
    "the task cannot be completed in sandbox.\n",
    "- The UI already shows raw tool output (stdout/stderr/exit). Summarize outcomes instead.\n\n",
    "## Silent Replies\n\n",
    "When you have nothing meaningful to add after a tool call, return an empty response.\n",
);
const MINIMAL_GUIDELINES: &str = concat!(
    "## Guidelines\n\n",
    "- Be helpful, accurate, and concise.\n",
    "- If you don't know something, say so rather than making things up.\n",
    "- For coding questions, provide clear explanations with examples.\n",
);
const DEFAULT_TOOLS_PROMPT_PREFIX: &str =
    "You are a helpful assistant. You can use tools when needed.\n\n";
const DEFAULT_MINIMAL_PROMPT_PREFIX: &str =
    "You are a helpful assistant. Answer questions clearly and concisely.\n\n";
const DEFAULT_PROMPT_TEMPLATE: &str = concat!(
    "{{default_prefix}}",
    "{{stable_sections}}",
    "{{dynamic_tail_sections}}",
);

#[derive(Debug, Clone, Copy)]
pub struct PromptTemplateVariable {
    pub name: &'static str,
    pub description: &'static str,
}

const PROMPT_TEMPLATE_VARIABLES: [PromptTemplateVariable; 44] = [
    PromptTemplateVariable {
        name: "default_prompt",
        description: "Default full prompt generated from section toggles/order.",
    },
    PromptTemplateVariable {
        name: "default_prefix",
        description: "Default leading sentence for tools/minimal modes.",
    },
    PromptTemplateVariable {
        name: "default_sections",
        description: "All rendered sections (stable + dynamic tail).",
    },
    PromptTemplateVariable {
        name: "stable_sections",
        description: "Rendered stable section block.",
    },
    PromptTemplateVariable {
        name: "dynamic_tail_sections",
        description: "Rendered dynamic tail section block.",
    },
    PromptTemplateVariable {
        name: "profile_name",
        description: "Resolved prompt profile name.",
    },
    PromptTemplateVariable {
        name: "profile_description",
        description: "Resolved prompt profile description.",
    },
    PromptTemplateVariable {
        name: "tool_count",
        description: "Number of available tools.",
    },
    PromptTemplateVariable {
        name: "flag_include_tools",
        description: "Whether tool sections are enabled for this request.",
    },
    PromptTemplateVariable {
        name: "flag_native_tools",
        description: "Whether provider-native tool calling is active.",
    },
    PromptTemplateVariable {
        name: "flag_voice_reply_mode",
        description: "Whether voice reply mode is active.",
    },
    PromptTemplateVariable {
        name: "assistant_name",
        description: "Assistant identity name.",
    },
    PromptTemplateVariable {
        name: "assistant_emoji",
        description: "Assistant identity emoji.",
    },
    PromptTemplateVariable {
        name: "assistant_creature",
        description: "Assistant identity creature.",
    },
    PromptTemplateVariable {
        name: "assistant_vibe",
        description: "Assistant identity vibe.",
    },
    PromptTemplateVariable {
        name: "user_name",
        description: "User display name.",
    },
    PromptTemplateVariable {
        name: "user_timezone",
        description: "User timezone name.",
    },
    PromptTemplateVariable {
        name: "user_location",
        description: "User location string.",
    },
    PromptTemplateVariable {
        name: "runtime_host",
        description: "Runtime host machine name.",
    },
    PromptTemplateVariable {
        name: "runtime_os",
        description: "Runtime host OS.",
    },
    PromptTemplateVariable {
        name: "runtime_arch",
        description: "Runtime host architecture.",
    },
    PromptTemplateVariable {
        name: "runtime_shell",
        description: "Runtime host shell.",
    },
    PromptTemplateVariable {
        name: "runtime_time",
        description: "Localized runtime datetime string.",
    },
    PromptTemplateVariable {
        name: "runtime_today",
        description: "Runtime date (`YYYY-MM-DD`).",
    },
    PromptTemplateVariable {
        name: "runtime_provider",
        description: "Resolved provider key for this request.",
    },
    PromptTemplateVariable {
        name: "runtime_model",
        description: "Resolved model id for this request.",
    },
    PromptTemplateVariable {
        name: "runtime_session_key",
        description: "Session key for this request.",
    },
    PromptTemplateVariable {
        name: "runtime_data_dir",
        description: "Resolved Moltis data dir path.",
    },
    PromptTemplateVariable {
        name: "runtime_timezone",
        description: "Runtime/user timezone value.",
    },
    PromptTemplateVariable {
        name: "runtime_accept_language",
        description: "Accepted language header from request context.",
    },
    PromptTemplateVariable {
        name: "runtime_remote_ip",
        description: "Remote client IP when available.",
    },
    PromptTemplateVariable {
        name: "runtime_location",
        description: "Runtime location (`lat,lon`) when available.",
    },
    PromptTemplateVariable {
        name: "identity",
        description: "Rendered identity section.",
    },
    PromptTemplateVariable {
        name: "user_details",
        description: "Rendered user details section.",
    },
    PromptTemplateVariable {
        name: "project_context",
        description: "Rendered project context section.",
    },
    PromptTemplateVariable {
        name: "workspace_files",
        description: "Rendered workspace files section.",
    },
    PromptTemplateVariable {
        name: "memory_bootstrap",
        description: "Rendered long-term memory section.",
    },
    PromptTemplateVariable {
        name: "available_tools",
        description: "Rendered available tools section.",
    },
    PromptTemplateVariable {
        name: "tool_call_guidance",
        description: "Rendered tool-call instructions section.",
    },
    PromptTemplateVariable {
        name: "runtime",
        description: "Rendered runtime section.",
    },
    PromptTemplateVariable {
        name: "guidelines",
        description: "Rendered guidelines section.",
    },
    PromptTemplateVariable {
        name: "skills",
        description: "Rendered skills section.",
    },
    PromptTemplateVariable {
        name: "voice_reply_mode",
        description: "Rendered voice reply mode section.",
    },
    PromptTemplateVariable {
        name: "runtime_datetime_tail",
        description: "Rendered datetime/date tail section.",
    },
];

#[derive(Debug, Clone)]
struct PromptSectionPlan {
    stable_prefix: Vec<PromptSectionId>,
    dynamic_tail: Vec<PromptSectionId>,
    options: PromptSectionOptions,
}

struct PromptRenderContext<'a> {
    native_tools: bool,
    include_tools: bool,
    project_context: Option<&'a str>,
    skills: &'a [SkillMetadata],
    identity: Option<&'a AgentIdentity>,
    user: Option<&'a UserProfile>,
    soul_text: Option<&'a str>,
    agents_text: Option<&'a str>,
    tools_text: Option<&'a str>,
    runtime_context: Option<&'a PromptRuntimeContext>,
    memory_text: Option<&'a str>,
    tool_schemas: &'a [serde_json::Value],
    voice_reply_mode: bool,
    section_options: &'a PromptSectionOptions,
    model_id: Option<&'a str>,
}

/// Returns the sections that are always required (cannot be disabled).
pub fn required_sections(include_tools: bool) -> Vec<PromptSectionId> {
    let mut required = vec![PromptSectionId::Guidelines];
    if include_tools {
        required.push(PromptSectionId::AvailableTools);
        required.push(PromptSectionId::ToolCallGuidance);
    }
    required
}

fn dedupe_section_list(items: Vec<PromptSectionId>) -> Vec<PromptSectionId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item) {
            out.push(item);
        }
    }
    out
}

fn resolve_section_plan(
    profile: Option<&PromptProfileConfig>,
    include_tools: bool,
) -> PromptSectionPlan {
    let default_profile = PromptProfileConfig::default();
    let active_profile = profile.unwrap_or(&default_profile);

    let mut enabled = if active_profile.enabled_sections.is_empty() {
        default_profile.enabled_sections.clone()
    } else {
        active_profile.enabled_sections.clone()
    };

    for section in required_sections(include_tools) {
        if !enabled.contains(&section) {
            enabled.push(section);
        }
    }
    enabled = dedupe_section_list(enabled);

    let order = if active_profile.section_order.is_empty() {
        default_profile.section_order.clone()
    } else {
        active_profile.section_order.clone()
    };

    let mut dynamic_tail = if active_profile.dynamic_tail_sections.is_empty() {
        default_profile.dynamic_tail_sections.clone()
    } else {
        active_profile.dynamic_tail_sections.clone()
    };
    dynamic_tail.retain(|section| enabled.contains(section));
    dynamic_tail = dedupe_section_list(dynamic_tail);

    if enabled.contains(&PromptSectionId::RuntimeDatetimeTail) {
        dynamic_tail.retain(|section| *section != PromptSectionId::RuntimeDatetimeTail);
        dynamic_tail.push(PromptSectionId::RuntimeDatetimeTail);
    } else {
        dynamic_tail.retain(|section| *section != PromptSectionId::RuntimeDatetimeTail);
    }

    let dynamic_set: HashSet<PromptSectionId> = dynamic_tail.iter().copied().collect();
    let mut stable_prefix = Vec::new();
    for section in order {
        if enabled.contains(&section) && !dynamic_set.contains(&section) {
            stable_prefix.push(section);
        }
    }
    stable_prefix = dedupe_section_list(stable_prefix);

    for section in enabled {
        if !dynamic_set.contains(&section) && !stable_prefix.contains(&section) {
            stable_prefix.push(section);
        }
    }

    PromptSectionPlan {
        stable_prefix,
        dynamic_tail,
        options: active_profile.section_options.clone(),
    }
}

/// Returns all prompt section IDs in their canonical order.
pub fn all_prompt_sections() -> [PromptSectionId; 12] {
    [
        PromptSectionId::Identity,
        PromptSectionId::UserDetails,
        PromptSectionId::ProjectContext,
        PromptSectionId::WorkspaceFiles,
        PromptSectionId::MemoryBootstrap,
        PromptSectionId::AvailableTools,
        PromptSectionId::ToolCallGuidance,
        PromptSectionId::Runtime,
        PromptSectionId::Guidelines,
        PromptSectionId::Skills,
        PromptSectionId::VoiceReplyMode,
        PromptSectionId::RuntimeDatetimeTail,
    ]
}

fn section_template_variable_name(section: PromptSectionId) -> &'static str {
    match section {
        PromptSectionId::Identity => "identity",
        PromptSectionId::UserDetails => "user_details",
        PromptSectionId::ProjectContext => "project_context",
        PromptSectionId::WorkspaceFiles => "workspace_files",
        PromptSectionId::MemoryBootstrap => "memory_bootstrap",
        PromptSectionId::AvailableTools => "available_tools",
        PromptSectionId::ToolCallGuidance => "tool_call_guidance",
        PromptSectionId::Runtime => "runtime",
        PromptSectionId::Guidelines => "guidelines",
        PromptSectionId::Skills => "skills",
        PromptSectionId::VoiceReplyMode => "voice_reply_mode",
        PromptSectionId::RuntimeDatetimeTail => "runtime_datetime_tail",
    }
}

fn render_section_to_string(section: PromptSectionId, ctx: &PromptRenderContext<'_>) -> String {
    let mut text = String::new();
    render_section(&mut text, section, ctx);
    text
}

fn render_section_list(
    sections: &[PromptSectionId],
    ctx: &PromptRenderContext<'_>,
) -> (String, HashMap<PromptSectionId, String>) {
    let mut rendered = String::new();
    let mut by_section = HashMap::new();

    for section in sections {
        let text = render_section_to_string(*section, ctx);
        rendered.push_str(&text);
        let _ = by_section.insert(*section, text);
    }

    (rendered, by_section)
}

fn render_template_with_values(template: &str, values: &HashMap<String, String>) -> String {
    let mut rendered = String::with_capacity(template.len() + 256);
    let mut cursor = 0usize;

    while let Some(rel_open) = template[cursor..].find("{{") {
        let open = cursor + rel_open;
        rendered.push_str(&template[cursor..open]);
        let after_open = open + 2;
        let Some(rel_close) = template[after_open..].find("}}") else {
            rendered.push_str(&template[open..]);
            return rendered;
        };
        let close = after_open + rel_close;
        let var_name = template[after_open..close].trim();
        if let Some(value) = values.get(var_name) {
            rendered.push_str(value);
        }
        cursor = close + 2;
    }

    rendered.push_str(&template[cursor..]);
    rendered
}

fn append_block_with_spacing(prompt: &mut String, block: &str) {
    if block.trim().is_empty() {
        return;
    }

    if !prompt.is_empty() && !prompt.ends_with('\n') {
        prompt.push('\n');
    }
    if !prompt.is_empty() && !prompt.ends_with("\n\n") {
        prompt.push('\n');
    }
    prompt.push_str(block);
}

fn build_template_values(
    default_prefix: &str,
    stable_sections: &str,
    dynamic_tail_sections: &str,
    section_blocks: &HashMap<PromptSectionId, String>,
    active_profile: &PromptProfileConfig,
    render_context: &PromptRenderContext<'_>,
) -> HashMap<String, String> {
    let default_sections = format!("{stable_sections}{dynamic_tail_sections}");
    let default_prompt = format!("{default_prefix}{default_sections}");
    let mut values = HashMap::new();

    let _ = values.insert("default_prompt".to_string(), default_prompt);
    let _ = values.insert("default_prefix".to_string(), default_prefix.to_string());
    let _ = values.insert("default_sections".to_string(), default_sections);
    let _ = values.insert("stable_sections".to_string(), stable_sections.to_string());
    let _ = values.insert(
        "dynamic_tail_sections".to_string(),
        dynamic_tail_sections.to_string(),
    );
    let _ = values.insert("profile_name".to_string(), active_profile.name.clone());
    let _ = values.insert(
        "profile_description".to_string(),
        active_profile.description.clone().unwrap_or_default(),
    );
    let _ = values.insert(
        "tool_count".to_string(),
        render_context.tool_schemas.len().to_string(),
    );
    let _ = values.insert(
        "flag_include_tools".to_string(),
        render_context.include_tools.to_string(),
    );
    let _ = values.insert(
        "flag_native_tools".to_string(),
        render_context.native_tools.to_string(),
    );
    let _ = values.insert(
        "flag_voice_reply_mode".to_string(),
        render_context.voice_reply_mode.to_string(),
    );

    let _ = values.insert(
        "assistant_name".to_string(),
        render_context
            .identity
            .and_then(|identity| identity.name.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "assistant_emoji".to_string(),
        render_context
            .identity
            .and_then(|identity| identity.emoji.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "user_name".to_string(),
        render_context
            .user
            .and_then(|user| user.name.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "user_timezone".to_string(),
        render_context
            .user
            .and_then(|user| user.timezone.as_ref())
            .map(|tz| tz.name().to_string())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "user_location".to_string(),
        render_context
            .user
            .and_then(|user| user.location.as_ref().map(ToString::to_string))
            .unwrap_or_default(),
    );

    let host = render_context.runtime_context.map(|runtime| &runtime.host);
    let _ = values.insert(
        "runtime_host".to_string(),
        host.and_then(|host| host.host.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_os".to_string(),
        host.and_then(|host| host.os.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_arch".to_string(),
        host.and_then(|host| host.arch.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_shell".to_string(),
        host.and_then(|host| host.shell.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_time".to_string(),
        host.and_then(|host| host.time.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_today".to_string(),
        host.and_then(|host| host.today.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_provider".to_string(),
        host.and_then(|host| host.provider.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_model".to_string(),
        host.and_then(|host| host.model.clone()).unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_session_key".to_string(),
        host.and_then(|host| host.session_key.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_data_dir".to_string(),
        host.and_then(|host| host.data_dir.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_timezone".to_string(),
        host.and_then(|host| host.timezone.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_accept_language".to_string(),
        host.and_then(|host| host.accept_language.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_remote_ip".to_string(),
        host.and_then(|host| host.remote_ip.clone())
            .unwrap_or_default(),
    );
    let _ = values.insert(
        "runtime_location".to_string(),
        host.and_then(|host| host.location.clone())
            .unwrap_or_default(),
    );

    for section in all_prompt_sections() {
        let key = section_template_variable_name(section);
        let value = section_blocks.get(&section).cloned().unwrap_or_default();
        let _ = values.insert(key.to_string(), value);
    }

    values
}

fn append_prompt_tail_template(
    prompt: &mut String,
    tail_template: Option<&str>,
    template_values: &HashMap<String, String>,
) {
    let Some(template) = tail_template.filter(|template| !template.trim().is_empty()) else {
        return;
    };

    let rendered_tail = render_template_with_values(template, template_values);
    append_block_with_spacing(prompt, &rendered_tail);
}

fn render_section(prompt: &mut String, section: PromptSectionId, ctx: &PromptRenderContext<'_>) {
    match section {
        PromptSectionId::Identity => append_identity_section(prompt, ctx.identity, ctx.soul_text),
        PromptSectionId::UserDetails => {
            append_user_details_section(prompt, ctx.user, &ctx.section_options.user_details);
        },
        PromptSectionId::ProjectContext => append_project_context(prompt, ctx.project_context),
        PromptSectionId::WorkspaceFiles => {
            append_workspace_files_section(prompt, ctx.agents_text, ctx.tools_text);
        },
        PromptSectionId::MemoryBootstrap => append_memory_section(
            prompt,
            ctx.memory_text,
            ctx.tool_schemas,
            &ctx.section_options.memory_bootstrap,
        ),
        PromptSectionId::AvailableTools => {
            append_available_tools_section(prompt, ctx.native_tools, ctx.tool_schemas);
        },
        PromptSectionId::ToolCallGuidance => {
            append_tool_call_guidance(prompt, ctx.native_tools, ctx.tool_schemas, ctx.model_id);
        },
        PromptSectionId::Runtime => append_runtime_section(
            prompt,
            ctx.runtime_context,
            ctx.include_tools,
            &ctx.section_options.runtime,
        ),
        PromptSectionId::Guidelines => append_guidelines_section(prompt, ctx.include_tools),
        PromptSectionId::Skills => append_skills_section(prompt, ctx.include_tools, ctx.skills),
        PromptSectionId::VoiceReplyMode => {
            append_voice_reply_mode_section(prompt, ctx.voice_reply_mode)
        },
        PromptSectionId::RuntimeDatetimeTail => append_runtime_datetime_tail(
            prompt,
            ctx.runtime_context,
            ctx.section_options.runtime_datetime_tail.mode,
        ),
    }
}

fn append_voice_reply_mode_section(prompt: &mut String, enabled: bool) {
    if enabled {
        prompt.push_str(VOICE_REPLY_SUFFIX);
    }
}

/// Internal: build system prompt with full control over what's included.
fn build_system_prompt_full(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    include_tools: bool,
    memory_text: Option<&str>,
    profile: Option<&PromptProfileConfig>,
    voice_reply_mode: bool,
) -> String {
    let tool_schemas = if include_tools {
        tools.list_schemas()
    } else {
        Vec::new()
    };
    let fallback_profile = PromptProfileConfig::default();
    let active_profile = profile.unwrap_or(&fallback_profile);
    let section_plan = resolve_section_plan(profile, include_tools);
    let default_prefix = if include_tools {
        DEFAULT_TOOLS_PROMPT_PREFIX
    } else {
        DEFAULT_MINIMAL_PROMPT_PREFIX
    };

    let model_id = runtime_context.and_then(|ctx| ctx.host.model.as_deref());
    let render_ctx = PromptRenderContext {
        native_tools,
        include_tools,
        project_context,
        skills,
        identity,
        user,
        soul_text,
        agents_text,
        tools_text,
        runtime_context,
        memory_text,
        tool_schemas: &tool_schemas,
        voice_reply_mode,
        section_options: &section_plan.options,
        model_id,
    };

    let (stable_sections, mut section_blocks) =
        render_section_list(&section_plan.stable_prefix, &render_ctx);
    let (dynamic_tail_sections, dynamic_tail_blocks) =
        render_section_list(&section_plan.dynamic_tail, &render_ctx);
    section_blocks.extend(dynamic_tail_blocks);

    let default_prompt = format!("{default_prefix}{stable_sections}{dynamic_tail_sections}");
    let template_values = build_template_values(
        default_prefix,
        &stable_sections,
        &dynamic_tail_sections,
        &section_blocks,
        active_profile,
        &render_ctx,
    );

    let mut prompt = if let Some(template) = active_profile
        .prompt_template
        .as_deref()
        .filter(|template| !template.trim().is_empty())
    {
        render_template_with_values(template, &template_values)
    } else {
        default_prompt
    };

    append_prompt_tail_template(
        &mut prompt,
        active_profile.prompt_tail_template.as_deref(),
        &template_values,
    );
    prompt
}

fn append_identity_section(
    prompt: &mut String,
    identity: Option<&AgentIdentity>,
    soul_text: Option<&str>,
) {
    if let Some(id) = identity {
        let mut parts = Vec::new();
        match (id.name.as_deref(), id.emoji.as_deref()) {
            (Some(name), Some(emoji)) => parts.push(format!("Your name is {name} {emoji}.")),
            (Some(name), None) => parts.push(format!("Your name is {name}.")),
            _ => {},
        }
        if let Some(theme) = id.theme.as_deref() {
            parts.push(format!("Your theme: {theme}."));
        }
        if !parts.is_empty() {
            prompt.push_str(&parts.join(" "));
            prompt.push('\n');
        }
        prompt.push_str("\n## Soul\n\n");
        prompt.push_str(soul_text.unwrap_or(DEFAULT_SOUL));
        prompt.push('\n');
    }
}

fn append_user_details_section(
    prompt: &mut String,
    user: Option<&UserProfile>,
    options: &UserDetailsSectionOptions,
) {
    let Some(user) = user else {
        return;
    };

    let mut emitted = false;
    if let Some(name) = user.name.as_deref() {
        prompt.push_str(&format!("The user's name is {name}.\n"));
        emitted = true;
    }

    if options.mode == UserDetailsMode::FullProfile {
        if let Some(timezone) = user.timezone.as_ref() {
            prompt.push_str(&format!("The user's timezone is {}.\n", timezone.name()));
            emitted = true;
        }
        if let Some(location) = user.location.as_ref() {
            prompt.push_str(&format!("The user's location is {location}.\n"));
            emitted = true;
        }
    }

    if emitted {
        prompt.push('\n');
    }
}

fn append_project_context(prompt: &mut String, project_context: Option<&str>) {
    if let Some(context) = project_context {
        append_truncated_text_block(
            prompt,
            context,
            PROJECT_CONTEXT_MAX_CHARS,
            "\n*(Project context truncated for prompt size; use tools/files for full details.)*\n",
        );
        prompt.push('\n');
    }
}

fn append_runtime_section(
    prompt: &mut String,
    runtime_context: Option<&PromptRuntimeContext>,
    include_tools: bool,
    options: &RuntimeSectionOptions,
) {
    let Some(runtime) = runtime_context else {
        return;
    };

    let host_line = options
        .include_host_fields
        .then(|| format_host_runtime_line(&runtime.host, options.include_network_sudo_fields))
        .flatten();
    let sandbox_line = if options.include_sandbox_fields {
        runtime.sandbox.as_ref().map(format_sandbox_runtime_line)
    } else {
        None
    };
    if host_line.is_none() && sandbox_line.is_none() {
        return;
    }

    prompt.push_str("## Runtime\n\n");
    if let Some(line) = host_line {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    if let Some(line) = sandbox_line {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    if include_tools {
        prompt.push_str(EXEC_ROUTING_GUIDANCE);
    } else {
        prompt.push('\n');
    }
}

fn append_skills_section(prompt: &mut String, include_tools: bool, skills: &[SkillMetadata]) {
    if include_tools && !skills.is_empty() {
        prompt.push_str(&moltis_skills::prompt_gen::generate_skills_prompt(skills));
    }
}

fn append_workspace_files_section(
    prompt: &mut String,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
) {
    if agents_text.is_none() && tools_text.is_none() {
        return;
    }

    prompt.push_str("## Workspace Files\n\n");
    if let Some(agents_md) = agents_text {
        prompt.push_str("### AGENTS.md (workspace)\n\n");
        append_truncated_text_block(
            prompt,
            agents_md,
            WORKSPACE_FILE_MAX_CHARS,
            "\n*(AGENTS.md truncated for prompt size.)*\n",
        );
        prompt.push_str("\n\n");
    }
    if let Some(tools_md) = tools_text {
        prompt.push_str("### TOOLS.md (workspace)\n\n");
        append_truncated_text_block(
            prompt,
            tools_md,
            WORKSPACE_FILE_MAX_CHARS,
            "\n*(TOOLS.md truncated for prompt size.)*\n",
        );
        prompt.push_str("\n\n");
    }
}

fn append_memory_section(
    prompt: &mut String,
    memory_text: Option<&str>,
    tool_schemas: &[serde_json::Value],
    options: &MemoryBootstrapSectionOptions,
) {
    let has_memory_search = has_tool_schema(tool_schemas, "memory_search");
    let has_memory_save = has_tool_schema(tool_schemas, "memory_save");
    let show_memory_search_guidance = has_memory_search || options.force_memory_search_guidance;
    let memory_content = if options.include_memory_md_snapshot {
        memory_text.filter(|text| !text.is_empty())
    } else {
        None
    };
    if memory_content.is_none() && !show_memory_search_guidance && !has_memory_save {
        return;
    }

    prompt.push_str("## Long-Term Memory\n\n");
    if let Some(text) = memory_content {
        append_truncated_text_block(
            prompt,
            text,
            MEMORY_BOOTSTRAP_MAX_CHARS,
            "\n\n*(MEMORY.md truncated — use `memory_search` for full content)*\n",
        );
        prompt.push_str(concat!(
            "\n\n**The information above is what you already know about the user. ",
            "Always include it in your answers.** ",
            "Even if a tool search returns no additional results, ",
            "this section still contains valid, current facts.\n",
        ));
    }
    if has_memory_search {
        prompt.push_str(concat!(
            "\nYou also have `memory_search` to find additional details from ",
            "`memory/*.md` files and past session history beyond what is shown above. ",
            "**Always search memory before claiming you don't know something.** ",
            "The long-term memory system holds user facts, past decisions, project context, ",
            "and anything previously stored.\n",
        ));
    } else if options.force_memory_search_guidance {
        prompt.push_str(
            "\nAlways search long-term memory before claiming you don't know something.\n",
        );
    }
    if has_memory_save {
        prompt.push_str(concat!(
            "\n**When the user asks you to remember, save, or note something, ",
            "you MUST call `memory_save` to persist it.** ",
            "Do not just acknowledge verbally — without calling the tool, ",
            "the information will be lost after the session.\n",
            "\nChoose the right target to keep context lean:\n",
            "- **MEMORY.md** — only core identity facts (name, age, location, ",
            "language, key preferences). This is loaded into every conversation, ",
            "so keep it short.\n",
            "- **memory/&lt;topic&gt;.md** — everything else (detailed notes, project ",
            "context, decisions, session summaries). These are only retrieved via ",
            "`memory_search` and do not consume prompt space.\n",
        ));
    }
    prompt.push('\n');
}

fn has_tool_schema(tool_schemas: &[serde_json::Value], tool_name: &str) -> bool {
    tool_schemas
        .iter()
        .any(|schema| schema["name"].as_str() == Some(tool_name))
}

fn append_available_tools_section(
    prompt: &mut String,
    native_tools: bool,
    tool_schemas: &[serde_json::Value],
) {
    if tool_schemas.is_empty() {
        return;
    }

    prompt.push_str("## Available Tools\n\n");
    if native_tools {
        // Native tool-calling providers already receive full schemas via API.
        // Keep this section compact so we don't duplicate large JSON payloads.
        for schema in tool_schemas {
            let name = schema["name"].as_str().unwrap_or("unknown");
            let desc = schema["description"].as_str().unwrap_or("");
            let compact_desc = truncate_prompt_text(desc, 160);
            if compact_desc.is_empty() {
                prompt.push_str(&format!("- `{name}`\n"));
            } else {
                prompt.push_str(&format!("- `{name}`: {compact_desc}\n"));
            }
        }
        prompt.push('\n');
        return;
    }

    // Text-mode: use compact schema format to save context tokens.
    for schema in tool_schemas {
        prompt.push_str(&format_compact_tool_schema(schema));
    }
}

fn append_tool_call_guidance(
    prompt: &mut String,
    native_tools: bool,
    tool_schemas: &[serde_json::Value],
    model_id: Option<&str>,
) {
    if !native_tools && !tool_schemas.is_empty() {
        prompt.push_str(&tool_call_guidance(model_id));
    }
}

fn append_guidelines_section(prompt: &mut String, include_tools: bool) {
    prompt.push_str(if include_tools {
        TOOL_GUIDELINES
    } else {
        MINIMAL_GUIDELINES
    });
}

fn append_runtime_datetime_tail(
    prompt: &mut String,
    runtime_context: Option<&PromptRuntimeContext>,
    mode: RuntimeDatetimeTailMode,
) {
    if mode == RuntimeDatetimeTailMode::Disabled {
        return;
    }

    let Some(runtime) = runtime_context else {
        return;
    };

    if mode == RuntimeDatetimeTailMode::Datetime
        && let Some(time) = runtime
            .host
            .time
            .as_deref()
            .filter(|value| !value.is_empty())
    {
        prompt.push_str("\nThe current user datetime is ");
        prompt.push_str(time);
        prompt.push_str(".\n");
        return;
    }

    if let Some(today) = runtime
        .host
        .today
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        prompt.push_str("\nThe current user date is ");
        prompt.push_str(today);
        prompt.push_str(".\n");
    }
}

fn push_non_empty_runtime_field(parts: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        parts.push(format!("{key}={value}"));
    }
}

fn format_host_runtime_line(
    host: &PromptHostRuntimeContext,
    include_network_sudo_fields: bool,
) -> Option<String> {
    let mut parts = Vec::new();
    for (key, value) in [
        ("host", host.host.as_deref()),
        ("os", host.os.as_deref()),
        ("arch", host.arch.as_deref()),
        ("shell", host.shell.as_deref()),
        ("today", host.today.as_deref()),
        ("provider", host.provider.as_deref()),
        ("model", host.model.as_deref()),
        ("session", host.session_key.as_deref()),
        ("surface", host.surface.as_deref()),
        ("session_kind", host.session_kind.as_deref()),
        ("channel_type", host.channel_type.as_deref()),
        ("channel_account", host.channel_account_id.as_deref()),
        ("channel_chat_id", host.channel_chat_id.as_deref()),
        ("channel_chat_type", host.channel_chat_type.as_deref()),
        ("data_dir", host.data_dir.as_deref()),
    ] {
        push_non_empty_runtime_field(&mut parts, key, value);
    }
    if include_network_sudo_fields {
        if let Some(sudo_non_interactive) = host.sudo_non_interactive {
            parts.push(format!("sudo_non_interactive={sudo_non_interactive}"));
        }
        for (key, value) in [
            ("sudo_status", host.sudo_status.as_deref()),
            ("timezone", host.timezone.as_deref()),
            ("accept_language", host.accept_language.as_deref()),
            ("remote_ip", host.remote_ip.as_deref()),
            ("location", host.location.as_deref()),
        ] {
            push_non_empty_runtime_field(&mut parts, key, value);
        }
    }

    (!parts.is_empty()).then(|| format!("Host: {}", parts.join(" | ")))
}

fn truncate_prompt_text(text: &str, max_chars: usize) -> String {
    if text.is_empty() || max_chars == 0 {
        return String::new();
    }
    let mut iter = text.chars();
    let taken: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{taken}...")
    } else {
        taken
    }
}

fn append_truncated_text_block(
    prompt: &mut String,
    text: &str,
    max_chars: usize,
    truncated_notice: &str,
) {
    let truncated = truncate_prompt_text(text, max_chars);
    prompt.push_str(&truncated);
    if text.chars().count() > max_chars {
        prompt.push_str(truncated_notice);
    }
}

fn format_sandbox_runtime_line(sandbox: &PromptSandboxRuntimeContext) -> String {
    let mut parts = vec![format!("enabled={}", sandbox.exec_sandboxed)];

    for (key, value) in [
        ("mode", sandbox.mode.as_deref()),
        ("backend", sandbox.backend.as_deref()),
        ("scope", sandbox.scope.as_deref()),
        ("image", sandbox.image.as_deref()),
        ("home", sandbox.home.as_deref()),
        ("workspace_mount", sandbox.workspace_mount.as_deref()),
        ("workspace_path", sandbox.workspace_path.as_deref()),
    ] {
        push_non_empty_runtime_field(&mut parts, key, value);
    }
    if let Some(no_network) = sandbox.no_network {
        let network_state = if no_network {
            "disabled"
        } else {
            "enabled"
        };
        parts.push(format!("network={network_state}"));
    }
    if let Some(session_override) = sandbox.session_override {
        parts.push(format!("session_override={session_override}"));
    }

    format!("Sandbox(exec): {}", parts.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_prompt_does_not_include_tool_call_format() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt(&tools, true, None);
        assert!(!prompt.contains("```tool_call"));
    }

    #[test]
    fn test_fallback_prompt_includes_tool_call_format() {
        let mut tools = ToolRegistry::new();
        struct Dummy;
        #[async_trait::async_trait]
        impl crate::tool_registry::AgentTool for Dummy {
            fn name(&self) -> &str {
                "test"
            }

            fn description(&self) -> &str {
                "A test tool"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }

            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({}))
            }
        }
        tools.register(Box::new(Dummy));

        let prompt = build_system_prompt(&tools, false, None);
        assert!(prompt.contains("```tool_call"));
        assert!(prompt.contains("### test"));
    }

    #[test]
    fn test_native_prompt_uses_compact_tool_list() {
        let mut tools = ToolRegistry::new();
        struct Dummy;
        #[async_trait::async_trait]
        impl crate::tool_registry::AgentTool for Dummy {
            fn name(&self) -> &str {
                "test"
            }

            fn description(&self) -> &str {
                "A test tool"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {"cmd": {"type": "string"}}})
            }

            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({}))
            }
        }
        tools.register(Box::new(Dummy));

        let prompt = build_system_prompt(&tools, true, None);
        assert!(prompt.contains("## Available Tools"));
        assert!(prompt.contains("- `test`: A test tool"));
        assert!(!prompt.contains("Parameters:"));
    }

    #[test]
    fn test_skills_injected_into_prompt() {
        let tools = ToolRegistry::new();
        let skills = vec![SkillMetadata {
            name: "commit".into(),
            description: "Create git commits".into(),
            license: None,
            compatibility: None,
            allowed_tools: vec![],
            homepage: None,
            dockerfile: None,
            requires: Default::default(),
            path: std::path::PathBuf::from("/skills/commit"),
            source: None,
        }];
        let prompt = build_system_prompt_with_session_runtime(
            &tools, true, None, &skills, None, None, None, None, None, None, None,
        );
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("commit"));
    }

    #[test]
    fn test_no_skills_block_when_empty() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(!prompt.contains("<available_skills>"));
    }

    #[test]
    fn test_identity_injected_into_prompt() {
        let tools = ToolRegistry::new();
        let identity = AgentIdentity {
            name: Some("Momo".into()),
            emoji: Some("🦜".into()),
            theme: Some("cheerful parrot".into()),
        };
        let user = UserProfile {
            name: Some("Alice".into()),
            timezone: None,
            location: None,
        };
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            Some(&identity),
            Some(&user),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(prompt.contains("Your name is Momo 🦜."));
        assert!(prompt.contains("Your theme: cheerful parrot."));
        assert!(prompt.contains("The user's name is Alice."));
        // Default soul should be injected when soul is None.
        assert!(prompt.contains("## Soul"));
        assert!(prompt.contains("Be genuinely helpful"));
    }

    #[test]
    fn test_custom_soul_injected() {
        let tools = ToolRegistry::new();
        let identity = AgentIdentity {
            name: Some("Rex".into()),
            ..Default::default()
        };
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            Some(&identity),
            None,
            Some("You are a loyal companion who loves fetch."),
            None,
            None,
            None,
            None,
        );
        assert!(prompt.contains("## Soul"));
        assert!(prompt.contains("loyal companion who loves fetch"));
        assert!(!prompt.contains("Be genuinely helpful"));
    }

    #[test]
    fn test_no_identity_no_extra_lines() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(!prompt.contains("Your name is"));
        assert!(!prompt.contains("The user's name is"));
        assert!(!prompt.contains("## Soul"));
    }

    #[test]
    fn test_workspace_files_injected_when_provided() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            Some("Follow workspace agent instructions."),
            Some("Prefer read-only tools first."),
            None,
            None,
        );
        assert!(prompt.contains("## Workspace Files"));
        assert!(prompt.contains("### AGENTS.md (workspace)"));
        assert!(prompt.contains("Follow workspace agent instructions."));
        assert!(prompt.contains("### TOOLS.md (workspace)"));
        assert!(prompt.contains("Prefer read-only tools first."));
    }

    #[test]
    fn test_runtime_context_injected_when_provided() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("moltis-devbox".into()),
                os: Some("macos".into()),
                arch: Some("aarch64".into()),
                shell: Some("zsh".into()),
                time: Some("2026-02-17 16:18:00 CET".into()),
                today: Some("2026-02-17".into()),
                provider: Some("openai".into()),
                model: Some("gpt-5".into()),
                session_key: Some("main".into()),
                surface: None,
                session_kind: None,
                channel_type: None,
                channel_account_id: None,
                channel_chat_id: None,
                channel_chat_type: None,
                data_dir: Some("/home/moltis/.moltis".into()),
                sudo_non_interactive: Some(true),
                sudo_status: Some("passwordless".into()),
                timezone: Some("Europe/Paris".into()),
                accept_language: Some("en-US,fr;q=0.9".into()),
                remote_ip: Some("203.0.113.42".into()),
                location: None,
            },
            sandbox: Some(PromptSandboxRuntimeContext {
                exec_sandboxed: true,
                mode: Some("all".into()),
                backend: Some("docker".into()),
                scope: Some("session".into()),
                image: Some("moltis-sandbox:abc123".into()),
                home: Some("/home/sandbox".into()),
                workspace_mount: Some("ro".into()),
                workspace_path: Some("/home/moltis/.moltis".into()),
                no_network: Some(true),
                session_override: Some(true),
            }),
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(prompt.contains("## Runtime"));
        assert!(prompt.contains("Host: host=moltis-devbox"));
        assert!(!prompt.contains("time=2026-02-17 16:18:00 CET"));
        assert!(prompt.contains("today=2026-02-17"));
        assert!(prompt.contains("The current user datetime is 2026-02-17 16:18:00 CET."));
        assert!(prompt.contains("provider=openai"));
        assert!(prompt.contains("model=gpt-5"));
        assert!(prompt.contains("data_dir=/home/moltis/.moltis"));
        assert!(prompt.contains("sudo_non_interactive=true"));
        assert!(prompt.contains("sudo_status=passwordless"));
        assert!(prompt.contains("timezone=Europe/Paris"));
        assert!(prompt.contains("accept_language=en-US,fr;q=0.9"));
        assert!(prompt.contains("remote_ip=203.0.113.42"));
        assert!(prompt.contains("Sandbox(exec): enabled=true"));
        assert!(prompt.contains("backend=docker"));
        assert!(prompt.contains("home=/home/sandbox"));
        assert!(prompt.contains("workspace_path=/home/moltis/.moltis"));
        assert!(prompt.contains("network=disabled"));
        assert!(prompt.contains("Execution routing:"));
        assert!(prompt.contains("`~` and relative paths resolve under"));
        assert!(prompt.contains("Sandbox/host routing changes are expected runtime behavior"));
    }

    #[test]
    fn test_runtime_context_includes_location_when_set() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("devbox".into()),
                location: Some("48.8566,2.3522".into()),
                ..Default::default()
            },
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(prompt.contains("location=48.8566,2.3522"));
    }

    #[test]
    fn test_runtime_context_includes_channel_surface_fields_when_set() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                session_key: Some("telegram:bot-main:123456".into()),
                surface: Some("telegram".into()),
                session_kind: Some("channel".into()),
                channel_type: Some("telegram".into()),
                channel_account_id: Some("bot-main".into()),
                channel_chat_id: Some("123456".into()),
                channel_chat_type: Some("private".into()),
                ..Default::default()
            },
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(prompt.contains("surface=telegram"));
        assert!(prompt.contains("session_kind=channel"));
        assert!(prompt.contains("channel_type=telegram"));
        assert!(prompt.contains("channel_account=bot-main"));
        assert!(prompt.contains("channel_chat_id=123456"));
        assert!(prompt.contains("channel_chat_type=private"));
    }

    #[test]
    fn test_runtime_context_omits_location_when_none() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("devbox".into()),
                location: None,
                ..Default::default()
            },
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(!prompt.contains("location="));
    }

    #[test]
    fn test_minimal_prompt_runtime_does_not_add_exec_routing_block() {
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("moltis-devbox".into()),
                ..Default::default()
            },
            sandbox: Some(PromptSandboxRuntimeContext {
                exec_sandboxed: false,
                ..Default::default()
            }),
        };

        let prompt = build_system_prompt_minimal_runtime(
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(prompt.contains("## Runtime"));
        assert!(prompt.contains("Host: host=moltis-devbox"));
        assert!(prompt.contains("Sandbox(exec): enabled=false"));
        assert!(!prompt.contains("Execution routing:"));
    }

    #[test]
    fn test_silent_replies_section_in_tool_prompt() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt(&tools, true, None);
        assert!(prompt.contains("## Silent Replies"));
        assert!(prompt.contains("empty response"));
        assert!(prompt.contains("Do not call tools for greetings"));
        assert!(prompt.contains("`/sh `"));
        assert!(prompt.contains("run it with `exec` exactly as written"));
        assert!(prompt.contains("Do not express surprise about sandbox vs host execution"));
        assert!(!prompt.contains("__SILENT__"));
    }

    #[test]
    fn test_silent_replies_not_in_minimal_prompt() {
        let prompt =
            build_system_prompt_minimal_runtime(None, None, None, None, None, None, None, None);
        assert!(!prompt.contains("## Silent Replies"));
    }

    #[test]
    fn test_memory_text_injected_into_prompt() {
        let tools = ToolRegistry::new();
        let memory = "## User Facts\n- Lives in Paris\n- Speaks French";
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            Some(memory),
        );
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("Lives in Paris"));
        assert!(prompt.contains("Speaks French"));
        // Memory content should include the "already know" hint so models
        // don't ignore it when tool searches return empty.
        assert!(prompt.contains("information above is what you already know"));
    }

    #[test]
    fn test_memory_text_truncated_at_limit() {
        let tools = ToolRegistry::new();
        // Create content larger than MEMORY_BOOTSTRAP_MAX_CHARS
        let large_memory = "x".repeat(MEMORY_BOOTSTRAP_MAX_CHARS + 500);
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&large_memory),
        );
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("MEMORY.md truncated"));
        // The full content should NOT be present
        assert!(!prompt.contains(&large_memory));
    }

    #[test]
    fn test_no_memory_section_without_memory_or_tools() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(!prompt.contains("## Long-Term Memory"));
    }

    #[test]
    fn test_memory_text_in_minimal_prompt() {
        let memory = "## Notes\n- Important fact";
        let prompt = build_system_prompt_minimal_runtime(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(memory),
        );
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("Important fact"));
        // Minimal prompts have no tools, so no memory_search hint
        assert!(!prompt.contains("memory_search"));
    }

    /// Helper to create a [`ToolRegistry`] with one or more named stub tools.
    fn registry_with_tools(names: &[&'static str]) -> ToolRegistry {
        struct NamedStub(&'static str);
        #[async_trait::async_trait]
        impl crate::tool_registry::AgentTool for NamedStub {
            fn name(&self) -> &str {
                self.0
            }

            fn description(&self) -> &str {
                "stub"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }

            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({}))
            }
        }
        let mut reg = ToolRegistry::new();
        for name in names {
            reg.register(Box::new(NamedStub(name)));
        }
        reg
    }

    #[test]
    fn test_memory_save_hint_injected_when_tool_registered() {
        let tools = registry_with_tools(&["memory_save"]);
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("MUST call `memory_save`"));
    }

    #[test]
    fn test_memory_save_hint_absent_without_tool() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(!prompt.contains("memory_save"));
    }

    #[test]
    fn test_memory_search_and_save_hints_both_present() {
        let tools = registry_with_tools(&["memory_search", "memory_save"]);
        let memory = "## User Facts\n- Likes coffee";
        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            Some(memory),
        );
        assert!(prompt.contains("## Long-Term Memory"));
        assert!(prompt.contains("Likes coffee"));
        assert!(prompt.contains("memory_search"));
        assert!(prompt.contains("MUST call `memory_save`"));
    }

    #[test]
    fn test_datetime_tail_appended_at_end_when_runtime_time_present() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                time: Some("2026-02-17 16:18:00 CET".into()),
                ..Default::default()
            },
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        let expected = "The current user datetime is 2026-02-17 16:18:00 CET.";
        assert!(prompt.contains(expected));
        assert!(prompt.trim_end().ends_with(expected));
    }

    #[test]
    fn test_datetime_tail_falls_back_to_today_when_time_missing() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                today: Some("2026-02-17".into()),
                ..Default::default()
            },
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(prompt.contains("The current user date is 2026-02-17."));
        assert!(
            prompt
                .trim_end()
                .ends_with("The current user date is 2026-02-17.")
        );
    }

    #[test]
    fn test_datetime_tail_not_injected_without_time_or_date() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext::default(),
            sandbox: None,
        };

        let prompt = build_system_prompt_with_session_runtime(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
        );

        assert!(!prompt.contains("The current user datetime is "));
        assert!(!prompt.contains("The current user date is "));
    }

    #[test]
    fn test_profile_section_order_and_dynamic_tail_rendering() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("devbox".to_string()),
                time: Some("2026-02-17 16:18:00 CET".to_string()),
                today: Some("2026-02-17".to_string()),
                ..Default::default()
            },
            sandbox: None,
        };
        let profile = PromptProfileConfig {
            name: "ordered".to_string(),
            description: None,
            prompt_template: None,
            prompt_tail_template: None,
            enabled_sections: vec![
                PromptSectionId::Runtime,
                PromptSectionId::Guidelines,
                PromptSectionId::VoiceReplyMode,
                PromptSectionId::RuntimeDatetimeTail,
            ],
            section_order: vec![
                PromptSectionId::Runtime,
                PromptSectionId::Guidelines,
                PromptSectionId::VoiceReplyMode,
            ],
            dynamic_tail_sections: vec![PromptSectionId::RuntimeDatetimeTail],
            section_options: PromptSectionOptions::default(),
        };

        let prompt = build_system_prompt_with_profile(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
            Some(&profile),
            true,
        );

        let runtime_ix = prompt.find("## Runtime");
        let guidelines_ix = prompt.find("## Guidelines");
        let voice_ix = prompt.find("## Voice Reply Mode");
        let tail_ix = prompt.rfind("The current user datetime is 2026-02-17 16:18:00 CET.");
        assert!(runtime_ix.is_some());
        assert!(guidelines_ix.is_some());
        assert!(voice_ix.is_some());
        assert!(tail_ix.is_some());
        assert!(runtime_ix.is_some_and(|ix| guidelines_ix.is_some_and(|next| ix < next)));
        assert!(guidelines_ix.is_some_and(|ix| voice_ix.is_some_and(|next| ix < next)));
        assert!(voice_ix.is_some_and(|ix| tail_ix.is_some_and(|next| ix < next)));
        assert!(
            prompt
                .trim_end()
                .ends_with("The current user datetime is 2026-02-17 16:18:00 CET.")
        );
    }

    #[test]
    fn test_required_guidelines_are_enforced_even_when_disabled() {
        let tools = ToolRegistry::new();
        let profile = PromptProfileConfig {
            name: "unsafe".to_string(),
            description: None,
            prompt_template: None,
            prompt_tail_template: None,
            enabled_sections: vec![PromptSectionId::Runtime],
            section_order: vec![PromptSectionId::Runtime],
            dynamic_tail_sections: vec![],
            section_options: PromptSectionOptions::default(),
        };

        let prompt = build_system_prompt_with_profile(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&profile),
            false,
        );

        assert!(prompt.contains("## Guidelines"));
    }

    #[test]
    fn test_runtime_section_option_hides_network_and_sudo_fields() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                host: Some("devbox".to_string()),
                os: Some("linux".to_string()),
                sudo_non_interactive: Some(true),
                sudo_status: Some("passwordless".to_string()),
                timezone: Some("UTC".to_string()),
                remote_ip: Some("203.0.113.22".to_string()),
                ..Default::default()
            },
            sandbox: None,
        };
        let profile = PromptProfileConfig {
            name: "runtime-lite".to_string(),
            description: None,
            prompt_template: None,
            prompt_tail_template: None,
            enabled_sections: vec![PromptSectionId::Runtime, PromptSectionId::Guidelines],
            section_order: vec![PromptSectionId::Runtime, PromptSectionId::Guidelines],
            dynamic_tail_sections: vec![],
            section_options: PromptSectionOptions {
                runtime: RuntimeSectionOptions {
                    include_host_fields: true,
                    include_sandbox_fields: true,
                    include_network_sudo_fields: false,
                },
                ..PromptSectionOptions::default()
            },
        };

        let prompt = build_system_prompt_with_profile(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
            Some(&profile),
            false,
        );

        assert!(prompt.contains("Host: host=devbox"));
        assert!(!prompt.contains("| sudo_non_interactive=true"));
        assert!(!prompt.contains("| sudo_status=passwordless"));
        assert!(!prompt.contains("| timezone=UTC"));
        assert!(!prompt.contains("| remote_ip=203.0.113.22"));
    }

    #[test]
    fn test_runtime_datetime_tail_mode_date_only() {
        let tools = ToolRegistry::new();
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                time: Some("2026-02-17 16:18:00 CET".to_string()),
                today: Some("2026-02-17".to_string()),
                ..Default::default()
            },
            sandbox: None,
        };
        let profile = PromptProfileConfig {
            name: "date-only".to_string(),
            description: None,
            prompt_template: None,
            prompt_tail_template: None,
            enabled_sections: vec![
                PromptSectionId::Guidelines,
                PromptSectionId::RuntimeDatetimeTail,
            ],
            section_order: vec![PromptSectionId::Guidelines],
            dynamic_tail_sections: vec![PromptSectionId::RuntimeDatetimeTail],
            section_options: PromptSectionOptions {
                runtime_datetime_tail: moltis_config::RuntimeDatetimeTailSectionOptions {
                    mode: RuntimeDatetimeTailMode::DateOnly,
                },
                ..PromptSectionOptions::default()
            },
        };

        let prompt = build_system_prompt_with_profile(
            &tools,
            true,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
            Some(&profile),
            false,
        );

        assert!(!prompt.contains("The current user datetime is 2026-02-17 16:18:00 CET."));
        assert!(prompt.contains("The current user date is 2026-02-17."));
    }

    #[test]
    fn test_custom_prompt_template_renders_variables_and_tail() {
        let identity = AgentIdentity {
            name: Some("Momo".to_string()),
            ..Default::default()
        };
        let runtime = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                today: Some("2026-02-17".to_string()),
                ..Default::default()
            },
            sandbox: None,
        };
        let profile = PromptProfileConfig {
            name: "templated".to_string(),
            description: None,
            prompt_template: Some(
                "{{default_prefix}}Name={{assistant_name}}\n{{guidelines}}".to_string(),
            ),
            prompt_tail_template: Some("Tail date={{runtime_today}}".to_string()),
            enabled_sections: vec![PromptSectionId::Guidelines],
            section_order: vec![PromptSectionId::Guidelines],
            dynamic_tail_sections: vec![],
            section_options: PromptSectionOptions::default(),
        };

        let prompt = build_system_prompt_minimal_with_profile(
            None,
            Some(&identity),
            None,
            None,
            None,
            None,
            Some(&runtime),
            None,
            Some(&profile),
            false,
        );

        assert!(prompt.contains("Name=Momo"));
        assert!(prompt.contains("## Guidelines"));
        assert!(prompt.trim_end().ends_with("Tail date=2026-02-17"));
    }

    #[test]
    fn test_custom_template_respects_exact_body_without_implicit_tool_sections() {
        let tools = registry_with_tools(&["calc"]);
        let profile = PromptProfileConfig {
            name: "custom".to_string(),
            description: None,
            prompt_template: Some("Custom template body.".to_string()),
            prompt_tail_template: None,
            enabled_sections: vec![PromptSectionId::Runtime],
            section_order: vec![PromptSectionId::Runtime],
            dynamic_tail_sections: vec![],
            section_options: PromptSectionOptions::default(),
        };

        let prompt = build_system_prompt_with_profile(
            &tools,
            false,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&profile),
            false,
        );

        assert!(prompt.starts_with("Custom template body."));
        assert_eq!(prompt.trim(), "Custom template body.");
    }

    #[test]
    fn test_unknown_template_variables_render_as_empty() {
        let profile = PromptProfileConfig {
            name: "unknown-var".to_string(),
            description: None,
            prompt_template: Some("A{{does_not_exist}}B".to_string()),
            prompt_tail_template: None,
            enabled_sections: vec![PromptSectionId::Guidelines],
            section_order: vec![PromptSectionId::Guidelines],
            dynamic_tail_sections: vec![],
            section_options: PromptSectionOptions::default(),
        };

        let prompt = build_system_prompt_minimal_with_profile(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&profile),
            false,
        );

        assert!(prompt.contains("AB"));
        assert!(!prompt.contains("## Guidelines"));
    }

    #[test]
    fn test_prompt_template_variable_catalog_contains_core_variables() {
        let names: HashSet<&str> = prompt_template_variables()
            .iter()
            .map(|variable| variable.name)
            .collect();
        assert!(names.contains("default_prompt"));
        assert!(names.contains("default_prefix"));
        assert!(names.contains("guidelines"));
        assert!(names.contains("runtime_today"));
        assert_eq!(
            default_prompt_template(),
            "{{default_prefix}}{{stable_sections}}{{dynamic_tail_sections}}"
        );
    }

    // ── Phase 4: ModelFamily, compact schema, tool call guidance ────────

    #[test]
    fn model_family_detects_llama() {
        assert_eq!(
            ModelFamily::from_model_id("llama3.1:8b"),
            ModelFamily::Llama
        );
        assert_eq!(
            ModelFamily::from_model_id("meta-llama/Llama-3.3-70B"),
            ModelFamily::Llama,
        );
    }

    #[test]
    fn model_family_detects_qwen() {
        assert_eq!(ModelFamily::from_model_id("qwen2.5:7b"), ModelFamily::Qwen);
        assert_eq!(
            ModelFamily::from_model_id("Qwen/Qwen2.5-Coder-32B"),
            ModelFamily::Qwen,
        );
    }

    #[test]
    fn model_family_detects_mistral() {
        assert_eq!(
            ModelFamily::from_model_id("mistral:latest"),
            ModelFamily::Mistral,
        );
        assert_eq!(
            ModelFamily::from_model_id("mixtral-8x7b"),
            ModelFamily::Mistral,
        );
    }

    #[test]
    fn model_family_detects_others() {
        assert_eq!(
            ModelFamily::from_model_id("deepseek-coder-v2:16b"),
            ModelFamily::DeepSeek,
        );
        assert_eq!(ModelFamily::from_model_id("gemma:7b"), ModelFamily::Gemma);
        assert_eq!(ModelFamily::from_model_id("phi-3:mini"), ModelFamily::Phi);
    }

    #[test]
    fn model_family_unknown_for_unrecognized() {
        assert_eq!(ModelFamily::from_model_id("gpt-4o"), ModelFamily::Unknown,);
        assert_eq!(
            ModelFamily::from_model_id("claude-3-opus"),
            ModelFamily::Unknown,
        );
    }

    #[test]
    fn compact_schema_formats_required_and_optional_params() {
        let schema = serde_json::json!({
            "name": "exec",
            "description": "Run a shell command",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout": {"type": "integer"}
                },
                "required": ["command"]
            }
        });
        let out = format_compact_tool_schema(&schema);
        assert!(out.contains("### exec"));
        assert!(out.contains("Run a shell command"));
        assert!(out.contains("command (string, required)"));
        assert!(out.contains("timeout (integer)"));
    }

    #[test]
    fn compact_schema_no_params_section_when_empty() {
        let schema = serde_json::json!({
            "name": "noop",
            "description": "Does nothing",
            "parameters": {"type": "object", "properties": {}}
        });
        let out = format_compact_tool_schema(&schema);
        assert!(out.contains("### noop"));
        assert!(!out.contains("Params:"));
    }

    #[test]
    fn tool_call_guidance_includes_fenced_example() {
        let g = tool_call_guidance(Some("llama3.1:8b"));
        assert!(g.contains("```tool_call"));
        assert!(g.contains("\"tool\":"));
        assert!(g.contains("Example:"));
    }

    #[test]
    fn tool_call_guidance_works_with_no_model() {
        let g = tool_call_guidance(None);
        assert!(g.contains("## How to call tools"));
        assert!(g.contains("```tool_call"));
    }

    #[test]
    fn text_mode_prompt_uses_compact_schema() {
        let mut tools = ToolRegistry::new();
        struct ParamTool;
        #[async_trait::async_trait]
        impl crate::tool_registry::AgentTool for ParamTool {
            fn name(&self) -> &str {
                "exec"
            }

            fn description(&self) -> &str {
                "Run a shell command"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"},
                        "timeout": {"type": "integer"}
                    },
                    "required": ["command"]
                })
            }

            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!({}))
            }
        }
        tools.register(Box::new(ParamTool));

        let prompt = build_system_prompt(&tools, false, None);
        // Text-mode should use compact format
        assert!(prompt.contains("### exec"));
        assert!(prompt.contains("Params: command (string, required)"));
        // Should include tool call guidance
        assert!(prompt.contains("## How to call tools"));
        assert!(prompt.contains("```tool_call"));
    }
}
