use {
    crate::tool_registry::ToolRegistry,
    moltis_config::{AgentIdentity, DEFAULT_SOUL, UserProfile},
    moltis_skills::types::SkillMetadata,
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
    )
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
) -> String {
    let tool_schemas = if include_tools {
        tools.list_schemas()
    } else {
        Vec::new()
    };
    let mut prompt = String::from(if include_tools {
        "You are a helpful assistant. You can use tools when needed.\n\n"
    } else {
        "You are a helpful assistant. Answer questions clearly and concisely.\n\n"
    });

    append_identity_and_user_sections(&mut prompt, identity, user, soul_text);
    append_project_context(&mut prompt, project_context);
    append_runtime_section(&mut prompt, runtime_context, include_tools);
    append_skills_section(&mut prompt, include_tools, skills);
    append_workspace_files_section(&mut prompt, agents_text, tools_text);
    append_memory_section(&mut prompt, memory_text, &tool_schemas);
    let model_id = runtime_context.and_then(|ctx| ctx.host.model.as_deref());
    append_available_tools_section(&mut prompt, native_tools, &tool_schemas);
    append_tool_call_guidance(&mut prompt, native_tools, &tool_schemas, model_id);
    append_guidelines_section(&mut prompt, include_tools);
    append_runtime_datetime_tail(&mut prompt, runtime_context);

    prompt
}

fn append_identity_and_user_sections(
    prompt: &mut String,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
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

    if let Some(name) = user.and_then(|profile| profile.name.as_deref()) {
        prompt.push_str(&format!("The user's name is {name}.\n"));
    }
    if identity.is_some() || user.is_some() {
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
) {
    let Some(runtime) = runtime_context else {
        return;
    };

    let host_line = format_host_runtime_line(&runtime.host);
    let sandbox_line = runtime.sandbox.as_ref().map(format_sandbox_runtime_line);
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
) {
    let has_memory_search = has_tool_schema(tool_schemas, "memory_search");
    let has_memory_save = has_tool_schema(tool_schemas, "memory_save");
    let memory_content = memory_text.filter(|text| !text.is_empty());
    if memory_content.is_none() && !has_memory_search && !has_memory_save {
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
) {
    let Some(runtime) = runtime_context else {
        return;
    };

    if let Some(time) = runtime
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

fn format_host_runtime_line(host: &PromptHostRuntimeContext) -> Option<String> {
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
