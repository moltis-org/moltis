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

pub async fn api_session_media_handler(
    Path((session_key, filename)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(ref store) = state.gateway.services.session_store else {
        return (StatusCode::NOT_FOUND, "session store not available").into_response();
    };
    match store.read_media(&session_key, &filename).await {
        Ok(data) => {
            let content_type = match filename.rsplit('.').next() {
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("ogg" | "oga") => "audio/ogg",
                Some("webm") => "audio/webm",
                Some("mp3") => "audio/mpeg",
                _ => "application/octet-stream",
            };
            ([(axum::http::header::CONTENT_TYPE, content_type)], data).into_response()
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

pub async fn api_bootstrap_handler(State(state): State<AppState>) -> impl IntoResponse {
    let gw = &state.gateway;
    let (channels, sessions, models, projects, onboarded) = tokio::join!(
        gw.services.channel.status(),
        gw.services.session.list(),
        gw.services.model.list(),
        gw.services.project.list(),
        onboarding_completed(gw),
    );
    let identity = gw.services.agent.identity_get().await.ok();
    let sandbox = if let Some(ref router) = state.gateway.sandbox_router {
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
    let counts = build_nav_counts(gw).await;
    Json(serde_json::json!({
        "channels": channels.ok(),
        "sessions": sessions.ok(),
        "models": models.ok(),
        "projects": projects.ok(),
        "onboarded": onboarded,
        "identity": identity,
        "sandbox": sandbox,
        "counts": counts,
    }))
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
                skills.push(serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "source": s.source,
                    "enabled": true,
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

    let output = tokio::process::Command::new("docker")
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
