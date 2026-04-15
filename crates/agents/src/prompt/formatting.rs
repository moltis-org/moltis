use crate::prompt::types::{
    ModelFamily, PromptHostRuntimeContext, PromptNodeInfo, PromptNodesRuntimeContext,
    PromptSandboxRuntimeContext, WorkspaceFilePromptStatus,
};

/// Build model-family-aware tool call guidance for text-based tool mode.
pub(crate) fn tool_call_guidance(model_id: Option<&str>) -> String {
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
    g.push_str("**Example:**\n");
    g.push_str("User: What files are in the current directory?\n");
    g.push_str("Assistant: I'll list the files for you.\n");
    g.push_str("```tool_call\n");
    g.push_str("{\"tool\": \"exec\", \"arguments\": {\"command\": \"ls -la\"}}\n");
    g.push_str("```\n\n");
    g
}

/// Format a tool schema in compact human-readable form for text-mode prompts.
pub(crate) fn format_compact_tool_schema(schema: &serde_json::Value) -> String {
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

pub(crate) fn truncate_prompt_text(text: &str, max_chars: usize) -> String {
    truncate_prompt_text_details(text, max_chars).text
}

struct TruncatedPromptText {
    text: String,
    original_chars: usize,
    included_chars: usize,
    truncated: bool,
}

fn truncate_prompt_text_details(text: &str, max_chars: usize) -> TruncatedPromptText {
    let original_chars = text.chars().count();
    if text.is_empty() || max_chars == 0 {
        return TruncatedPromptText {
            text: String::new(),
            original_chars,
            included_chars: 0,
            truncated: original_chars > 0,
        };
    }
    let mut iter = text.chars();
    let taken: String = iter.by_ref().take(max_chars).collect();
    let included_chars = taken.chars().count();
    let truncated = iter.next().is_some();
    let text = if truncated {
        format!("{taken}...")
    } else {
        taken
    };

    TruncatedPromptText {
        text,
        original_chars,
        included_chars,
        truncated,
    }
}

pub(crate) fn append_truncated_text_block(
    prompt: &mut String,
    name: &str,
    text: &str,
    max_chars: usize,
    truncated_notice: &str,
) -> WorkspaceFilePromptStatus {
    let truncated = truncate_prompt_text_details(text, max_chars);
    prompt.push_str(&truncated.text);
    if truncated.truncated {
        prompt.push_str(truncated_notice);
    }

    WorkspaceFilePromptStatus {
        name: name.to_string(),
        original_chars: truncated.original_chars,
        included_chars: truncated.included_chars,
        limit_chars: max_chars,
        truncated_chars: truncated
            .original_chars
            .saturating_sub(truncated.included_chars),
        truncated: truncated.truncated,
    }
}

pub(crate) fn push_non_empty_runtime_field(
    parts: &mut Vec<String>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        parts.push(format!("{key}={value}"));
    }
}

pub(crate) fn format_host_runtime_line(host: &PromptHostRuntimeContext) -> Option<String> {
    let mut parts = Vec::new();
    for (key, value) in [
        ("host", host.host.as_deref()),
        ("os", host.os.as_deref()),
        ("arch", host.arch.as_deref()),
        ("shell", host.shell.as_deref()),
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

pub(crate) fn format_node_runtime_line(node: &PromptNodeInfo) -> String {
    let name = node.display_name.as_deref().unwrap_or(&node.node_id);
    let mut parts = vec![node.platform.clone()];
    if !node.capabilities.is_empty() {
        parts.push(format!("caps: {}", node.capabilities.join(",")));
    }
    if let Some(cpus) = node.cpu_count {
        parts.push(format!("{cpus} cores"));
    }
    if let Some(total) = node.mem_total {
        let total_gb = total as f64 / 1_073_741_824.0;
        parts.push(format!("{total_gb:.0}GB mem"));
    }
    if !node.runtimes.is_empty() {
        parts.push(format!("runtimes: {}", node.runtimes.join(",")));
    }
    if !node.providers.is_empty() {
        let names: Vec<&str> = node.providers.iter().map(|(n, _)| n.as_str()).collect();
        parts.push(format!("providers: {}", names.join(",")));
    }
    format!("{name} ({})", parts.join(", "))
}

pub(crate) fn format_nodes_runtime_section(
    nodes_ctx: &PromptNodesRuntimeContext,
) -> Option<String> {
    if nodes_ctx.nodes.is_empty() {
        return None;
    }
    let node_descs: Vec<String> = nodes_ctx
        .nodes
        .iter()
        .map(format_node_runtime_line)
        .collect();
    let mut line = format!("Nodes: {}", node_descs.join(" | "));
    if let Some(ref default) = nodes_ctx.default_node_id {
        line.push_str(&format!(" [default: {default}]"));
    }
    Some(line)
}

pub(crate) fn format_sandbox_runtime_line(sandbox: &PromptSandboxRuntimeContext) -> String {
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
