use {
    super::{
        helpers::{
            StartupMemProbe, approval_manager_from_config, env_flag_enabled,
            log_startup_config_storage_diagnostics, log_startup_model_inventory,
            maybe_deliver_cron_output, restore_saved_local_llm_models,
            validate_proxy_tls_configuration,
        },
        init_channels, init_memory,
        prepared::PreparedGatewayCore,
        workspace::{
            seed_default_workspace_markdown_files, sync_persona_into_preset,
            warn_on_workspace_prompt_file_truncation,
        },
    },
    crate::{
        approval::LiveExecApprovalService,
        auth,
        broadcast::{BroadcastOpts, broadcast},
        chat::LiveModelService,
        provider_setup::LiveProviderSetupService,
        services::GatewayServices,
        session::LiveSessionService,
        state::GatewayState,
    },
    moltis_projects::ProjectStore,
    moltis_providers::ProviderRegistry,
    moltis_sessions::{
        metadata::{SessionMetadata, SqliteSessionMetadata},
        session_events::SessionEventBus,
        store::SessionStore,
    },
    secrecy::{ExposeSecret, Secret},
    std::{
        path::PathBuf,
        sync::{Arc, atomic::Ordering},
    },
    tracing::{debug, info, warn},
};
mod log_persistence;
mod post_state;
/// Prepare the core gateway: load config, run migrations, wire services,
/// spawn background tasks, and return the core state without any HTTP layer.
/// This is the transport-agnostic initialisation. Non-HTTP consumers (TUI,
/// tests) can stop here. HTTP consumers call [`prepare_gateway`] which
/// delegates to this and then adds the router + middleware.
#[allow(clippy::expect_used)] // Startup fail-fast: DB, migrations, credential store must succeed.
pub async fn prepare_gateway_core(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<crate::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    tailscale_mode_override: Option<String>,
    tailscale_reset_on_exit_override: Option<bool>,
    session_event_bus: Option<SessionEventBus>,
) -> anyhow::Result<PreparedGatewayCore> {
    let session_event_bus = session_event_bus.unwrap_or_default();
    #[cfg(not(feature = "tailscale"))]
    let _ = (&tailscale_mode_override, &tailscale_reset_on_exit_override);

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

    // Cloud deploy platform — hides local-only providers (local-llm, ollama).
    let deploy_platform = std::env::var("MOLTIS_DEPLOY_PLATFORM").ok();
    let resolved_auth = auth::resolve_auth(token, password.clone());

    // Load config file (moltis.toml / .yaml / .json) if present.
    let mut config = moltis_config::discover_and_load();
    info!(
        offered_channels = ?config.channels.offered,
        "loaded offered channels from config"
    );
    let config_env_overrides = config.env.clone();
    let instance_slug_value = super::helpers::instance_slug(&config);
    let browser_container_prefix = super::helpers::browser_container_prefix(&instance_slug_value);
    let sandbox_container_prefix = super::helpers::sandbox_container_prefix(&instance_slug_value);
    let mut startup_mem_probe = StartupMemProbe::new();
    startup_mem_probe.checkpoint("prepare_gateway.start");

    // CLI --no-tls / MOLTIS_NO_TLS overrides config file TLS setting.
    if no_tls {
        config.tls.enabled = false;
    }
    let behind_proxy = env_flag_enabled("MOLTIS_BEHIND_PROXY");
    let allow_tls_behind_proxy = env_flag_enabled("MOLTIS_ALLOW_TLS_BEHIND_PROXY");
    #[cfg(feature = "tls")]
    let tls_enabled_for_gateway = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_enabled_for_gateway = false;
    validate_proxy_tls_configuration(
        behind_proxy,
        tls_enabled_for_gateway,
        allow_tls_behind_proxy,
    )?;
    if behind_proxy && tls_enabled_for_gateway && allow_tls_behind_proxy {
        warn!(
            "MOLTIS_ALLOW_TLS_BEHIND_PROXY=true is set; ensure your proxy uses HTTPS upstream or TLS passthrough to avoid redirect loops"
        );
    }
    let base_provider_config = config.providers.clone();

    // Merge any previously saved API keys into the provider config so they
    // survive gateway restarts without requiring env vars.
    let key_store = crate::provider_setup::KeyStore::new();
    #[cfg(feature = "local-llm")]
    let local_model_ids: Vec<String> = crate::local_llm_setup::LocalLlmConfig::load()
        .map(|c| c.models.iter().map(|m| m.model_id.clone()).collect())
        .unwrap_or_default();
    #[cfg(not(feature = "local-llm"))]
    let local_model_ids: Vec<String> = Vec::new();

    let effective_providers = crate::provider_setup::config_with_saved_keys(
        &base_provider_config,
        &key_store,
        &local_model_ids,
    );

    let has_explicit_provider_settings =
        crate::provider_setup::has_explicit_provider_settings(&config.providers);
    let auto_detected_provider_sources = if has_explicit_provider_settings {
        Vec::new()
    } else {
        crate::provider_setup::detect_auto_provider_sources_with_overrides(
            &config.providers,
            deploy_platform.as_deref(),
            &config_env_overrides,
        )
    };

    // Kick off discovery workers immediately, but build a static startup
    // registry first so gateway startup does not block on network I/O.
    let startup_discovery_pending =
        ProviderRegistry::fire_discoveries(&effective_providers, &config_env_overrides);
    let registry = Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_config_with_static_catalogs(
            &effective_providers,
            &config_env_overrides,
            moltis_providers::extract_cw_overrides(&config.models),
        ),
    ));
    {
        let mut reg = registry.write().await;
        restore_saved_local_llm_models(&mut reg, &effective_providers);
    }
    let (provider_summary, providers_available_at_startup) = {
        let reg = registry.read().await;
        log_startup_model_inventory(&reg);
        (reg.provider_summary(), !reg.is_empty())
    };
    if !providers_available_at_startup {
        let config_path = moltis_config::find_or_default_config_path();
        let provider_keys_path = moltis_config::config_dir()
            .unwrap_or_else(|| PathBuf::from(".moltis"))
            .join("provider_keys.json");
        warn!(
            provider_summary = %provider_summary,
            config_path = %config_path.display(),
            provider_keys_path = %provider_keys_path.display(),
            "no LLM providers in static startup catalog; model/chat services remain active and will pick up providers after credentials are saved or background discovery completes"
        );
    }

    if !has_explicit_provider_settings {
        if auto_detected_provider_sources.is_empty() {
            info!("llm auto-detect: no providers detected from env/files");
        } else {
            for detected in &auto_detected_provider_sources {
                info!(
                    provider = %detected.provider,
                    source = %detected.source,
                    "llm auto-detected provider source"
                );
            }
            let import_token_store = moltis_oauth::TokenStore::new();
            crate::provider_setup::import_detected_oauth_tokens(
                &auto_detected_provider_sources,
                &import_token_store,
            );
        }
    }
    startup_mem_probe.checkpoint("providers.registry.initialized");

    // Refresh dynamic provider model discovery daily.
    const DYNAMIC_PROVIDER_MODEL_REFRESH_INTERVAL: std::time::Duration =
        std::time::Duration::from_secs(24 * 60 * 60);
    {
        let registry_for_refresh = Arc::clone(&registry);
        let provider_config_for_refresh = base_provider_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(DYNAMIC_PROVIDER_MODEL_REFRESH_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                let mut reg = registry_for_refresh.write().await;
                let refresh_results = reg.refresh_dynamic_models(&provider_config_for_refresh);
                for (provider_name, refreshed) in refresh_results {
                    if !refreshed {
                        continue;
                    }
                    let model_count = reg
                        .list_models()
                        .iter()
                        .filter(|m| m.provider == provider_name)
                        .count();
                    info!(
                        provider = %provider_name,
                        models = model_count,
                        "daily dynamic provider model refresh complete"
                    );
                }
            }
        });
    }

    // Create shared approval manager from config.
    let approval_manager = Arc::new(approval_manager_from_config(&config));

    let mut services = GatewayServices::noop();

    // Wire live logs service if a log buffer is available.
    if let Some(ref buf) = log_buffer {
        services.logs = Arc::new(crate::logs::LiveLogsService::new(buf.clone()));
    }

    services.exec_approval = Arc::new(LiveExecApprovalService::new(Arc::clone(&approval_manager)));

    // Wire browser service if enabled.
    if let Some(browser_svc) =
        crate::services::RealBrowserService::from_config(&config, browser_container_prefix)
    {
        services.browser = Arc::new(browser_svc);
    }

    // Wire live onboarding service.
    let onboarding_config_path = moltis_config::find_or_default_config_path();
    let live_onboarding =
        moltis_onboarding::service::LiveOnboardingService::new(onboarding_config_path);
    #[cfg(feature = "local-llm")]
    let local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>> = {
        let svc = Arc::new(crate::local_llm_setup::LiveLocalLlmService::new(
            Arc::clone(&registry),
        ));
        services =
            services.with_local_llm(Arc::clone(&svc) as Arc<dyn crate::services::LocalLlmService>);
        Some(svc)
    };

    // Wire live voice services when the feature is enabled.
    #[cfg(feature = "voice")]
    {
        use crate::voice::{LiveSttService, LiveTtsService, SttServiceConfig};
        services.tts = Arc::new(LiveTtsService::new(moltis_voice::TtsConfig::default()));
        services.stt = Arc::new(LiveSttService::new(SttServiceConfig::default()));
    }

    let model_store = Arc::new(tokio::sync::RwLock::new(
        crate::chat::DisabledModelsStore::load(),
    ));

    let live_model_service = Arc::new(
        LiveModelService::new(
            Arc::clone(&registry),
            Arc::clone(&model_store),
            config.chat.priority_models.clone(),
        )
        .with_show_legacy_models(config.providers.show_legacy_models)
        .with_discovery_config(effective_providers.clone(), config_env_overrides.clone()),
    );
    services = services
        .with_model(Arc::clone(&live_model_service) as Arc<dyn crate::services::ModelService>);

    let mut provider_setup = LiveProviderSetupService::new(
        Arc::clone(&registry),
        config.providers.clone(),
        deploy_platform.clone(),
    )
    .with_env_overrides(config_env_overrides.clone())
    .with_global_cw_overrides(moltis_providers::extract_cw_overrides(&config.models))
    .with_error_parser(crate::chat_error::parse_chat_error)
    .with_callback_bind_addr(bind.to_string());
    provider_setup.set_priority_models(live_model_service.priority_models_handle());
    let provider_setup_service = Arc::new(provider_setup);
    services.provider_setup =
        Arc::clone(&provider_setup_service) as Arc<dyn crate::services::ProviderSetupService>;

    // Wire live MCP service.
    let mcp_configured_count;
    let live_mcp: Arc<crate::mcp_service::LiveMcpService>;
    {
        let mcp_registry_path = moltis_config::data_dir().join("mcp-servers.json");
        let mcp_reg = moltis_mcp::McpRegistry::load(&mcp_registry_path).unwrap_or_default();
        let mut merged = mcp_reg;
        for (name, entry) in &config.mcp.servers {
            if !merged.servers.contains_key(name) {
                let transport = match entry.transport.as_str() {
                    "sse" => moltis_mcp::registry::TransportType::Sse,
                    "streamable_http" | "streamable-http" | "http" => {
                        moltis_mcp::registry::TransportType::StreamableHttp
                    },
                    _ => moltis_mcp::registry::TransportType::Stdio,
                };
                let oauth = entry
                    .oauth
                    .as_ref()
                    .map(|o| moltis_mcp::registry::McpOAuthConfig {
                        client_id: o.client_id.clone(),
                        auth_url: o.auth_url.clone(),
                        token_url: o.token_url.clone(),
                        scopes: o.scopes.clone(),
                    });
                merged
                    .servers
                    .insert(name.clone(), moltis_mcp::McpServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env: entry.env.clone(),
                        enabled: entry.enabled,
                        request_timeout_secs: entry.request_timeout_secs,
                        transport,
                        url: entry.url.clone().map(Secret::new),
                        headers: entry
                            .headers
                            .iter()
                            .map(|(key, value)| (key.clone(), Secret::new(value.clone())))
                            .collect(),
                        oauth,
                        display_name: entry.display_name.clone(),
                    });
            }
        }
        mcp_configured_count = merged.servers.values().filter(|s| s.enabled).count();
        let mcp_manager = Arc::new(moltis_mcp::McpManager::new_with_env_overrides(
            merged,
            config_env_overrides.clone(),
            std::time::Duration::from_secs(config.mcp.request_timeout_secs.max(1)),
        ));
        live_mcp = Arc::new(crate::mcp_service::LiveMcpService::new(
            Arc::clone(&mcp_manager),
            config_env_overrides.clone(),
            None,
        ));
        services.mcp = live_mcp.clone() as Arc<dyn crate::services::McpService>;
    }
    startup_mem_probe.checkpoint("services.core_wired");

    // Initialize data directory and SQLite database.
    let data_dir = data_dir.unwrap_or_else(moltis_config::data_dir);
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create data directory {}: {e}",
            data_dir.display()
        )
    });

    let config_dir_resolved =
        moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    std::fs::create_dir_all(&config_dir_resolved).unwrap_or_else(|e| {
        panic!(
            "failed to create config directory {}: {e}",
            config_dir_resolved.display()
        )
    });
    log_startup_config_storage_diagnostics();

    log_persistence::spawn_startup_log_persistence(log_buffer.as_ref(), &data_dir);
    let db_path = data_dir.join("moltis.db");
    let db_pool = {
        use {
            sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
            std::str::FromStr,
        };
        let db_exists = db_path.exists();
        let mut options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
            .expect("invalid database path")
            .create_if_missing(true)
            .foreign_keys(true)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));
        if !db_exists {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }

        let started = std::time::Instant::now();
        let pool = sqlx::pool::PoolOptions::new()
            .max_connections(config.server.db_pool_max_connections)
            .connect_with(options)
            .await
            .expect("failed to open moltis.db");
        debug!(
            path = %db_path.display(),
            db_exists,
            elapsed_ms = started.elapsed().as_millis(),
            "startup sqlite pool connected"
        );
        pool
    };

    // Run database migrations from each crate in dependency order.
    moltis_projects::run_migrations(&db_pool)
        .await
        .expect("failed to run projects migrations");
    moltis_sessions::run_migrations(&db_pool)
        .await
        .expect("failed to run sessions migrations");
    moltis_cron::run_migrations(&db_pool)
        .await
        .expect("failed to run cron migrations");
    moltis_webhooks::run_migrations(&db_pool)
        .await
        .expect("failed to run webhooks migrations");
    crate::run_migrations(&db_pool)
        .await
        .expect("failed to run gateway migrations");

    #[cfg(feature = "vault")]
    moltis_vault::run_migrations(&db_pool)
        .await
        .expect("failed to run vault migrations");

    moltis_skills::migration::migrate_plugins_to_skills(&data_dir).await;
    startup_mem_probe.checkpoint("sqlite.migrations.complete");

    #[cfg(feature = "vault")]
    let vault: Option<Arc<moltis_vault::Vault>> = {
        match moltis_vault::Vault::new(db_pool.clone()).await {
            Ok(v) => {
                info!(status = ?v.status().await, "vault ready");
                Some(Arc::new(v))
            },
            Err(e) => {
                warn!(error = %e, "vault init failed, encryption disabled");
                None
            },
        }
    };

    #[cfg(feature = "vault")]
    let credential_store = Arc::new(
        auth::CredentialStore::with_vault(db_pool.clone(), &config.auth, vault.clone())
            .await
            .expect("failed to init credential store"),
    );
    #[cfg(not(feature = "vault"))]
    let credential_store = Arc::new(
        auth::CredentialStore::new(db_pool.clone())
            .await
            .expect("failed to init credential store"),
    );

    let runtime_env_overrides = match credential_store.get_all_env_values().await {
        Ok(db_env_vars) => {
            crate::mcp_service::merge_env_overrides(&config_env_overrides, db_env_vars)
        },
        Err(error) => {
            warn!(%error, "failed to load persisted env overrides from credential store");
            config_env_overrides.clone()
        },
    };
    live_mcp
        .manager()
        .set_env_overrides(runtime_env_overrides.clone())
        .await;
    *live_model_service.env_overrides_handle().write().await = runtime_env_overrides.clone();
    live_mcp
        .set_credential_store(Arc::clone(&credential_store))
        .await;
    let mgr = Arc::clone(live_mcp.manager());
    let mcp_for_sync = Arc::clone(&live_mcp);
    tokio::spawn(async move {
        let started = mgr.start_enabled().await;
        if !started.is_empty() {
            tracing::info!(servers = ?started, "MCP servers started");
        }
        mcp_for_sync.sync_tools_if_ready().await;
    });

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
    let config_dir_for_migration =
        moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    let projects_toml_path = config_dir_for_migration.join("projects.toml");
    if projects_toml_path.exists() {
        info!("migrating projects.toml to SQLite");
        let old_store = moltis_projects::TomlProjectStore::new(projects_toml_path.clone());
        let sqlite_store = moltis_projects::SqliteProjectStore::new(db_pool.clone());
        if let Ok(projects) =
            <moltis_projects::TomlProjectStore as ProjectStore>::list(&old_store).await
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
    let project_store: Arc<dyn ProjectStore> =
        Arc::new(moltis_projects::SqliteProjectStore::new(db_pool.clone()));
    let session_store = Arc::new(SessionStore::new(sessions_dir));
    let event_bus_for_metadata = session_event_bus.clone();
    let session_metadata = Arc::new(SqliteSessionMetadata::with_event_bus(
        db_pool.clone(),
        event_bus_for_metadata,
    ));
    let session_share_store = Arc::new(crate::share_store::ShareStore::new(db_pool.clone()));
    let session_state_store = Arc::new(moltis_sessions::state_store::SessionStateStore::new(
        db_pool.clone(),
    ));

    let agent_persona_store = Arc::new(crate::agent_persona::AgentPersonaStore::new(
        db_pool.clone(),
    ));
    if let Err(e) = agent_persona_store.ensure_main_workspace_seeded() {
        tracing::warn!(error = %e, "failed to seed main agent workspace");
    }

    let deferred_state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());

    services =
        services.with_onboarding(Arc::new(crate::onboarding::GatewayOnboardingService::new(
            live_onboarding,
            Arc::clone(&session_metadata),
            Arc::clone(&agent_persona_store),
            Arc::clone(&deferred_state),
        )));

    services.project = Arc::new(crate::project::LiveProjectService::new(Arc::clone(
        &project_store,
    )));

    // Initialize cron service.
    let cron_store: Arc<dyn moltis_cron::store::CronStore> =
        match moltis_cron::store_file::FileStore::default_path() {
            Ok(fs) => Arc::new(fs),
            Err(e) => {
                tracing::warn!("cron file store unavailable ({e}), using in-memory");
                Arc::new(moltis_cron::store_memory::InMemoryStore::new())
            },
        };

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

    let events_queue = moltis_cron::system_events::SystemEventsQueue::new();

    let agent_state = Arc::clone(&deferred_state);
    let agent_events_queue = Arc::clone(&events_queue);
    let global_auto_prune_containers = config.cron.auto_prune_cron_containers;
    let on_agent_turn: moltis_cron::service::AgentTurnFn = Arc::new(move |req| {
        let st = Arc::clone(&agent_state);
        let eq = Arc::clone(&agent_events_queue);
        Box::pin(async move {
            let state = st
                .get()
                .ok_or_else(|| moltis_cron::Error::message("gateway not ready"))?;

            let is_heartbeat_turn = matches!(
                &req.session_target,
                moltis_cron::types::SessionTarget::Named(name) if name == "heartbeat"
            );
            let has_pending_events = is_heartbeat_turn && !eq.is_empty().await;
            if is_heartbeat_turn && !has_pending_events {
                let hb_cfg = state.inner.read().await.heartbeat_config.clone();
                let has_prompt_override = hb_cfg
                    .prompt
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty());
                let heartbeat_path = moltis_config::heartbeat_path();
                let heartbeat_file_exists = heartbeat_path.exists();
                let heartbeat_md = moltis_config::load_heartbeat_md();
                if heartbeat_file_exists && heartbeat_md.is_none() && !has_prompt_override {
                    tracing::info!(
                        path = %heartbeat_path.display(),
                        "skipping heartbeat LLM turn: HEARTBEAT.md is empty"
                    );
                    return Ok(moltis_cron::service::AgentTurnResult {
                        output: moltis_cron::heartbeat::HEARTBEAT_OK.to_string(),
                        input_tokens: None,
                        output_tokens: None,
                        session_key: None,
                    });
                }
            }

            let chat = state.chat().await;
            let session_key = match &req.session_target {
                moltis_cron::types::SessionTarget::Named(name) => {
                    format!("cron:{name}")
                },
                _ => format!("cron:{}", uuid::Uuid::new_v4()),
            };

            if matches!(
                req.session_target,
                moltis_cron::types::SessionTarget::Named(_)
            ) {
                let _ = chat
                    .clear(serde_json::json!({ "_session_key": session_key }))
                    .await;
            }

            if let Some(ref router) = state.sandbox_router {
                router.set_override(&session_key, req.sandbox.enabled).await;
                if let Some(ref image) = req.sandbox.image {
                    router.set_image_override(&session_key, image.clone()).await;
                } else {
                    router.remove_image_override(&session_key).await;
                }
            }

            let prompt_text = if is_heartbeat_turn {
                let events = eq.drain().await;
                if events.is_empty() {
                    req.message.clone()
                } else {
                    tracing::info!(
                        event_count = events.len(),
                        "enriching heartbeat prompt with system events"
                    );
                    moltis_cron::heartbeat::build_event_enriched_prompt(&events, &req.message)
                }
            } else {
                req.message.clone()
            };

            let prompt_text = if req.deliver && !is_heartbeat_turn {
                format!(
                    "Your response will be delivered to an external chat channel. \
                     Keep it concise and prefer plain text with minimal formatting.\n\n\
                     {prompt_text}"
                )
            } else {
                prompt_text
            };

            let mut params = serde_json::json!({
                "text": prompt_text,
                "_session_key": session_key,
            });
            if let Some(ref model) = req.model {
                params["model"] = serde_json::Value::String(model.clone());
            }
            let result = chat
                .send_sync(params)
                .await
                .map_err(|e| moltis_cron::Error::message(e.to_string()));

            let auto_prune = req
                .sandbox
                .auto_prune_container
                .unwrap_or(global_auto_prune_containers);
            if req.sandbox.enabled && auto_prune {
                if let Some(ref router) = state.sandbox_router
                    && let Err(e) = router.cleanup_session(&session_key).await
                {
                    tracing::debug!(
                        session_key = %session_key,
                        error = %e,
                        "cron sandbox container cleanup failed"
                    );
                }
            } else if let Some(ref router) = state.sandbox_router {
                router.remove_override(&session_key).await;
                router.remove_image_override(&session_key).await;
            }

            let val = result?;
            let input_tokens = val.get("inputTokens").and_then(|v| v.as_u64());
            let output_tokens = val.get("outputTokens").and_then(|v| v.as_u64());
            let text = val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let delivery_text = if is_heartbeat_turn {
                let hb_cfg = state.inner.read().await.heartbeat_config.clone();
                moltis_cron::heartbeat::strip_heartbeat_token(
                    &text,
                    moltis_cron::heartbeat::StripMode::Trim,
                    hb_cfg.ack_max_chars,
                )
                .text
            } else {
                text.clone()
            };

            maybe_deliver_cron_output(state.services.channel_outbound_arc(), &req, &delivery_text)
                .await;

            Ok(moltis_cron::service::AgentTurnResult {
                output: text,
                input_tokens,
                output_tokens,
                session_key: Some(session_key),
            })
        })
    });

    let deferred_for_cron = Arc::clone(&deferred_state);
    let on_cron_notify: moltis_cron::service::NotifyFn =
        Arc::new(move |notification: moltis_cron::types::CronNotification| {
            let state_opt = deferred_for_cron.get();
            let Some(state) = state_opt else {
                return;
            };
            let (event, payload) = match &notification {
                moltis_cron::types::CronNotification::Created { job } => {
                    ("cron.job.created", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Updated { job } => {
                    ("cron.job.updated", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Removed { job_id } => {
                    ("cron.job.removed", serde_json::json!({ "jobId": job_id }))
                },
            };
            let state = Arc::clone(state);
            tokio::spawn(async move {
                broadcast(&state, event, payload, BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                })
                .await;
            });
        });

    let rate_limit_config = moltis_cron::service::RateLimitConfig {
        max_per_window: config.cron.rate_limit_max,
        window_ms: config.cron.rate_limit_window_secs * 1000,
    };

    let cron_store_for_pruning = Arc::clone(&cron_store);
    let cron_service = moltis_cron::service::CronService::with_events_queue(
        cron_store,
        on_system_event,
        on_agent_turn,
        Some(on_cron_notify),
        rate_limit_config,
        events_queue,
    );

    let live_cron = Arc::new(crate::cron::LiveCronService::new(Arc::clone(&cron_service)));
    services = services.with_cron(live_cron);

    // Webhooks
    let webhook_store_inner: Arc<dyn moltis_webhooks::store::WebhookStore> = Arc::new(
        moltis_webhooks::store::SqliteWebhookStore::with_pool(db_pool.clone()),
    );
    #[cfg(feature = "vault")]
    let webhook_store: Arc<dyn moltis_webhooks::store::WebhookStore> = Arc::new(
        crate::webhooks::VaultWebhookStore::new(Arc::clone(&webhook_store_inner), vault.clone()),
    );
    #[cfg(not(feature = "vault"))]
    let webhook_store = webhook_store_inner;
    let live_webhooks = Arc::new(crate::webhooks::LiveWebhooksService::new(Arc::clone(
        &webhook_store,
    )));
    services = services.with_webhooks(live_webhooks);

    // Build sandbox router from config.
    let mut sandbox_config = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    sandbox_config.container_prefix = Some(sandbox_container_prefix);
    sandbox_config.timezone = config
        .user
        .timezone
        .as_ref()
        .map(|tz| tz.name().to_string());
    let sandbox_router = Arc::new(moltis_tools::sandbox::SandboxRouter::new(
        sandbox_config.clone(),
    ));

    // ── Upstream proxy (user-configured) ─────────────────────────────────
    let upstream_proxy = config
        .upstream_proxy
        .as_ref()
        .map(|s| s.expose_secret().as_str());
    if let Some(url) = upstream_proxy {
        moltis_common::http_client::set_upstream_proxy(url);
        let redacted = moltis_common::http_client::redact_proxy_url(url);
        info!(upstream_proxy = %redacted, "upstream proxy configured for providers and channels");
    }
    moltis_providers::init_shared_http_client(upstream_proxy);

    // ── Trusted-network proxy + audit ────────────────────────────────────
    #[cfg(feature = "trusted-network")]
    let audit_buffer_for_broadcast: Option<crate::network_audit::NetworkAuditBuffer>;
    #[cfg(feature = "trusted-network")]
    let proxy_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>;
    #[cfg(feature = "trusted-network")]
    {
        use std::net::SocketAddr;
        let (audit_tx, audit_rx) =
            tokio::sync::mpsc::channel::<moltis_network_filter::NetworkAuditEntry>(1024);

        info!(
            network_policy = ?sandbox_config.network,
            trusted_domains = ?sandbox_config.trusted_domains,
            "trusted-network: evaluating network policy"
        );

        if sandbox_config.network == moltis_network_filter::NetworkPolicy::Trusted {
            let domain_mgr = Arc::new(
                moltis_network_filter::domain_approval::DomainApprovalManager::new(
                    &sandbox_config.trusted_domains,
                    std::time::Duration::from_secs(30),
                ),
            );
            let proxy_addr: SocketAddr =
                ([0, 0, 0, 0], moltis_network_filter::DEFAULT_PROXY_PORT).into();
            let proxy = moltis_network_filter::proxy::NetworkProxyServer::new(
                proxy_addr,
                Arc::clone(&domain_mgr),
                Some(audit_tx.clone()),
            );
            let (shutdown_tx, proxy_shutdown_rx) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                if let Err(e) = proxy.run(proxy_shutdown_rx).await {
                    tracing::warn!("network proxy exited: {e}");
                }
            });
            let url = format!(
                "http://127.0.0.1:{}",
                moltis_network_filter::DEFAULT_PROXY_PORT
            );
            info!(
                proxy_url = %url,
                "trusted-network proxy started, routing all HTTP tools through proxy"
            );
            moltis_tools::init_shared_http_client(Some(&url));
            proxy_shutdown_tx = Some(shutdown_tx);
        } else {
            info!(
                network_policy = ?sandbox_config.network,
                "trusted-network proxy not started (policy is not Trusted)"
            );
            moltis_tools::init_shared_http_client(upstream_proxy);
            proxy_shutdown_tx = None;
        }

        let audit_log_path = data_dir.join("network-audit.jsonl");
        let audit_service =
            crate::network_audit::LiveNetworkAuditService::new(audit_rx, audit_log_path, 2048);
        audit_buffer_for_broadcast = Some(audit_service.buffer().clone());
        services = services.with_network_audit(Arc::new(audit_service));
    }

    #[cfg(not(feature = "trusted-network"))]
    {
        moltis_tools::init_shared_http_client(upstream_proxy);
    }

    // Spawn background image pre-build.
    {
        let router = Arc::clone(&sandbox_router);
        let backend = Arc::clone(router.backend());
        let packages = router.config().packages.clone();
        let base_image = router
            .config()
            .image
            .clone()
            .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string());

        if super::helpers::should_prebuild_sandbox_image(router.mode(), &packages) {
            let deferred_for_build = Arc::clone(&deferred_state);
            sandbox_router.building_flag.store(true, Ordering::Relaxed);
            let build_router = Arc::clone(&sandbox_router);
            tokio::spawn(async move {
                if let Some(state) = deferred_for_build.get() {
                    broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({ "phase": "start", "packages": packages }),
                        BroadcastOpts {
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
                        build_router.building_flag.store(false, Ordering::Relaxed);

                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
                                }),
                                BroadcastOpts {
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
                        build_router.building_flag.store(false, Ordering::Relaxed);
                    },
                    Err(e) => {
                        tracing::warn!("sandbox image pre-build failed: {e}");
                        build_router.building_flag.store(false, Ordering::Relaxed);
                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
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

    // Host package provisioning when no container runtime is available.
    {
        let packages = sandbox_router.config().packages.clone();
        if sandbox_router.backend_name() == "none"
            && !packages.is_empty()
            && moltis_tools::sandbox::is_debian_host()
        {
            let deferred_for_host = Arc::clone(&deferred_state);
            let pkg_count = packages.len();
            tokio::spawn(async move {
                if let Some(state) = deferred_for_host.get() {
                    broadcast(
                        state,
                        "sandbox.host.provision",
                        serde_json::json!({
                            "phase": "start",
                            "count": pkg_count,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match moltis_tools::sandbox::provision_host_packages(&packages).await {
                    Ok(Some(result)) => {
                        info!(
                            installed = result.installed.len(),
                            skipped = result.skipped.len(),
                            sudo = result.used_sudo,
                            "host package provisioning complete"
                        );
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "installed": result.installed.len(),
                                    "skipped": result.skipped.len(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!("host package provisioning: no-op (not debian or empty packages)");
                    },
                    Err(e) => {
                        warn!("host package provisioning failed: {e}");
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
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

    // Startup GC: remove orphaned session containers.
    if sandbox_router.backend_name() != "none" {
        let prefix = sandbox_router.config().container_prefix.clone();
        tokio::spawn(async move {
            if let Some(prefix) = prefix {
                match moltis_tools::sandbox::clean_all_containers(&prefix).await {
                    Ok(0) => {},
                    Ok(n) => info!(
                        removed = n,
                        "startup GC: cleaned orphaned session containers"
                    ),
                    Err(e) => debug!("startup GC: container cleanup skipped: {e}"),
                }
            }
        });
    }

    // Periodic cron session retention pruning.
    if let Some(retention_days) = config.cron.session_retention_days
        && retention_days > 0
    {
        let prune_store = Arc::clone(&cron_store_for_pruning);
        let prune_session_store = Arc::clone(&session_store);
        let prune_session_metadata = Arc::clone(&session_metadata);
        let prune_sandbox = Arc::clone(&sandbox_router);
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(60 * 60);
            loop {
                tokio::time::sleep(interval).await;
                let retention_ms = time::Duration::days(retention_days as i64)
                    .whole_milliseconds()
                    .unsigned_abs() as u64;
                let cutoff_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let before_ms = cutoff_ms.saturating_sub(retention_ms);

                let session_keys = match prune_store.list_session_keys_before(before_ms).await {
                    Ok(keys) => keys,
                    Err(e) => {
                        tracing::debug!(error = %e, "cron session pruning: failed to list session keys");
                        continue;
                    },
                };

                let mut cleaned = 0u64;
                for key in &session_keys {
                    let suffix = key.strip_prefix("cron:").unwrap_or(key.as_str());
                    if uuid::Uuid::parse_str(suffix).is_err() {
                        continue;
                    }
                    if let Err(e) = prune_session_store.clear(key).await {
                        tracing::debug!(key, error = %e, "cron prune: failed to clear session");
                    }
                    prune_session_metadata.remove(key).await;
                    if let Err(e) = prune_sandbox.cleanup_session(key).await {
                        tracing::debug!(key, error = %e, "cron prune: sandbox cleanup failed");
                    }
                    cleaned += 1;
                }

                match prune_store.prune_runs_before(before_ms).await {
                    Ok(0) => {},
                    Ok(n) => tracing::info!(
                        pruned_runs = n,
                        pruned_sessions = cleaned,
                        retention_days,
                        "cron retention: pruned old runs and sessions"
                    ),
                    Err(e) => {
                        tracing::debug!(error = %e, "cron retention: failed to prune runs")
                    },
                }
            }
        });
    }

    // Pre-pull browser container image.
    if config.tools.browser.enabled
        && !matches!(
            sandbox_router.config().mode,
            moltis_tools::sandbox::SandboxMode::Off
        )
        && sandbox_router.backend_name() != "none"
    {
        let sandbox_image = config.tools.browser.sandbox_image.clone();
        let deferred_for_browser = Arc::clone(&deferred_state);
        tokio::spawn(async move {
            if let Some(state) = deferred_for_browser.get() {
                broadcast(
                    state,
                    "browser.image.pull",
                    serde_json::json!({
                        "phase": "start",
                        "image": sandbox_image,
                    }),
                    BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;
            }

            match moltis_browser::container::ensure_image(&sandbox_image) {
                Ok(()) => {
                    info!(image = %sandbox_image, "browser container image ready");
                    if let Some(state) = deferred_for_browser.get() {
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "done",
                                "image": sandbox_image,
                            }),
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                },
                Err(e) => {
                    tracing::warn!(image = %sandbox_image, error = %e, "browser container image pull failed");
                    if let Some(state) = deferred_for_browser.get() {
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "error",
                                "image": sandbox_image,
                                "error": e.to_string(),
                            }),
                            BroadcastOpts {
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

    // Load persisted sandbox overrides from session metadata.
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

    // ── Channel initialization ───────────────────────────────────────────
    let channel_result = init_channels::init_channels(
        services,
        &config,
        db_pool.clone(),
        #[cfg(feature = "vault")]
        vault.clone(),
        Arc::clone(&message_log),
        Arc::clone(&session_metadata),
        Arc::clone(&deferred_state),
        &data_dir,
    )
    .await;
    services = channel_result.services;
    let msteams_webhook_plugin = channel_result.msteams_webhook_plugin;
    #[cfg(feature = "slack")]
    let slack_webhook_plugin = channel_result.slack_webhook_plugin;

    services = services.with_session_metadata(Arc::clone(&session_metadata));
    services = services.with_session_store(Arc::clone(&session_store));
    services = services.with_session_share_store(Arc::clone(&session_share_store));
    services = services.with_agent_persona_store(Arc::clone(&agent_persona_store));
    startup_mem_probe.checkpoint("channels.initialized");

    let agents_config = Arc::new(tokio::sync::RwLock::new(config.agents.clone()));
    {
        let personas = agent_persona_store.list().await;
        if let Ok(personas) = personas {
            let mut guard = agents_config.write().await;
            for persona in &personas {
                if persona.id == "main" {
                    continue;
                }
                sync_persona_into_preset(&mut guard, persona);
            }
        }
    }
    services = services.with_agents_config(Arc::clone(&agents_config));

    // ── Hook discovery & registration ─────────────────────────────────────
    seed_default_workspace_markdown_files();
    warn_on_workspace_prompt_file_truncation();
    super::hooks::seed_example_skill();
    super::hooks::seed_example_hook();
    super::hooks::seed_dcg_guard_hook().await;
    let persisted_disabled = crate::methods::load_disabled_hooks();
    let (hook_registry, discovered_hooks_info) =
        crate::server::discover_and_build_hooks(&persisted_disabled, Some(&session_store)).await;

    #[cfg(feature = "fs-tools")]
    let shared_fs_state = if config.tools.fs.track_reads {
        Some(moltis_tools::fs::new_fs_state(
            config.tools.fs.must_read_before_write,
        ))
    } else {
        None
    };

    // Wire live session service.
    {
        let mut session_svc =
            LiveSessionService::new(Arc::clone(&session_store), Arc::clone(&session_metadata))
                .with_tts_service(Arc::clone(&services.tts))
                .with_share_store(Arc::clone(&session_share_store))
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_agent_persona_store(Arc::clone(&agent_persona_store))
                .with_project_store(Arc::clone(&project_store))
                .with_state_store(Arc::clone(&session_state_store))
                .with_browser_service(Arc::clone(&services.browser));
        #[cfg(feature = "fs-tools")]
        if let Some(ref fs_state) = shared_fs_state {
            session_svc = session_svc.with_fs_state(Arc::clone(fs_state));
        }
        if let Some(ref hooks) = hook_registry {
            session_svc = session_svc.with_hooks(Arc::clone(hooks));
        }
        services.session = Arc::new(session_svc);
    }

    // ── Memory system initialization ─────────────────────────────────────
    let memory_manager = init_memory::init_memory_system(
        &config,
        &data_dir,
        &effective_providers,
        &runtime_env_overrides,
        config.server.db_pool_max_connections,
    )
    .await;
    startup_mem_probe.checkpoint("memory_manager.initialized");

    post_state::complete_startup(post_state::PostStateInputs {
        bind: bind.to_string(),
        port,
        config,
        log_buffer,
        data_dir,
        resolved_auth,
        deploy_platform,
        session_event_bus,
        services,
        registry,
        effective_providers,
        config_env_overrides,
        runtime_env_overrides,
        provider_summary,
        mcp_configured_count,
        startup_discovery_pending,
        model_store,
        live_model_service,
        provider_setup_service,
        live_mcp,
        memory_manager,
        credential_store,
        db_pool,
        session_store,
        session_metadata,
        session_share_store,
        session_state_store,
        agent_persona_store,
        sandbox_router,
        cron_service,
        deferred_state,
        startup_mem_probe,
        approval_manager,
        hook_registry,
        discovered_hooks_info,
        persisted_disabled,
        agents_config,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
        #[cfg(feature = "local-llm")]
        local_llm_service,
        #[cfg(feature = "vault")]
        vault,
        #[cfg(feature = "trusted-network")]
        audit_buffer: audit_buffer_for_broadcast,
        #[cfg(feature = "trusted-network")]
        proxy_shutdown_tx,
        #[cfg(feature = "tailscale")]
        tailscale_mode_override,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit_override,
    })
    .await
}
