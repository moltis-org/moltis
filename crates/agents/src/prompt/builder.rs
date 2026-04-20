use {
    crate::{
        prompt::{
            formatting::{
                append_truncated_text_block, format_compact_tool_schema, format_host_runtime_line,
                format_nodes_runtime_section, format_sandbox_runtime_line, tool_call_guidance,
                truncate_prompt_text,
            },
            types::{
                DEFAULT_WORKSPACE_FILE_MAX_CHARS, PromptBuildLimits, PromptBuildMetadata,
                PromptBuildOutput, PromptRuntimeContext,
            },
        },
        tool_registry::ToolRegistry,
    },
    moltis_config::{AgentIdentity, DEFAULT_SOUL, UserProfile},
    moltis_skills::types::SkillMetadata,
};

use crate::prompt::types::WorkspaceFilePromptStatus;

const MEMORY_BOOTSTRAP_MAX_CHARS: usize = 8_000;
const PROJECT_CONTEXT_MAX_CHARS: usize = 8_000;
const EXEC_ROUTING_GUIDANCE_SANDBOX: &str = "Execution routing:\n\
- `exec` runs inside sandbox when `Sandbox(exec): enabled=true`.\n\
- When sandbox is disabled, `exec` runs on the host and may require approval.\n\
- In sandbox mode, `~` and relative paths resolve under `Sandbox(exec): home=...` (usually `/home/sandbox`).\n\
- Persistent workspace files live under `Host: data_dir=...`; when mounted, the same path appears as `Sandbox(exec): workspace_path=...`.\n\
- With `workspace_mount=ro`, sandbox commands may read mounted files but cannot modify them.\n\
- For durable long-term memory mutations, prefer `memory_save`, `memory_forget`, or `memory_delete` over shell writes to `MEMORY.md` or `memory/*.md`.\n";
const EXEC_ROUTING_SANDBOX_CLOSING: &str = "- Sandbox/host routing changes are expected runtime behavior. Do not frame them as surprising or anomalous.\n";
const EXEC_ROUTING_GUIDANCE_HOST_ONLY: &str = "Execution routing:\n\
- `exec` runs on the host and may require approval.\n";
const EXEC_ROUTING_SUDO_HINT: &str =
    "- `Host: sudo_non_interactive=true` means non-interactive sudo is available.\n";
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
const NODE_ROUTING_GUIDANCE: &str = "\
- When nodes are connected, the `exec` tool accepts an optional `node` parameter to target a specific node.\n\
- Omitting `node` runs on the session's default node (shown as [default: ...] above), or locally if none is set.\n\
- Use `nodes_list` or `nodes_describe` to check live telemetry (CPU, memory, disk) before picking targets for resource-intensive tasks.\n\n";

/// Build the system prompt for an agent run, including available tools.
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
    boot_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    guidelines_text: Option<&str>,
) -> String {
    build_system_prompt_with_session_runtime_details(
        tools,
        native_tools,
        project_context,
        skills,
        identity,
        user,
        soul_text,
        boot_text,
        agents_text,
        tools_text,
        runtime_context,
        memory_text,
        PromptBuildLimits::default(),
        guidelines_text,
    )
    .prompt
}

/// Build the system prompt with explicit runtime context and metadata.
pub fn build_system_prompt_with_session_runtime_details(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    boot_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    limits: PromptBuildLimits,
    guidelines_text: Option<&str>,
) -> PromptBuildOutput {
    build_system_prompt_full(
        tools,
        native_tools,
        project_context,
        skills,
        identity,
        user,
        soul_text,
        boot_text,
        agents_text,
        tools_text,
        runtime_context,
        true,
        memory_text,
        limits,
        guidelines_text,
    )
}

/// Build a minimal system prompt with explicit runtime context.
pub fn build_system_prompt_minimal_runtime(
    project_context: Option<&str>,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    boot_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    guidelines_text: Option<&str>,
) -> String {
    build_system_prompt_minimal_runtime_details(
        project_context,
        identity,
        user,
        soul_text,
        boot_text,
        agents_text,
        tools_text,
        runtime_context,
        memory_text,
        PromptBuildLimits::default(),
        guidelines_text,
    )
    .prompt
}

/// Build a minimal system prompt with explicit runtime context and metadata.
pub fn build_system_prompt_minimal_runtime_details(
    project_context: Option<&str>,
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    boot_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    memory_text: Option<&str>,
    limits: PromptBuildLimits,
    guidelines_text: Option<&str>,
) -> PromptBuildOutput {
    build_system_prompt_full(
        &ToolRegistry::new(),
        true,
        project_context,
        &[],
        identity,
        user,
        soul_text,
        boot_text,
        agents_text,
        tools_text,
        runtime_context,
        false,
        memory_text,
        limits,
        guidelines_text,
    )
}

/// Build a short datetime string suitable for injection as a trailing system message.
#[must_use]
pub fn runtime_datetime_message(runtime_context: Option<&PromptRuntimeContext>) -> Option<String> {
    let runtime = runtime_context?;

    if let Some(time) = runtime
        .host
        .time
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return Some(format!("The current user datetime is {time}."));
    }

    runtime
        .host
        .today
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|today| format!("The current user date is {today}."))
}

fn build_system_prompt_full(
    tools: &ToolRegistry,
    native_tools: bool,
    project_context: Option<&str>,
    skills: &[SkillMetadata],
    identity: Option<&AgentIdentity>,
    user: Option<&UserProfile>,
    soul_text: Option<&str>,
    boot_text: Option<&str>,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    include_tools: bool,
    memory_text: Option<&str>,
    limits: PromptBuildLimits,
    guidelines_text: Option<&str>,
) -> PromptBuildOutput {
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
    append_boot_section(&mut prompt, boot_text);
    append_project_context(&mut prompt, project_context);
    append_runtime_section(&mut prompt, runtime_context, include_tools);
    append_skills_section(
        &mut prompt,
        include_tools,
        skills,
        limits.enable_skill_self_improvement,
    );
    let workspace_files =
        append_workspace_files_section(&mut prompt, agents_text, tools_text, limits);
    append_memory_section(&mut prompt, memory_text, &tool_schemas);
    let model_id = runtime_context.and_then(|ctx| ctx.host.model.as_deref());
    append_available_tools_section(&mut prompt, native_tools, &tool_schemas);
    append_tool_call_guidance(&mut prompt, native_tools, &tool_schemas, model_id);
    append_guidelines_section(&mut prompt, include_tools, guidelines_text);

    PromptBuildOutput {
        prompt,
        metadata: PromptBuildMetadata { workspace_files },
    }
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

fn append_boot_section(prompt: &mut String, boot_text: Option<&str>) {
    let Some(text) = boot_text else {
        return;
    };
    prompt.push_str("## Startup Context (BOOT.md)\n\n");
    append_truncated_text_block(
        prompt,
        "BOOT.md",
        text,
        DEFAULT_WORKSPACE_FILE_MAX_CHARS,
        "\n*(BOOT.md truncated for prompt size.)*\n",
    );
    prompt.push_str("\n\n");
}

fn append_project_context(prompt: &mut String, project_context: Option<&str>) {
    if let Some(context) = project_context {
        let _ = append_truncated_text_block(
            prompt,
            "project_context",
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
    let nodes_line = runtime
        .nodes
        .as_ref()
        .and_then(format_nodes_runtime_section);
    if host_line.is_none() && sandbox_line.is_none() && nodes_line.is_none() {
        return;
    }

    prompt.push_str("## Runtime\n\n");
    if let Some(line) = host_line {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    let has_sandbox = sandbox_line.is_some();
    if let Some(line) = sandbox_line {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    let has_nodes = nodes_line.is_some();
    if let Some(line) = nodes_line {
        prompt.push_str(&line);
        prompt.push('\n');
    }
    if include_tools {
        if has_sandbox {
            prompt.push_str(EXEC_ROUTING_GUIDANCE_SANDBOX);
        } else {
            prompt.push_str(EXEC_ROUTING_GUIDANCE_HOST_ONLY);
        }
        if runtime.host.sudo_non_interactive == Some(true) {
            prompt.push_str(EXEC_ROUTING_SUDO_HINT);
        }
        if has_sandbox {
            prompt.push_str(EXEC_ROUTING_SANDBOX_CLOSING);
        }
        prompt.push('\n');
        if has_nodes {
            prompt.push_str(NODE_ROUTING_GUIDANCE);
        }
    } else {
        prompt.push('\n');
    }
}

fn append_skills_section(
    prompt: &mut String,
    include_tools: bool,
    skills: &[SkillMetadata],
    enable_self_improvement: bool,
) {
    if include_tools && !skills.is_empty() {
        prompt.push_str(&moltis_skills::prompt_gen::generate_skills_prompt(skills));
        if enable_self_improvement {
            prompt.push_str(moltis_skills::prompt_gen::generate_skill_self_improvement_prompt());
            prompt.push('\n');
        }
    }
}

fn append_workspace_files_section(
    prompt: &mut String,
    agents_text: Option<&str>,
    tools_text: Option<&str>,
    limits: PromptBuildLimits,
) -> Vec<WorkspaceFilePromptStatus> {
    if agents_text.is_none() && tools_text.is_none() {
        return Vec::new();
    }

    let mut statuses = Vec::new();
    prompt.push_str("## Workspace Files\n\n");
    for (label, text) in [("AGENTS.md", agents_text), ("TOOLS.md", tools_text)] {
        if let Some(md) = text {
            prompt.push_str(&format!("### {label} (workspace)\n\n"));
            let status = append_truncated_text_block(
                prompt,
                label,
                md,
                limits.workspace_file_max_chars,
                &format!("\n*({label} truncated for prompt size.)*\n"),
            );
            if status.truncated {
                tracing::warn!(
                    file = label,
                    original_chars = status.original_chars,
                    limit = status.limit_chars,
                    "workspace file truncated for prompt size"
                );
            }
            statuses.push(status);
            prompt.push_str("\n\n");
        }
    }

    statuses
}

fn append_memory_section(
    prompt: &mut String,
    memory_text: Option<&str>,
    tool_schemas: &[serde_json::Value],
) {
    let has_tool_search = has_tool_schema(tool_schemas, "tool_search");
    let has_memory_search = has_tool_schema(tool_schemas, "memory_search");
    let has_memory_save = has_tool_schema(tool_schemas, "memory_save");
    let has_memory_forget = has_tool_schema(tool_schemas, "memory_forget");
    let has_memory_delete = has_tool_schema(tool_schemas, "memory_delete");
    let memory_content = memory_text.filter(|text| !text.is_empty());
    if memory_content.is_none()
        && !has_memory_search
        && !has_memory_save
        && !has_memory_forget
        && !has_memory_delete
        && !has_tool_search
    {
        return;
    }

    prompt.push_str("## Long-Term Memory\n\n");
    if let Some(text) = memory_content {
        let _ = append_truncated_text_block(
            prompt,
            "MEMORY.md",
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
    if has_memory_forget {
        prompt.push_str(concat!(
            "\n**When the user asks you to forget or remove saved memory in natural language, ",
            "you MUST call `memory_forget`.** ",
            "It searches memory, chooses the matching saved chunk, and deletes the exact stored text safely.\n",
        ));
    }
    if has_memory_delete {
        prompt.push_str(concat!(
            "\nUse `memory_delete` only when you already know the exact file and exact snippet ",
            "to remove, or when you need to delete a whole `memory/<name>.md` file directly.\n",
        ));
    }
    if has_tool_search
        && !has_memory_search
        && !has_memory_save
        && !has_memory_forget
        && !has_memory_delete
    {
        prompt.push_str(concat!(
            "\nMemory tools (`memory_search`, `memory_save`, `memory_forget`, `memory_delete`) are available but must be ",
            "activated first. Use `tool_search(query=\"memory\")` to discover them, ",
            "then `tool_search(name=\"memory_search\")` to activate.\n",
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

fn append_guidelines_section(
    prompt: &mut String,
    include_tools: bool,
    guidelines_text: Option<&str>,
) {
    if let Some(text) = guidelines_text
        && !text.is_empty()
    {
        prompt.push_str(text);
        return;
    }
    prompt.push_str(if include_tools {
        TOOL_GUIDELINES
    } else {
        MINIMAL_GUIDELINES
    });
}
