//! SPA templates, gon data, and template rendering.

use std::collections::HashSet;

use {
    askama::Template,
    axum::response::{Html, IntoResponse},
    moltis_gateway::state::GatewayState,
    tracing::warn,
};

use crate::assets::{asset_content_hash, is_dev_assets};

// ── SPA routes ───────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpaRoutes {
    chats: &'static str,
    settings: &'static str,
    providers: &'static str,
    security: &'static str,
    identity: &'static str,
    config: &'static str,
    logs: &'static str,
    onboarding: &'static str,
    projects: &'static str,
    skills: &'static str,
    crons: &'static str,
    monitoring: &'static str,
    graphql: &'static str,
}

pub(crate) static SPA_ROUTES: SpaRoutes = SpaRoutes {
    chats: "/chats",
    settings: "/settings",
    providers: "/settings/providers",
    security: "/settings/security",
    identity: "/settings/identity",
    config: "/settings/config",
    logs: "/settings/logs",
    onboarding: "/onboarding",
    projects: "/projects",
    skills: "/skills",
    crons: "/settings/crons",
    monitoring: "/monitoring",
    graphql: "/settings/graphql",
};

// ── GonData ──────────────────────────────────────────────────────────────────

/// Server-side data injected into every page as `window.__MOLTIS__`
/// (gon pattern — see CLAUDE.md § Server-Injected Data).
#[derive(serde::Serialize)]
pub(crate) struct GonData {
    pub(crate) identity: moltis_config::ResolvedIdentity,
    port: u16,
    counts: NavCounts,
    crons: Vec<moltis_cron::types::CronJob>,
    cron_status: moltis_cron::types::CronStatus,
    heartbeat_config: moltis_config::schema::HeartbeatConfig,
    heartbeat_runs: Vec<moltis_cron::types::CronRunRecord>,
    voice_enabled: bool,
    graphql_enabled: bool,
    git_branch: Option<String>,
    mem: MemSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    deploy_platform: Option<String>,
    channels_offered: Vec<String>,
    update: moltis_gateway::update_check::UpdateAvailability,
    sandbox: SandboxGonInfo,
    routes: SpaRoutes,
    started_at: u64,
    agents: Vec<serde_json::Value>,
    #[cfg(feature = "vault")]
    vault_status: String,
}

#[derive(serde::Serialize)]
struct SandboxGonInfo {
    backend: String,
    os: &'static str,
    default_image: String,
    image_building: bool,
}

/// Memory snapshot included in gon data and tick broadcasts.
#[derive(serde::Serialize)]
pub(crate) struct MemSnapshot {
    process: u64,
    available: u64,
    total: u64,
}

/// Collect a point-in-time memory snapshot (process RSS + system memory).
pub(crate) fn collect_mem_snapshot() -> MemSnapshot {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let pid = sysinfo::get_current_pid().ok();
    if let Some(pid) = pid {
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            false,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
    }
    let process = pid
        .and_then(|p| sys.process(p))
        .map(|p| p.memory())
        .unwrap_or(0);
    let total = sys.total_memory();
    // available_memory() returns 0 on macOS; fall back to total − used.
    let available = match sys.available_memory() {
        0 => total.saturating_sub(sys.used_memory()),
        v => v,
    };
    MemSnapshot {
        process,
        available,
        total,
    }
}

// ── Git branch ───────────────────────────────────────────────────────────────

fn detect_git_branch() -> Option<String> {
    static BRANCH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BRANCH
        .get_or_init(|| {
            let repo = gix::discover(".").ok()?;
            let head = repo.head().ok()?;
            let branch = head.referent_name()?.shorten().to_string();
            parse_git_branch(&branch)
        })
        .clone()
}

fn parse_git_branch(raw: &str) -> Option<String> {
    let branch = raw.trim();
    if branch.is_empty() || branch == "main" || branch == "master" {
        None
    } else {
        Some(branch.to_owned())
    }
}

// ── NavCounts ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct NavCounts {
    projects: usize,
    providers: usize,
    channels: usize,
    skills: usize,
    mcp: usize,
    crons: usize,
    hooks: usize,
}

pub(crate) async fn build_nav_counts(gw: &GatewayState) -> NavCounts {
    let (projects, models, channels, mcp, crons) = tokio::join!(
        gw.services.project.list(),
        gw.services.model.list(),
        gw.services.channel.status(),
        gw.services.mcp.list(),
        gw.services.cron.list(),
    );

    let projects = projects
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0);

    let providers = models
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                let mut names: HashSet<&str> = HashSet::new();
                for m in arr {
                    if let Some(p) = m.get("provider").and_then(|p| p.as_str()) {
                        names.insert(p);
                    }
                }
                names.len()
            })
        })
        .unwrap_or(0);

    let channels = channels
        .ok()
        .and_then(|v| {
            v.get("channels")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0);

    let mut skills = 0usize;
    if let Ok(path) = moltis_skills::manifest::ManifestStore::default_path() {
        let store = moltis_skills::manifest::ManifestStore::new(path);
        if let Ok(m) = store.load() {
            skills = m
                .repos
                .iter()
                .flat_map(|r| &r.skills)
                .filter(|s| s.enabled)
                .count();
        }
    }

    let mcp = mcp
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter(|s| s.get("state").and_then(|s| s.as_str()) == Some("running"))
                    .count()
            })
        })
        .unwrap_or(0);

    let crons = crons
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter(|j| {
                        let enabled = j.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
                        let system = j.get("system").and_then(|s| s.as_bool()).unwrap_or(false);
                        enabled && !system
                    })
                    .count()
            })
        })
        .unwrap_or(0);

    let hooks = gw.inner.read().await.discovered_hooks.len();

    NavCounts {
        projects,
        providers,
        channels,
        skills,
        mcp,
        crons,
        hooks,
    }
}

// ── GonData builder ──────────────────────────────────────────────────────────

pub(crate) async fn build_gon_data(gw: &GatewayState) -> GonData {
    let port = gw.port;
    let identity = gw
        .services
        .onboarding
        .identity_get()
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let counts = build_nav_counts(gw).await;
    let (crons, cron_status) = tokio::join!(gw.services.cron.list(), gw.services.cron.status());
    let crons: Vec<moltis_cron::types::CronJob> = crons
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let cron_status: moltis_cron::types::CronStatus = cron_status
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let (heartbeat_config, channels_offered) = {
        let inner = gw.inner.read().await;
        (
            inner.heartbeat_config.clone(),
            inner.channels_offered.clone(),
        )
    };

    let heartbeat_runs: Vec<moltis_cron::types::CronRunRecord> = gw
        .services
        .cron
        .runs(serde_json::json!({ "id": "__heartbeat__", "limit": 10 }))
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let sandbox = if let Some(ref router) = gw.sandbox_router {
        SandboxGonInfo {
            backend: router.backend_name().to_owned(),
            os: std::env::consts::OS,
            default_image: router.default_image().await,
            image_building: router
                .building_flag
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    } else {
        SandboxGonInfo {
            backend: "none".to_owned(),
            os: std::env::consts::OS,
            default_image: moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_owned(),
            image_building: false,
        }
    };

    // Fetch agent personas for the gon data.
    let agents: Vec<serde_json::Value> = if let Some(ref store) = gw.services.agent_persona_store {
        store
            .list()
            .await
            .ok()
            .map(|list| {
                list.into_iter()
                    .map(|a| serde_json::to_value(a).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    GonData {
        identity,
        port,
        counts,
        crons,
        cron_status,
        heartbeat_config,
        heartbeat_runs,
        voice_enabled: cfg!(feature = "voice"),
        graphql_enabled: cfg!(feature = "graphql"),
        git_branch: detect_git_branch(),
        mem: collect_mem_snapshot(),
        deploy_platform: gw.deploy_platform.clone(),
        channels_offered,
        update: gw.inner.read().await.update.clone(),
        sandbox,
        routes: SPA_ROUTES.clone(),
        started_at: *PROCESS_STARTED_AT_MS,
        agents,
        #[cfg(feature = "vault")]
        vault_status: {
            if let Some(ref vault) = gw.vault {
                match vault.status().await {
                    Ok(s) => format!("{s:?}").to_lowercase(),
                    Err(_) => "error".to_owned(),
                }
            } else {
                "disabled".to_owned()
            }
        },
    }
}

// ── Templates ────────────────────────────────────────────────────────────────

/// Unix epoch (milliseconds) captured once at process startup.
pub(crate) static PROCESS_STARTED_AT_MS: std::sync::LazyLock<u64> =
    std::sync::LazyLock::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });

pub(crate) const SHARE_IMAGE_URL: &str = "https://www.moltis.org/og-social.jpg?v=4";

#[derive(Clone, Copy)]
pub(crate) enum SpaTemplate {
    Index,
    Login,
    Onboarding,
}

pub(crate) struct ShareMeta {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) site_name: String,
    pub(crate) image_alt: String,
}

#[derive(Template)]
#[template(path = "index.html", escape = "html")]
struct IndexHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    gon_json: &'a str,
    share_title: &'a str,
    share_description: &'a str,
    share_site_name: &'a str,
    share_image_url: &'a str,
    share_image_alt: &'a str,
    routes: &'a SpaRoutes,
}

#[derive(Template)]
#[template(path = "login.html", escape = "html")]
struct LoginHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    page_title: &'a str,
    gon_json: &'a str,
}

#[derive(Template)]
#[template(path = "onboarding.html", escape = "html")]
struct OnboardingHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    page_title: &'a str,
    gon_json: &'a str,
}

#[derive(serde::Deserialize)]
pub struct ShareAccessQuery {
    #[serde(default)]
    pub k: Option<String>,
}

pub(crate) fn script_safe_json<T: serde::Serialize>(value: &T) -> String {
    let json = match serde_json::to_string(value) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "failed to serialize gon data for html template");
            "{}".to_owned()
        },
    };
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub(crate) fn build_share_meta(identity: &moltis_config::ResolvedIdentity) -> ShareMeta {
    let agent_name = identity_name(identity);
    let user_name = identity
        .user_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let title = match user_name {
        Some(user_name) => format!("{agent_name}: {user_name} AI assistant"),
        None => format!("{agent_name}: AI assistant"),
    };
    let description = match user_name {
        Some(user_name) => format!(
            "{agent_name} is {user_name}'s personal AI assistant. Multi-provider models, tools, memory, sandboxed execution, and channel access in one Rust binary."
        ),
        None => format!(
            "{agent_name} is a personal AI assistant. Multi-provider models, tools, memory, sandboxed execution, and channel access in one Rust binary."
        ),
    };
    let image_alt = format!("{agent_name} - personal AI assistant");

    ShareMeta {
        title,
        description,
        site_name: agent_name.to_owned(),
        image_alt,
    }
}

pub(crate) fn identity_name(identity: &moltis_config::ResolvedIdentity) -> &str {
    let name = identity.name.trim();
    if name.is_empty() {
        "moltis"
    } else {
        name
    }
}

pub(crate) async fn render_spa_template(
    gateway: &GatewayState,
    template: SpaTemplate,
) -> axum::response::Response {
    let (build_ts, asset_prefix) = if is_dev_assets() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        ("dev".to_owned(), format!("/assets/v/{ts}/"))
    } else {
        static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(asset_content_hash);
        (HASH.to_string(), format!("/assets/v/{}/", *HASH))
    };

    let nonce = uuid::Uuid::new_v4().to_string();
    let body = match template {
        SpaTemplate::Index => {
            let gon = build_gon_data(gateway).await;
            let share_meta = build_share_meta(&gon.identity);
            let gon_json = script_safe_json(&gon);
            let template = IndexHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                gon_json: &gon_json,
                share_title: &share_meta.title,
                share_description: &share_meta.description,
                share_site_name: &share_meta.site_name,
                share_image_url: SHARE_IMAGE_URL,
                share_image_alt: &share_meta.image_alt,
                routes: &SPA_ROUTES,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render index template");
                    String::new()
                },
            }
        },
        SpaTemplate::Login => {
            let gon = build_gon_data(gateway).await;
            let gon_json = script_safe_json(&gon);
            let page_title = identity_name(&gon.identity).to_owned();
            let template = LoginHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                page_title: &page_title,
                gon_json: &gon_json,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render login template");
                    String::new()
                },
            }
        },
        SpaTemplate::Onboarding => {
            let gon = build_gon_data(gateway).await;
            let gon_json = script_safe_json(&gon);
            let page_title = format!("{} onboarding", identity_name(&gon.identity));
            let template = OnboardingHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                page_title: &page_title,
                gon_json: &gon_json,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render onboarding template");
                    String::new()
                },
            }
        },
    };

    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; \
         media-src 'self' blob:; \
         font-src 'self'; \
         connect-src 'self' ws: wss:; \
         frame-ancestors 'none'; \
         form-action 'self'; \
         base-uri 'self'; \
         object-src 'none'"
    );

    let mut response = Html(body).into_response();
    let headers = response.headers_mut();
    if let Ok(val) = "no-cache, no-store".parse() {
        headers.insert(axum::http::header::CACHE_CONTROL, val);
    }
    if let Ok(val) = csp.parse() {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, val);
    }
    response
}

// ── Onboarding helpers ───────────────────────────────────────────────────────

pub(crate) fn should_redirect_to_onboarding(path: &str, onboarded: bool) -> bool {
    !is_onboarding_path(path) && !onboarded
}

pub(crate) fn should_redirect_from_onboarding(onboarded: bool) -> bool {
    onboarded
}

fn is_onboarding_path(path: &str) -> bool {
    path == "/onboarding" || path == "/onboarding/"
}

pub(crate) async fn onboarding_completed(gw: &GatewayState) -> bool {
    gw.services
        .onboarding
        .wizard_status()
        .await
        .ok()
        .and_then(|v| v.get("onboarded").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_branch_filters_defaults() {
        assert_eq!(parse_git_branch("main"), None);
        assert_eq!(parse_git_branch("master"), None);
        assert_eq!(parse_git_branch(""), None);
        assert_eq!(parse_git_branch("  "), None);
        assert_eq!(
            parse_git_branch("feature/foo"),
            Some("feature/foo".to_owned())
        );
    }

    #[test]
    fn script_safe_json_escapes_html() {
        let val = "<script>alert(1)</script>";
        let safe = script_safe_json(&val);
        assert!(!safe.contains('<'));
        assert!(!safe.contains('>'));
    }
}
