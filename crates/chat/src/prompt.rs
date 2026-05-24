//! Prompt persona, memory management, and runtime context building.

use std::sync::Arc;

use {serde_json::Value, tracing::warn};

use {
    moltis_agents::{
        prompt::{
            PromptBuildLimits, PromptHostRuntimeContext, PromptModeRuntimeContext, PromptNodeInfo,
            PromptNodesRuntimeContext, PromptRuntimeContext, PromptSandboxRuntimeContext,
        },
        tool_registry::ToolSource,
    },
    moltis_config::{AgentMemoryWriteMode, LoadedWorkspaceMarkdown, MemoryStyle, PromptMemoryMode},
    moltis_sessions::{metadata::SessionEntry, state_store::SessionStateStore},
    moltis_tools::policy::{PolicyContext, resolve_effective_policy},
};

use crate::{
    runtime::ChatRuntime,
    types::{
        PromptMemoryStatus, PromptPersona, default_user_prompt_timezone, detect_host_sudo_access,
        detect_runtime_shell, memory_style_allows_prompt, normalized_iana_timezone,
        prompt_sandbox_no_network_state, refresh_runtime_prompt_time, server_prompt_timezone,
    },
};

pub(crate) const PROMPT_MEMORY_NAMESPACE: &str = "__prompt_memory";

pub(crate) fn prompt_memory_snapshot_key(agent_id: &str) -> String {
    format!("snapshot:{agent_id}")
}

pub(crate) async fn clear_prompt_memory_snapshot(
    session_key: &str,
    agent_id: &str,
    state_store: Option<&SessionStateStore>,
) -> bool {
    let Some(store) = state_store else {
        return false;
    };
    let key = prompt_memory_snapshot_key(agent_id);
    match store
        .delete(session_key, PROMPT_MEMORY_NAMESPACE, &key)
        .await
    {
        Ok(deleted) => deleted,
        Err(error) => {
            warn!(
                session = %session_key,
                agent_id,
                error = %error,
                "failed to clear prompt memory snapshot"
            );
            false
        },
    }
}

pub(crate) fn prompt_memory_status(
    style: MemoryStyle,
    mode: PromptMemoryMode,
    write_mode: AgentMemoryWriteMode,
    snapshot_active: bool,
    memory: Option<&LoadedWorkspaceMarkdown>,
) -> PromptMemoryStatus {
    PromptMemoryStatus {
        style,
        mode,
        write_mode,
        snapshot_active,
        present: memory.is_some(),
        chars: memory.map_or(0, |entry| entry.content.chars().count()),
        path: memory.map(|entry| entry.path.to_string_lossy().into_owned()),
        file_source: memory.map(|entry| entry.source),
    }
}

pub(crate) fn resolve_prompt_agent_id(session_entry: Option<&SessionEntry>) -> String {
    let Some(entry) = session_entry else {
        return "main".to_string();
    };
    let Some(agent_id) = entry
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return "main".to_string();
    };
    if moltis_config::agent_workspace_dir(agent_id).exists() {
        return agent_id.to_string();
    }
    warn!(
        session = %entry.key,
        agent_id,
        "session references unknown agent workspace, falling back to main prompt persona"
    );
    "main".to_string()
}

pub(crate) fn resolve_prompt_mode_context(
    config: &moltis_config::MoltisConfig,
    session_entry: Option<&SessionEntry>,
) -> Option<PromptModeRuntimeContext> {
    let mode_id = session_entry?
        .mode_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mode = config.modes.get_preset(mode_id)?;
    let prompt = mode.prompt.trim();
    if prompt.is_empty() {
        return None;
    }
    Some(PromptModeRuntimeContext {
        id: mode_id.to_string(),
        name: mode.name.clone().unwrap_or_else(|| mode_id.to_string()),
        prompt: prompt.to_string(),
    })
}

/// Load identity, user profile, soul, and workspace text for one agent.
pub(crate) fn load_prompt_persona_base_for_agent(agent_id: &str) -> PromptPersona {
    let config = moltis_config::discover_and_load();
    let prompt_memory_mode = config.chat.prompt_memory_mode;
    let agent_write_mode = config.memory.agent_write_mode;
    let memory_style = config.memory.style;
    let identity =
        moltis_config::load_identity_for_agent(agent_id).unwrap_or_else(|| config.identity.clone());
    let user = moltis_config::resolve_user_profile_from_config(&config);
    PromptPersona {
        config,
        identity,
        user,
        soul_text: moltis_config::load_soul_for_agent(agent_id),
        boot_text: moltis_config::load_boot_md_for_agent(agent_id),
        agents_text: moltis_config::load_agents_md_for_agent(agent_id),
        tools_text: moltis_config::load_tools_md_for_agent(agent_id),
        guidelines_text: moltis_config::load_guidelines_md_for_agent(agent_id),
        memory_text: None,
        memory_status: prompt_memory_status(
            memory_style,
            prompt_memory_mode,
            agent_write_mode,
            false,
            None,
        ),
    }
}

pub(crate) fn load_prompt_persona_for_agent(agent_id: &str) -> PromptPersona {
    let mut persona = load_prompt_persona_base_for_agent(agent_id);
    let style = persona.config.memory.style;
    let mode = persona.config.chat.prompt_memory_mode;
    let write_mode = persona.config.memory.agent_write_mode;
    let memory = if memory_style_allows_prompt(style) {
        moltis_config::load_memory_md_for_agent_with_source(agent_id)
    } else {
        None
    };
    persona.memory_text = memory.as_ref().map(|entry| entry.content.clone());
    persona.memory_status = prompt_memory_status(style, mode, write_mode, false, memory.as_ref());
    persona
}

pub(crate) async fn load_prompt_memory_for_session(
    session_key: &str,
    agent_id: &str,
    mode: PromptMemoryMode,
    state_store: Option<&SessionStateStore>,
) -> (Option<LoadedWorkspaceMarkdown>, bool) {
    let live_memory = || moltis_config::load_memory_md_for_agent_with_source(agent_id);

    if !matches!(mode, PromptMemoryMode::FrozenAtSessionStart) {
        return (live_memory(), false);
    }

    let Some(store) = state_store else {
        return (live_memory(), false);
    };

    let key = prompt_memory_snapshot_key(agent_id);
    match store.get(session_key, PROMPT_MEMORY_NAMESPACE, &key).await {
        Ok(Some(raw)) => match serde_json::from_str::<Option<LoadedWorkspaceMarkdown>>(&raw) {
            Ok(snapshot) => return (snapshot, true),
            Err(error) => warn!(
                session = %session_key,
                agent_id,
                error = %error,
                "failed to deserialize prompt memory snapshot, rebuilding"
            ),
        },
        Ok(None) => {},
        Err(error) => warn!(
            session = %session_key,
            agent_id,
            error = %error,
            "failed to read prompt memory snapshot, falling back to live memory"
        ),
    }

    let memory = live_memory();
    match serde_json::to_string(&memory) {
        Ok(serialized) => {
            if let Err(error) = store
                .set(session_key, PROMPT_MEMORY_NAMESPACE, &key, &serialized)
                .await
            {
                warn!(
                    session = %session_key,
                    agent_id,
                    error = %error,
                    "failed to persist prompt memory snapshot"
                );
                return (memory, false);
            }
            (memory, true)
        },
        Err(error) => {
            warn!(
                session = %session_key,
                agent_id,
                error = %error,
                "failed to serialize prompt memory snapshot"
            );
            (memory, false)
        },
    }
}

pub(crate) async fn load_prompt_persona_for_session(
    session_key: &str,
    session_entry: Option<&SessionEntry>,
    state_store: Option<&SessionStateStore>,
) -> PromptPersona {
    let agent_id = resolve_prompt_agent_id(session_entry);
    let mut persona = load_prompt_persona_base_for_agent(&agent_id);
    let style = persona.config.memory.style;
    let mode = persona.config.chat.prompt_memory_mode;
    let write_mode = persona.config.memory.agent_write_mode;
    let (memory, snapshot_active) = if memory_style_allows_prompt(style) {
        load_prompt_memory_for_session(session_key, &agent_id, mode, state_store).await
    } else {
        (None, false)
    };
    persona.memory_text = memory.as_ref().map(|entry| entry.content.clone());
    persona.memory_status =
        prompt_memory_status(style, mode, write_mode, snapshot_active, memory.as_ref());
    persona
}

pub(crate) fn prompt_build_limits_from_config(
    config: &moltis_config::MoltisConfig,
) -> PromptBuildLimits {
    PromptBuildLimits {
        workspace_file_max_chars: config.chat.workspace_file_max_chars,
        enable_skill_self_improvement: config.skills.enable_self_improvement,
    }
}

/// Discover skills from the default filesystem paths, honoring `[skills] enabled`.
///
/// Returns an empty list when `config.skills.enabled` is `false`, so callers can
/// unconditionally feed the result into prompt building / tool filtering without
/// injecting skills into the LLM context when the operator has disabled them.
pub(crate) async fn discover_skills_if_enabled(
    config: &moltis_config::MoltisConfig,
) -> Vec<moltis_skills::types::SkillMetadata> {
    if !config.skills.enabled {
        return Vec::new();
    }
    let fs_discoverer = moltis_skills::discover::FsSkillDiscoverer::new(
        moltis_skills::discover::FsSkillDiscoverer::default_paths(),
    );

    #[cfg(feature = "bundled-skills")]
    let skills = {
        use moltis_skills::discover::SkillDiscoverer;
        let bundled = Arc::new(moltis_skills::bundled::BundledSkillStore::new());
        let composite = moltis_skills::discover::CompositeSkillDiscoverer::new(
            Box::new(fs_discoverer),
            bundled,
        );
        composite.discover().await
    };
    #[cfg(not(feature = "bundled-skills"))]
    let skills = {
        use moltis_skills::discover::SkillDiscoverer;
        fs_discoverer.discover().await
    };

    let disabled_cats = &config.skills.disabled_bundled_categories;

    match skills {
        Ok(skills) if disabled_cats.is_empty() => skills,
        Ok(skills) => skills
            .into_iter()
            .filter(|s| {
                // Only filter bundled skills; non-bundled skills pass through.
                if s.source != Some(moltis_skills::types::SkillSource::Bundled) {
                    return true;
                }
                // Keep the skill if its category is not in the disabled list.
                s.category
                    .as_deref()
                    .is_none_or(|cat| !disabled_cats.iter().any(|d| d == cat))
            })
            .collect(),
        Err(e) => {
            warn!("failed to discover skills: {e}");
            Vec::new()
        },
    }
}

/// Apply per-agent skill policy to a discovered skill list.
///
/// When the agent preset has a `skills.allow` list, only skills matching
/// by name or category are kept. Skills in `skills.deny` are then removed.
pub(crate) fn filter_skills_for_agent(
    skills: Vec<moltis_skills::types::SkillMetadata>,
    policy: &moltis_config::schema::PresetSkillPolicy,
) -> Vec<moltis_skills::types::SkillMetadata> {
    if policy.is_empty() {
        return skills;
    }
    skills
        .into_iter()
        .filter(|s| {
            // If allow is Some, must match by name or category.
            // Some(vec![]) means "no skills allowed" — filters everything.
            if let Some(ref allow) = policy.allow
                && !allow
                    .iter()
                    .any(|a| a == &s.name || s.category.as_deref().is_some_and(|cat| a == cat))
            {
                return false;
            }
            // Deny by name or category (if present).
            if let Some(ref deny) = policy.deny
                && deny
                    .iter()
                    .any(|d| d == &s.name || s.category.as_deref().is_some_and(|cat| d == cat))
            {
                return false;
            }
            true
        })
        .collect()
}

pub(crate) fn resolve_channel_runtime_context(
    session_key: &str,
    session_entry: Option<&SessionEntry>,
) -> moltis_common::hooks::ChannelBinding {
    match moltis_channels::resolve_session_channel_binding(
        session_key,
        session_entry.and_then(|entry| entry.channel_binding.as_deref()),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            warn!(
                error = %error,
                session = %session_key,
                "failed to parse channel_binding JSON; falling back to web"
            );
            moltis_channels::web_session_channel_binding()
        },
    }
}

pub(crate) fn channel_binding_from_runtime_context(
    runtime_context: Option<&PromptRuntimeContext>,
) -> Option<moltis_common::hooks::ChannelBinding> {
    let host = &runtime_context?.host;
    let binding = moltis_common::hooks::ChannelBinding {
        surface: host.surface.clone(),
        session_kind: host.session_kind.clone(),
        channel_type: host.channel_type.clone(),
        account_id: host.channel_account_id.clone(),
        chat_id: host.channel_chat_id.clone(),
        chat_type: host.channel_chat_type.clone(),
        sender_id: host.channel_sender_id.clone(),
    };
    (!binding.is_empty()).then_some(binding)
}

pub(crate) fn build_tool_context(
    session_key: &str,
    accept_language: Option<&str>,
    conn_id: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
) -> Value {
    let mut tool_context = serde_json::json!({
        "_session_key": session_key,
    });
    if let Some(channel_binding) = channel_binding_from_runtime_context(runtime_context)
        && let Ok(channel_value) = serde_json::to_value(channel_binding)
    {
        tool_context["_channel"] = channel_value;
    }
    if let Some(lang) = accept_language {
        tool_context["_accept_language"] = serde_json::json!(lang);
    }
    if let Some(cid) = conn_id {
        tool_context["_conn_id"] = serde_json::json!(cid);
    }
    tool_context
}

pub(crate) async fn build_prompt_runtime_context(
    state: &Arc<dyn ChatRuntime>,
    config: &moltis_config::MoltisConfig,
    provider: &Arc<dyn moltis_agents::model::LlmProvider>,
    session_key: &str,
    session_entry: Option<&SessionEntry>,
) -> PromptRuntimeContext {
    let data_dir = moltis_config::data_dir();
    let data_dir_display = data_dir.display().to_string();
    let docs_reference =
        moltis_agents::docs::cached_moltis_docs_reference(&data_dir, config.server.port);

    let sudo_fut = detect_host_sudo_access();
    let sandbox_fut = async {
        if let Some(router) = state.sandbox_router() {
            let is_sandboxed = router.is_sandboxed(session_key).await;
            // Only include sandbox context when sandbox is actually enabled for
            // this session.  When disabled, omitting it prevents the LLM from
            // hallucinating sandbox usage (see #360).  This intentionally
            // discards `session_override` — its only consumer is the prompt
            // line we are omitting, and no other code reads it from
            // `PromptSandboxRuntimeContext`.
            if !is_sandboxed {
                return None;
            }
            let config = router.config();
            let backend_name = router.backend_name();
            let workspace_mount = config.workspace_mount.to_string();
            let workspace_path = (workspace_mount != "none").then(|| data_dir_display.clone());
            Some(PromptSandboxRuntimeContext {
                exec_sandboxed: true,
                mode: Some(config.mode.to_string()),
                backend: Some(backend_name.to_string()),
                scope: Some(config.scope.to_string()),
                image: Some(router.resolve_image_nowait(session_key, None).await),
                home: Some("/home/sandbox".to_string()),
                workspace_mount: Some(workspace_mount),
                workspace_path,
                no_network: prompt_sandbox_no_network_state(backend_name, config.no_network),
                session_override: session_entry.and_then(|entry| entry.sandbox_enabled),
            })
        } else {
            None
        }
    };

    let ((sudo_non_interactive, sudo_status), sandbox_ctx) = tokio::join!(sudo_fut, sandbox_fut);

    let configured_timezone = state
        .sandbox_router()
        .and_then(|r| r.config().timezone.clone());
    let timezone = Some(server_prompt_timezone(configured_timezone.as_deref()));

    let location = state
        .cached_location()
        .await
        .as_ref()
        .map(|loc| loc.to_string());
    let channel_context = resolve_channel_runtime_context(session_key, session_entry);

    let mut host_ctx = PromptHostRuntimeContext {
        host: Some(state.hostname().to_string()),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        shell: detect_runtime_shell(),
        time: None,
        provider: Some(provider.name().to_string()),
        model: Some(provider.id().to_string()),
        session_key: Some(session_key.to_string()),
        surface: channel_context.surface,
        session_kind: channel_context.session_kind,
        channel_type: channel_context.channel_type,
        channel_account_id: channel_context.account_id,
        channel_chat_id: channel_context.chat_id,
        channel_chat_type: channel_context.chat_type,
        data_dir: Some(data_dir_display),
        docs_path: docs_reference
            .as_ref()
            .map(|reference| reference.docs_dir.display().to_string()),
        config_template_path: docs_reference
            .as_ref()
            .and_then(|reference| reference.config_template_path.as_ref())
            .map(|path| path.display().to_string()),
        sudo_non_interactive,
        sudo_status,
        timezone,
        location,
        ..Default::default()
    };
    refresh_runtime_prompt_time(&mut host_ctx);

    // Build nodes context from connected remote nodes.
    let connected = state.connected_nodes().await;
    let nodes_ctx = if connected.is_empty() {
        None
    } else {
        let default_node_id = session_entry.and_then(|e| e.node_id.clone());
        Some(PromptNodesRuntimeContext {
            nodes: connected
                .into_iter()
                .map(|n| PromptNodeInfo {
                    node_id: n.node_id,
                    display_name: n.display_name,
                    platform: n.platform,
                    capabilities: n.capabilities,
                    cpu_count: n.cpu_count,
                    mem_total: n.mem_total,
                    runtimes: n.runtimes,
                    providers: n.providers,
                })
                .collect(),
            default_node_id,
        })
    };

    PromptRuntimeContext {
        host: host_ctx,
        sandbox: sandbox_ctx,
        nodes: nodes_ctx,
        mode: None,
    }
}

pub(crate) fn apply_request_runtime_context(host: &mut PromptHostRuntimeContext, params: &Value) {
    host.accept_language = params
        .get("_accept_language")
        .and_then(|v| v.as_str())
        .map(String::from);
    host.remote_ip = params
        .get("_remote_ip")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Extract sender_id from channel metadata (set by channel handlers).
    if host.channel_sender_id.is_none() {
        host.channel_sender_id = params
            .get("channel")
            .and_then(|ch| ch.get("sender_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
    }

    if let Some(timezone) =
        normalized_iana_timezone(params.get("_timezone").and_then(|v| v.as_str()))
            .or_else(default_user_prompt_timezone)
    {
        host.timezone = Some(timezone);
    }

    refresh_runtime_prompt_time(host);
}

pub(crate) fn apply_runtime_tool_filters(
    base: &moltis_agents::tool_registry::ToolRegistry,
    config: &moltis_config::MoltisConfig,
    _skills: &[moltis_skills::types::SkillMetadata],
    mcp_disabled: bool,
    policy_context: &PolicyContext,
) -> moltis_agents::tool_registry::ToolRegistry {
    let base_registry = if mcp_disabled {
        base.clone_without_mcp()
    } else {
        base.clone_without(&[])
    };

    let policy = resolve_effective_policy(config, policy_context);

    // Resolve MCP allow-list: if the agent preset uses Allow mode, only
    // tools from listed servers pass through. This is handled here (not
    // in the policy deny layer) because ToolPolicy's deny-wins-over-allow
    // semantics can't express "deny all MCP except these servers".
    let mcp_allow: Option<&[moltis_config::schema::McpServerId]> = config
        .agents
        .get_preset(&policy_context.agent_id)
        .and_then(|p| match &p.mcp {
            moltis_config::schema::PresetMcpPolicy::Allow(servers) => Some(servers.as_slice()),
            _ => None,
        });

    base_registry.clone_allowed_entries(|name, source| {
        if !policy.is_allowed(name) {
            return false;
        }
        if let (Some(allowed_servers), ToolSource::Mcp { server }) = (mcp_allow, source) {
            return allowed_servers.iter().any(|allowed| allowed == server);
        }
        true
    })
}

/// Build a `PolicyContext` from runtime context and request parameters.
pub(crate) fn build_policy_context(
    agent_id: &str,
    runtime_context: Option<&PromptRuntimeContext>,
    params: Option<&Value>,
) -> PolicyContext {
    let host = runtime_context.map(|rc| &rc.host);
    // sender_id: prefer params["channel"]["sender_id"] (fresh from channel
    // dispatch), fall back to host.channel_sender_id (set by
    // apply_request_runtime_context earlier in the call chain).
    let sender_id = params
        .and_then(|p| p.get("channel"))
        .and_then(|ch| ch.get("sender_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| host.and_then(|h| h.channel_sender_id.clone()));
    PolicyContext {
        agent_id: agent_id.to_string(),
        provider: host.and_then(|h| h.provider.clone()),
        channel: host.and_then(|h| h.channel_type.clone()),
        channel_account_id: host.and_then(|h| h.channel_account_id.clone()),
        group_id: host.and_then(|h| h.channel_chat_type.clone()),
        sender_id,
        sandboxed: runtime_context
            .and_then(|rc| rc.sandbox.as_ref())
            .is_some_and(|s| s.exec_sandboxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool(&'static str);

    #[async_trait::async_trait]
    impl moltis_agents::tool_registry::AgentTool for DummyTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _params: Value) -> anyhow::Result<Value> {
            Ok(serde_json::json!({}))
        }
    }

    fn registry_with_mcp_tools() -> moltis_agents::tool_registry::ToolRegistry {
        let mut registry = moltis_agents::tool_registry::ToolRegistry::new();
        registry.register(Box::new(DummyTool("exec")));
        registry.register(Box::new(DummyTool("mcp__github__builtin_named_like_mcp")));
        registry.register_mcp(Box::new(DummyTool("mcp__github__search")), "github".into());
        registry.register_mcp(Box::new(DummyTool("mcp__memory__store")), "memory".into());
        registry
    }

    fn unrestricted_policy_context(agent_id: &str) -> PolicyContext {
        PolicyContext {
            agent_id: agent_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn mcp_allow_empty_denies_all_mcp_tools_only() {
        let mut config = moltis_config::MoltisConfig::default();
        config.tools.policy.allow = vec!["*".into()];
        config
            .agents
            .presets
            .insert("locked".into(), moltis_config::schema::AgentPreset {
                mcp: moltis_config::schema::PresetMcpPolicy::Allow(vec![]),
                ..Default::default()
            });

        let filtered = apply_runtime_tool_filters(
            &registry_with_mcp_tools(),
            &config,
            &[],
            false,
            &unrestricted_policy_context("locked"),
        );

        assert!(filtered.get("exec").is_some());
        assert!(
            filtered
                .get("mcp__github__builtin_named_like_mcp")
                .is_some()
        );
        assert!(filtered.get("mcp__github__search").is_none());
        assert!(filtered.get("mcp__memory__store").is_none());
    }

    #[test]
    fn mcp_allow_keeps_only_listed_mcp_server() {
        let mut config = moltis_config::MoltisConfig::default();
        config.tools.policy.allow = vec!["*".into()];
        config
            .agents
            .presets
            .insert("github-only".into(), moltis_config::schema::AgentPreset {
                mcp: moltis_config::schema::PresetMcpPolicy::Allow(vec!["github".into()]),
                ..Default::default()
            });

        let filtered = apply_runtime_tool_filters(
            &registry_with_mcp_tools(),
            &config,
            &[],
            false,
            &unrestricted_policy_context("github-only"),
        );

        assert!(filtered.get("exec").is_some());
        assert!(filtered.get("mcp__github__search").is_some());
        assert!(filtered.get("mcp__memory__store").is_none());
    }

    #[test]
    fn skill_policy_allows_then_denies_by_name_or_category() {
        let skills = vec![
            moltis_skills::types::SkillMetadata {
                name: "web-search".into(),
                category: Some("research".into()),
                ..Default::default()
            },
            moltis_skills::types::SkillMetadata {
                name: "games".into(),
                category: Some("gaming".into()),
                ..Default::default()
            },
            moltis_skills::types::SkillMetadata {
                name: "writer".into(),
                category: Some("creative".into()),
                ..Default::default()
            },
        ];
        let policy = moltis_config::schema::PresetSkillPolicy {
            allow: Some(vec!["research".into(), "writer".into()]),
            deny: Some(vec!["writer".into()]),
        };

        let filtered = filter_skills_for_agent(skills, &policy);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "web-search");
    }
}
