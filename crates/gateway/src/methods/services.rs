use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use tracing::warn;

use {
    moltis_agents::prompt::WorkspaceFilePromptStatus,
    moltis_protocol::{ErrorShape, error_codes},
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    services::ServiceError,
};

use super::{MethodContext, MethodRegistry};

async fn active_session_key_for_ctx(ctx: &MethodContext) -> Option<String> {
    if let Some(session_key) = ctx
        .params
        .get("_session_key")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        return Some(session_key.to_string());
    }
    let inner = ctx.state.inner.read().await;
    inner.active_sessions.get(&ctx.client_conn_id).cloned()
}

async fn default_agent_id_for_ctx(ctx: &MethodContext) -> String {
    if let Some(ref store) = ctx.state.services.agent_persona_store {
        return store
            .default_id()
            .await
            .unwrap_or_else(|_| "main".to_string());
    }
    "main".to_string()
}

async fn agent_exists_for_ctx(ctx: &MethodContext, agent_id: &str) -> bool {
    if let Some(ref store) = ctx.state.services.agent_persona_store {
        return store.get(agent_id).await.ok().flatten().is_some();
    }
    // Without a persona store, only "main" is assumed valid.
    agent_id == "main"
}

async fn resolve_session_agent_id_for_ctx(ctx: &MethodContext) -> String {
    let default_id = default_agent_id_for_ctx(ctx).await;
    let Some(session_key) = active_session_key_for_ctx(ctx).await else {
        return default_id;
    };
    let Some(ref metadata) = ctx.state.services.session_metadata else {
        return default_id;
    };
    let Some(entry) = metadata.get(&session_key).await else {
        return default_id;
    };
    let Some(agent_id) = entry
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return default_id;
    };
    if agent_exists_for_ctx(ctx, agent_id).await {
        return agent_id.to_string();
    }
    warn!(
        session = %session_key,
        agent_id,
        fallback = %default_id,
        "session references unknown agent, falling back to default"
    );
    let _ = metadata.set_agent_id(&session_key, Some(&default_id)).await;
    default_id
}

fn parse_agent_id_param(params: &serde_json::Value) -> Option<String> {
    params
        .get("agent_id")
        .or_else(|| params.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn resolve_requested_agent_id(
    ctx: &MethodContext,
    params: &serde_json::Value,
) -> Result<String, ErrorShape> {
    if let Some(id) = parse_agent_id_param(params) {
        if agent_exists_for_ctx(ctx, &id).await {
            return Ok(id);
        }
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!("agent '{id}' not found"),
        ));
    }
    Ok(default_agent_id_for_ctx(ctx).await)
}

fn read_identity_payload_for_agent(agent_id: &str) -> serde_json::Value {
    let config = moltis_config::discover_and_load();
    let identity = moltis_config::load_identity_for_agent(agent_id).unwrap_or_default();
    let user = moltis_config::resolve_user_profile_from_config(&config);
    let resolved_name = identity
        .name
        .clone()
        .unwrap_or_else(|| "moltis".to_string());
    let identity_path = moltis_config::agent_workspace_dir(agent_id).join("IDENTITY.md");
    let identity_text = std::fs::read_to_string(identity_path)
        .ok()
        .and_then(|content| moltis_config::extract_yaml_frontmatter(&content).map(str::to_string));
    let soul = moltis_config::load_soul_for_agent(agent_id);
    let user_name = user.name.clone();
    let user_timezone = user.timezone.as_ref().map(|tz| tz.name().to_string());
    serde_json::json!({
        "name": resolved_name,
        "emoji": identity.emoji.clone(),
        "theme": identity.theme.clone(),
        "user_name": user_name,
        "user_timezone": user_timezone,
        "identity": identity_text,
        "identity_fields": {
            "name": identity.name,
            "emoji": identity.emoji,
            "theme": identity.theme,
        },
        "soul": soul,
    })
}

fn write_soul_for_agent(agent_id: &str, soul: Option<String>) -> Result<(), ErrorShape> {
    moltis_config::save_soul_for_agent(agent_id, soul.as_deref())
        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
    Ok(())
}

/// Write the `.onboarded` sentinel when both agent name and user name are
/// present — mirrors the old `onboarding.identity_update()` behavior so the
/// onboarding wizard doesn't re-appear after identity is saved.
fn mark_onboarded_if_ready(
    identity: &moltis_config::schema::AgentIdentity,
    params: &serde_json::Value,
) {
    let has_agent_name = identity.name.as_ref().is_some_and(|n| !n.is_empty());
    let has_user_name = params
        .get("user_name")
        .and_then(|v| v.as_str())
        .is_some_and(|n| !n.is_empty())
        || moltis_config::resolve_user_profile()
            .name
            .as_ref()
            .is_some_and(|n| !n.is_empty());

    if has_agent_name && has_user_name {
        let sentinel = moltis_config::data_dir().join(".onboarded");
        let _ = std::fs::write(&sentinel, "");
    }
}

/// Save user profile fields (user_name, user_timezone, user_location) from
/// identity update params. These are persisted to `[user]` in `moltis.toml`
/// and `USER.md`, independent of which agent is being updated.
fn save_user_profile_fields(params: &serde_json::Value) -> Result<(), ErrorShape> {
    let has_user_field = params.get("user_name").is_some()
        || params.get("user_timezone").is_some()
        || params.get("timezone").is_some()
        || params.get("user_location").is_some()
        || params.get("location").is_some();

    if !has_user_field {
        return Ok(());
    }

    moltis_config::update_config(|cfg| {
        let mut user = moltis_config::resolve_user_profile_from_config(cfg);

        if let Some(v) = params.get("user_name").and_then(|v| v.as_str()) {
            user.name = if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            };
        }
        if let Some(raw) = params
            .get("user_timezone")
            .or_else(|| params.get("timezone"))
            .and_then(|v| v.as_str())
        {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                user.timezone = None;
            } else if let Ok(tz) = trimmed.parse::<moltis_config::Timezone>() {
                user.timezone = Some(tz);
            }
        }
        if let Some(loc_val) = params
            .get("user_location")
            .or_else(|| params.get("location"))
        {
            if loc_val.is_null() {
                user.location = None;
            } else if let (Some(lat), Some(lon)) = (
                loc_val.get("latitude").and_then(|v| v.as_f64()),
                loc_val.get("longitude").and_then(|v| v.as_f64()),
            ) {
                let place = loc_val
                    .get("place")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                user.location = Some(moltis_config::GeoLocation::now(lat, lon, place));
            }
        }

        cfg.user = user;
    })
    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;

    // Also persist to USER.md.
    let config = moltis_config::discover_and_load();
    let user = moltis_config::resolve_user_profile_from_config(&config);
    let _ = moltis_config::save_user_with_mode(&user, config.memory.user_profile_write_mode);

    Ok(())
}

fn normalize_relative_agent_path(path: &str) -> Result<PathBuf, ErrorShape> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing 'path' parameter",
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "path must be relative",
        ));
    }
    for component in candidate.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(ErrorShape::new(
                error_codes::INVALID_REQUEST,
                "path traversal is not allowed",
            ));
        }
    }
    Ok(candidate.to_path_buf())
}

#[derive(Debug, Clone, serde::Serialize)]
struct WorkspacePromptFileStatusResponse {
    path: String,
    source: &'static str,
    size: Option<u64>,
    #[serde(flatten)]
    prompt_status: WorkspaceFilePromptStatus,
}

fn workspace_file_limit_chars(ctx: &MethodContext) -> usize {
    ctx.state.config.chat.workspace_file_max_chars
}

fn invalid_memory_config_value(field: &str, value: &str) -> ErrorShape {
    ErrorShape::new(
        error_codes::INVALID_REQUEST,
        format!("invalid memory config value for '{field}': '{value}'"),
    )
}

fn parse_memory_style(value: &str) -> Result<moltis_config::MemoryStyle, ErrorShape> {
    match value {
        "hybrid" => Ok(moltis_config::MemoryStyle::Hybrid),
        "prompt-only" => Ok(moltis_config::MemoryStyle::PromptOnly),
        "search-only" => Ok(moltis_config::MemoryStyle::SearchOnly),
        "off" => Ok(moltis_config::MemoryStyle::Off),
        _ => Err(invalid_memory_config_value("style", value)),
    }
}

fn parse_agent_memory_write_mode(
    value: &str,
) -> Result<moltis_config::AgentMemoryWriteMode, ErrorShape> {
    match value {
        "hybrid" => Ok(moltis_config::AgentMemoryWriteMode::Hybrid),
        "prompt-only" => Ok(moltis_config::AgentMemoryWriteMode::PromptOnly),
        "search-only" => Ok(moltis_config::AgentMemoryWriteMode::SearchOnly),
        "off" => Ok(moltis_config::AgentMemoryWriteMode::Off),
        _ => Err(invalid_memory_config_value("agent_write_mode", value)),
    }
}

fn parse_user_profile_write_mode(
    value: &str,
) -> Result<moltis_config::UserProfileWriteMode, ErrorShape> {
    match value {
        "explicit-and-auto" => Ok(moltis_config::UserProfileWriteMode::ExplicitAndAuto),
        "explicit-only" => Ok(moltis_config::UserProfileWriteMode::ExplicitOnly),
        "off" => Ok(moltis_config::UserProfileWriteMode::Off),
        _ => Err(invalid_memory_config_value(
            "user_profile_write_mode",
            value,
        )),
    }
}

fn parse_memory_backend(value: &str) -> Result<moltis_config::MemoryBackend, ErrorShape> {
    match value {
        "builtin" => Ok(moltis_config::MemoryBackend::Builtin),
        "qmd" => Ok(moltis_config::MemoryBackend::Qmd),
        _ => Err(invalid_memory_config_value("backend", value)),
    }
}

fn parse_memory_provider(value: &str) -> Result<Option<moltis_config::MemoryProvider>, ErrorShape> {
    match value {
        "auto" => Ok(None),
        "local" => Ok(Some(moltis_config::MemoryProvider::Local)),
        "ollama" => Ok(Some(moltis_config::MemoryProvider::Ollama)),
        "openai" => Ok(Some(moltis_config::MemoryProvider::OpenAi)),
        "custom" => Ok(Some(moltis_config::MemoryProvider::Custom)),
        _ => Err(invalid_memory_config_value("provider", value)),
    }
}

fn parse_memory_citations_mode(
    value: &str,
) -> Result<moltis_config::MemoryCitationsMode, ErrorShape> {
    match value {
        "on" => Ok(moltis_config::MemoryCitationsMode::On),
        "off" => Ok(moltis_config::MemoryCitationsMode::Off),
        "auto" => Ok(moltis_config::MemoryCitationsMode::Auto),
        _ => Err(invalid_memory_config_value("citations", value)),
    }
}

fn parse_memory_search_merge_strategy(
    value: &str,
) -> Result<moltis_config::MemorySearchMergeStrategy, ErrorShape> {
    match value {
        "rrf" => Ok(moltis_config::MemorySearchMergeStrategy::Rrf),
        "linear" => Ok(moltis_config::MemorySearchMergeStrategy::Linear),
        _ => Err(invalid_memory_config_value("search_merge_strategy", value)),
    }
}

fn parse_session_export_mode(
    value: &serde_json::Value,
) -> Result<moltis_config::SessionExportMode, ErrorShape> {
    match value {
        serde_json::Value::Bool(false) => Ok(moltis_config::SessionExportMode::Off),
        serde_json::Value::Bool(true) => Ok(moltis_config::SessionExportMode::OnNewOrReset),
        serde_json::Value::String(string) => match string.as_str() {
            "off" => Ok(moltis_config::SessionExportMode::Off),
            "on-new-or-reset" => Ok(moltis_config::SessionExportMode::OnNewOrReset),
            _ => Err(invalid_memory_config_value("session_export", string)),
        },
        _ => Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "invalid memory config value for 'session_export': expected bool or string",
        )),
    }
}

fn parse_prompt_memory_mode(value: &str) -> Result<moltis_config::PromptMemoryMode, ErrorShape> {
    match value {
        "live-reload" => Ok(moltis_config::PromptMemoryMode::LiveReload),
        "frozen-at-session-start" => Ok(moltis_config::PromptMemoryMode::FrozenAtSessionStart),
        _ => Err(invalid_memory_config_value("prompt_memory_mode", value)),
    }
}

fn should_fallback_agent_file_to_root(agent_id: &str, relative_path: &Path) -> bool {
    if agent_id == "main" {
        return true;
    }

    matches!(relative_path.to_str(), Some("AGENTS.md") | Some("TOOLS.md"))
}

fn resolve_agent_file_target(
    agent_id: &str,
    relative_path: &Path,
) -> Option<(PathBuf, &'static str)> {
    let primary = moltis_config::agent_workspace_dir(agent_id).join(relative_path);
    if primary.exists() {
        return Some((primary, "agent"));
    }

    if should_fallback_agent_file_to_root(agent_id, relative_path) {
        let fallback = moltis_config::data_dir().join(relative_path);
        if fallback.exists() {
            return Some((fallback, "root"));
        }
    }

    None
}

fn workspace_prompt_file_status(
    agent_id: &str,
    file_name: &str,
    limit_chars: usize,
) -> Option<WorkspacePromptFileStatusResponse> {
    let relative_path = Path::new(file_name);
    let (path, source) = resolve_agent_file_target(agent_id, relative_path)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let normalized = moltis_config::normalize_workspace_markdown_content(&content)?;
    let original_chars = normalized.chars().count();
    let size_bytes = std::fs::metadata(&path).ok().map(|meta| meta.len());
    Some(WorkspacePromptFileStatusResponse {
        path: file_name.to_string(),
        source,
        size: size_bytes,
        prompt_status: WorkspaceFilePromptStatus {
            name: file_name.to_string(),
            original_chars,
            included_chars: original_chars.min(limit_chars),
            limit_chars,
            truncated_chars: original_chars.saturating_sub(limit_chars),
            truncated: original_chars > limit_chars,
        },
    })
}

fn workspace_prompt_files_status(agent_id: &str, limit_chars: usize) -> Vec<serde_json::Value> {
    ["AGENTS.md", "TOOLS.md"]
        .iter()
        .filter_map(|file_name| {
            workspace_prompt_file_status(agent_id, file_name, limit_chars)
                .and_then(|status| serde_json::to_value(status).ok())
        })
        .collect()
}

fn read_agent_file(agent_id: &str, relative_path: &Path) -> Result<String, ErrorShape> {
    let (target, _) = resolve_agent_file_target(agent_id, relative_path)
        .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "file not found"))?;

    std::fs::read_to_string(target)
        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
}

fn list_agent_workspace_files_recursively(
    root: &Path,
    base: &Path,
    files: &mut Vec<serde_json::Value>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            list_agent_workspace_files_recursively(&path, base, files);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(base) {
            files.push(serde_json::json!({
                "path": relative.to_string_lossy(),
                "size": entry.metadata().ok().map(|m| m.len()),
            }));
        }
    }
}

mod admin;
mod agents;
mod channels;
mod core;
mod modes;
mod sessions;
mod system;

pub(super) fn register(reg: &mut MethodRegistry) {
    agents::register(reg);
    modes::register(reg);
    sessions::register(reg);
    channels::register(reg);
    core::register(reg);
    system::register(reg);
    admin::register(reg);
}
async fn reload_hooks(state: &Arc<crate::state::GatewayState>) {
    let disabled = state.inner.read().await.disabled_hooks.clone();
    let session_store = state.services.session_store.as_ref();
    let (new_registry, new_info) =
        crate::server::discover_and_build_hooks(&disabled, session_store).await;

    {
        let mut inner = state.inner.write().await;
        inner.hook_registry = new_registry;
        inner.discovered_hooks = new_info.clone();
    }

    // Broadcast hooks.status event so connected UIs auto-refresh.
    broadcast(
        state,
        "hooks.status",
        serde_json::json!({ "hooks": new_info }),
        BroadcastOpts::default(),
    )
    .await;
}

/// Persist the disabled hooks set to `data_dir/disabled_hooks.json`.
async fn persist_disabled_hooks(state: &Arc<crate::state::GatewayState>) {
    let disabled = state.inner.read().await.disabled_hooks.clone();
    let path = moltis_config::data_dir().join("disabled_hooks.json");
    let json = serde_json::to_string_pretty(&disabled).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, json) {
        warn!("failed to persist disabled hooks: {e}");
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            auth::{AuthMode, ResolvedAuth},
            services::GatewayServices,
            state::GatewayState,
        },
        tempfile::TempDir,
    };

    struct MemoryConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _config_dir: TempDir,
        _data_dir: TempDir,
    }

    impl MemoryConfigTestGuard {
        fn new() -> Self {
            let lock = crate::config_override_test_lock();
            let config_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("config tempdir should be created: {error}"));
            let data_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("data tempdir should be created: {error}"));
            moltis_config::set_config_dir(config_dir.path().to_path_buf());
            moltis_config::set_data_dir(data_dir.path().to_path_buf());
            Self {
                _lock: lock,
                _config_dir: config_dir,
                _data_dir: data_dir,
            }
        }
    }

    impl Drop for MemoryConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }

    async fn dispatch_memory_method(method: &str, params: serde_json::Value) -> serde_json::Value {
        let mut reg = MethodRegistry::default();
        register(&mut reg);
        let response = reg
            .dispatch(MethodContext {
                request_id: "test".into(),
                method: method.to_string(),
                params,
                client_conn_id: "conn-1".into(),
                client_role: "operator".into(),
                client_scopes: vec!["operator.write".into(), "operator.read".into()],
                state: GatewayState::new(
                    ResolvedAuth {
                        mode: AuthMode::Token,
                        token: None,
                        password: None,
                    },
                    GatewayServices::noop(),
                ),
                channel: None,
            })
            .await;

        assert!(response.ok, "method failed: {:?}", response.error);
        match response.payload {
            Some(payload) => payload,
            None => panic!("method {method} returned no payload"),
        }
    }

    async fn dispatch_memory_method_response(
        method: &str,
        params: serde_json::Value,
    ) -> moltis_protocol::ResponseFrame {
        let mut reg = MethodRegistry::default();
        register(&mut reg);
        reg.dispatch(MethodContext {
            request_id: "test".into(),
            method: method.to_string(),
            params,
            client_conn_id: "conn-1".into(),
            client_role: "operator".into(),
            client_scopes: vec!["operator.write".into(), "operator.read".into()],
            state: GatewayState::new(
                ResolvedAuth {
                    mode: AuthMode::Token,
                    token: None,
                    password: None,
                },
                GatewayServices::noop(),
            ),
            channel: None,
        })
        .await
    }

    #[tokio::test]
    async fn memory_config_get_reports_typed_memory_fields() {
        let _guard = MemoryConfigTestGuard::new();
        let update_result = moltis_config::update_config(|cfg| {
            cfg.memory.style = moltis_config::MemoryStyle::SearchOnly;
            cfg.memory.agent_write_mode = moltis_config::AgentMemoryWriteMode::PromptOnly;
            cfg.memory.user_profile_write_mode = moltis_config::UserProfileWriteMode::ExplicitOnly;
            cfg.memory.backend = moltis_config::MemoryBackend::Qmd;
            cfg.memory.provider = Some(moltis_config::MemoryProvider::OpenAi);
            cfg.memory.citations = moltis_config::MemoryCitationsMode::Off;
            cfg.memory.disable_rag = true;
            cfg.memory.llm_reranking = true;
            cfg.memory.search_merge_strategy = moltis_config::MemorySearchMergeStrategy::Linear;
            cfg.memory.session_export = moltis_config::SessionExportMode::Off;
            cfg.chat.prompt_memory_mode = moltis_config::PromptMemoryMode::FrozenAtSessionStart;
        });
        assert!(update_result.is_ok(), "config update should succeed");

        let payload = dispatch_memory_method("memory.config.get", serde_json::json!({})).await;
        assert_eq!(payload["style"], "search-only");
        assert_eq!(payload["agent_write_mode"], "prompt-only");
        assert_eq!(payload["user_profile_write_mode"], "explicit-only");
        assert_eq!(payload["backend"], "qmd");
        assert_eq!(payload["provider"], "openai");
        assert_eq!(payload["citations"], "off");
        assert_eq!(payload["disable_rag"], true);
        assert_eq!(payload["llm_reranking"], true);
        assert_eq!(payload["search_merge_strategy"], "linear");
        assert_eq!(payload["session_export"], "off");
        assert_eq!(payload["prompt_memory_mode"], "frozen-at-session-start");
    }

    #[tokio::test]
    async fn memory_config_update_persists_typed_memory_fields() {
        let _guard = MemoryConfigTestGuard::new();

        let payload = dispatch_memory_method(
            "memory.config.update",
            serde_json::json!({
                "style": "prompt-only",
                "agent_write_mode": "search-only",
                "user_profile_write_mode": "off",
                "backend": "qmd",
                "provider": "custom",
                "citations": "on",
                "disable_rag": true,
                "llm_reranking": true,
                "search_merge_strategy": "linear",
                "session_export": false,
                "prompt_memory_mode": "frozen-at-session-start",
            }),
        )
        .await;

        assert_eq!(payload["style"], "prompt-only");
        assert_eq!(payload["agent_write_mode"], "search-only");
        assert_eq!(payload["user_profile_write_mode"], "off");
        assert_eq!(payload["backend"], "qmd");
        assert_eq!(payload["provider"], "custom");
        assert_eq!(payload["citations"], "on");
        assert_eq!(payload["disable_rag"], true);
        assert_eq!(payload["llm_reranking"], true);
        assert_eq!(payload["search_merge_strategy"], "linear");
        assert_eq!(payload["session_export"], "off");
        assert_eq!(payload["prompt_memory_mode"], "frozen-at-session-start");

        let config = moltis_config::discover_and_load();
        assert_eq!(config.memory.style, moltis_config::MemoryStyle::PromptOnly);
        assert_eq!(
            config.memory.agent_write_mode,
            moltis_config::AgentMemoryWriteMode::SearchOnly
        );
        assert_eq!(
            config.memory.user_profile_write_mode,
            moltis_config::UserProfileWriteMode::Off
        );
        assert_eq!(config.memory.backend, moltis_config::MemoryBackend::Qmd);
        assert_eq!(
            config.memory.provider,
            Some(moltis_config::MemoryProvider::Custom)
        );
        assert_eq!(
            config.memory.citations,
            moltis_config::MemoryCitationsMode::On
        );
        assert!(config.memory.disable_rag);
        assert!(config.memory.llm_reranking);
        assert_eq!(
            config.memory.search_merge_strategy,
            moltis_config::MemorySearchMergeStrategy::Linear
        );
        assert_eq!(
            config.memory.session_export,
            moltis_config::SessionExportMode::Off
        );
        assert_eq!(
            config.chat.prompt_memory_mode,
            moltis_config::PromptMemoryMode::FrozenAtSessionStart
        );
    }

    #[tokio::test]
    async fn memory_config_update_rejects_unknown_enum_values() {
        let _guard = MemoryConfigTestGuard::new();
        let response = dispatch_memory_method_response(
            "memory.config.update",
            serde_json::json!({
                "style": "surprise-mode",
            }),
        )
        .await;

        assert!(!response.ok, "invalid enum value should fail");
        let error = match response.error {
            Some(error) => error,
            None => panic!("expected invalid request error"),
        };
        assert_eq!(error.code, error_codes::INVALID_REQUEST);
        assert_eq!(
            error.message,
            "invalid memory config value for 'style': 'surprise-mode'"
        );
    }
}
