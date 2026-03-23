//! Web-UI API handlers (bootstrap, skills, images, containers, media, logs).

use std::{collections::HashMap, path::PathBuf};

use {
    axum::{
        Json,
        extract::{Path, Query, State},
        http::StatusCode,
        response::{IntoResponse, Response},
    },
    moltis_gateway::server::AppState,
    moltis_tools::image_cache::ImageBuilder,
    tracing::warn,
};

use crate::templates::{build_nav_counts, onboarding_completed};

const MCP_LIST_FAILED: &str = "MCP_LIST_FAILED";
const IMAGE_CACHE_DELETE_FAILED: &str = "IMAGE_CACHE_DELETE_FAILED";
const IMAGE_CACHE_PRUNE_FAILED: &str = "IMAGE_CACHE_PRUNE_FAILED";
const SANDBOX_CHECK_PACKAGES_FAILED: &str = "SANDBOX_CHECK_PACKAGES_FAILED";
const SANDBOX_BACKEND_UNAVAILABLE: &str = "SANDBOX_BACKEND_UNAVAILABLE";
const SANDBOX_IMAGE_NAME_REQUIRED: &str = "SANDBOX_IMAGE_NAME_REQUIRED";
const SANDBOX_IMAGE_PACKAGES_REQUIRED: &str = "SANDBOX_IMAGE_PACKAGES_REQUIRED";
const SANDBOX_IMAGE_NAME_INVALID: &str = "SANDBOX_IMAGE_NAME_INVALID";
const SANDBOX_TMP_DIR_CREATE_FAILED: &str = "SANDBOX_TMP_DIR_CREATE_FAILED";
const SANDBOX_DOCKERFILE_WRITE_FAILED: &str = "SANDBOX_DOCKERFILE_WRITE_FAILED";
const SANDBOX_IMAGE_BUILD_FAILED: &str = "SANDBOX_IMAGE_BUILD_FAILED";
const SANDBOX_CONTAINERS_LIST_FAILED: &str = "SANDBOX_CONTAINERS_LIST_FAILED";
const SANDBOX_CONTAINER_PREFIX_MISMATCH: &str = "SANDBOX_CONTAINER_PREFIX_MISMATCH";
const SANDBOX_CONTAINER_STOP_FAILED: &str = "SANDBOX_CONTAINER_STOP_FAILED";
const SANDBOX_CONTAINER_REMOVE_FAILED: &str = "SANDBOX_CONTAINER_REMOVE_FAILED";
const SANDBOX_CONTAINERS_CLEAN_FAILED: &str = "SANDBOX_CONTAINERS_CLEAN_FAILED";
const SANDBOX_DISK_USAGE_FAILED: &str = "SANDBOX_DISK_USAGE_FAILED";
const SANDBOX_DAEMON_RESTART_FAILED: &str = "SANDBOX_DAEMON_RESTART_FAILED";
const SANDBOX_SHARED_HOME_SAVE_FAILED: &str = "SANDBOX_SHARED_HOME_SAVE_FAILED";
const SESSION_HISTORY_FAILED: &str = "SESSION_HISTORY_FAILED";
const SESSION_LIST_FAILED: &str = "SESSION_LIST_FAILED";
const SESSION_LIST_DEFAULT_LIMIT: usize = 40;
const SESSION_LIST_MAX_LIMIT: usize = 200;
const SESSION_HISTORY_DEFAULT_LIMIT: usize = 120;
const SESSION_HISTORY_MAX_LIMIT: usize = 500;

fn api_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "error": error.into(),
    })
}

fn api_error_response(status: StatusCode, code: &str, error: impl Into<String>) -> Response {
    (status, Json(api_error(code, error))).into_response()
}

#[derive(serde::Deserialize)]
pub struct SandboxSharedHomeUpdateRequest {
    enabled: bool,
    path: Option<String>,
}

fn shared_home_config_payload(config: &moltis_config::MoltisConfig) -> serde_json::Value {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let mode = match config.tools.exec.sandbox.home_persistence {
        moltis_config::schema::HomePersistenceConfig::Off => "off",
        moltis_config::schema::HomePersistenceConfig::Session => "session",
        moltis_config::schema::HomePersistenceConfig::Shared => "shared",
    };
    serde_json::json!({
        "enabled": matches!(
            config.tools.exec.sandbox.home_persistence,
            moltis_config::schema::HomePersistenceConfig::Shared
        ),
        "mode": mode,
        "path": moltis_tools::sandbox::shared_home_dir_path(&runtime_cfg)
            .display()
            .to_string(),
        "configured_path": config.tools.exec.sandbox.shared_home_dir.clone(),
    })
}

// ── Session media ────────────────────────────────────────────────────────────

/// Build a `Content-Disposition` header value for a media file.
/// Uses `inline` for types browsers can render natively (PDF, text, images)
/// and `attachment` for everything else so they trigger a download.
/// NOTE: `text/html` is deliberately excluded — serving LLM-generated HTML
/// inline on our origin would enable stored XSS.
fn media_content_disposition(filename: &str, content_type: &str) -> String {
    let inline = content_type.starts_with("image/")
        || content_type.starts_with("audio/")
        || matches!(
            content_type,
            "application/pdf" | "text/plain" | "text/csv" | "text/markdown"
        );
    let disposition = if inline {
        "inline"
    } else {
        "attachment"
    };
    // Sanitise filename for the header (strip quotes, newlines, semicolons,
    // backslashes — all of which can break or inject Content-Disposition).
    let safe_name: String = filename
        .chars()
        .filter(|c| *c != '"' && *c != '\n' && *c != '\r' && *c != ';' && *c != '\\')
        .collect();
    format!("{disposition}; filename=\"{safe_name}\"")
}

#[derive(serde::Deserialize, Default)]
pub struct SessionListQuery {
    #[serde(default)]
    cursor: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

fn clamp_session_list_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(SESSION_LIST_DEFAULT_LIMIT)
        .clamp(1, SESSION_LIST_MAX_LIMIT)
}

pub async fn api_sessions_handler(
    Query(query): Query<SessionListQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.gateway.services.session.list().await {
        Ok(payload) => {
            if query.cursor.is_none() && query.limit.is_none() {
                return Json(payload).into_response();
            }

            let sessions = payload
                .as_array()
                .cloned()
                .unwrap_or_else(Vec::<serde_json::Value>::new);
            let limit = clamp_session_list_limit(query.limit);
            let start = query
                .cursor
                .and_then(|idx| usize::try_from(idx).ok())
                .unwrap_or(0)
                .min(sessions.len());
            let end = start.saturating_add(limit).min(sessions.len());
            let page = sessions[start..end].to_vec();
            let next_cursor = if end < sessions.len() {
                u64::try_from(end).ok()
            } else {
                None
            };

            Json(serde_json::json!({
                "sessions": page,
                "nextCursor": next_cursor,
                "hasMore": next_cursor.is_some(),
                "total": sessions.len(),
            }))
            .into_response()
        },
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SESSION_LIST_FAILED,
            e.to_string(),
        ),
    }
}

#[derive(serde::Deserialize)]
pub struct SessionHistoryQuery {
    #[serde(default)]
    cached_message_count: Option<u64>,
    #[serde(default)]
    cursor: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

fn filter_ui_history(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(idx, mut msg)| {
            if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                let has_content = msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                let has_reasoning = msg
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                let has_audio = msg
                    .get("audio")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                if !(has_content || has_reasoning || has_audio) {
                    return None;
                }
            }
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("historyIndex".to_string(), serde_json::json!(idx));
            }
            Some(msg)
        })
        .collect()
}

fn history_index(msg: &serde_json::Value) -> Option<usize> {
    msg.get("historyIndex")
        .and_then(|v| v.as_u64())
        .and_then(|idx| usize::try_from(idx).ok())
}

fn paginated_history(
    history: Vec<serde_json::Value>,
    cursor: Option<usize>,
    limit: usize,
) -> (Vec<serde_json::Value>, bool, Option<u64>) {
    let mut scoped = if let Some(cursor_idx) = cursor {
        history
            .into_iter()
            .filter(|msg| history_index(msg).is_some_and(|idx| idx < cursor_idx))
            .collect::<Vec<_>>()
    } else {
        history
    };

    let len = scoped.len();
    if len > limit {
        scoped.drain(0..(len - limit));
    }

    let next_cursor = scoped
        .first()
        .and_then(history_index)
        .filter(|idx| *idx > 0)
        .and_then(|idx| u64::try_from(idx).ok());

    (scoped, next_cursor.is_some(), next_cursor)
}

fn clamp_history_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(SESSION_HISTORY_DEFAULT_LIMIT)
        .clamp(1, SESSION_HISTORY_MAX_LIMIT)
}

#[derive(serde::Deserialize, Default)]
pub struct BootstrapQuery {
    #[serde(default)]
    include_channels: Option<bool>,
    #[serde(default)]
    include_sessions: Option<bool>,
    #[serde(default)]
    include_models: Option<bool>,
    #[serde(default)]
    include_projects: Option<bool>,
    #[serde(default)]
    include_counts: Option<bool>,
    #[serde(default)]
    include_identity: Option<bool>,
}

impl BootstrapQuery {
    fn channels_enabled(&self) -> bool {
        self.include_channels.unwrap_or(true)
    }

    fn sessions_enabled(&self) -> bool {
        self.include_sessions.unwrap_or(true)
    }

    fn models_enabled(&self) -> bool {
        self.include_models.unwrap_or(true)
    }

    fn projects_enabled(&self) -> bool {
        self.include_projects.unwrap_or(true)
    }

    fn counts_enabled(&self) -> bool {
        self.include_counts.unwrap_or(true)
    }

    fn identity_enabled(&self) -> bool {
        self.include_identity.unwrap_or(true)
    }
}

pub async fn api_session_history_handler(
    Path(session_key): Path<String>,
    Query(query): Query<SessionHistoryQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(store) = state.gateway.services.session_store.as_ref() else {
        return api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SESSION_HISTORY_FAILED,
            "session store not available",
        );
    };

    let cursor = query.cursor.and_then(|idx| usize::try_from(idx).ok());
    let limit = clamp_history_limit(query.limit);

    let metadata_entry = if let Some(ref metadata) = state.gateway.services.session_metadata {
        metadata.get(&session_key).await
    } else {
        None
    };
    let metadata_count = metadata_entry
        .as_ref()
        .map(|entry| u64::from(entry.message_count));

    if cursor.is_none()
        && let (Some(cached), Some(server_count)) = (query.cached_message_count, metadata_count)
        && cached == server_count
    {
        return Json(serde_json::json!({
            "history": [],
            "historyCacheHit": true,
            "hasMore": false,
            "nextCursor": null,
            "totalMessages": server_count,
            "historyTruncated": false,
            "historyDroppedCount": 0,
        }))
        .into_response();
    }

    let raw_history = match store.read(&session_key).await {
        Ok(history) => history,
        Err(e) => {
            return api_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                SESSION_HISTORY_FAILED,
                e.to_string(),
            );
        },
    };

    let server_count = metadata_count.unwrap_or(raw_history.len() as u64);

    let full_history = filter_ui_history(raw_history);
    let total_messages = full_history.len() as u64;
    let (mut history, has_more, next_cursor) = paginated_history(full_history, cursor, limit);

    let history_cache_hit = cursor.is_none()
        && query
            .cached_message_count
            .is_some_and(|cached| cached == server_count);
    if history_cache_hit {
        history.clear();
    }

    Json(serde_json::json!({
        "history": history,
        "historyCacheHit": history_cache_hit,
        "hasMore": has_more,
        "nextCursor": next_cursor,
        "totalMessages": total_messages,
        "historyTruncated": false,
        "historyDroppedCount": 0,
    }))
    .into_response()
}

pub async fn api_session_media_handler(
    Path((session_key, filename)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(ref store) = state.gateway.services.session_store else {
        return (StatusCode::NOT_FOUND, "session store not available").into_response();
    };
    match store.read_media(&session_key, &filename).await {
        Ok(data) => {
            let content_type = filename
                .rsplit('.')
                .next()
                .and_then(moltis_media::mime::mime_from_extension)
                .unwrap_or("application/octet-stream");
            let disposition = media_content_disposition(&filename, content_type);
            (
                [
                    (axum::http::header::CONTENT_TYPE, content_type.to_string()),
                    (axum::http::header::CONTENT_DISPOSITION, disposition),
                ],
                data,
            )
                .into_response()
        },
        Err(_) => (StatusCode::NOT_FOUND, "media file not found").into_response(),
    }
}

// ── Logs download ────────────────────────────────────────────────────────────

pub async fn api_logs_download_handler(State(state): State<AppState>) -> impl IntoResponse {
    use {axum::http::header, tokio_util::io::ReaderStream};

    let Some(path) = state.gateway.services.logs.log_file_path() else {
        return (StatusCode::NOT_FOUND, "log file not available").into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "log file not found").into_response(),
    };
    let stream = ReaderStream::new(tokio::io::BufReader::new(file));
    let body = axum::body::Body::from_stream(stream);
    let headers = [
        (header::CONTENT_TYPE, "application/x-ndjson"),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"moltis-logs.jsonl\"",
        ),
    ];
    (headers, body).into_response()
}

// ── Bootstrap ────────────────────────────────────────────────────────────────

pub async fn api_bootstrap_handler(
    Query(query): Query<BootstrapQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let gw = &state.gateway;
    api_bootstrap_with_query(gw, &query).await
}

async fn api_bootstrap_with_query(
    gw: &moltis_gateway::state::GatewayState,
    query: &BootstrapQuery,
) -> Response {
    let channels_enabled = query.channels_enabled();
    let sessions_enabled = query.sessions_enabled();
    let models_enabled = query.models_enabled();
    let projects_enabled = query.projects_enabled();
    let counts_enabled = query.counts_enabled();
    let identity_enabled = query.identity_enabled();

    let (channels, sessions, models, projects, identity, counts, onboarded) = tokio::join!(
        async {
            if channels_enabled {
                gw.services.channel.status().await.ok()
            } else {
                None
            }
        },
        async {
            if sessions_enabled {
                gw.services.session.list().await.ok()
            } else {
                None
            }
        },
        async {
            if models_enabled {
                gw.services.model.list().await.ok()
            } else {
                None
            }
        },
        async {
            if projects_enabled {
                gw.services.project.list().await.ok()
            } else {
                None
            }
        },
        async {
            if identity_enabled {
                gw.services.agent.identity_get().await.ok()
            } else {
                None
            }
        },
        async {
            if counts_enabled {
                Some(build_nav_counts(gw).await)
            } else {
                None
            }
        },
        onboarding_completed(gw),
    );

    let sandbox = if let Some(ref router) = gw.sandbox_router {
        let default_image = router.default_image().await;
        serde_json::json!({
            "backend": router.backend_name(),
            "os": std::env::consts::OS,
            "default_image": default_image,
        })
    } else {
        serde_json::json!({
            "backend": "none",
            "os": std::env::consts::OS,
            "default_image": moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE,
        })
    };
    Json(serde_json::json!({
        "channels": channels,
        "sessions": sessions,
        "models": models,
        "projects": projects,
        "onboarded": onboarded,
        "identity": identity,
        "sandbox": sandbox,
        "counts": counts,
    }))
    .into_response()
}

// ── MCP / Hooks ──────────────────────────────────────────────────────────────

pub async fn api_mcp_handler(State(state): State<AppState>) -> impl IntoResponse {
    let servers = state.gateway.services.mcp.list().await;
    match servers {
        Ok(val) => Json(val).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            MCP_LIST_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_hooks_handler(State(state): State<AppState>) -> impl IntoResponse {
    let hooks = state.gateway.inner.read().await;
    Json(serde_json::json!({ "hooks": hooks.discovered_hooks }))
}

// ── Skills ───────────────────────────────────────────────────────────────────

fn enabled_from_manifest<E>(path_result: Result<PathBuf, E>) -> Vec<serde_json::Value>
where
    E: std::fmt::Display,
{
    let Ok(path) = path_result else {
        return Vec::new();
    };
    let store = moltis_skills::manifest::ManifestStore::new(path);
    store
        .load()
        .map(|m| {
            m.repos
                .iter()
                .flat_map(|repo| {
                    let source = repo.source.clone();
                    repo.skills.iter().filter(|s| s.enabled).map(move |s| {
                        serde_json::json!({
                            "name": s.name,
                            "source": source,
                            "enabled": true,
                        })
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn api_skills_handler(State(state): State<AppState>) -> impl IntoResponse {
    let repos = state
        .gateway
        .services
        .skills
        .repos_list()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let mut skills = enabled_from_manifest(moltis_skills::manifest::ManifestStore::default_path());

    {
        use moltis_skills::discover::{FsSkillDiscoverer, SkillDiscoverer};
        let data_dir = moltis_config::data_dir();
        let search_paths = vec![
            (
                data_dir.join("skills"),
                moltis_skills::types::SkillSource::Personal,
            ),
            (
                data_dir.join(".moltis/skills"),
                moltis_skills::types::SkillSource::Project,
            ),
        ];
        let discoverer = FsSkillDiscoverer::new(search_paths);
        if let Ok(discovered) = discoverer.discover().await {
            for s in discovered {
                let protected = moltis_gateway::services::is_protected_discovered_skill(&s.name);
                skills.push(serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "source": s.source,
                    "enabled": true,
                    "protected": protected,
                }));
            }
        }
    }

    Json(serde_json::json!({ "skills": skills, "repos": repos }))
}

async fn api_search_handler(
    repos: Vec<serde_json::Value>,
    source: &str,
    query: &str,
) -> Json<serde_json::Value> {
    let query = query.to_lowercase();
    let skills: Vec<serde_json::Value> = repos
        .into_iter()
        .find(|repo| {
            repo.get("source")
                .and_then(|s| s.as_str())
                .map(|s| s == source)
                .unwrap_or(false)
        })
        .and_then(|repo| repo.get("skills").and_then(|s| s.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|skill| {
            if query.is_empty() {
                return true;
            }
            let name = skill
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            let display = skill
                .get("display_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            let desc = skill
                .get("description")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            name.contains(&query) || display.contains(&query) || desc.contains(&query)
        })
        .take(30)
        .collect();

    Json(serde_json::json!({ "skills": skills }))
}

pub async fn api_skills_search_handler(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let source = params.get("source").cloned().unwrap_or_default();
    let query = params.get("q").cloned().unwrap_or_default();
    let repos = state
        .gateway
        .services
        .skills
        .repos_list_full()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    api_search_handler(repos, &source, &query).await
}

// ── Images ───────────────────────────────────────────────────────────────────

pub async fn api_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    let (cached, sandbox) = tokio::join!(
        builder.list_cached(),
        moltis_tools::sandbox::list_sandbox_images(),
    );

    let mut images: Vec<serde_json::Value> = Vec::new();

    match cached {
        Ok(list) => {
            for img in list {
                images.push(serde_json::json!({
                    "tag": img.tag,
                    "size": img.size,
                    "created": img.created,
                    "kind": "tool",
                }));
            }
        },
        Err(e) => {
            warn!("failed to list cached tool images: {e}");
        },
    }

    match sandbox {
        Ok(list) => {
            for img in list {
                images.push(serde_json::json!({
                    "tag": img.tag,
                    "size": img.size,
                    "created": img.created,
                    "kind": "sandbox",
                }));
            }
        },
        Err(e) => {
            warn!("failed to list sandbox images: {e}");
        },
    }

    Json(serde_json::json!({ "images": images })).into_response()
}

pub async fn api_delete_cached_image_handler(Path(tag): Path<String>) -> impl IntoResponse {
    let result = if tag.contains("-sandbox:") {
        moltis_tools::sandbox::remove_sandbox_image(&tag).await
    } else {
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let full_tag = if tag.starts_with("moltis-cache/") {
            tag
        } else {
            format!("moltis-cache/{tag}")
        };
        builder.remove_cached(&full_tag).await
    };
    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            IMAGE_CACHE_DELETE_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_prune_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    let (tool_result, sandbox_result) = tokio::join!(
        builder.prune_all(),
        moltis_tools::sandbox::clean_sandbox_images(),
    );
    let mut count = 0;
    if let Ok(n) = tool_result {
        count += n;
    }
    if let Ok(n) = sandbox_result {
        count += n;
    }
    if let (Err(e1), Err(e2)) = (&tool_result, &sandbox_result) {
        let msg = format!("tool images: {e1}; sandbox images: {e2}");
        return api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            IMAGE_CACHE_PRUNE_FAILED,
            msg,
        );
    }
    Json(serde_json::json!({ "pruned": count })).into_response()
}

pub async fn api_check_packages_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let base = body
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or("ubuntu:25.10")
        .trim()
        .to_string();
    let packages: Vec<String> = body
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    if packages.is_empty() {
        return Json(serde_json::json!({ "found": {} })).into_response();
    }

    let checks: Vec<String> = packages
        .iter()
        .map(|pkg| {
            format!(
                r#"if dpkg -s '{pkg}' >/dev/null 2>&1 || command -v '{pkg}' >/dev/null 2>&1; then echo "FOUND:{pkg}"; fi"#
            )
        })
        .collect();
    let script = checks.join("\n");

    let cli = moltis_tools::sandbox::container_cli();
    let output = tokio::process::Command::new(cli)
        .args(["run", "--rm", "--entrypoint", "sh", &base, "-c", &script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut found = serde_json::Map::new();
            for pkg in &packages {
                let present = stdout.lines().any(|l| l.trim() == format!("FOUND:{pkg}"));
                found.insert(pkg.clone(), serde_json::Value::Bool(present));
            }
            Json(serde_json::json!({ "found": found })).into_response()
        },
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_CHECK_PACKAGES_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_get_default_image_handler(State(state): State<AppState>) -> impl IntoResponse {
    let image = if let Some(ref router) = state.gateway.sandbox_router {
        router.default_image().await
    } else {
        moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string()
    };
    Json(serde_json::json!({ "image": image }))
}

pub async fn api_set_default_image_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let image = body.get("image").and_then(|v| v.as_str()).map(|s| s.trim());

    if let Some(ref router) = state.gateway.sandbox_router {
        let value = image.filter(|s| !s.is_empty()).map(String::from);
        router.set_global_image(value.clone()).await;
        let effective = router.default_image().await;
        Json(serde_json::json!({ "image": effective })).into_response()
    } else {
        api_error_response(
            StatusCode::BAD_REQUEST,
            SANDBOX_BACKEND_UNAVAILABLE,
            "no sandbox backend available",
        )
    }
}

pub async fn api_get_shared_home_handler() -> impl IntoResponse {
    let config = moltis_config::discover_and_load();
    Json(shared_home_config_payload(&config))
}

pub async fn api_set_shared_home_handler(
    Json(body): Json<SandboxSharedHomeUpdateRequest>,
) -> impl IntoResponse {
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(ToOwned::to_owned);

    let update_result = moltis_config::update_config(|cfg| {
        cfg.tools.exec.sandbox.shared_home_dir = path.clone();
        if body.enabled {
            cfg.tools.exec.sandbox.home_persistence =
                moltis_config::schema::HomePersistenceConfig::Shared;
        } else if matches!(
            cfg.tools.exec.sandbox.home_persistence,
            moltis_config::schema::HomePersistenceConfig::Shared
        ) {
            cfg.tools.exec.sandbox.home_persistence =
                moltis_config::schema::HomePersistenceConfig::Off;
        }
    });

    match update_result {
        Ok(saved_path) => {
            let config = moltis_config::discover_and_load();
            Json(serde_json::json!({
                "ok": true,
                "restart_required": true,
                "config_path": saved_path.display().to_string(),
                "config": shared_home_config_payload(&config),
            }))
            .into_response()
        },
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_SHARED_HOME_SAVE_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_build_image_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let base = body
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or("ubuntu:25.10")
        .trim();
    let packages: Vec<&str> = body
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if name.is_empty() {
        return api_error_response(
            StatusCode::BAD_REQUEST,
            SANDBOX_IMAGE_NAME_REQUIRED,
            "name is required",
        );
    }
    if packages.is_empty() {
        return api_error_response(
            StatusCode::BAD_REQUEST,
            SANDBOX_IMAGE_PACKAGES_REQUIRED,
            "packages list is empty",
        );
    }

    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return api_error_response(
            StatusCode::BAD_REQUEST,
            SANDBOX_IMAGE_NAME_INVALID,
            "name must be alphanumeric, dash, or underscore",
        );
    }

    let pkg_list = packages.join(" ");
    let dockerfile_contents = format!(
        "FROM {base}\n\
RUN apt-get update && apt-get install -y {pkg_list}\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
WORKDIR /home/sandbox\n"
    );

    let tmp_dir = std::env::temp_dir().join(format!("moltis-build-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        return api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_TMP_DIR_CREATE_FAILED,
            e.to_string(),
        );
    }

    let dockerfile_path = tmp_dir.join("Dockerfile");
    if let Err(e) = std::fs::write(&dockerfile_path, &dockerfile_contents) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_DOCKERFILE_WRITE_FAILED,
            e.to_string(),
        );
    }

    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    let result = builder.ensure_image(name, &dockerfile_path, &tmp_dir).await;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    match result {
        Ok(tag) => Json(serde_json::json!({ "tag": tag })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_IMAGE_BUILD_FAILED,
            e.to_string(),
        ),
    }
}

// ── Containers ───────────────────────────────────────────────────────────────

pub async fn api_list_containers_handler(State(state): State<AppState>) -> impl IntoResponse {
    let prefix = state
        .gateway
        .sandbox_router
        .as_ref()
        .map(|r| {
            r.config()
                .container_prefix
                .clone()
                .unwrap_or_else(|| "moltis-sandbox".to_string())
        })
        .unwrap_or_else(|| "moltis-sandbox".to_string());
    match moltis_tools::sandbox::list_running_containers(&prefix).await {
        Ok(containers) => Json(serde_json::json!({ "containers": containers })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_CONTAINERS_LIST_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_stop_container_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let prefix = state
        .gateway
        .sandbox_router
        .as_ref()
        .map(|r| {
            r.config()
                .container_prefix
                .clone()
                .unwrap_or_else(|| "moltis-sandbox".to_string())
        })
        .unwrap_or_else(|| "moltis-sandbox".to_string());
    if !name.starts_with(&prefix) {
        return api_error_response(
            StatusCode::FORBIDDEN,
            SANDBOX_CONTAINER_PREFIX_MISMATCH,
            "container name does not match expected prefix",
        );
    }
    match moltis_tools::sandbox::stop_container(&name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_CONTAINER_STOP_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_remove_container_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let prefix = state
        .gateway
        .sandbox_router
        .as_ref()
        .map(|r| {
            r.config()
                .container_prefix
                .clone()
                .unwrap_or_else(|| "moltis-sandbox".to_string())
        })
        .unwrap_or_else(|| "moltis-sandbox".to_string());
    if !name.starts_with(&prefix) {
        return api_error_response(
            StatusCode::FORBIDDEN,
            SANDBOX_CONTAINER_PREFIX_MISMATCH,
            "container name does not match expected prefix",
        );
    }
    match moltis_tools::sandbox::remove_container(&name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_CONTAINER_REMOVE_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_clean_all_containers_handler(State(state): State<AppState>) -> impl IntoResponse {
    let prefix = state
        .gateway
        .sandbox_router
        .as_ref()
        .map(|r| {
            r.config()
                .container_prefix
                .clone()
                .unwrap_or_else(|| "moltis-sandbox".to_string())
        })
        .unwrap_or_else(|| "moltis-sandbox".to_string());
    match moltis_tools::sandbox::clean_all_containers(&prefix).await {
        Ok(removed) => Json(serde_json::json!({ "ok": true, "removed": removed })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_CONTAINERS_CLEAN_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_disk_usage_handler() -> impl IntoResponse {
    match moltis_tools::sandbox::container_disk_usage().await {
        Ok(usage) => Json(serde_json::json!({ "usage": usage })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_DISK_USAGE_FAILED,
            e.to_string(),
        ),
    }
}

pub async fn api_restart_daemon_handler() -> impl IntoResponse {
    match moltis_tools::sandbox::restart_container_daemon().await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            SANDBOX_DAEMON_RESTART_FAILED,
            e.to_string(),
        ),
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_inline_for_pdf() {
        let result = media_content_disposition("report.pdf", "application/pdf");
        assert!(result.starts_with("inline;"));
        assert!(result.contains("report.pdf"));
    }

    #[test]
    fn content_disposition_inline_for_text() {
        let result = media_content_disposition("notes.txt", "text/plain");
        assert!(result.starts_with("inline;"));
    }

    #[test]
    fn content_disposition_attachment_for_html() {
        // HTML is forced to attachment to prevent stored XSS.
        let result = media_content_disposition("page.html", "text/html");
        assert!(result.starts_with("attachment;"));
    }

    #[test]
    fn content_disposition_inline_for_csv() {
        let result = media_content_disposition("data.csv", "text/csv");
        assert!(result.starts_with("inline;"));
    }

    #[test]
    fn content_disposition_inline_for_images() {
        let result = media_content_disposition("photo.png", "image/png");
        assert!(result.starts_with("inline;"));
    }

    #[test]
    fn content_disposition_attachment_for_zip() {
        let result = media_content_disposition("archive.zip", "application/zip");
        assert!(result.starts_with("attachment;"));
        assert!(result.contains("archive.zip"));
    }

    #[test]
    fn content_disposition_attachment_for_docx() {
        let result = media_content_disposition(
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        );
        assert!(result.starts_with("attachment;"));
    }

    #[test]
    fn content_disposition_attachment_for_xlsx() {
        let result = media_content_disposition(
            "data.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        );
        assert!(result.starts_with("attachment;"));
    }

    #[test]
    fn content_disposition_sanitises_quotes() {
        let result = media_content_disposition("my\"file.pdf", "application/pdf");
        assert!(!result.contains(r#"my""#));
        assert!(result.contains("myfile.pdf"));
    }

    #[test]
    fn content_disposition_attachment_for_octet_stream() {
        let result = media_content_disposition("data.bin", "application/octet-stream");
        assert!(result.starts_with("attachment;"));
    }

    #[test]
    fn content_disposition_sanitises_semicolons_and_backslashes() {
        let result = media_content_disposition("my;file\\.pdf", "application/pdf");
        // The filename should have semicolons and backslashes stripped.
        assert!(result.contains("myfile.pdf"));
        assert!(!result.contains("my;"));
        assert!(!result.contains('\\'));
    }
}
