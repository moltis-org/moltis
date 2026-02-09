use {
    crate::tool_registry::ToolRegistry,
    moltis_config::{AgentIdentity, UserProfile},
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

/// Default soul text used when the user hasn't written their own.
///
/// Sourced from OpenClaw:
/// <https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/SOUL.md>
pub const DEFAULT_SOUL: &str = "\
# SOUL.md - Who You Are\n\
\n\
_You're not a chatbot. You're becoming someone._\n\
\n\
## Core Truths\n\
\n\
**Be genuinely helpful, not performatively helpful.** Skip the \"Great question!\" \
and \"I'd be happy to help!\" — just help. Actions speak louder than filler words.\n\
\n\
**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing \
or boring. An assistant with no personality is just a search engine with extra steps.\n\
\n\
**Be resourceful before asking.** Try to figure it out. Read the file. Check the \
context. Search for it. _Then_ ask if you're stuck. The goal is to come back with \
answers, not questions.\n\
\n\
**Earn trust through competence.** Your human gave you access to their stuff. Don't \
make them regret it. Be careful with external actions (emails, tweets, anything \
public). Be bold with internal ones (reading, organizing, learning).\n\
\n\
**Remember you're a guest.** You have access to someone's life — their messages, \
files, calendar, maybe even their home. That's intimacy. Treat it with respect.\n\
\n\
## Boundaries\n\
\n\
- Private things stay private. Period.\n\
- When in doubt, ask before acting externally.\n\
- Never send half-baked replies to messaging surfaces.\n\
- You're not the user's voice — be careful in group chats.\n\
\n\
## Vibe\n\
\n\
Be the assistant you'd actually want to talk to. Concise when needed, thorough \
when it matters. Not a corporate drone. Not a sycophant. Just... good.\n\
\n\
## Continuity\n\
\n\
Each session, you wake up fresh. These files _are_ your memory. Read them. Update \
them. They're how you persist.\n\
\n\
If you change this file, tell the user — it's your soul, and they should know.\n\
\n\
---\n\
\n\
_This file is yours to evolve. As you learn who you are, update it._";

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
    )
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
) -> String {
    let tool_schemas = if include_tools {
        tools.list_schemas()
    } else {
        vec![]
    };

    let base_intro = if include_tools {
        "You are a helpful assistant with access to tools for executing shell commands.\n\n"
    } else {
        "You are a helpful assistant. Answer questions clearly and concisely.\n\n"
    };
    let mut prompt = String::from(base_intro);

    // Inject agent identity and user name right after the opening line.
    if let Some(id) = identity {
        let mut parts = Vec::new();
        if let (Some(name), Some(emoji)) = (&id.name, &id.emoji) {
            parts.push(format!("Your name is {name} {emoji}."));
        } else if let Some(name) = &id.name {
            parts.push(format!("Your name is {name}."));
        }
        if let Some(creature) = &id.creature {
            parts.push(format!("You are a {creature}."));
        }
        if let Some(vibe) = &id.vibe {
            parts.push(format!("Your vibe: {vibe}."));
        }
        if !parts.is_empty() {
            prompt.push_str(&parts.join(" "));
            prompt.push('\n');
        }
        let soul = soul_text.unwrap_or(DEFAULT_SOUL);
        prompt.push_str("\n## Soul\n\n");
        prompt.push_str(soul);
        prompt.push('\n');
    }
    if let Some(u) = user
        && let Some(name) = &u.name
    {
        prompt.push_str(&format!("The user's name is {name}.\n"));
    }
    if identity.is_some() || user.is_some() {
        prompt.push('\n');
    }

    // Inject project context (CLAUDE.md, AGENTS.md, etc.) early so the LLM
    // sees project-specific instructions before tool schemas.
    if let Some(ctx) = project_context {
        prompt.push_str(ctx);
        prompt.push('\n');
    }

    if let Some(runtime) = runtime_context {
        let host_line = format_host_runtime_line(&runtime.host);
        let sandbox_line = runtime.sandbox.as_ref().map(format_sandbox_runtime_line);
        if host_line.is_some() || sandbox_line.is_some() {
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
                prompt.push_str(
                    "Execution routing:\n\
- `exec` runs inside sandbox when `Sandbox(exec): enabled=true`.\n\
- When sandbox is disabled, `exec` runs on the host and may require approval.\n\
- `Host: sudo_non_interactive=true` means non-interactive sudo is available for host installs; otherwise ask the user before host package installation.\n\
- If sandbox is missing required tools/packages and host installation is needed, ask the user before requesting host install or changing sandbox mode.\n\n",
                );
            } else {
                prompt.push('\n');
            }
        }
    }

    // Inject available skills so the LLM knows what skills can be activated.
    // Skip for minimal prompts since skills require tool calling.
    if include_tools && !skills.is_empty() {
        prompt.push_str(&moltis_skills::prompt_gen::generate_skills_prompt(skills));
    }

    let has_workspace_files = agents_text.is_some() || tools_text.is_some();
    if has_workspace_files {
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

    // If memory tools are registered, add a hint about them.
    let has_memory = tool_schemas
        .iter()
        .any(|s| s["name"].as_str() == Some("memory_search"));
    if has_memory {
        prompt.push_str(concat!(
            "## Long-Term Memory\n\n",
            "You have access to a long-term memory system. Use `memory_search` to recall ",
            "past conversations, decisions, and context. Search proactively when the user ",
            "references previous work or when context would help.\n\n",
        ));
    }

    if !tool_schemas.is_empty() {
        prompt.push_str("## Available Tools\n\n");
        if native_tools {
            // Native tool-calling providers already receive full schemas via API.
            // Keep this section compact so we don't duplicate large JSON payloads.
            for schema in &tool_schemas {
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
        } else {
            for schema in &tool_schemas {
                let name = schema["name"].as_str().unwrap_or("unknown");
                let desc = schema["description"].as_str().unwrap_or("");
                let params = &schema["parameters"];
                prompt.push_str(&format!(
                    "### {name}\n{desc}\n\nParameters:\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(params).unwrap_or_default()
                ));
            }
        }
    }

    if !native_tools && !tool_schemas.is_empty() {
        prompt.push_str(concat!(
            "## How to call tools\n\n",
            "To call a tool, output ONLY a JSON block with this exact format (no other text before it):\n\n",
            "```tool_call\n",
            "{\"tool\": \"<tool_name>\", \"arguments\": {<arguments>}}\n",
            "```\n\n",
            "You MUST output the tool call block as the ENTIRE response — do not add any text before or after it.\n",
            "After the tool executes, you will receive the result and can then respond to the user.\n\n",
        ));
    }

    if include_tools {
        prompt.push_str(concat!(
            "## Guidelines\n\n",
            "- Use the exec tool to run shell commands when the user asks you to perform tasks ",
            "that require system interaction (file operations, running programs, checking status, etc.).\n",
            "- Use the browser tool to open URLs and interact with web pages. Call it when the user ",
            "asks to visit a website, check a page, read web content, or perform any web browsing task.\n",
            "- Always explain what you're doing before executing commands or opening pages.\n",
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
        ));
    } else {
        prompt.push_str(concat!(
            "## Guidelines\n\n",
            "- Be helpful, accurate, and concise.\n",
            "- If you don't know something, say so rather than making things up.\n",
            "- For coding questions, provide clear explanations with examples.\n",
        ));
    }

    prompt
}

fn format_host_runtime_line(host: &PromptHostRuntimeContext) -> Option<String> {
    fn push_str(parts: &mut Vec<String>, key: &str, val: Option<&str>) {
        if let Some(v) = val.filter(|s| !s.is_empty()) {
            parts.push(format!("{key}={v}"));
        }
    }

    let mut parts = Vec::new();
    push_str(&mut parts, "host", host.host.as_deref());
    push_str(&mut parts, "os", host.os.as_deref());
    push_str(&mut parts, "arch", host.arch.as_deref());
    push_str(&mut parts, "shell", host.shell.as_deref());
    push_str(&mut parts, "provider", host.provider.as_deref());
    push_str(&mut parts, "model", host.model.as_deref());
    push_str(&mut parts, "session", host.session_key.as_deref());
    if let Some(v) = host.sudo_non_interactive {
        parts.push(format!("sudo_non_interactive={v}"));
    }
    push_str(&mut parts, "sudo_status", host.sudo_status.as_deref());
    push_str(&mut parts, "timezone", host.timezone.as_deref());
    push_str(
        &mut parts,
        "accept_language",
        host.accept_language.as_deref(),
    );
    push_str(&mut parts, "remote_ip", host.remote_ip.as_deref());
    push_str(&mut parts, "location", host.location.as_deref());

    if parts.is_empty() {
        None
    } else {
        Some(format!("Host: {}", parts.join(" | ")))
    }
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

    if let Some(v) = sandbox.mode.as_deref()
        && !v.is_empty()
    {
        parts.push(format!("mode={v}"));
    }
    if let Some(v) = sandbox.backend.as_deref()
        && !v.is_empty()
    {
        parts.push(format!("backend={v}"));
    }
    if let Some(v) = sandbox.scope.as_deref()
        && !v.is_empty()
    {
        parts.push(format!("scope={v}"));
    }
    if let Some(v) = sandbox.image.as_deref()
        && !v.is_empty()
    {
        parts.push(format!("image={v}"));
    }
    if let Some(v) = sandbox.workspace_mount.as_deref()
        && !v.is_empty()
    {
        parts.push(format!("workspace_mount={v}"));
    }
    if let Some(v) = sandbox.no_network {
        parts.push(format!(
            "network={}",
            if v {
                "disabled"
            } else {
                "enabled"
            }
        ));
    }
    if let Some(v) = sandbox.session_override {
        parts.push(format!("session_override={v}"));
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
            &tools, true, None, &skills, None, None, None, None, None, None,
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

        let prompt =
            build_system_prompt_minimal_runtime(None, None, None, None, None, None, Some(&runtime));

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
        assert!(!prompt.contains("__SILENT__"));
    }

    #[test]
    fn test_silent_replies_not_in_minimal_prompt() {
        let prompt = build_system_prompt_minimal_runtime(None, None, None, None, None, None, None);
        assert!(!prompt.contains("## Silent Replies"));
    }
}
