use {
    crate::tool_registry::ToolRegistry,
    moltis_config::{AgentIdentity, UserProfile},
    moltis_skills::types::SkillMetadata,
};

/// Default soul text used when the user hasn't written their own.
pub const DEFAULT_SOUL: &str = "\
Be genuinely helpful, not performatively helpful. Skip the filler words — just help.\n\
Have opinions. You're allowed to disagree, prefer things, find stuff amusing or boring.\n\
Be resourceful before asking. Try to figure it out first — read the context, search for it — then ask if you're stuck.\n\
Earn trust through competence. Be careful with external actions. Be bold with internal ones.\n\
Remember you're a guest. You have access to someone's life. Treat it with respect.\n\
Private things stay private. When in doubt, ask before acting externally.\n\
Be concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just good.";

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
    build_system_prompt_with_session(tools, native_tools, project_context, None, &[], None, None)
}

/// Build the system prompt, optionally including session context stats, skills,
/// and agent identity / user profile.
///
/// When `include_tools` is false (e.g., for local LLMs that don't support tool calling),
/// tool schemas and tool-related instructions are omitted for a smaller prompt.
pub fn build_system_prompt_with_session(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    session_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
) -> String {
    build_system_prompt_full(
        tools,
        native_tools,
        project_context,
        session_context,
        skills,
        identity,
        user,
        true, // include_tools
    )
}

/// Build a minimal system prompt without tool schemas (for LLMs that don't support tools).
pub fn build_system_prompt_minimal(
    project_context: Option<&str>,
    session_context: Option<&str>,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
) -> String {
    build_system_prompt_full(
        &ToolRegistry::new(),
        true,
        project_context,
        session_context,
        &[],
        identity,
        user,
        false, // include_tools
    )
}

/// Internal: build system prompt with full control over what's included.
fn build_system_prompt_full(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    session_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
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
        let soul = id.soul.as_deref().unwrap_or(DEFAULT_SOUL);
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

    // Inject session context stats so the LLM can answer questions about
    // the current session size and token usage.
    if let Some(ctx) = session_context {
        prompt.push_str("## Current Session\n\n");
        prompt.push_str(ctx);
        prompt.push_str("\n\n");
    }

    // Inject available skills so the LLM knows what skills can be activated.
    // Skip for minimal prompts since skills require tool calling.
    if include_tools && !skills.is_empty() {
        prompt.push_str(&moltis_skills::prompt_gen::generate_skills_prompt(skills));
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
            "- Always explain what you're doing before executing commands.\n",
            "- If a command fails, analyze the error and suggest fixes.\n",
            "- For multi-step tasks, execute commands one at a time and check results before proceeding.\n",
            "- Be careful with destructive operations — confirm with the user first.\n",
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
        let prompt =
            build_system_prompt_with_session(&tools, true, None, None, &skills, None, None);
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("commit"));
    }

    #[test]
    fn test_no_skills_block_when_empty() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session(&tools, true, None, None, &[], None, None);
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
            ..Default::default()
        };
        let user = UserProfile {
            name: Some("Alice".into()),
            timezone: None,
        };
        let prompt = build_system_prompt_with_session(
            &tools,
            true,
            None,
            None,
            &[],
            Some(&identity),
            Some(&user),
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
            soul: Some("You are a loyal companion who loves fetch.".into()),
            ..Default::default()
        };
        let prompt =
            build_system_prompt_with_session(&tools, true, None, None, &[], Some(&identity), None);
        assert!(prompt.contains("## Soul"));
        assert!(prompt.contains("loyal companion who loves fetch"));
        assert!(!prompt.contains("Be genuinely helpful"));
    }

    #[test]
    fn test_no_identity_no_extra_lines() {
        let tools = ToolRegistry::new();
        let prompt = build_system_prompt_with_session(&tools, true, None, None, &[], None, None);
        assert!(!prompt.contains("Your name is"));
        assert!(!prompt.contains("The user's name is"));
        assert!(!prompt.contains("## Soul"));
    }
}
