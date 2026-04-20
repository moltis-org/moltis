use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use {
    async_trait::async_trait,
    secrecy::Secret,
    tracing::{debug, info, warn},
};

#[cfg(feature = "wasm")]
use secrecy::ExposeSecret;

use {
    moltis_providers::{PendingDiscoveries, ProviderRegistry},
    moltis_sessions::{
        metadata::SqliteSessionMetadata, session_events::SessionEventBus, store::SessionStore,
    },
};

use crate::{
    approval::GatewayApprovalBroadcaster,
    auth,
    broadcast::{BroadcastOpts, broadcast},
    chat::{LiveChatService, LiveModelService},
    methods::MethodRegistry,
    provider_setup::LiveProviderSetupService,
    services::GatewayServices,
    state::{DiscoveredHookInfo, GatewayState},
};

#[cfg(feature = "tailscale")]
use crate::tailscale::{TailscaleMode, validate_tailscale_config};

use crate::server::{
    helpers::{StartupMemProbe, env_flag_enabled, instance_slug, restore_saved_local_llm_models},
    prepared::PreparedGatewayCore,
    startup::deferred_openclaw_status,
};

#[cfg(feature = "wasm")]
use crate::server::helpers::env_value_with_overrides;

#[cfg(feature = "fs-tools")]
use crate::server::helpers::fs_tools_host_warning_message;

#[cfg(feature = "file-watcher")]
use crate::server::helpers::start_skill_hot_reload_watcher;

pub(super) struct PostStateInputs {
    pub bind: String,
    pub port: u16,
    pub config: moltis_config::MoltisConfig,
    pub log_buffer: Option<crate::logs::LogBuffer>,
    pub data_dir: PathBuf,
    pub resolved_auth: auth::ResolvedAuth,
    pub deploy_platform: Option<String>,
    pub session_event_bus: SessionEventBus,
    pub services: GatewayServices,
    pub registry: Arc<tokio::sync::RwLock<ProviderRegistry>>,
    pub effective_providers: moltis_config::schema::ProvidersConfig,
    pub config_env_overrides: std::collections::HashMap<String, String>,
    pub runtime_env_overrides: std::collections::HashMap<String, String>,
    pub provider_summary: String,
    pub mcp_configured_count: usize,
    pub startup_discovery_pending: PendingDiscoveries,
    pub model_store: Arc<tokio::sync::RwLock<crate::chat::DisabledModelsStore>>,
    pub live_model_service: Arc<LiveModelService>,
    pub provider_setup_service: Arc<LiveProviderSetupService>,
    pub live_mcp: Arc<crate::mcp_service::LiveMcpService>,
    pub memory_manager: Option<moltis_memory::runtime::DynMemoryRuntime>,
    pub code_index: Arc<moltis_code_index::CodeIndex>,
    pub credential_store: Arc<auth::CredentialStore>,
    pub db_pool: sqlx::SqlitePool,
    pub session_store: Arc<SessionStore>,
    pub session_metadata: Arc<SqliteSessionMetadata>,
    pub session_share_store: Arc<crate::share_store::ShareStore>,
    pub session_state_store: Arc<moltis_sessions::state_store::SessionStateStore>,
    pub agent_persona_store: Arc<crate::agent_persona::AgentPersonaStore>,
    pub sandbox_router: Arc<moltis_tools::sandbox::SandboxRouter>,
    pub cron_service: Arc<moltis_cron::service::CronService>,
    pub deferred_state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    pub startup_mem_probe: StartupMemProbe,
    pub approval_manager: Arc<moltis_tools::approval::ApprovalManager>,
    pub hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    pub discovered_hooks_info: Vec<DiscoveredHookInfo>,
    pub persisted_disabled: std::collections::HashSet<String>,
    pub agents_config: Arc<tokio::sync::RwLock<moltis_config::AgentsConfig>>,
    pub msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>,
    #[cfg(feature = "slack")]
    pub slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
    #[cfg(feature = "local-llm")]
    pub local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>>,
    #[cfg(feature = "vault")]
    pub vault: Option<Arc<moltis_vault::Vault>>,
    #[cfg(feature = "trusted-network")]
    pub audit_buffer: Option<crate::network_audit::NetworkAuditBuffer>,
    #[cfg(feature = "trusted-network")]
    pub proxy_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    #[cfg(feature = "tailscale")]
    pub tailscale_mode_override: Option<String>,
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit_override: Option<bool>,
}

struct CredentialEnvVarProvider {
    store: Arc<auth::CredentialStore>,
}

#[async_trait]
impl moltis_tools::exec::EnvVarProvider for CredentialEnvVarProvider {
    async fn get_env_vars(&self) -> Vec<(String, Secret<String>)> {
        match self.store.get_all_env_values().await {
            Ok(values) => values
                .into_iter()
                .map(|(key, value)| (key, Secret::new(value)))
                .collect(),
            Err(error) => {
                warn!(error = %error, "failed to load runtime env overrides for tools");
                Vec::new()
            },
        }
    }
}

async fn build_webauthn_registry(
    config: &moltis_config::MoltisConfig,
    port: u16,
) -> anyhow::Result<Option<crate::auth_webauthn::SharedWebAuthnRegistry>> {
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };

    // Derive RP ID and origin from server.external_url / MOLTIS_EXTERNAL_URL
    // when available, before falling back to fine-grained env vars.
    let (external_rp_id, external_origin) = if let Some(ref ext_url) =
        config.server.effective_external_url()
    {
        match url::Url::parse(ext_url) {
            Ok(parsed) => {
                let host = parsed.host_str().unwrap_or_default().to_string();
                if host.is_empty() {
                    warn!(
                        "server.external_url '{ext_url}' parsed successfully but has no hostname; ignoring"
                    );
                    (None, None)
                } else {
                    (Some(host), Some(ext_url.clone()))
                }
            },
            Err(e) => {
                warn!("invalid server.external_url '{ext_url}': {e}");
                (None, None)
            },
        }
    } else {
        (None, None)
    };

    let explicit_rp_id = external_rp_id
        .or_else(|| std::env::var("MOLTIS_WEBAUTHN_RP_ID").ok())
        .or_else(|| std::env::var("APP_DOMAIN").ok())
        .or_else(|| std::env::var("RENDER_EXTERNAL_HOSTNAME").ok())
        .or_else(|| {
            std::env::var("FLY_APP_NAME")
                .ok()
                .map(|name| format!("{name}.fly.dev"))
        })
        .or_else(|| std::env::var("RAILWAY_PUBLIC_DOMAIN").ok());
    let explicit_origin = external_origin
        .or_else(|| std::env::var("MOLTIS_WEBAUTHN_ORIGIN").ok())
        .or_else(|| std::env::var("APP_URL").ok())
        .or_else(|| std::env::var("RENDER_EXTERNAL_URL").ok());

    let mut wa_registry = crate::auth_webauthn::WebAuthnRegistry::new();
    let mut any_ok = false;

    let mut try_add = |rp_id: &str, origin_str: &str, extras: &[webauthn_rs::prelude::Url]| {
        let rp_id = crate::auth_webauthn::normalize_host(rp_id);
        if rp_id.is_empty() || wa_registry.contains_host(&rp_id) {
            return;
        }
        let Ok(origin_url) = webauthn_rs::prelude::Url::parse(origin_str) else {
            tracing::warn!("invalid WebAuthn origin URL '{origin_str}'");
            return;
        };
        match crate::auth_webauthn::WebAuthnState::new(&rp_id, &origin_url, extras) {
            Ok(wa) => {
                info!(rp_id = %rp_id, origins = ?wa.get_allowed_origins(), "WebAuthn RP registered");
                wa_registry.add(rp_id.clone(), wa);
                any_ok = true;
            },
            Err(e) => tracing::warn!(rp_id = %rp_id, "failed to init WebAuthn: {e}"),
        }
    };

    if let Some(ref rp_id) = explicit_rp_id {
        let origin = explicit_origin
            .clone()
            .unwrap_or_else(|| format!("https://{rp_id}"));
        try_add(rp_id, &origin, &[]);
    } else {
        let localhost_origin = format!("{default_scheme}://localhost:{port}");
        let moltis_localhost: Vec<webauthn_rs::prelude::Url> = webauthn_rs::prelude::Url::parse(
            &format!("{default_scheme}://moltis.localhost:{port}"),
        )
        .into_iter()
        .collect();
        try_add("localhost", &localhost_origin, &moltis_localhost);

        let instance_slug_value = instance_slug(config);
        if instance_slug_value != "localhost" {
            let bot_origin = format!("{default_scheme}://{instance_slug_value}:{port}");
            try_add(&instance_slug_value, &bot_origin, &[]);

            let bot_local = format!("{instance_slug_value}.local");
            let bot_local_origin = format!("{default_scheme}://{bot_local}:{port}");
            try_add(&bot_local, &bot_local_origin, &[]);
        }

        if let Ok(hn) = hostname::get() {
            let hn_str = hn.to_string_lossy();
            if hn_str != "localhost" {
                let local_name = if hn_str.ends_with(".local") {
                    hn_str.to_string()
                } else {
                    format!("{hn_str}.local")
                };
                let local_origin = format!("{default_scheme}://{local_name}:{port}");
                try_add(&local_name, &local_origin, &[]);

                let bare = hn_str.strip_suffix(".local").unwrap_or(&hn_str);
                if bare != local_name {
                    let bare_origin = format!("{default_scheme}://{bare}:{port}");
                    try_add(bare, &bare_origin, &[]);
                }
            }
        }
    }

    if any_ok {
        info!(origins = ?wa_registry.get_all_origins(), "WebAuthn passkeys enabled");
        Ok(Some(Arc::new(tokio::sync::RwLock::new(wa_registry))))
    } else {
        Ok(None)
    }
}

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(super) async fn complete_startup(
    inputs: PostStateInputs,
) -> anyhow::Result<PreparedGatewayCore> {
    let PostStateInputs {
        bind,
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
        session_share_store: _session_share_store,
        session_state_store,
        agent_persona_store: _agent_persona_store,
        sandbox_router,
        cron_service,
        deferred_state,
        mut startup_mem_probe,
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
        audit_buffer,
        #[cfg(feature = "trusted-network")]
        proxy_shutdown_tx,
        #[cfg(feature = "tailscale")]
        tailscale_mode_override,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit_override,
        code_index,
    } = inputs;

    let openclaw_startup_status = deferred_openclaw_status();

    let is_localhost =
        matches!(bind.as_str(), "127.0.0.1" | "::1" | "localhost") || bind.ends_with(".localhost");

    #[cfg(feature = "metrics")]
    let metrics_handle = {
        let metrics_config = moltis_metrics::MetricsRecorderConfig {
            enabled: config.metrics.enabled,
            prefix: None,
            global_labels: vec![
                ("service".to_string(), "moltis-gateway".to_string()),
                ("version".to_string(), moltis_config::VERSION.to_string()),
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

    let browser_for_lifecycle = Arc::clone(&services.browser);
    let pairing_store = Arc::new(crate::pairing::PairingStore::new(db_pool.clone()));
    #[cfg(feature = "tls")]
    let tls_enabled_for_gateway = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_enabled_for_gateway = false;

    #[cfg(feature = "qmd")]
    let code_index_for_tools = Arc::clone(&code_index);

    #[cfg(feature = "code-index-builtin")]
    #[allow(unused_variables)]
    let code_index_for_tools_builtin = Arc::clone(&code_index);

    let state = GatewayState::with_options(
        resolved_auth,
        services,
        config.clone(),
        Some(Arc::clone(&sandbox_router)),
        Some(Arc::clone(&credential_store)),
        Some(pairing_store),
        is_localhost,
        env_flag_enabled("MOLTIS_BEHIND_PROXY"),
        tls_enabled_for_gateway,
        hook_registry.clone(),
        memory_manager.clone(),
        code_index,
        port,
        config.server.ws_request_logs,
        deploy_platform.clone(),
        Some(session_event_bus),
        #[cfg(feature = "metrics")]
        metrics_handle,
        #[cfg(feature = "metrics")]
        metrics_store.clone(),
        #[cfg(feature = "vault")]
        vault.clone(),
    );

    {
        let (webhook_tx, webhook_rx) = tokio::sync::mpsc::channel::<i64>(256);
        let webhook_store: Arc<dyn moltis_webhooks::store::WebhookStore> = {
            let inner: Arc<dyn moltis_webhooks::store::WebhookStore> = Arc::new(
                moltis_webhooks::store::SqliteWebhookStore::with_pool(db_pool.clone()),
            );
            #[cfg(feature = "vault")]
            {
                Arc::new(crate::webhooks::VaultWebhookStore::new(
                    Arc::clone(&inner),
                    vault.clone(),
                ))
            }
            #[cfg(not(feature = "vault"))]
            {
                inner
            }
        };
        let _ = state.webhook_store.set(Arc::clone(&webhook_store));
        let _ = state.webhook_worker_tx.set(webhook_tx);

        let worker_store = Arc::clone(&webhook_store);
        let worker_state_ref = Arc::clone(&state);
        let worker = moltis_webhooks::worker::WebhookWorker::new(
            webhook_rx,
            worker_store,
            Arc::new(move |req: moltis_webhooks::worker::ExecuteRequest| {
                let chat_state = Arc::clone(&worker_state_ref);
                Box::pin(async move {
                    let chat = chat_state.chat().await;
                    let mut params = serde_json::json!({
                        "text": req.message,
                        "_session_key": req.session_key,
                    });
                    if let Some(ref model) = req.model {
                        params["model"] = serde_json::Value::String(model.clone());
                    }
                    if let Some(ref agent_id) = req.agent_id {
                        params["agent_id"] = serde_json::Value::String(agent_id.clone());
                    }
                    if let Some(ref tool_policy) = req.tool_policy {
                        params["_tool_policy"] = serde_json::to_value(tool_policy)
                            .map_err(|error| anyhow::anyhow!(error))?;
                    }
                    let result = chat
                        .send_sync(params)
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    let input_tokens = result.get("inputTokens").and_then(|v| v.as_i64());
                    let output_tokens = result.get("outputTokens").and_then(|v| v.as_i64());
                    let output = result
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    Ok(moltis_webhooks::worker::ProcessResult {
                        output,
                        input_tokens,
                        output_tokens,
                        session_key: req.session_key,
                    })
                })
            }),
        );
        tokio::spawn(worker.run());
    }

    let _ = deferred_state.set(Arc::clone(&state));

    #[cfg(feature = "local-llm")]
    if let Some(svc) = &local_llm_service {
        svc.set_state(Arc::clone(&state));
    }

    provider_setup_service.set_broadcaster(Arc::new(crate::provider_setup::GatewayBroadcaster {
        state: Arc::clone(&state),
    }));
    live_model_service.set_state(crate::chat::GatewayChatRuntime::from_state(Arc::clone(
        &state,
    )));

    match credential_store.ssh_target_count().await {
        Ok(count) => state.ssh_target_count.store(count, Ordering::Relaxed),
        Err(error) => warn!(%error, "failed to load ssh target count"),
    }

    {
        let mut inner = state.inner.write().await;
        inner.discovered_hooks = discovered_hooks_info;
        inner.disabled_hooks = persisted_disabled;
        inner.shiki_cdn_url = config.server.shiki_cdn_url.clone();
        #[cfg(feature = "metrics")]
        {
            inner.metrics_history =
                crate::state::MetricsHistory::new(config.metrics.history_points);
        }
    }

    let setup_code_display =
        if !credential_store.is_setup_complete() && !credential_store.is_auth_disabled() {
            let code = std::env::var("MOLTIS_E2E_SETUP_CODE")
                .unwrap_or_else(|_| auth::generate_setup_code());
            {
                let mut inner = state.inner.write().await;
                inner.setup_code = Some(Secret::new(code.clone()));
                inner.setup_code_created_at = Some(std::time::Instant::now());
            }
            Some(code)
        } else {
            None
        };

    #[cfg(feature = "tailscale")]
    let tailscale_mode: TailscaleMode = {
        let mode_str = tailscale_mode_override.unwrap_or_else(|| config.tailscale.mode.clone());
        mode_str.parse().unwrap_or(TailscaleMode::Off)
    };
    #[cfg(feature = "tailscale")]
    let tailscale_reset_on_exit =
        tailscale_reset_on_exit_override.unwrap_or(config.tailscale.reset_on_exit);

    #[cfg(feature = "tailscale")]
    if tailscale_mode != TailscaleMode::Off {
        validate_tailscale_config(tailscale_mode, &bind, credential_store.is_setup_complete())?;
    }

    let webauthn_registry = build_webauthn_registry(&config, port).await?;

    if startup_discovery_pending.is_empty() {
        debug!("startup model discovery skipped, no pending provider discoveries");
    } else {
        let registry_for_startup_discovery = Arc::clone(&registry);
        let state_for_startup_discovery = Arc::clone(&state);
        let provider_config_for_startup_discovery = effective_providers.clone();
        let provider_config_for_registry_rebuild = provider_config_for_startup_discovery.clone();
        let global_cw_overrides = moltis_providers::extract_cw_overrides(&config.models);
        let env_overrides_for_startup_discovery = config_env_overrides.clone();
        tokio::spawn(async move {
            let startup_discovery_started = std::time::Instant::now();
            let prefetched = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::collect_discoveries(startup_discovery_pending)
            })
            .await
            {
                Ok(prefetched) => prefetched,
                Err(error) => {
                    warn!(
                        error = %error,
                        "startup background model discovery worker failed while collecting results"
                    );
                    return;
                },
            };

            let prefetched_models: usize = prefetched.values().map(Vec::len).sum();
            let mut new_registry = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::from_config_with_prefetched(
                    &provider_config_for_registry_rebuild,
                    &env_overrides_for_startup_discovery,
                    &prefetched,
                    global_cw_overrides.clone(),
                )
            })
            .await
            {
                Ok(new_registry) => new_registry,
                Err(error) => {
                    warn!(
                        error = %error,
                        "startup background model discovery worker failed while rebuilding registry"
                    );
                    return;
                },
            };

            restore_saved_local_llm_models(
                &mut new_registry,
                &provider_config_for_startup_discovery,
            );
            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            {
                let mut reg = registry_for_startup_discovery.write().await;
                *reg = new_registry;
            }

            info!(
                provider_summary = %provider_summary,
                models = model_count,
                prefetched_models,
                elapsed_ms = startup_discovery_started.elapsed().as_millis(),
                "startup background model discovery complete, provider registry updated"
            );

            broadcast(
                &state_for_startup_discovery,
                "models.updated",
                serde_json::json!({
                    "reason": "startup-discovery",
                    "models": model_count,
                    "providerSummary": provider_summary,
                }),
                BroadcastOpts::default(),
            )
            .await;
        });
    }

    {
        let mut inner = state.inner.write().await;
        inner.heartbeat_config = config.heartbeat.clone();
        inner.channels_offered = config.channels.offered.clone();
    }
    #[cfg(feature = "graphql")]
    state.set_graphql_enabled(config.graphql.enabled);

    {
        let broadcaster: Arc<dyn moltis_tools::exec::ApprovalBroadcaster> =
            Arc::new(GatewayApprovalBroadcaster::new(Arc::clone(&state)));
        let env_provider: Arc<dyn moltis_tools::exec::EnvVarProvider> =
            Arc::new(CredentialEnvVarProvider {
                store: Arc::clone(&credential_store),
            });
        let eq = cron_service.events_queue().clone();
        let cs = Arc::clone(&cron_service);
        let exec_cb: moltis_tools::exec::ExecCompletionFn = Arc::new(move |event| {
            let summary = format!("Command `{}` exited {}", event.command, event.exit_code);
            let eq = Arc::clone(&eq);
            let cs = Arc::clone(&cs);
            tokio::spawn(async move {
                eq.enqueue(summary, "exec-event".into()).await;
                cs.wake("exec-event").await;
            });
        });
        let mut exec_tool = moltis_tools::exec::ExecTool::default()
            .with_default_timeout(std::time::Duration::from_secs(
                config.tools.exec.default_timeout_secs,
            ))
            .with_max_output_bytes(config.tools.exec.max_output_bytes)
            .with_approval(Arc::clone(&approval_manager), Arc::clone(&broadcaster))
            .with_sandbox_router(Arc::clone(&sandbox_router))
            .with_env_provider(Arc::clone(&env_provider))
            .with_completion_callback(exec_cb);

        {
            let provider = Arc::new(crate::node_exec::GatewayNodeExecProvider::new(
                Arc::clone(&state),
                Arc::clone(&state.node_count),
                Arc::clone(&state.ssh_target_count),
                config.tools.exec.ssh_target.clone(),
                config.tools.exec.max_output_bytes,
            ));
            let default_node = match config.tools.exec.host.as_str() {
                "node" => config.tools.exec.node.clone(),
                "ssh" => config.tools.exec.ssh_target.clone(),
                _ => None,
            };
            exec_tool = exec_tool.with_node_provider(provider, default_node);
        }

        let cron_tool = moltis_tools::cron_tool::CronTool::new(Arc::clone(&cron_service));

        let mut tool_registry = moltis_agents::tool_registry::ToolRegistry::new();
        let process_tool = moltis_tools::process::ProcessTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        let sandbox_packages_tool = moltis_tools::sandbox_packages::SandboxPackagesTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        tool_registry.register(Box::new(exec_tool));
        tool_registry.register(Box::new(moltis_tools::calc::CalcTool::new()));
        #[cfg(feature = "fs-tools")]
        {
            use moltis_config::schema::FsBinaryPolicy;
            let fs_cfg = &config.tools.fs;
            let path_policy = match moltis_tools::fs::FsPathPolicy::new(
                &fs_cfg.allow_paths,
                &fs_cfg.deny_paths,
            ) {
                Ok(p) => {
                    if p.is_empty() {
                        None
                    } else {
                        Some(p)
                    }
                },
                Err(e) => {
                    warn!(error = %e, "invalid tools.fs path policy — fs tools will run without path allow/deny");
                    None
                },
            };
            let workspace_root = fs_cfg.workspace_root.as_ref().map(PathBuf::from);
            let binary_policy = match fs_cfg.binary_policy {
                FsBinaryPolicy::Reject => moltis_tools::fs::BinaryPolicy::Reject,
                FsBinaryPolicy::Base64 => moltis_tools::fs::BinaryPolicy::Base64,
            };
            let checkpoint_manager = if fs_cfg.checkpoint_before_mutation {
                Some(Arc::new(moltis_tools::checkpoints::CheckpointManager::new(
                    moltis_config::data_dir(),
                )))
            } else {
                None
            };
            let shared_fs_state = if fs_cfg.track_reads {
                Some(moltis_tools::fs::new_fs_state(
                    fs_cfg.must_read_before_write,
                ))
            } else {
                None
            };
            let ctx = moltis_tools::fs::FsToolsContext {
                workspace_root,
                fs_state: shared_fs_state.clone(),
                path_policy,
                binary_policy,
                respect_gitignore: fs_cfg.respect_gitignore,
                checkpoint_manager,
                sandbox_router: Some(Arc::clone(&sandbox_router)),
                approval_manager: fs_cfg
                    .require_approval
                    .then(|| Arc::clone(&approval_manager)),
                broadcaster: fs_cfg.require_approval.then(|| Arc::clone(&broadcaster)),
                max_read_bytes: Some(fs_cfg.max_read_bytes),
                context_window_tokens: fs_cfg.context_window_tokens,
            };
            moltis_tools::fs::register_fs_tools(&mut tool_registry, ctx);
            if let Some(message) = fs_tools_host_warning_message(&sandbox_router) {
                warn!("{message}");
            }
        }
        #[cfg(feature = "wasm")]
        {
            let wasm_limits = sandbox_router
                .config()
                .wasm_tool_limits
                .clone()
                .unwrap_or_default();
            let epoch_interval_ms = sandbox_router
                .config()
                .wasm_epoch_interval_ms
                .unwrap_or(100);
            let brave_api_key = config
                .tools
                .web
                .search
                .api_key
                .as_ref()
                .map(|s| s.expose_secret().clone())
                .or_else(|| env_value_with_overrides(&runtime_env_overrides, "BRAVE_API_KEY"))
                .filter(|k| !k.trim().is_empty());
            if let Err(e) = moltis_tools::wasm_tool_runner::register_wasm_tools(
                &mut tool_registry,
                &wasm_limits,
                epoch_interval_ms,
                config.tools.web.fetch.timeout_seconds,
                config.tools.web.fetch.cache_ttl_minutes,
                config.tools.web.search.timeout_seconds,
                config.tools.web.search.cache_ttl_minutes,
                brave_api_key.as_deref(),
            ) {
                warn!(%e, "wasm tool registration failed");
            }
        }
        tool_registry.register(Box::new(process_tool));
        tool_registry.register(Box::new(sandbox_packages_tool));
        tool_registry.register(Box::new(cron_tool));
        tool_registry.register(Box::new(crate::channel_agent_tools::SendMessageTool::new(
            Arc::clone(&state.services.channel),
        )));
        {
            let tp = Arc::clone(&msteams_webhook_plugin);
            tool_registry.register(Box::new(
                crate::teams_agent_tools::TeamsSearchMessagesTool::new(Arc::clone(&tp)),
            ));
            tool_registry.register(Box::new(
                crate::teams_agent_tools::TeamsMemberInfoTool::new(Arc::clone(&tp)),
            ));
            tool_registry.register(Box::new(
                crate::teams_agent_tools::TeamsPinMessageTool::new(Arc::clone(&tp)),
            ));
            tool_registry.register(Box::new(
                crate::teams_agent_tools::TeamsEditMessageTool::new(Arc::clone(&tp)),
            ));
            tool_registry.register(Box::new(
                crate::teams_agent_tools::TeamsReadMessageTool::new(Arc::clone(&tp)),
            ));
        }
        tool_registry.register(Box::new(
            crate::channel_agent_tools::UpdateChannelSettingsTool::new(
                Arc::clone(&state.services.channel),
                state.services.channel_store.clone(),
            ),
        ));
        tool_registry.register(Box::new(
            moltis_tools::send_image::SendImageTool::new()
                .with_sandbox_router(Arc::clone(&sandbox_router)),
        ));
        tool_registry.register(Box::new(
            moltis_tools::send_document::SendDocumentTool::new()
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_session_store(Arc::clone(&session_store)),
        ));
        if let Some(t) = moltis_tools::web_search::WebSearchTool::from_config_with_env_overrides(
            &config.tools.web.search,
            &runtime_env_overrides,
        ) {
            #[cfg(feature = "firecrawl")]
            let t = t.with_firecrawl_config(&config.tools.web.firecrawl);
            tool_registry.register(Box::new(t.with_env_provider(Arc::clone(&env_provider))));
        }
        if let Some(t) = moltis_tools::web_fetch::WebFetchTool::from_config(&config.tools.web.fetch)
        {
            #[cfg(feature = "firecrawl")]
            let t = t.with_firecrawl(&config.tools.web.firecrawl);
            tool_registry.register(Box::new(t));
        }
        #[cfg(feature = "firecrawl")]
        if let Some(t) =
            moltis_tools::firecrawl::FirecrawlScrapeTool::from_config(&config.tools.web.firecrawl)
        {
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::browser::BrowserTool::from_config(&config.tools.browser) {
            let t = if sandbox_router.backend_name() != "none" {
                t.with_sandbox_router(Arc::clone(&sandbox_router))
            } else {
                t
            };
            tool_registry.register(Box::new(t));
        }

        #[cfg(feature = "caldav")]
        {
            if let Some(t) = moltis_caldav::tool::CalDavTool::from_config(&config.caldav) {
                tool_registry.register(Box::new(t));
            }
        }

        if let Some(ref mm) = memory_manager {
            tool_registry.register(Box::new(moltis_memory::tools::MemorySearchTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryGetTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemorySaveTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryDeleteTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_chat::MemoryForgetTool::new(
                Arc::clone(mm),
                Arc::clone(&registry),
                Arc::clone(&session_metadata),
            )));
        }

        // ── Code index tools ─────────────────────────────────────────────
        #[cfg(feature = "qmd")]
        {
            moltis_code_index::tools::register_tools(&mut tool_registry, code_index_for_tools);
        }

        #[cfg(all(feature = "code-index-builtin", not(feature = "qmd")))]
        {
            moltis_code_index::tools::register_tools(
                &mut tool_registry,
                code_index_for_tools_builtin,
            );
        }

        {
            let node_info_provider: Arc<dyn moltis_tools::nodes::NodeInfoProvider> =
                Arc::new(crate::node_exec::GatewayNodeInfoProvider::new(
                    Arc::clone(&state),
                    config.tools.exec.ssh_target.clone(),
                ));
            tool_registry.register(Box::new(moltis_tools::nodes::NodesListTool::new(
                Arc::clone(&node_info_provider),
            )));
            tool_registry.register(Box::new(moltis_tools::nodes::NodesDescribeTool::new(
                Arc::clone(&node_info_provider),
            )));
            tool_registry.register(Box::new(moltis_tools::nodes::NodesSelectTool::new(
                Arc::clone(&node_info_provider),
            )));
        }

        tool_registry.register(Box::new(
            moltis_tools::session_state::SessionStateTool::new(Arc::clone(&session_state_store)),
        ));

        let state_for_session_create = Arc::clone(&state);
        let metadata_for_session_create = Arc::clone(&session_metadata);
        let create_session: moltis_tools::sessions_manage::CreateSessionFn = Arc::new(
            move |req: moltis_tools::sessions_manage::CreateSessionRequest| {
                let state = Arc::clone(&state_for_session_create);
                let metadata = Arc::clone(&metadata_for_session_create);
                Box::pin(async move {
                    let key = req.key;

                    let mut resolve_params = serde_json::json!({ "key": key.clone() });
                    if let Some(inherit) = req.inherit_agent_from {
                        resolve_params["inherit_agent_from"] = serde_json::json!(inherit);
                    }
                    state
                        .services
                        .session
                        .resolve(resolve_params)
                        .await
                        .map_err(|e| moltis_tools::Error::message(e.to_string()))?;

                    let mut patch = serde_json::Map::new();
                    patch.insert("key".to_string(), serde_json::json!(key.clone()));
                    if let Some(label) = req.label {
                        patch.insert("label".to_string(), serde_json::json!(label));
                    }
                    if let Some(model) = req.model {
                        patch.insert("model".to_string(), serde_json::json!(model));
                    }
                    if let Some(project_id) = req.project_id {
                        patch.insert("projectId".to_string(), serde_json::json!(project_id));
                    }
                    if patch.len() > 1 {
                        state
                            .services
                            .session
                            .patch(serde_json::Value::Object(patch))
                            .await
                            .map_err(|e| moltis_tools::Error::message(e.to_string()))?;
                    }

                    let entry = metadata.get(&key).await.ok_or_else(|| {
                        moltis_tools::Error::message(format!(
                            "session '{key}' not found after create"
                        ))
                    })?;
                    Ok(serde_json::json!({
                        "entry": {
                            "id": entry.id,
                            "key": entry.key,
                            "label": entry.label,
                            "model": entry.model,
                            "createdAt": entry.created_at,
                            "updatedAt": entry.updated_at,
                            "messageCount": entry.message_count,
                            "projectId": entry.project_id,
                            "agent_id": entry.agent_id,
                            "agentId": entry.agent_id,
                            "version": entry.version,
                        }
                    }))
                })
            },
        );

        let state_for_session_delete = Arc::clone(&state);
        let delete_session: moltis_tools::sessions_manage::DeleteSessionFn = Arc::new(
            move |req: moltis_tools::sessions_manage::DeleteSessionRequest| {
                let state = Arc::clone(&state_for_session_delete);
                Box::pin(async move {
                    state
                        .services
                        .session
                        .delete(serde_json::json!({
                            "key": req.key,
                            "force": req.force,
                        }))
                        .await
                        .map_err(|e| moltis_tools::Error::message(e.to_string()))
                })
            },
        );

        tool_registry.register(Box::new(
            moltis_tools::sessions_manage::SessionsCreateTool::new(
                Arc::clone(&session_metadata),
                create_session,
            ),
        ));
        tool_registry.register(Box::new(
            moltis_tools::sessions_manage::SessionsDeleteTool::new(
                Arc::clone(&session_metadata),
                delete_session,
            ),
        ));
        tool_registry.register(Box::new(
            moltis_tools::sessions_communicate::SessionsListTool::new(Arc::clone(
                &session_metadata,
            )),
        ));
        tool_registry.register(Box::new(
            moltis_tools::sessions_communicate::SessionsHistoryTool::new(
                Arc::clone(&session_store),
                Arc::clone(&session_metadata),
            ),
        ));
        tool_registry.register(Box::new(
            moltis_tools::sessions_communicate::SessionsSearchTool::new(
                Arc::clone(&session_store),
                Arc::clone(&session_metadata),
            ),
        ));

        let state_for_session_send = Arc::clone(&state);
        let send_to_session: moltis_tools::sessions_communicate::SendToSessionFn = Arc::new(
            move |req: moltis_tools::sessions_communicate::SendToSessionRequest| {
                let state = Arc::clone(&state_for_session_send);
                Box::pin(async move {
                    let mut params = serde_json::json!({
                        "text": req.message,
                        "_session_key": req.key,
                    });
                    if let Some(model) = req.model {
                        params["model"] = serde_json::json!(model);
                    }
                    let chat = state.chat().await;
                    if req.wait_for_reply {
                        chat.send_sync(params)
                            .await
                            .map_err(|e| moltis_tools::Error::message(e.to_string()))
                    } else {
                        chat.send(params)
                            .await
                            .map_err(|e| moltis_tools::Error::message(e.to_string()))
                    }
                })
            },
        );
        tool_registry.register(Box::new(
            moltis_tools::sessions_communicate::SessionsSendTool::new(
                Arc::clone(&session_metadata),
                send_to_session,
            ),
        ));
        tool_registry.register(Box::new(
            moltis_tools::checkpoints::CheckpointsListTool::new(data_dir.clone()),
        ));
        tool_registry.register(Box::new(
            moltis_tools::checkpoints::CheckpointRestoreTool::new(data_dir.clone()),
        ));

        tool_registry.register(Box::new(moltis_tools::task_list::TaskListTool::new(
            &data_dir,
        )));
        tool_registry.register(Box::new(crate::voice_agent_tools::SpeakTool::new(
            Arc::clone(&state.services.tts),
        )));
        tool_registry.register(Box::new(crate::voice_agent_tools::TranscribeTool::new(
            Arc::clone(&state.services.stt),
        )));

        {
            use moltis_skills::discover::FsSkillDiscoverer;

            tool_registry.register(Box::new(moltis_tools::skill_tools::CreateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::UpdateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::PatchSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::DeleteSkillTool::new(
                data_dir.clone(),
            )));
            let read_discoverer = Arc::new(FsSkillDiscoverer::new(
                FsSkillDiscoverer::default_paths_for(&data_dir),
            ));
            tool_registry.register(Box::new(moltis_tools::skill_tools::ReadSkillTool::new(
                read_discoverer,
            )));
            if config.skills.enable_agent_sidecar_files {
                tool_registry.register(Box::new(
                    moltis_tools::skill_tools::WriteSkillFilesTool::new(data_dir.clone()),
                ));
            }
        }

        tool_registry.register(Box::new(
            moltis_tools::branch_session::BranchSessionTool::new(
                Arc::clone(&session_store),
                Arc::clone(&session_metadata),
            ),
        ));

        let location_requester = Arc::new(crate::server::location::GatewayLocationRequester {
            state: Arc::clone(&state),
        });
        tool_registry.register(Box::new(moltis_tools::location::LocationTool::new(
            location_requester,
        )));

        let map_provider = match config.tools.maps.provider {
            moltis_config::schema::MapProvider::GoogleMaps => {
                moltis_tools::map::MapProvider::GoogleMaps
            },
            moltis_config::schema::MapProvider::AppleMaps => {
                moltis_tools::map::MapProvider::AppleMaps
            },
            moltis_config::schema::MapProvider::OpenStreetMap => {
                moltis_tools::map::MapProvider::OpenStreetMap
            },
        };
        tool_registry.register(Box::new(moltis_tools::map::ShowMapTool::with_provider(
            map_provider,
        )));

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
                    _ => return,
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
            .with_on_event(on_spawn_event)
            .with_agents_config(agents_config);
            tool_registry.register(Box::new(spawn_tool));
        }

        let shared_tool_registry = Arc::new(tokio::sync::RwLock::new(tool_registry));
        let mut chat_service = LiveChatService::new(
            Arc::clone(&registry),
            Arc::clone(&model_store),
            crate::chat::GatewayChatRuntime::from_state(Arc::clone(&state)),
            Arc::clone(&session_store),
            Arc::clone(&session_metadata),
        )
        .with_session_state_store(Arc::clone(&session_state_store))
        .with_tools(Arc::clone(&shared_tool_registry))
        .with_failover(config.failover.clone());

        if let Some(ref hooks) = state.inner.read().await.hook_registry {
            chat_service = chat_service.with_hooks_arc(Arc::clone(hooks));
        }

        let live_chat = Arc::new(chat_service);
        state.set_chat(live_chat).await;

        live_mcp
            .set_tool_registry(Arc::clone(&shared_tool_registry))
            .await;
        crate::mcp_service::sync_mcp_tools(live_mcp.manager(), &shared_tool_registry).await;

        let schemas = shared_tool_registry.read().await.list_schemas();
        let tool_names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        info!(tools = ?tool_names, "agent tools registered");
    }

    #[cfg(feature = "file-watcher")]
    {
        let watcher_state = Arc::clone(&state);
        tokio::spawn(async move {
            let (mut watcher, mut rx) = match start_skill_hot_reload_watcher().await {
                Ok(started) => started,
                Err(error) => {
                    tracing::warn!("skills: failed to start file watcher: {error}");
                    return;
                },
            };

            loop {
                let Some(event) = rx.recv().await else {
                    break;
                };
                broadcast(
                    &watcher_state,
                    "skills.changed",
                    serde_json::json!({}),
                    BroadcastOpts::default(),
                )
                .await;

                if matches!(
                    event,
                    moltis_skills::watcher::SkillWatchEvent::ManifestChanged
                ) {
                    match start_skill_hot_reload_watcher().await {
                        Ok((new_watcher, new_rx)) => {
                            watcher = new_watcher;
                            rx = new_rx;
                        },
                        Err(error) => {
                            tracing::warn!("skills: failed to refresh file watcher: {error}");
                        },
                    }
                }
            }

            drop(watcher);
        });
    }

    {
        let health_state = Arc::clone(&state);
        let health_mcp = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            crate::mcp_health::run_health_monitor(health_state, health_mcp).await;
        });
    }

    let methods = Arc::new(MethodRegistry::new());

    #[cfg(feature = "push-notifications")]
    let push_service: Option<Arc<crate::push::PushService>> = {
        match crate::push::PushService::new(&data_dir).await {
            Ok(svc) => {
                info!("push notification service initialized");
                state.set_push_service(Arc::clone(&svc)).await;
                Some(svc)
            },
            Err(e) => {
                tracing::warn!("failed to initialize push notification service: {e}");
                None
            },
        }
    };

    startup_mem_probe.checkpoint("prepare_gateway.ready");

    Ok(PreparedGatewayCore {
        state: Arc::clone(&state),
        methods: Arc::clone(&methods),
        webauthn_registry,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
        #[cfg(feature = "push-notifications")]
        push_service,
        #[cfg(feature = "trusted-network")]
        audit_buffer,
        sandbox_router,
        browser_for_lifecycle,
        cron_service,
        log_buffer,
        config,
        data_dir,
        provider_summary,
        mcp_configured_count,
        openclaw_status: openclaw_startup_status,
        setup_code_display,
        port,
        tls_enabled: tls_enabled_for_gateway,
        #[cfg(feature = "tailscale")]
        tailscale_mode,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit,
        browser_tool_for_warmup: None,
        #[cfg(feature = "trusted-network")]
        _proxy_shutdown_tx: proxy_shutdown_tx,
    })
}
