use {
    crate::tool_registry::ToolRegistry,
    moltis_config::{AgentIdentity, DEFAULT_SOUL, UserProfile},
    moltis_skills::types::SkillMetadata,
};

/// Runtime context for the host process running the current agent turn.
#[derive(Debug, Clone, Default)]
pub struct PromptHostRuntimeContext {
    pub host: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub shell: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_key: Option<String>,
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
    pub workspace_mount: Option<String>,
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
The user will hear your response as spoken audio. Write for speech, not for reading:\n\
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
/// prompt. Matches OpenClaw's `bootstrapMaxChars`.
const MEMORY_BOOTSTRAP_MAX_CHARS: usize = 20_000;
const EXEC_ROUTING_GUIDANCE: &str = "Execution routing:\n\
- `exec` runs inside sandbox when `Sandbox(exec): enabled=true`.\n\
- When sandbox is disabled, `exec` runs on the host and may require approval.\n\
- `Host: sudo_non_interactive=true` means non-interactive sudo is available for host installs; otherwise ask the user before host package installation.\n\
- If sandbox is missing required tools/packages and host installation is needed, ask the user before requesting host install or changing sandbox mode.\n\
- Sandbox/host routing changes are expected runtime behavior. Do not frame them as surprising or anomalous.\n\
- If outputs differ after a routing change, state the active route briefly and continue.\n\n";
const TOOL_CALL_GUIDANCE: &str = concat!(
    "## How to call tools\n\n",
    "To call a tool, output ONLY a JSON block with this exact format (no other text before it):\n\n",
    "```tool_call\n",
    "{\"tool\": \"<tool_name>\", \"arguments\": {<arguments>}}\n",
    "```\n\n",
    "You MUST output the tool call block as the ENTIRE response — do not add any text before or after it.\n",
    "After the tool executes, you will receive the result and can then respond to the user.\n\n",
);
const TOOL_GUIDELINES: &str = concat!(
    "## Guidelines\n\n",
    "- Start with a normal conversational response. Do not call tools for greetings, small talk, ",
    "or questions you can answer directly.\n",
    "- Use the calc tool for arithmetic or expression evaluation instead of estimating math in text.\n",
    "- Use the exec tool when the user asks you to run shell commands, or when system interaction ",
    "is required to complete the task.\n",
    "- If the user starts a message with `/sh `, treat the remaining text as an explicit shell ",
    "command and run it with `exec` exactly as written.\n",
    "- Do not express surprise about sandbox vs host execution. Route changes are normal.\n",
    "- Do not suggest disabling sandbox unless the user explicitly asks for host execution or ",
    "the task cannot be completed in sandbox.\n",
    "- Use the browser tool to open URLs and interact with web pages. Call it when the user ",
    "asks to visit a website, check a page, read web content, or perform any web browsing task.\n",
    "- Before executing commands or opening pages, briefly explain what you're going to do.\n",
    "- If a command or browser action fails, analyze the error and suggest fixes.\n",
    "- For multi-step tasks, execute one step at a time and check results before proceeding.\n",
    "- Be careful with destructive operations — confirm with the user first.\n",
    "- IMPORTANT: The user's UI already displays tool execution results (stdout, stderr, exit code) ",
    "in a dedicated panel. Do NOT repeat or echo raw tool output in your response. Instead, ",
    "summarize what happened, highlight key findings, or explain errors. ",
    "Simply parroting the output wastes the user's time.\n\n",
    "## Silent Replies\n\n",
    "When you have nothing meaningful to add after a tool call — the output ",
    "speaks for itself — do NOT produce any text. Simply return an empty response.\n",
    "The user's UI already shows tool results, so there is no need to repeat or ",
    "acknowledge them. Stay silent when the output answers the user's question.\n",
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
    append_available_tools_section(&mut prompt, native_tools, &tool_schemas);
    append_tool_call_guidance(&mut prompt, native_tools, &tool_schemas);
    append_guidelines_section(&mut prompt, include_tools);

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
        if let Some(creature) = id.creature.as_deref() {
            parts.push(format!("You are a {creature}."));
        }
        if let Some(vibe) = id.vibe.as_deref() {
            parts.push(format!("Your vibe: {vibe}."));
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
        prompt.push_str(context);
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
        prompt.push_str(agents_md);
        prompt.push_str("\n\n");
    }
    if let Some(tools_md) = tools_text {
        prompt.push_str("### TOOLS.md (workspace)\n\n");
        prompt.push_str(tools_md);
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
        // Truncate to the bootstrap limit to keep the context window manageable.
        let truncated = truncate_prompt_text(text, MEMORY_BOOTSTRAP_MAX_CHARS);
        prompt.push_str(&truncated);
        if text.len() > MEMORY_BOOTSTRAP_MAX_CHARS {
            prompt.push_str("\n\n*(MEMORY.md truncated — use `memory_search` for full content)*\n");
        }
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

    for schema in tool_schemas {
        let name = schema["name"].as_str().unwrap_or("unknown");
        let desc = schema["description"].as_str().unwrap_or("");
        let params = &schema["parameters"];
        prompt.push_str(&format!(
            "### {name}\n{desc}\n\nParameters:\n```json\n{}\n```\n\n",
            serde_json::to_string_pretty(params).unwrap_or_default()
        ));
    }
}

fn append_tool_call_guidance(
    prompt: &mut String,
    native_tools: bool,
    tool_schemas: &[serde_json::Value],
) {
    if !native_tools && !tool_schemas.is_empty() {
        prompt.push_str(TOOL_CALL_GUIDANCE);
    }
}

fn append_guidelines_section(prompt: &mut String, include_tools: bool) {
    prompt.push_str(if include_tools {
        TOOL_GUIDELINES
    } else {
        MINIMAL_GUIDELINES
    });
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
        ("provider", host.provider.as_deref()),
        ("model", host.model.as_deref()),
        ("session", host.session_key.as_deref()),
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

fn format_sandbox_runtime_line(sandbox: &PromptSandboxRuntimeContext) -> String {
    let mut parts = vec![format!("enabled={}", sandbox.exec_sandboxed)];

    for (key, value) in [
        ("mode", sandbox.mode.as_deref()),
        ("backend", sandbox.backend.as_deref()),
        ("scope", sandbox.scope.as_deref()),
        ("image", sandbox.image.as_deref()),
        ("workspace_mount", sandbox.workspace_mount.as_deref()),
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
            creature: Some("parrot".into()),
            vibe: Some("cheerful and curious".into()),
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
        assert!(prompt.contains("You are a parrot."));
        assert!(prompt.contains("Your vibe: cheerful and curious."));
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
                provider: Some("openai".into()),
                model: Some("gpt-5".into()),
                session_key: Some("main".into()),
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
                workspace_mount: Some("ro".into()),
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
        assert!(prompt.contains("provider=openai"));
        assert!(prompt.contains("model=gpt-5"));
        assert!(prompt.contains("sudo_non_interactive=true"));
        assert!(prompt.contains("sudo_status=passwordless"));
        assert!(prompt.contains("timezone=Europe/Paris"));
        assert!(prompt.contains("accept_language=en-US,fr;q=0.9"));
        assert!(prompt.contains("remote_ip=203.0.113.42"));
        assert!(prompt.contains("Sandbox(exec): enabled=true"));
        assert!(prompt.contains("backend=docker"));
        assert!(prompt.contains("network=disabled"));
        assert!(prompt.contains("Execution routing:"));
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
}
