use std::{net::SocketAddr, sync::Arc};

use secrecy::ExposeSecret;

#[cfg(feature = "tls")]
use std::path::PathBuf;

#[cfg(feature = "web-ui")]
use axum::response::Html;
use {
    axum::{
        Router,
        extract::{ConnectInfo, State, WebSocketUpgrade},
        response::{IntoResponse, Json},
        routing::get,
    },
    tower_http::cors::{Any, CorsLayer},
    tracing::{debug, info, warn},
};

#[cfg(feature = "web-ui")]
use axum::{extract::Path, http::StatusCode};

use {moltis_channels::ChannelPlugin, moltis_protocol::TICK_INTERVAL_MS};

use moltis_agents::providers::ProviderRegistry;

use moltis_tools::{approval::ApprovalManager, exec::EnvVarProvider, image_cache::ImageBuilder};

use {
    moltis_projects::ProjectStore,
    moltis_sessions::{
        metadata::{SessionMetadata, SqliteSessionMetadata},
        store::SessionStore,
    },
};

use crate::{
    approval::{GatewayApprovalBroadcaster, LiveExecApprovalService},
    auth,
    auth_routes::{AuthState, auth_router},
    broadcast::{BroadcastOpts, broadcast, broadcast_tick},
    chat::{LiveChatService, LiveModelService},
    methods::MethodRegistry,
    provider_setup::LiveProviderSetupService,
    services::GatewayServices,
    session::LiveSessionService,
    state::GatewayState,
    ws::handle_connection,
};

#[cfg(feature = "tailscale")]
use crate::tailscale::{
    CliTailscaleManager, TailscaleManager, TailscaleMode, validate_tailscale_config,
};

#[cfg(feature = "tls")]
use crate::tls::CertManager;

/// Options for tailscale serve/funnel passed from CLI flags.
#[cfg(feature = "tailscale")]
pub struct TailscaleOpts {
    pub mode: String,
    pub reset_on_exit: bool,
}

// ── Shared app state ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub gateway: Arc<GatewayState>,
    pub methods: Arc<MethodRegistry>,
}

// ── Server startup ───────────────────────────────────────────────────────────

/// Build the gateway router (shared between production startup and tests).
pub fn build_gateway_app(state: Arc<GatewayState>, methods: Arc<MethodRegistry>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws_upgrade_handler));

    // Add Prometheus metrics endpoint (unauthenticated for scraping).
    #[cfg(feature = "prometheus")]
    {
        router = router.route(
            "/metrics",
            get(crate::metrics_routes::prometheus_metrics_handler),
        );
    }

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_state: state.webauthn_state.clone(),
            gateway_state: Arc::clone(&state),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    let app_state = AppState {
        gateway: state,
        methods,
    };

    #[cfg(feature = "web-ui")]
    let router = {
        // Protected API routes — require auth when credential store is configured.
        let protected = Router::new()
            .route("/api/bootstrap", get(api_bootstrap_handler))
            .route("/api/gon", get(api_gon_handler))
            .route("/api/skills", get(api_skills_handler))
            .route("/api/skills/search", get(api_skills_search_handler))
            .route("/api/mcp", get(api_mcp_handler))
            .route("/api/plugins", get(api_plugins_handler))
            .route("/api/plugins/search", get(api_plugins_search_handler))
            .route(
                "/api/images/cached",
                get(api_cached_images_handler).delete(api_prune_cached_images_handler),
            )
            .route(
                "/api/images/cached/{tag}",
                axum::routing::delete(api_delete_cached_image_handler),
            )
            .route(
                "/api/images/build",
                axum::routing::post(api_build_image_handler),
            )
            .route(
                "/api/images/check-packages",
                axum::routing::post(api_check_packages_handler),
            )
            .route(
                "/api/images/default",
                get(api_get_default_image_handler).put(api_set_default_image_handler),
            )
            .route(
                "/api/env",
                get(crate::env_routes::env_list).post(crate::env_routes::env_set),
            )
            .route(
                "/api/env/{id}",
                axum::routing::delete(crate::env_routes::env_delete),
            );

        // Add metrics API routes (protected).
        #[cfg(feature = "metrics")]
        let protected = protected
            .route(
                "/api/metrics",
                get(crate::metrics_routes::api_metrics_handler),
            )
            .route(
                "/api/metrics/summary",
                get(crate::metrics_routes::api_metrics_summary_handler),
            )
            .route(
                "/api/metrics/history",
                get(crate::metrics_routes::api_metrics_history_handler),
            );

        let protected = protected.layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            crate::auth_middleware::require_auth,
        ));

        // Mount tailscale routes (protected) when the feature is enabled.
        #[cfg(feature = "tailscale")]
        let protected = protected.nest(
            "/api/tailscale",
            crate::tailscale_routes::tailscale_router(),
        );

        // Public routes (assets, SPA fallback).
        router
            .route("/assets/v/{version}/{*path}", get(versioned_asset_handler))
            .route("/assets/{*path}", get(asset_handler))
            .merge(protected)
            .fallback(spa_fallback)
    };

    router.layer(cors).with_state(app_state)
}

/// Start the gateway HTTP + WebSocket server.
pub async fn start_gateway(
    bind: &str,
    port: u16,
    log_buffer: Option<crate::logs::LogBuffer>,
    config_dir: Option<std::path::PathBuf>,
    data_dir: Option<std::path::PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
) -> anyhow::Result<()> {
    // Apply directory overrides before loading config.
    if let Some(dir) = config_dir {
        moltis_config::set_config_dir(dir);
    }
    if let Some(ref dir) = data_dir {
        moltis_config::set_data_dir(dir.clone());
    }

    // Resolve auth from environment (MOLTIS_TOKEN / MOLTIS_PASSWORD).
    let token = std::env::var("MOLTIS_TOKEN").ok();
    let password = std::env::var("MOLTIS_PASSWORD").ok();
    let resolved_auth = auth::resolve_auth(token, password.clone());

    // Load config file (moltis.toml / .yaml / .json) if present.
    let config = moltis_config::discover_and_load();

    // Merge any previously saved API keys into the provider config so they
    // survive gateway restarts without requiring env vars.
    let key_store = crate::provider_setup::KeyStore::new();
    let effective_providers =
        crate::provider_setup::config_with_saved_keys(&config.providers, &key_store);

    // Discover LLM providers from env + config + saved keys.
    let registry = Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_env_with_config(&effective_providers),
    ));
    let provider_summary = registry.read().await.provider_summary();

    // Create shared approval manager.
    let approval_manager = Arc::new(ApprovalManager::default());

    let mut services = GatewayServices::noop();

    // Wire live logs service if a log buffer is available.
    if let Some(ref buf) = log_buffer {
        services.logs = Arc::new(crate::logs::LiveLogsService::new(buf.clone()));
    }

    services.exec_approval = Arc::new(LiveExecApprovalService::new(Arc::clone(&approval_manager)));

    // Wire live onboarding service.
    let onboarding_config_path = moltis_config::find_or_default_config_path();
    let live_onboarding =
        moltis_onboarding::service::LiveOnboardingService::new(onboarding_config_path);
    services = services.with_onboarding(Arc::new(
        crate::onboarding::GatewayOnboardingService::new(live_onboarding),
    ));
    services.provider_setup = Arc::new(LiveProviderSetupService::new(
        Arc::clone(&registry),
        config.providers.clone(),
    ));

    // Wire live local-llm service when the feature is enabled.
    #[cfg(feature = "local-llm")]
    let local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>> = {
        let svc = Arc::new(crate::local_llm_setup::LiveLocalLlmService::new(
            Arc::clone(&registry),
        ));
        services =
            services.with_local_llm(Arc::clone(&svc) as Arc<dyn crate::services::LocalLlmService>);
        Some(svc)
    };
    #[cfg(not(feature = "local-llm"))]
    let local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>> = None;

    if !registry.read().await.is_empty() {
        services = services.with_model(Arc::new(LiveModelService::new(Arc::clone(&registry))));
    }

    // Wire live MCP service.
    let mcp_configured_count;
    let live_mcp: Arc<crate::mcp_service::LiveMcpService>;
    {
        let mcp_registry_path = moltis_config::data_dir().join("mcp-servers.json");
        let mcp_reg = moltis_mcp::McpRegistry::load(&mcp_registry_path).unwrap_or_default();
        // Seed from config file servers that aren't already in the registry.
        let mut merged = mcp_reg;
        for (name, entry) in &config.mcp.servers {
            if !merged.servers.contains_key(name) {
                let transport = match entry.transport.as_str() {
                    "sse" => moltis_mcp::registry::TransportType::Sse,
                    _ => moltis_mcp::registry::TransportType::Stdio,
                };
                merged
                    .servers
                    .insert(name.clone(), moltis_mcp::McpServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env: entry.env.clone(),
                        enabled: entry.enabled,
                        transport,
                        url: entry.url.clone(),
                    });
            }
        }
        mcp_configured_count = merged.servers.values().filter(|s| s.enabled).count();
        let mcp_manager = Arc::new(moltis_mcp::McpManager::new(merged));
        live_mcp = Arc::new(crate::mcp_service::LiveMcpService::new(Arc::clone(
            &mcp_manager,
        )));
        // Start enabled servers in the background; sync tools once done.
        let mgr = Arc::clone(&mcp_manager);
        let mcp_for_sync = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            let started = mgr.start_enabled().await;
            if !started.is_empty() {
                tracing::info!(servers = ?started, "MCP servers started");
            }
            // Sync newly started tools into the agent tool registry.
            mcp_for_sync.sync_tools_if_ready().await;
        });
        services.mcp = live_mcp.clone() as Arc<dyn crate::services::McpService>;
    }

    // Initialize data directory and SQLite database.
    let data_dir = data_dir.unwrap_or_else(moltis_config::data_dir);
    std::fs::create_dir_all(&data_dir).ok();

    // Enable log persistence so entries survive restarts.
    if let Some(ref buf) = log_buffer {
        buf.enable_persistence(data_dir.join("logs.jsonl"));
    }
    let db_path = data_dir.join("moltis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let db_pool = sqlx::SqlitePool::connect(&db_url)
        .await
        .expect("failed to open moltis.db");

    // Run database migrations from each crate in dependency order.
    // Order matters: sessions depends on projects (FK reference).
    moltis_projects::run_migrations(&db_pool)
        .await
        .expect("failed to run projects migrations");
    moltis_sessions::run_migrations(&db_pool)
        .await
        .expect("failed to run sessions migrations");
    moltis_cron::run_migrations(&db_pool)
        .await
        .expect("failed to run cron migrations");
    // Gateway's own tables (auth, message_log, channels).
    crate::run_migrations(&db_pool)
        .await
        .expect("failed to run gateway migrations");

    // Initialize credential store (auth tables).
    let credential_store = Arc::new(
        auth::CredentialStore::new(db_pool.clone())
            .await
            .expect("failed to init credential store"),
    );

    // Initialize WebAuthn state for passkey support.
    // RP ID defaults to "localhost"; override with MOLTIS_WEBAUTHN_RP_ID.
    let rp_id = std::env::var("MOLTIS_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };
    let rp_origin_str = std::env::var("MOLTIS_WEBAUTHN_ORIGIN")
        .unwrap_or_else(|_| format!("{default_scheme}://{rp_id}:{port}"));
    let webauthn_state = match webauthn_rs::prelude::Url::parse(&rp_origin_str) {
        Ok(rp_origin) => match crate::auth_webauthn::WebAuthnState::new(&rp_id, &rp_origin) {
            Ok(wa) => Some(Arc::new(wa)),
            Err(e) => {
                tracing::warn!("failed to init WebAuthn: {e}");
                None
            },
        },
        Err(e) => {
            tracing::warn!("invalid WebAuthn origin URL '{rp_origin_str}': {e}");
            None
        },
    };

    // If MOLTIS_PASSWORD is set and no password in DB yet, migrate it.
    if let Some(ref pw) = password
        && !credential_store.is_setup_complete()
    {
        info!("migrating MOLTIS_PASSWORD env var to credential store");
        if let Err(e) = credential_store.set_initial_password(pw).await {
            tracing::warn!("failed to migrate env password: {e}");
        }
    }

    let message_log: Arc<dyn moltis_channels::message_log::MessageLog> = Arc::new(
        crate::message_log_store::SqliteMessageLog::new(db_pool.clone()),
    );

    // Migrate from projects.toml if it exists.
    let config_dir =
        moltis_config::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".moltis"));
    let projects_toml_path = config_dir.join("projects.toml");
    if projects_toml_path.exists() {
        info!("migrating projects.toml to SQLite");
        let old_store = moltis_projects::TomlProjectStore::new(projects_toml_path.clone());
        let sqlite_store = moltis_projects::SqliteProjectStore::new(db_pool.clone());
        if let Ok(projects) =
            <moltis_projects::TomlProjectStore as moltis_projects::ProjectStore>::list(&old_store)
                .await
        {
            for p in projects {
                if let Err(e) = sqlite_store.upsert(p).await {
                    tracing::warn!("failed to migrate project: {e}");
                }
            }
        }
        let bak = projects_toml_path.with_extension("toml.bak");
        std::fs::rename(&projects_toml_path, &bak).ok();
    }

    // Migrate from metadata.json if it exists.
    let sessions_dir = data_dir.join("sessions");
    let metadata_json_path = sessions_dir.join("metadata.json");
    if metadata_json_path.exists() {
        info!("migrating metadata.json to SQLite");
        if let Ok(old_meta) = SessionMetadata::load(metadata_json_path.clone()) {
            let sqlite_meta = SqliteSessionMetadata::new(db_pool.clone());
            for entry in old_meta.list() {
                if let Err(e) = sqlite_meta.upsert(&entry.key, entry.label.clone()).await {
                    tracing::warn!("failed to migrate session {}: {e}", entry.key);
                }
                if entry.model.is_some() {
                    sqlite_meta.set_model(&entry.key, entry.model.clone()).await;
                }
                sqlite_meta.touch(&entry.key, entry.message_count).await;
                if entry.project_id.is_some() {
                    sqlite_meta
                        .set_project_id(&entry.key, entry.project_id.clone())
                        .await;
                }
            }
        }
        let bak = metadata_json_path.with_extension("json.bak");
        std::fs::rename(&metadata_json_path, &bak).ok();
    }

    // Wire stores.
    let project_store: Arc<dyn moltis_projects::ProjectStore> =
        Arc::new(moltis_projects::SqliteProjectStore::new(db_pool.clone()));
    let session_store = Arc::new(SessionStore::new(sessions_dir));
    let session_metadata = Arc::new(SqliteSessionMetadata::new(db_pool.clone()));

    // Session service wired below after sandbox_router is created.

    // Wire live project service.
    services.project = Arc::new(crate::project::LiveProjectService::new(Arc::clone(
        &project_store,
    )));

    // Initialize cron service with file-backed store.
    let cron_store: Arc<dyn moltis_cron::store::CronStore> =
        match moltis_cron::store_file::FileStore::default_path() {
            Ok(fs) => Arc::new(fs),
            Err(e) => {
                tracing::warn!("cron file store unavailable ({e}), using in-memory");
                Arc::new(moltis_cron::store_memory::InMemoryStore::new())
            },
        };

    // Deferred reference: populated once GatewayState is ready.
    let deferred_state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());

    // System event: inject text into the main session and trigger an agent response.
    let sys_state = Arc::clone(&deferred_state);
    let on_system_event: moltis_cron::service::SystemEventFn = Arc::new(move |text| {
        let st = Arc::clone(&sys_state);
        tokio::spawn(async move {
            if let Some(state) = st.get() {
                let chat = state.chat().await;
                let params = serde_json::json!({ "text": text });
                if let Err(e) = chat.send(params).await {
                    tracing::error!("cron system event failed: {e}");
                }
            }
        });
    });

    // Agent turn: run an LLM turn in a session determined by the job's session_target.
    let agent_state = Arc::clone(&deferred_state);
    let on_agent_turn: moltis_cron::service::AgentTurnFn = Arc::new(move |req| {
        let st = Arc::clone(&agent_state);
        Box::pin(async move {
            let state = st
                .get()
                .ok_or_else(|| anyhow::anyhow!("gateway not ready"))?;
            let chat = state.chat().await;
            let session_key = match &req.session_target {
                moltis_cron::types::SessionTarget::Named(name) => {
                    format!("cron:{name}")
                },
                _ => format!("cron:{}", uuid::Uuid::new_v4()),
            };

            // Clear session history for named cron sessions before execution
            // so the run starts fresh but the history remains readable for debugging.
            if matches!(
                req.session_target,
                moltis_cron::types::SessionTarget::Named(_)
            ) {
                let _ = chat
                    .clear(serde_json::json!({ "_session_key": session_key }))
                    .await;
            }

            // Apply sandbox overrides for this cron session.
            if let Some(ref router) = state.sandbox_router {
                router.set_override(&session_key, req.sandbox.enabled).await;
                if let Some(ref image) = req.sandbox.image {
                    router.set_image_override(&session_key, image.clone()).await;
                }
            }

            let mut params = serde_json::json!({
                "text": req.message,
                "_session_key": session_key,
            });
            if let Some(ref model) = req.model {
                params["model"] = serde_json::Value::String(model.clone());
            }
            let result = chat.send_sync(params).await.map_err(|e| anyhow::anyhow!(e));

            // Clean up sandbox overrides.
            if let Some(ref router) = state.sandbox_router {
                router.remove_override(&session_key).await;
            }

            let val = result?;
            let input_tokens = val.get("inputTokens").and_then(|v| v.as_u64());
            let output_tokens = val.get("outputTokens").and_then(|v| v.as_u64());
            let text = val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(moltis_cron::service::AgentTurnResult {
                output: text,
                input_tokens,
                output_tokens,
            })
        })
    });

    let cron_service =
        moltis_cron::service::CronService::new(cron_store, on_system_event, on_agent_turn);

    // Wire cron into gateway services.
    let live_cron = Arc::new(crate::cron::LiveCronService::new(Arc::clone(&cron_service)));
    services = services.with_cron(live_cron);

    // Build sandbox router from config (shared across sessions).
    let sandbox_config = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let sandbox_router = Arc::new(moltis_tools::sandbox::SandboxRouter::new(sandbox_config));

    // Spawn background image pre-build. This bakes configured packages into a
    // container image so container creation is instant. Backends that don't
    // support image building return Ok(None) and the spawn is harmless.
    {
        let router = Arc::clone(&sandbox_router);
        let backend = Arc::clone(router.backend());
        let packages = router.config().packages.clone();
        let base_image = router
            .config()
            .image
            .clone()
            .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string());

        if !packages.is_empty() {
            let deferred_for_build = Arc::clone(&deferred_state);
            tokio::spawn(async move {
                // Broadcast build start event.
                if let Some(state) = deferred_for_build.get() {
                    crate::broadcast::broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({ "phase": "start", "packages": packages }),
                        crate::broadcast::BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match backend.build_image(&base_image, &packages).await {
                    Ok(Some(result)) => {
                        info!(
                            tag = %result.tag,
                            built = result.built,
                            "sandbox image pre-build complete"
                        );
                        router.set_global_image(Some(result.tag.clone())).await;

                        if let Some(state) = deferred_for_build.get() {
                            crate::broadcast::broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
                                }),
                                crate::broadcast::BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                    },
                    Err(e) => {
                        tracing::warn!("sandbox image pre-build failed: {e}");
                        if let Some(state) = deferred_for_build.get() {
                            crate::broadcast::broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                crate::broadcast::BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // Load any persisted sandbox overrides from session metadata.
    {
        for entry in session_metadata.list().await {
            if let Some(enabled) = entry.sandbox_enabled {
                sandbox_router.set_override(&entry.key, enabled).await;
            }
            if let Some(ref image) = entry.sandbox_image {
                sandbox_router
                    .set_image_override(&entry.key, image.clone())
                    .await;
            }
        }
    }

    // Session service is wired after hook registry is built (below).

    // Wire channel store and Telegram channel service.
    {
        use moltis_channels::store::ChannelStore;

        let channel_store: Arc<dyn ChannelStore> = Arc::new(
            crate::channel_store::SqliteChannelStore::new(db_pool.clone()),
        );

        let channel_sink = Arc::new(crate::channel_events::GatewayChannelEventSink::new(
            Arc::clone(&deferred_state),
        ));
        let mut tg_plugin = moltis_telegram::TelegramPlugin::new()
            .with_message_log(Arc::clone(&message_log))
            .with_event_sink(channel_sink);

        // Start channels from config file (these take precedence).
        let tg_accounts = &config.channels.telegram;
        let mut started: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (account_id, account_config) in tg_accounts {
            if let Err(e) = tg_plugin
                .start_account(account_id, account_config.clone())
                .await
            {
                tracing::warn!(account_id, "failed to start telegram account: {e}");
            } else {
                started.insert(account_id.clone());
            }
        }

        // Load persisted channels that weren't in the config file.
        match channel_store.list().await {
            Ok(stored) => {
                info!("{} stored channel(s) found in database", stored.len());
                for ch in stored {
                    if started.contains(&ch.account_id) {
                        info!(
                            account_id = ch.account_id,
                            "skipping stored channel (already started from config)"
                        );
                        continue;
                    }
                    info!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "starting stored channel"
                    );
                    if let Err(e) = tg_plugin.start_account(&ch.account_id, ch.config).await {
                        tracing::warn!(
                            account_id = ch.account_id,
                            "failed to start stored telegram account: {e}"
                        );
                    } else {
                        started.insert(ch.account_id);
                    }
                }
            },
            Err(e) => {
                tracing::warn!("failed to load stored channels: {e}");
            },
        }

        if !started.is_empty() {
            info!("{} telegram account(s) started", started.len());
        }

        // Grab shared outbound before moving tg_plugin into the channel service.
        let tg_outbound = tg_plugin.shared_outbound();
        services = services.with_channel_outbound(tg_outbound);

        services.channel = Arc::new(crate::channel::LiveChannelService::new(
            tg_plugin,
            channel_store,
            Arc::clone(&message_log),
            Arc::clone(&session_metadata),
        ));
    }

    services = services.with_session_metadata(Arc::clone(&session_metadata));
    services = services.with_session_store(Arc::clone(&session_store));

    // ── Hook discovery & registration ─────────────────────────────────────
    let hook_registry = {
        use moltis_plugins::{
            hook_discovery::{FsHookDiscoverer, HookDiscoverer},
            hook_eligibility::check_hook_eligibility,
            shell_hook::ShellHookHandler,
        };

        let cwd = std::env::current_dir().unwrap_or_default();
        let discoverer = FsHookDiscoverer::new(FsHookDiscoverer::default_paths(&cwd));
        let discovered = discoverer.discover().await.unwrap_or_default();

        let mut registry = moltis_common::hooks::HookRegistry::new();

        // Load shell hooks from discovered HOOK.md files.
        for (parsed, source) in &discovered {
            let meta = &parsed.metadata;
            let elig = check_hook_eligibility(meta);
            if !elig.eligible {
                info!(
                    hook = %meta.name,
                    source = ?source,
                    missing_os = elig.missing_os,
                    missing_bins = ?elig.missing_bins,
                    missing_env = ?elig.missing_env,
                    "hook ineligible, skipping"
                );
                continue;
            }
            if let Some(ref command) = meta.command {
                let handler = ShellHookHandler::new(
                    meta.name.clone(),
                    command.clone(),
                    meta.events.clone(),
                    std::time::Duration::from_secs(meta.timeout),
                    meta.env.clone(),
                );
                registry.register(Arc::new(handler));
            }
        }

        if !discovered.is_empty() {
            info!(
                "{} hook(s) discovered, {} registered",
                discovered.len(),
                registry.handler_names().len()
            );
        }

        Some(Arc::new(registry))
    };

    // Wire live session service with sandbox router, project store, and hooks.
    {
        let mut session_svc =
            LiveSessionService::new(Arc::clone(&session_store), Arc::clone(&session_metadata))
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_project_store(Arc::clone(&project_store));
        if let Some(ref hooks) = hook_registry {
            session_svc = session_svc.with_hooks(Arc::clone(hooks));
        }
        services.session = Arc::new(session_svc);
    }

    // ── Memory system initialization ─────────────────────────────────────
    let memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>> = {
        // Build embedding provider(s) for the fallback chain.
        let mut embedding_providers: Vec<(
            String,
            Box<dyn moltis_memory::embeddings::EmbeddingProvider>,
        )> = Vec::new();

        let mem_cfg = &config.memory;

        // 1. If user explicitly configured an embedding provider, use it.
        if let Some(ref provider_name) = mem_cfg.provider {
            match provider_name.as_str() {
                "local" => {
                    // Local GGUF embeddings require the `local-embeddings` feature on moltis-memory.
                    #[cfg(feature = "local-embeddings")]
                    {
                        let cache_dir = mem_cfg
                            .base_url
                            .as_ref()
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(
                                moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::default_cache_dir,
                            );
                        match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::ensure_model(
                            cache_dir,
                        )
                        .await
                        {
                            Ok(path) => {
                                match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::new(
                                    path,
                                ) {
                                    Ok(p) => embedding_providers.push(("local-gguf".into(), Box::new(p))),
                                    Err(e) => warn!("memory: failed to load local GGUF model: {e}"),
                                }
                            },
                            Err(e) => warn!("memory: failed to ensure local model: {e}"),
                        }
                    }
                    #[cfg(not(feature = "local-embeddings"))]
                    warn!(
                        "memory: 'local' embedding provider requires the 'local-embeddings' feature"
                    );
                },
                "ollama" | "custom" | "openai" => {
                    let base_url = mem_cfg
                        .base_url
                        .clone()
                        .unwrap_or_else(|| match provider_name.as_str() {
                            "ollama" => "http://localhost:11434".into(),
                            _ => "https://api.openai.com".into(),
                        });
                    let api_key = mem_cfg
                        .api_key
                        .as_ref()
                        .map(|k| k.expose_secret().clone())
                        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                        .unwrap_or_default();
                    let mut e =
                        moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                    if base_url != "https://api.openai.com" {
                        e = e.with_base_url(base_url);
                    }
                    if let Some(ref model) = mem_cfg.model {
                        // Use a sensible default dims; the API returns the actual dims.
                        e = e.with_model(model.clone(), 1536);
                    }
                    embedding_providers.push((provider_name.clone(), Box::new(e)));
                },
                other => warn!("memory: unknown embedding provider '{other}'"),
            }
        }

        // 2. Auto-detect: try Ollama health check.
        if embedding_providers.is_empty() {
            let ollama_ok = reqwest::Client::new()
                .get("http://localhost:11434/api/tags")
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
                .is_ok();
            if ollama_ok {
                let e =
                    moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(String::new())
                        .with_base_url("http://localhost:11434".into())
                        .with_model("nomic-embed-text".into(), 768);
                embedding_providers.push(("ollama".into(), Box::new(e)));
                info!("memory: detected Ollama at localhost:11434");
            }
        }

        // 3. Auto-detect: try remote API-key providers.
        const EMBEDDING_CANDIDATES: &[(&str, &str, &str)] = &[
            ("openai", "OPENAI_API_KEY", "https://api.openai.com"),
            ("mistral", "MISTRAL_API_KEY", "https://api.mistral.ai/v1"),
            (
                "openrouter",
                "OPENROUTER_API_KEY",
                "https://openrouter.ai/api/v1",
            ),
            ("groq", "GROQ_API_KEY", "https://api.groq.com/openai"),
            ("xai", "XAI_API_KEY", "https://api.x.ai"),
            ("deepseek", "DEEPSEEK_API_KEY", "https://api.deepseek.com"),
            ("cerebras", "CEREBRAS_API_KEY", "https://api.cerebras.ai/v1"),
            ("minimax", "MINIMAX_API_KEY", "https://api.minimax.chat/v1"),
            ("moonshot", "MOONSHOT_API_KEY", "https://api.moonshot.ai/v1"),
            ("venice", "VENICE_API_KEY", "https://api.venice.ai/api/v1"),
        ];

        for (config_name, env_key, default_base) in EMBEDDING_CANDIDATES {
            let key = effective_providers
                .get(config_name)
                .and_then(|e| e.api_key.as_ref().map(|k| k.expose_secret().clone()))
                .or_else(|| std::env::var(env_key).ok())
                .filter(|k| !k.is_empty());
            if let Some(api_key) = key {
                let base = effective_providers
                    .get(config_name)
                    .and_then(|e| e.base_url.clone())
                    .unwrap_or_else(|| default_base.to_string());
                let mut e = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                if base != "https://api.openai.com" {
                    e = e.with_base_url(base);
                }
                embedding_providers.push((config_name.to_string(), Box::new(e)));
            }
        }

        // Build the final embedder: fallback chain, single provider, or keyword-only.
        let embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>> =
            if embedding_providers.is_empty() {
                info!("memory: no embedding provider found, using keyword-only search");
                None
            } else {
                let names: Vec<&str> = embedding_providers
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect();
                if embedding_providers.len() == 1 {
                    let (name, provider) = embedding_providers.into_iter().next().unwrap();
                    info!(provider = %name, "memory: using single embedding provider");
                    Some(provider)
                } else {
                    info!(providers = ?names, active = names[0], "memory: fallback chain configured");
                    Some(Box::new(
                        moltis_memory::embeddings_fallback::FallbackEmbeddingProvider::new(
                            embedding_providers,
                        ),
                    ))
                }
            };

        let memory_db_path = data_dir.join("memory.db");
        let memory_db_url = format!("sqlite:{}?mode=rwc", memory_db_path.display());
        match sqlx::SqlitePool::connect(&memory_db_url).await {
            Ok(memory_pool) => {
                if let Err(e) = moltis_memory::schema::run_migrations(&memory_pool).await {
                    tracing::warn!("memory migration failed: {e}");
                    None
                } else {
                    // Scan the data directory for memory files written by the
                    // silent memory turn (MEMORY.md, memory/*.md).
                    let data_memory_root = data_dir.clone();
                    let data_memory_sub = data_dir.join("memory");

                    let config = moltis_memory::config::MemoryConfig {
                        db_path: memory_db_path.to_string_lossy().into(),
                        memory_dirs: vec![data_memory_root, data_memory_sub],
                        ..Default::default()
                    };

                    let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(
                        memory_pool,
                    ));
                    let watch_dirs = config.memory_dirs.clone();
                    let manager = Arc::new(if let Some(embedder) = embedder {
                        moltis_memory::manager::MemoryManager::new(config, store, embedder)
                    } else {
                        moltis_memory::manager::MemoryManager::keyword_only(config, store)
                    });

                    // Initial sync + periodic re-sync (15min with watcher, 5min without).
                    let sync_manager = Arc::clone(&manager);
                    tokio::spawn(async move {
                        match sync_manager.sync().await {
                            Ok(report) => {
                                info!(
                                    updated = report.files_updated,
                                    unchanged = report.files_unchanged,
                                    removed = report.files_removed,
                                    errors = report.errors,
                                    cache_hits = report.cache_hits,
                                    cache_misses = report.cache_misses,
                                    "memory: initial sync complete"
                                );
                                match sync_manager.status().await {
                                    Ok(status) => info!(
                                        files = status.total_files,
                                        chunks = status.total_chunks,
                                        db_size = %status.db_size_display(),
                                        model = %status.embedding_model,
                                        "memory: status"
                                    ),
                                    Err(e) => tracing::warn!("memory: failed to get status: {e}"),
                                }
                            },
                            Err(e) => tracing::warn!("memory: initial sync failed: {e}"),
                        }

                        // Start file watcher for real-time sync (if feature enabled).
                        #[cfg(feature = "file-watcher")]
                        {
                            let watcher_manager = Arc::clone(&sync_manager);
                            match moltis_memory::watcher::MemoryFileWatcher::start(watch_dirs) {
                                Ok((_watcher, mut rx)) => {
                                    info!("memory: file watcher started");
                                    tokio::spawn(async move {
                                        while let Some(event) = rx.recv().await {
                                            let path = match &event {
                                                moltis_memory::watcher::WatchEvent::Created(p)
                                                | moltis_memory::watcher::WatchEvent::Modified(p) => {
                                                    Some(p.clone())
                                                },
                                                moltis_memory::watcher::WatchEvent::Removed(p) => {
                                                    // For removed files, trigger a full sync
                                                    if let Err(e) = watcher_manager.sync().await {
                                                        tracing::warn!(
                                                            path = %p.display(),
                                                            error = %e,
                                                            "memory: watcher sync (removal) failed"
                                                        );
                                                    }
                                                    None
                                                },
                                            };
                                            if let Some(path) = path
                                                && let Err(e) =
                                                    watcher_manager.sync_path(&path).await
                                            {
                                                tracing::warn!(
                                                    path = %path.display(),
                                                    error = %e,
                                                    "memory: watcher sync_path failed"
                                                );
                                            }
                                        }
                                    });
                                },
                                Err(e) => {
                                    tracing::warn!("memory: failed to start file watcher: {e}");
                                },
                            }
                        }

                        // Periodic full sync as safety net (longer interval with watcher).
                        #[cfg(feature = "file-watcher")]
                        let interval_secs = 900; // 15 minutes
                        #[cfg(not(feature = "file-watcher"))]
                        let interval_secs = 300; // 5 minutes

                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                        interval.tick().await; // skip first immediate tick
                        loop {
                            interval.tick().await;
                            if let Err(e) = sync_manager.sync().await {
                                tracing::warn!("memory: periodic sync failed: {e}");
                            }
                        }
                    });

                    info!(
                        embeddings = manager.has_embeddings(),
                        "memory system initialized"
                    );
                    Some(manager)
                }
            },
            Err(e) => {
                tracing::warn!("memory: failed to open memory.db: {e}");
                None
            },
        }
    };

    let is_localhost = matches!(bind, "127.0.0.1" | "::1" | "localhost");
    #[cfg(feature = "tls")]
    let tls_active_for_state = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active_for_state = false;

    // Initialize metrics system.
    #[cfg(feature = "metrics")]
    let metrics_handle = {
        let metrics_config = moltis_metrics::MetricsRecorderConfig {
            enabled: config.metrics.enabled,
            prefix: None,
            global_labels: vec![
                ("service".to_string(), "moltis-gateway".to_string()),
                ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
            ],
        };
        match moltis_metrics::init_metrics(metrics_config) {
            Ok(handle) => {
                if config.metrics.enabled {
                    info!("Metrics collection enabled");
                }
                Some(handle)
            },
            Err(e) => {
                warn!("Failed to initialize metrics: {e}");
                None
            },
        }
    };

    // Initialize metrics store for persistence.
    #[cfg(feature = "metrics")]
    let metrics_store: Option<Arc<dyn crate::state::MetricsStore>> = {
        let metrics_db_path = data_dir.join("metrics.db");
        match moltis_metrics::SqliteMetricsStore::new(&metrics_db_path).await {
            Ok(store) => {
                info!(
                    "Metrics history store initialized at {}",
                    metrics_db_path.display()
                );
                Some(Arc::new(store))
            },
            Err(e) => {
                warn!("Failed to initialize metrics store: {e}");
                None
            },
        }
    };

    let state = GatewayState::with_options(
        resolved_auth,
        services,
        Arc::clone(&approval_manager),
        Some(Arc::clone(&sandbox_router)),
        Some(Arc::clone(&credential_store)),
        webauthn_state,
        is_localhost,
        tls_active_for_state,
        hook_registry.clone(),
        memory_manager.clone(),
        port,
        #[cfg(feature = "metrics")]
        metrics_handle,
        #[cfg(feature = "metrics")]
        metrics_store.clone(),
    );

    // Generate a one-time setup code if setup is pending and auth is not disabled.
    let setup_code_display =
        if !credential_store.is_setup_complete() && !credential_store.is_auth_disabled() {
            let code = crate::auth_routes::generate_setup_code();
            *state.setup_code.write().await = Some(secrecy::Secret::new(code.clone()));
            Some(code)
        } else {
            None
        };

    // ── Tailscale Serve/Funnel ─────────────────────────────────────────
    #[cfg(feature = "tailscale")]
    let tailscale_mode: TailscaleMode = {
        // CLI flag overrides config file.
        let mode_str = tailscale_opts
            .as_ref()
            .map(|o| o.mode.clone())
            .unwrap_or_else(|| config.tailscale.mode.clone());
        mode_str.parse().unwrap_or(TailscaleMode::Off)
    };
    #[cfg(feature = "tailscale")]
    let tailscale_reset_on_exit = tailscale_opts
        .as_ref()
        .map(|o| o.reset_on_exit)
        .unwrap_or(config.tailscale.reset_on_exit);

    #[cfg(feature = "tailscale")]
    if tailscale_mode != TailscaleMode::Off {
        validate_tailscale_config(tailscale_mode, bind, credential_store.is_setup_complete())?;
    }

    // Populate the deferred reference so cron callbacks can reach the gateway.
    let _ = deferred_state.set(Arc::clone(&state));

    // Set the state on local-llm service for broadcasting download progress.
    #[cfg(feature = "local-llm")]
    if let Some(svc) = &local_llm_service {
        svc.set_state(Arc::clone(&state));
    }

    // Store heartbeat config on state for gon data and RPC methods.
    *state.heartbeat_config.write().await = config.heartbeat.clone();

    // Wire live chat service (needs state reference, so done after state creation).
    if !registry.read().await.is_empty() {
        let broadcaster = Arc::new(GatewayApprovalBroadcaster::new(Arc::clone(&state)));
        let env_provider: Arc<dyn EnvVarProvider> = credential_store.clone();
        let exec_tool = moltis_tools::exec::ExecTool::default()
            .with_approval(Arc::clone(&approval_manager), broadcaster)
            .with_sandbox_router(Arc::clone(&sandbox_router))
            .with_env_provider(env_provider);

        let cron_tool = moltis_tools::cron_tool::CronTool::new(Arc::clone(&cron_service));

        let mut tool_registry = moltis_agents::tool_registry::ToolRegistry::new();
        tool_registry.register(Box::new(exec_tool));
        tool_registry.register(Box::new(cron_tool));
        if let Some(t) =
            moltis_tools::web_search::WebSearchTool::from_config(&config.tools.web.search)
        {
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::web_fetch::WebFetchTool::from_config(&config.tools.web.fetch)
        {
            tool_registry.register(Box::new(t));
        }

        // Register memory tools if the memory system is available.
        if let Some(ref mm) = memory_manager {
            tool_registry.register(Box::new(moltis_memory::tools::MemorySearchTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryGetTool::new(
                Arc::clone(mm),
            )));
        }

        // Register spawn_agent tool for sub-agent support.
        // The tool gets a snapshot of the current registry (without itself)
        // so sub-agents have access to all other tools.
        if let Some(default_provider) = registry.read().await.first_with_tools() {
            let base_tools = Arc::new(tool_registry.clone_without(&[]));
            let state_for_spawn = Arc::clone(&state);
            let on_spawn_event: moltis_tools::spawn_agent::OnSpawnEvent = Arc::new(move |event| {
                use moltis_agents::runner::RunnerEvent;
                let state = Arc::clone(&state_for_spawn);
                let payload = match &event {
                    RunnerEvent::SubAgentStart { task, model, depth } => {
                        serde_json::json!({
                            "state": "sub_agent_start",
                            "task": task,
                            "model": model,
                            "depth": depth,
                        })
                    },
                    RunnerEvent::SubAgentEnd {
                        task,
                        model,
                        depth,
                        iterations,
                        tool_calls_made,
                    } => serde_json::json!({
                        "state": "sub_agent_end",
                        "task": task,
                        "model": model,
                        "depth": depth,
                        "iterations": iterations,
                        "toolCallsMade": tool_calls_made,
                    }),
                    _ => return, // Only broadcast sub-agent lifecycle events.
                };
                tokio::spawn(async move {
                    broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
                });
            });
            let spawn_tool = moltis_tools::spawn_agent::SpawnAgentTool::new(
                Arc::clone(&registry),
                default_provider,
                base_tools,
            )
            .with_on_event(on_spawn_event);
            tool_registry.register(Box::new(spawn_tool));
        }

        let shared_tool_registry = Arc::new(tokio::sync::RwLock::new(tool_registry));
        let mut chat_service = LiveChatService::new(
            Arc::clone(&registry),
            Arc::clone(&state),
            Arc::clone(&session_store),
            Arc::clone(&session_metadata),
        )
        .with_tools(Arc::clone(&shared_tool_registry))
        .with_failover(config.failover.clone());

        if let Some(ref hooks) = state.hook_registry {
            chat_service = chat_service.with_hooks_arc(Arc::clone(hooks));
        }

        let live_chat = Arc::new(chat_service);
        state.set_chat(live_chat).await;

        // Store registry in the MCP service so runtime mutations auto-sync,
        // and do an initial sync for any servers that already started.
        live_mcp
            .set_tool_registry(Arc::clone(&shared_tool_registry))
            .await;
        crate::mcp_service::sync_mcp_tools(live_mcp.manager(), &shared_tool_registry).await;
    }

    // Spawn MCP health polling + auto-restart background task.
    {
        let health_state = Arc::clone(&state);
        let health_mcp = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            crate::mcp_health::run_health_monitor(health_state, health_mcp).await;
        });
    }

    let methods = Arc::new(MethodRegistry::new());

    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    let mut app = build_gateway_app(Arc::clone(&state), Arc::clone(&methods));

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    #[cfg(feature = "tls")]
    let tls_active = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active = false;

    #[cfg(feature = "tls")]
    let mut ca_cert_path: Option<PathBuf> = None;
    #[cfg(feature = "tls")]
    let mut rustls_config: Option<rustls::ServerConfig> = None;

    #[cfg(feature = "tls")]
    if tls_active {
        let tls_config = &config.tls;
        let (ca_path, cert_path, key_path) = if let (Some(cert_str), Some(key_str)) =
            (&tls_config.cert_path, &tls_config.key_path)
        {
            // User-provided certs.
            let cert = PathBuf::from(cert_str);
            let key = PathBuf::from(key_str);
            let ca = tls_config.ca_cert_path.as_ref().map(PathBuf::from);
            (ca, cert, key)
        } else if tls_config.auto_generate {
            // Auto-generate certificates.
            let mgr = crate::tls::FsCertManager::new()?;
            let (ca, cert, key) = mgr.ensure_certs()?;
            (Some(ca), cert, key)
        } else {
            anyhow::bail!(
                "TLS is enabled but no certificates configured and auto_generate is false"
            );
        };

        ca_cert_path = ca_path.clone();

        let mgr = crate::tls::FsCertManager::new()?;
        rustls_config = Some(mgr.build_rustls_config(&cert_path, &key_path)?);

        // Add /certs/ca.pem route to the main HTTPS app if we have a CA cert.
        if let Some(ref ca) = ca_path {
            let ca_bytes = Arc::new(std::fs::read(ca)?);
            let ca_clone = Arc::clone(&ca_bytes);
            app = app.route(
                "/certs/ca.pem",
                get(move || {
                    let data = Arc::clone(&ca_clone);
                    async move {
                        (
                            [
                                ("content-type", "application/x-pem-file"),
                                (
                                    "content-disposition",
                                    "attachment; filename=\"moltis-ca.pem\"",
                                ),
                            ],
                            data.as_ref().clone(),
                        )
                    }
                }),
            );
        }
    }

    // Count enabled skills and repos for startup banner.
    let (skill_count, repo_count) = {
        use moltis_skills::discover::{FsSkillDiscoverer, SkillDiscoverer};
        let cwd = std::env::current_dir().unwrap_or_default();
        let discoverer = FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths(&cwd));
        let sc = discoverer.discover().await.map(|s| s.len()).unwrap_or(0);
        let rc = moltis_skills::manifest::ManifestStore::default_path()
            .ok()
            .map(|p| {
                let store = moltis_skills::manifest::ManifestStore::new(p);
                store.load().map(|m| m.repos.len()).unwrap_or(0)
            })
            .unwrap_or(0);
        (sc, rc)
    };

    // Startup banner.
    let scheme = if tls_active {
        "https"
    } else {
        "http"
    };
    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    let mut lines = vec![
        format!("moltis gateway v{}", state.version),
        format!(
            "protocol v{}, listening on {}://{} ({})",
            moltis_protocol::PROTOCOL_VERSION,
            scheme,
            addr,
            if tls_active {
                "HTTP/2 + HTTP/1.1"
            } else {
                "HTTP/1.1"
            },
        ),
        format!("{} methods registered", methods.method_names().len()),
        format!("llm: {}", provider_summary),
        format!(
            "skills: {} enabled, {} repo{}",
            skill_count,
            repo_count,
            if repo_count == 1 {
                ""
            } else {
                "s"
            }
        ),
        format!(
            "mcp: {} configured{}",
            mcp_configured_count,
            if mcp_configured_count > 0 {
                " (starting in background)"
            } else {
                ""
            }
        ),
        format!("sandbox: {} backend", sandbox_router.backend_name()),
        format!(
            "config: {}",
            moltis_config::find_or_default_config_path().display()
        ),
        format!("data: {}", data_dir.display()),
    ];
    // Hint about Apple Container on macOS when using Docker.
    #[cfg(target_os = "macos")]
    if sandbox_router.backend_name() == "docker" {
        lines.push(
            "hint: install Apple Container for VM-isolated sandboxing (see docs/sandbox.md)".into(),
        );
    }
    // Warn when no sandbox backend is available.
    if sandbox_router.backend_name() == "none" {
        lines.push("⚠ no container runtime found; commands run on host".into());
    }
    // Display setup code if one was generated.
    if let Some(ref code) = setup_code_display {
        lines.push(format!(
            "setup code: {code} (enter this in the browser to set your password)"
        ));
    }
    #[cfg(feature = "tls")]
    if tls_active {
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(18790);
            lines.push(format!(
                "CA cert: http://{}:{}/certs/ca.pem",
                bind, http_port
            ));
            lines.push(format!("  or: {}", ca.display()));
        }
        lines.push("run `moltis trust-ca` to remove browser warnings".into());
    }
    // Tailscale: enable serve/funnel and show in banner.
    #[cfg(feature = "tailscale")]
    {
        if tailscale_mode != TailscaleMode::Off {
            let manager = CliTailscaleManager::new();
            let ts_result = match tailscale_mode {
                TailscaleMode::Serve => manager.enable_serve(port, tls_active).await,
                TailscaleMode::Funnel => manager.enable_funnel(port, tls_active).await,
                TailscaleMode::Off => unreachable!(),
            };
            match ts_result {
                Ok(()) => {
                    if let Ok(Some(hostname)) = manager.hostname().await {
                        lines.push(format!("tailscale {tailscale_mode}: https://{hostname}"));
                    } else {
                        lines.push(format!("tailscale {tailscale_mode}: enabled"));
                    }
                },
                Err(e) => {
                    warn!("failed to enable tailscale {tailscale_mode}: {e}");
                    lines.push(format!("tailscale {tailscale_mode}: FAILED ({e})"));
                },
            }
        }
    }
    let width = lines.iter().map(|l| l.len()).max().unwrap_or(0) + 4;
    info!("┌{}┐", "─".repeat(width));
    for line in &lines {
        info!("│  {:<w$}│", line, w = width - 2);
    }
    info!("└{}┘", "─".repeat(width));

    // Dispatch GatewayStart hook.
    if let Some(ref hooks) = state.hook_registry {
        let payload = moltis_common::hooks::HookPayload::GatewayStart {
            address: addr.to_string(),
        };
        if let Err(e) = hooks.dispatch(&payload).await {
            tracing::warn!("GatewayStart hook dispatch failed: {e}");
        }
    }

    // Register tailscale shutdown hook (reset serve/funnel on exit).
    #[cfg(feature = "tailscale")]
    if tailscale_mode != TailscaleMode::Off && tailscale_reset_on_exit {
        let ts_mode = tailscale_mode;
        tokio::spawn(async move {
            // Wait for ctrl-c or shutdown signal.
            tokio::signal::ctrl_c().await.ok();
            info!("shutting down tailscale {ts_mode}");
            let manager = CliTailscaleManager::new();
            if let Err(e) = manager.disable().await {
                warn!("failed to reset tailscale on exit: {e}");
            }
            std::process::exit(0);
        });
    }

    // Spawn tick timer.
    let tick_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(TICK_INTERVAL_MS));
        loop {
            interval.tick().await;
            broadcast_tick(&tick_state).await;
        }
    });

    // Spawn metrics history collection and broadcast task (every 10 seconds).
    #[cfg(feature = "metrics")]
    {
        let metrics_state = Arc::clone(&state);
        tokio::spawn(async move {
            // Load history from persistent store on startup.
            if let Some(ref store) = metrics_state.metrics_store {
                // Load last 7 days of history (max points for charts).
                let seven_days_ago = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    - (7 * 24 * 60 * 60 * 1000);
                match store.load_history(seven_days_ago, 60480).await {
                    Ok(points) => {
                        let mut history = metrics_state.metrics_history.write().await;
                        for point in points {
                            history.push(point);
                        }
                        info!(
                            "Loaded {} historical metrics points from store",
                            history.iter().count()
                        );
                    },
                    Err(e) => {
                        warn!("Failed to load metrics history: {e}");
                    },
                }
            }

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            let mut cleanup_counter = 0u32;
            loop {
                interval.tick().await;
                if let Some(ref handle) = metrics_state.metrics_handle {
                    let prometheus_text = handle.render();
                    let snapshot =
                        moltis_metrics::MetricsSnapshot::from_prometheus_text(&prometheus_text);
                    // Convert per-provider metrics to history format.
                    let by_provider = snapshot
                        .categories
                        .llm
                        .by_provider
                        .iter()
                        .map(|(name, metrics)| {
                            (name.clone(), moltis_metrics::ProviderTokens {
                                input_tokens: metrics.input_tokens,
                                output_tokens: metrics.output_tokens,
                                completions: metrics.completions,
                                errors: metrics.errors,
                            })
                        })
                        .collect();

                    let point = crate::state::MetricsHistoryPoint {
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        llm_completions: snapshot.categories.llm.completions_total,
                        llm_input_tokens: snapshot.categories.llm.input_tokens,
                        llm_output_tokens: snapshot.categories.llm.output_tokens,
                        llm_errors: snapshot.categories.llm.errors,
                        by_provider,
                        http_requests: snapshot.categories.http.total,
                        http_active: snapshot.categories.http.active,
                        ws_connections: snapshot.categories.websocket.total,
                        ws_active: snapshot.categories.websocket.active,
                        tool_executions: snapshot.categories.tools.total,
                        tool_errors: snapshot.categories.tools.errors,
                        mcp_calls: snapshot.categories.mcp.total,
                        active_sessions: snapshot.categories.system.active_sessions,
                    };

                    // Push to in-memory history.
                    metrics_state
                        .metrics_history
                        .write()
                        .await
                        .push(point.clone());

                    // Persist to store if available.
                    if let Some(ref store) = metrics_state.metrics_store
                        && let Err(e) = store.save_point(&point).await
                    {
                        warn!("Failed to persist metrics point: {e}");
                    }

                    // Broadcast metrics update to all connected clients.
                    let payload = crate::state::MetricsUpdatePayload { snapshot, point };
                    if let Ok(payload_json) = serde_json::to_value(&payload) {
                        crate::broadcast::broadcast(
                            &metrics_state,
                            "metrics.update",
                            payload_json,
                            crate::broadcast::BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }

                    // Cleanup old data once per hour (360 ticks at 10s interval).
                    cleanup_counter += 1;
                    if cleanup_counter >= 360 {
                        cleanup_counter = 0;
                        if let Some(ref store) = metrics_state.metrics_store {
                            // Keep 7 days of history.
                            let cutoff = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64
                                - (7 * 24 * 60 * 60 * 1000);
                            match store.cleanup_before(cutoff).await {
                                Ok(deleted) if deleted > 0 => {
                                    info!("Cleaned up {} old metrics points", deleted);
                                },
                                Err(e) => {
                                    warn!("Failed to cleanup old metrics: {e}");
                                },
                                _ => {},
                            }
                        }
                    }
                }
            }
        });
    }

    // Spawn sandbox event broadcast task: forwards provision events to WS clients.
    {
        let event_state = Arc::clone(&state);
        let mut event_rx = sandbox_router.subscribe_events();
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let (event_name, payload) = match event {
                            moltis_tools::sandbox::SandboxEvent::Provisioning {
                                container,
                                packages,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "start",
                                    "container": container,
                                    "packages": packages,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Provisioned { container } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "container": container,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::ProvisionFailed {
                                container,
                                error,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "container": container,
                                    "error": error,
                                }),
                            ),
                        };
                        crate::broadcast::broadcast(
                            &event_state,
                            event_name,
                            payload,
                            crate::broadcast::BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn log broadcast task: forwards captured tracing events to WS clients.
    if let Some(buf) = log_buffer {
        let log_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut rx = buf.subscribe();
            loop {
                match rx.recv().await {
                    Ok(entry) => {
                        if let Ok(payload) = serde_json::to_value(&entry) {
                            crate::broadcast::broadcast(
                                &log_state,
                                "logs.entry",
                                payload,
                                crate::broadcast::BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Start the cron scheduler (loads persisted jobs, arms the timer).
    if let Err(e) = cron_service.start().await {
        tracing::warn!("failed to start cron scheduler: {e}");
    }

    // Upsert the built-in heartbeat job from config.
    // Use a fixed ID so run history persists across restarts.
    {
        use moltis_cron::{
            heartbeat::{DEFAULT_INTERVAL_MS, parse_interval_ms, resolve_heartbeat_prompt},
            types::{CronJobCreate, CronJobPatch, CronPayload, CronSchedule, SessionTarget},
        };
        const HEARTBEAT_JOB_ID: &str = "__heartbeat__";

        let hb = &config.heartbeat;
        let interval_ms = parse_interval_ms(&hb.every).unwrap_or(DEFAULT_INTERVAL_MS);
        let prompt = resolve_heartbeat_prompt(hb.prompt.as_deref());

        // Check if heartbeat job already exists.
        let existing = cron_service.list().await;
        let existing_job = existing.iter().find(|j| j.id == HEARTBEAT_JOB_ID);

        if hb.enabled {
            if existing_job.is_some() {
                // Update existing job to match config.
                let patch = CronJobPatch {
                    schedule: Some(CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    }),
                    payload: Some(CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: false,
                        channel: None,
                        to: None,
                    }),
                    enabled: Some(true),
                    sandbox: Some(moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                    }),
                    ..Default::default()
                };
                match cron_service.update(HEARTBEAT_JOB_ID, patch).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job updated"),
                    Err(e) => tracing::warn!("failed to update heartbeat job: {e}"),
                }
            } else {
                // Create new job with fixed ID.
                let create = CronJobCreate {
                    id: Some(HEARTBEAT_JOB_ID.into()),
                    name: "__heartbeat__".into(),
                    schedule: CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    },
                    payload: CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: false,
                        channel: None,
                        to: None,
                    },
                    session_target: SessionTarget::Named("heartbeat".into()),
                    delete_after_run: false,
                    enabled: true,
                    system: true,
                    sandbox: moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                    },
                };
                match cron_service.add(create).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job created"),
                    Err(e) => tracing::warn!("failed to create heartbeat job: {e}"),
                }
            }
        } else if existing_job.is_some() {
            // Heartbeat is disabled, remove the job.
            let _ = cron_service.remove(HEARTBEAT_JOB_ID).await;
            tracing::info!("heartbeat job removed (disabled)");
        }
    }

    #[cfg(feature = "tls")]
    if tls_active {
        // Spawn HTTP redirect server on secondary port.
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(18790);
            let bind_clone = bind.to_string();
            let ca_clone = ca.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    crate::tls::start_http_redirect_server(&bind_clone, http_port, port, &ca_clone)
                        .await
                {
                    tracing::error!("HTTP redirect server failed: {e}");
                }
            });
        }

        // Run HTTPS server.
        let tls_cfg = rustls_config.expect("rustls config must be set when TLS is active");
        let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_cfg));
        axum_server::bind_rustls(addr, rustls_cfg)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
        return Ok(());
    }

    // Plain HTTP server (existing behavior, or TLS feature disabled).
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let count = state.gateway.client_count().await;
    Json(serde_json::json!({
        "status": "ok",
        "version": state.gateway.version,
        "protocol": moltis_protocol::PROTOCOL_VERSION,
        "connections": count,
    }))
}

async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // ── CSWSH protection ────────────────────────────────────────────────
    // Reject cross-origin WebSocket upgrades.  Browsers always send an
    // Origin header on cross-origin requests; non-browser clients (CLI,
    // SDKs) typically omit it — those are allowed through.
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !is_same_origin(origin, host) {
            tracing::warn!(origin, host, remote = %addr, "rejected cross-origin WebSocket upgrade");
            return (
                axum::http::StatusCode::FORBIDDEN,
                "cross-origin WebSocket connections are not allowed",
            )
                .into_response();
        }
    }

    let accept_language = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    ws.on_upgrade(move |socket| {
        handle_connection(socket, state.gateway, state.methods, addr, accept_language)
    })
    .into_response()
}

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
fn is_same_origin(origin: &str, host: &str) -> bool {
    // Origin is a full URL (e.g. "https://localhost:8080"), Host is just
    // "host:port" or "host".
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    fn strip_port(h: &str) -> &str {
        if h.starts_with('[') {
            // IPv6: [::1]:port
            h.rsplit_once("]:")
                .map_or(h, |(addr, _)| addr)
                .trim_start_matches('[')
                .trim_end_matches(']')
        } else {
            h.rsplit_once(':').map_or(h, |(addr, _)| addr)
        }
    }
    fn get_port(h: &str) -> Option<&str> {
        if h.starts_with('[') {
            h.rsplit_once("]:").map(|(_, p)| p)
        } else {
            h.rsplit_once(':').map(|(_, p)| p)
        }
    }

    let origin_port = get_port(origin_host);
    let host_port = get_port(host);

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    let is_loopback = |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}

/// SPA fallback: serve `index.html` for any path not matched by an explicit
/// route (assets, ws, health). This lets client-side routing handle `/crons`,
/// `/logs`, etc.
///
/// Injects a `<script>` tag with pre-fetched bootstrap data (channels,
/// sessions, models, projects) so the UI can render synchronously without
/// waiting for the WebSocket handshake — similar to the gon pattern in Rails.
/// Server-side data injected into every page as `window.__MOLTIS__`
/// (gon pattern — see CLAUDE.md § Server-Injected Data).
///
/// Add new fields here when the frontend needs data at page load
/// without an async fetch. Fields must not depend on the request
/// (no cookies, no session — use `/api/auth/status` for that).
#[cfg(feature = "web-ui")]
#[derive(serde::Serialize)]
struct GonData {
    identity: moltis_config::ResolvedIdentity,
    port: u16,
    counts: NavCounts,
    crons: Vec<moltis_cron::types::CronJob>,
    cron_status: moltis_cron::types::CronStatus,
    heartbeat_config: moltis_config::schema::HeartbeatConfig,
    heartbeat_runs: Vec<moltis_cron::types::CronRunRecord>,
}

/// Counts shown as badges in the sidebar navigation.
#[cfg(feature = "web-ui")]
#[derive(Debug, Default, serde::Serialize)]
struct NavCounts {
    projects: usize,
    providers: usize,
    channels: usize,
    plugins: usize,
    skills: usize,
    mcp: usize,
    crons: usize,
    images: usize,
}

#[cfg(feature = "web-ui")]
async fn build_gon_data(gw: &GatewayState) -> GonData {
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
    let heartbeat_config = gw.heartbeat_config.read().await.clone();

    // Get heartbeat runs using the fixed heartbeat job ID.
    // This preserves run history across restarts.
    let heartbeat_runs: Vec<moltis_cron::types::CronRunRecord> = gw
        .services
        .cron
        .runs(serde_json::json!({ "id": "__heartbeat__", "limit": 10 }))
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    GonData {
        identity,
        port,
        counts,
        crons,
        cron_status,
        heartbeat_config,
        heartbeat_runs,
    }
}

#[cfg(feature = "web-ui")]
async fn build_nav_counts(gw: &GatewayState) -> NavCounts {
    let (projects, models, channels, mcp, crons, images) = tokio::join!(
        gw.services.project.list(),
        gw.services.model.list(),
        gw.services.channel.status(),
        gw.services.mcp.list(),
        gw.services.cron.list(),
        async {
            let builder = moltis_tools::image_cache::DockerImageBuilder::new();
            builder.list_cached().await.ok()
        },
    );

    let projects = projects
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0);

    let providers = models
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                let mut names: std::collections::HashSet<&str> = std::collections::HashSet::new();
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

    // Count enabled skills from skills manifest only.
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

    // Count enabled plugins from plugins manifest.
    let mut plugins = 0usize;
    if let Ok(path) = moltis_plugins::install::default_manifest_path() {
        let store = moltis_skills::manifest::ManifestStore::new(path);
        if let Ok(m) = store.load() {
            plugins = m
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

    // Count enabled user cron jobs (exclude system jobs like heartbeat).
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

    let images = images.map(|imgs| imgs.len()).unwrap_or(0);

    NavCounts {
        projects,
        providers,
        channels,
        plugins,
        skills,
        mcp,
        crons,
        images,
    }
}

#[cfg(feature = "web-ui")]
async fn api_gon_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(build_gon_data(&state.gateway).await)
}

#[cfg(feature = "web-ui")]
async fn spa_fallback(State(state): State<AppState>, uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path();
    if path.starts_with("/assets/") || path.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let raw = read_asset("index.html")
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();

    // Build server-side data blob (gon pattern) injected into <head>.
    let gon = build_gon_data(&state.gateway).await;
    let gon_script = format!(
        "<script>window.__MOLTIS__={};</script>",
        serde_json::to_string(&gon).unwrap_or_else(|_| "{}".into()),
    );

    let body = if is_dev_assets() {
        // Dev: no versioned URLs, just serve directly with no-cache
        raw.replace("__BUILD_TS__", "dev")
    } else {
        // Production: inject content-hash versioned URLs for immutable caching
        static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(asset_content_hash);
        let versioned = format!("/assets/v/{}/", *HASH);
        raw.replace("__BUILD_TS__", &HASH)
            .replace("/assets/", &versioned)
    };

    // Inject gon data into <head> so it's available before any module scripts run.
    // An inline <script> in the <body> (right after the title elements) reads
    // window.__MOLTIS__.identity to set emoji/name before the first paint.
    let body = body.replace("</head>", &format!("{gon_script}\n</head>"));

    ([("cache-control", "no-cache, no-store")], Html(body)).into_response()
}

#[cfg(feature = "web-ui")]
async fn api_bootstrap_handler(State(state): State<AppState>) -> impl IntoResponse {
    let gw = &state.gateway;
    let (channels, sessions, models, projects, wizard_status) = tokio::join!(
        gw.services.channel.status(),
        gw.services.session.list(),
        gw.services.model.list(),
        gw.services.project.list(),
        gw.services.onboarding.wizard_status(),
    );
    let onboarded = wizard_status
        .as_ref()
        .ok()
        .and_then(|v| v.get("onboarded"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
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

/// MCP servers list for the UI (HTTP endpoint for initial page load).
#[cfg(feature = "web-ui")]
async fn api_mcp_handler(State(state): State<AppState>) -> impl IntoResponse {
    let servers = state.gateway.services.mcp.list().await;
    match servers {
        Ok(val) => axum::Json(val).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Lightweight skills overview: repo summaries + enabled skills only.
/// Full skill lists are loaded on-demand via /api/skills/search.
/// Returns enabled skills from the skills manifest and skill repos.
#[cfg(feature = "web-ui")]
fn enabled_from_manifest(
    path_result: anyhow::Result<std::path::PathBuf>,
) -> Vec<serde_json::Value> {
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

/// Skills endpoint: repos and enabled skills from the skills manifest only.
#[cfg(feature = "web-ui")]
async fn api_skills_handler(State(state): State<AppState>) -> impl IntoResponse {
    let repos = state
        .gateway
        .services
        .skills
        .repos_list()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let skills = enabled_from_manifest(moltis_skills::manifest::ManifestStore::default_path());

    Json(serde_json::json!({ "skills": skills, "repos": repos }))
}

/// Plugins endpoint: repos and enabled skills from the plugins manifest only.
#[cfg(feature = "web-ui")]
async fn api_plugins_handler(State(state): State<AppState>) -> impl IntoResponse {
    let repos = state
        .gateway
        .services
        .plugins
        .repos_list()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let skills = enabled_from_manifest(moltis_plugins::install::default_manifest_path());

    Json(serde_json::json!({ "skills": skills, "repos": repos }))
}

/// Search skills within a specific repo. Query params: source, q (optional).
#[cfg(feature = "web-ui")]
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

#[cfg(feature = "web-ui")]
async fn api_skills_search_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
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

#[cfg(feature = "web-ui")]
async fn api_plugins_search_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let source = params.get("source").cloned().unwrap_or_default();
    let query = params.get("q").cloned().unwrap_or_default();
    let repos = state
        .gateway
        .services
        .plugins
        .repos_list_full()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    api_search_handler(repos, &source, &query).await
}

/// List cached tool images.
#[cfg(feature = "web-ui")]
async fn api_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    match builder.list_cached().await {
        Ok(images) => Json(serde_json::json!({ "images": images })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Delete a specific cached tool image.
#[cfg(feature = "web-ui")]
async fn api_delete_cached_image_handler(Path(tag): Path<String>) -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    // The tag comes URL-encoded; the path captures "moltis-cache/skill:hash" as a single segment.
    let full_tag = if tag.starts_with("moltis-cache/") {
        tag
    } else {
        format!("moltis-cache/{tag}")
    };
    match builder.remove_cached(&full_tag).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Prune all cached tool images.
#[cfg(feature = "web-ui")]
async fn api_prune_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    match builder.prune_all().await {
        Ok(count) => Json(serde_json::json!({ "pruned": count })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Check which packages already exist in a base image.
///
/// Runs `dpkg -s <pkg>` and `which <pkg>` inside the base image to detect
/// packages that are already installed. Returns a map of package name to
/// boolean (true = already present).
#[cfg(feature = "web-ui")]
async fn api_check_packages_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
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

    // Build a shell command that checks each package via dpkg -s or which.
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Get the current default sandbox image.
#[cfg(feature = "web-ui")]
async fn api_get_default_image_handler(State(state): State<AppState>) -> impl IntoResponse {
    let image = if let Some(ref router) = state.gateway.sandbox_router {
        router.default_image().await
    } else {
        moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string()
    };
    Json(serde_json::json!({ "image": image }))
}

/// Set the default sandbox image.
#[cfg(feature = "web-ui")]
async fn api_set_default_image_handler(
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
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "no sandbox backend available" })),
        )
            .into_response()
    }
}

/// Build a custom image from a base + apt packages.
#[cfg(feature = "web-ui")]
async fn api_build_image_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
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
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name is required" })),
        )
            .into_response();
    }
    if packages.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "packages list is empty" })),
        )
            .into_response();
    }

    // Validate name: only allow alphanumeric, dash, underscore
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name must be alphanumeric, dash, or underscore" })),
        )
            .into_response();
    }

    let pkg_list = packages.join(" ");
    let dockerfile_contents = format!(
        "FROM {base}\nRUN apt-get update && apt-get install -y {pkg_list} && rm -rf /var/lib/apt/lists/*\n"
    );

    let tmp_dir = std::env::temp_dir().join(format!("moltis-build-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let dockerfile_path = tmp_dir.join("Dockerfile");
    if let Err(e) = std::fs::write(&dockerfile_path, &dockerfile_contents) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    let result = builder.ensure_image(name, &dockerfile_path, &tmp_dir).await;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    match result {
        Ok(tag) => Json(serde_json::json!({ "tag": tag })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(feature = "web-ui")]
static ASSETS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// ── Asset serving: filesystem (dev) or embedded (release) ───────────────────

/// Filesystem path to serve assets from, if available. Checked once at startup.
/// Set via `MOLTIS_ASSETS_DIR` env var, or auto-detected from the crate source
/// tree when running via `cargo run`.
#[cfg(feature = "web-ui")]
static FS_ASSETS_DIR: std::sync::LazyLock<Option<std::path::PathBuf>> =
    std::sync::LazyLock::new(|| {
        use std::path::PathBuf;

        // Explicit env var takes precedence
        if let Ok(dir) = std::env::var("MOLTIS_ASSETS_DIR") {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                info!("Serving assets from filesystem: {}", p.display());
                return Some(p);
            }
        }

        // Auto-detect: works when running from the repo via `cargo run`
        let cargo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets");
        if cargo_dir.is_dir() {
            info!("Serving assets from filesystem: {}", cargo_dir.display());
            return Some(cargo_dir);
        }

        info!("Serving assets from embedded binary");
        None
    });

/// Whether we're serving from the filesystem (dev mode) or embedded (release).
#[cfg(feature = "web-ui")]
fn is_dev_assets() -> bool {
    FS_ASSETS_DIR.is_some()
}

/// Compute a short content hash of all embedded assets. Only used in release
/// mode (embedded assets) for cache-busting versioned URLs.
#[cfg(feature = "web-ui")]
fn asset_content_hash() -> String {
    use std::{collections::BTreeMap, hash::Hasher};

    let mut files = BTreeMap::new();
    let mut stack: Vec<&include_dir::Dir<'_>> = vec![&ASSETS];
    while let Some(dir) = stack.pop() {
        for file in dir.files() {
            files.insert(file.path().display().to_string(), file.contents());
        }
        for sub in dir.dirs() {
            stack.push(sub);
        }
    }

    let mut h = std::hash::DefaultHasher::new();
    for (path, contents) in &files {
        h.write(path.as_bytes());
        h.write(contents);
    }
    format!("{:016x}", h.finish())
}

#[cfg(feature = "web-ui")]
fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Read an asset file, preferring filesystem over embedded.
#[cfg(feature = "web-ui")]
fn read_asset(path: &str) -> Option<Vec<u8>> {
    if let Some(dir) = FS_ASSETS_DIR.as_ref() {
        let file_path = dir.join(path);
        // Prevent path traversal
        if file_path.starts_with(dir)
            && let Ok(bytes) = std::fs::read(&file_path)
        {
            return Some(bytes);
        }
    }
    ASSETS.get_file(path).map(|f| f.contents().to_vec())
}

/// Versioned assets: `/assets/v/<hash>/path` — immutable, cached forever.
#[cfg(feature = "web-ui")]
async fn versioned_asset_handler(
    Path((_version, path)): Path<(String, String)>,
) -> impl IntoResponse {
    let cache = if is_dev_assets() {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    serve_asset(&path, cache)
}

/// Unversioned assets: `/assets/path` — always no-cache.
#[cfg(feature = "web-ui")]
async fn asset_handler(Path(path): Path<String>) -> impl IntoResponse {
    serve_asset(&path, "no-cache")
}

#[cfg(feature = "web-ui")]
fn serve_asset(path: &str, cache_control: &'static str) -> axum::response::Response {
    match read_asset(path) {
        Some(body) => (
            StatusCode::OK,
            [
                ("content-type", mime_for_path(path)),
                ("cache-control", cache_control),
            ],
            body,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_exact_match() {
        assert!(is_same_origin(
            "https://example.com:8080",
            "example.com:8080"
        ));
        assert!(is_same_origin(
            "http://example.com:3000",
            "example.com:3000"
        ));
    }

    #[test]
    fn same_origin_localhost_variants() {
        // localhost ↔ 127.0.0.1
        assert!(is_same_origin("http://localhost:8080", "127.0.0.1:8080"));
        assert!(is_same_origin("https://127.0.0.1:8080", "localhost:8080"));
        // localhost ↔ ::1
        assert!(is_same_origin("http://localhost:8080", "[::1]:8080"));
        assert!(is_same_origin("http://[::1]:8080", "localhost:8080"));
        // 127.0.0.1 ↔ ::1
        assert!(is_same_origin("http://127.0.0.1:8080", "[::1]:8080"));
    }

    #[test]
    fn cross_origin_rejected() {
        // Different host
        assert!(!is_same_origin("https://attacker.com", "localhost:8080"));
        assert!(!is_same_origin("https://evil.com:8080", "localhost:8080"));
        // Different port
        assert!(!is_same_origin("http://localhost:9999", "localhost:8080"));
    }

    #[test]
    fn same_origin_no_port() {
        assert!(is_same_origin("https://example.com", "example.com"));
        assert!(is_same_origin("http://localhost", "localhost"));
        assert!(is_same_origin("http://localhost", "127.0.0.1"));
    }

    #[test]
    fn cross_origin_port_mismatch() {
        // One has port, other doesn't — different origins.
        assert!(!is_same_origin("http://localhost:8080", "localhost"));
        assert!(!is_same_origin("http://localhost", "localhost:8080"));
    }
}
