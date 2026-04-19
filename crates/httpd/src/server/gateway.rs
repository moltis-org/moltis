//! Full gateway preparation: config loading, migration, service wiring,
//! background task spawning, and the composed axum application.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use {
    axum::{
        extract::ConnectInfo,
        http::StatusCode,
        response::{IntoResponse, Json},
    },
    moltis_channels::ChannelPlugin,
    moltis_gateway::server::{PreparedGatewayCore, prepare_gateway_core},
    moltis_sessions::session_events::SessionEventBus,
};

#[cfg(not(feature = "ngrok"))]
use super::builder::build_gateway_base;
#[cfg(feature = "ngrok")]
use super::builder::build_gateway_base_internal;
use super::{
    PreparedGateway, RouteEnhancer, builder::finalize_gateway_app, runtime::FinalizeGatewayArgs,
};

#[cfg(feature = "tailscale")]
use super::TailscaleOpts;

/// Prepare the full gateway: load config, run migrations, wire services,
/// spawn background tasks, and return the composed axum application.
///
/// This is the HTTP layer on top of [`prepare_gateway_core`]. The swift-bridge
/// calls this directly and manages its own TCP listener + graceful shutdown.
///
/// `extra_routes` is an optional callback that returns additional routes
/// (e.g. the web-UI) to merge before finalization.
#[allow(clippy::expect_used)]
pub async fn prepare_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    extra_routes: Option<RouteEnhancer>,
    session_event_bus: Option<SessionEventBus>,
) -> crate::error::Result<PreparedGateway> {
    // Install a process-level rustls CryptoProvider early, before any channel
    // plugin (Slack, Discord, etc.) creates outbound TLS connections via
    // hyper-rustls.  Without this, `--no-tls` deployments skip the TLS cert
    // setup path where `install_default()` previously lived, causing a panic
    // the first time an outbound HTTPS request is made (see #329).
    #[cfg(feature = "tls")]
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(feature = "tailscale")]
    let tailscale_mode_override = tailscale_opts.as_ref().map(|opts| opts.mode.clone());
    #[cfg(feature = "tailscale")]
    let tailscale_reset_on_exit_override = tailscale_opts.as_ref().map(|opts| opts.reset_on_exit);
    #[cfg(not(feature = "tailscale"))]
    let tailscale_mode_override: Option<String> = None;
    #[cfg(not(feature = "tailscale"))]
    let tailscale_reset_on_exit_override: Option<bool> = None;

    let core = prepare_gateway_core(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        tailscale_mode_override,
        tailscale_reset_on_exit_override,
        session_event_bus,
    )
    .await
    .map_err(|e| crate::Error::Config(e.to_string()))?;

    let PreparedGatewayCore {
        state,
        methods,
        webauthn_registry,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
        #[cfg(feature = "push-notifications")]
        push_service,
        #[cfg(feature = "trusted-network")]
            audit_buffer: audit_buffer_for_broadcast,
        #[cfg(feature = "trusted-network")]
        _proxy_shutdown_tx,
        sandbox_router,
        browser_for_lifecycle,
        browser_tool_for_warmup,
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
        ..
    } = core;

    #[cfg(feature = "push-notifications")]
    #[cfg(feature = "ngrok")]
    let (router, mut app_state, ngrok_controller) = build_gateway_base_internal(
        Arc::clone(&state),
        Arc::clone(&methods),
        push_service,
        webauthn_registry.clone(),
    );
    #[cfg(feature = "push-notifications")]
    #[cfg(feature = "ngrok")]
    super::runtime::attach_ngrok_controller_owner(&mut app_state, &ngrok_controller);
    #[cfg(all(feature = "push-notifications", not(feature = "ngrok")))]
    let (router, app_state) = build_gateway_base(
        Arc::clone(&state),
        Arc::clone(&methods),
        push_service,
        webauthn_registry.clone(),
    );
    #[cfg(not(feature = "push-notifications"))]
    #[cfg(feature = "ngrok")]
    let (router, mut app_state, ngrok_controller) = build_gateway_base_internal(
        Arc::clone(&state),
        Arc::clone(&methods),
        webauthn_registry.clone(),
    );
    #[cfg(not(feature = "push-notifications"))]
    #[cfg(feature = "ngrok")]
    super::runtime::attach_ngrok_controller_owner(&mut app_state, &ngrok_controller);
    #[cfg(all(not(feature = "push-notifications"), not(feature = "ngrok")))]
    let (router, app_state) = build_gateway_base(
        Arc::clone(&state),
        Arc::clone(&methods),
        webauthn_registry.clone(),
    );

    // Merge caller-provided routes (e.g. web-UI) before finalization.
    let router = if let Some(enhance) = extra_routes {
        router.merge(enhance())
    } else {
        router
    };

    let mut app = finalize_gateway_app(router, app_state, config.server.http_request_logs);

    {
        let teams_plugin_for_webhook = Arc::clone(&msteams_webhook_plugin);
        let state_for_teams_webhook = Arc::clone(&state);
        app = app.route(
            "/api/channels/msteams/{account_id}/webhook",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let teams_plugin = Arc::clone(&teams_plugin_for_webhook);
                    let gw_state = Arc::clone(&state_for_teams_webhook);
                    async move {
                        // JWT pre-validation: if a JWT validator is configured,
                        // the Authorization header is mandatory and must be valid.
                        // A missing header is treated as an auth failure (not skipped).
                        let jwt_validator = {
                            let plugin = teams_plugin.read().await;
                            plugin.jwt_validator(&account_id)
                        };
                        if let Some(validator) = jwt_validator {
                            let header_str = headers
                                .get("authorization")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("");
                            if !validator.validate(header_str).await {
                                return (
                                    StatusCode::UNAUTHORIZED,
                                    Json(serde_json::json!({ "ok": false, "error": "invalid JWT" })),
                                )
                                    .into_response();
                            }
                        }

                        // Get the verifier from the plugin.
                        let verifier = {
                            let plugin = teams_plugin.read().await;
                            plugin.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Teams account" })),
                            )
                                .into_response();
                        };

                        // Inject query-param secret as header for the verifier.
                        let mut merged_headers = headers;
                        if let Some(secret) = query.get("secret")
                            && let Ok(val) = secret.parse()
                        {
                            merged_headers.insert("x-moltis-webhook-secret", val);
                        }

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &merged_headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                crate::channel_webhook_middleware::rejection_into_response(
                                    rejection,
                                )
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Parse verified body.
                                let payload: serde_json::Value =
                                    match serde_json::from_slice(&verified.body) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            return (
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
                                            )
                                                .into_response();
                                        },
                                    };

                                // Spawn processing asynchronously and return 202
                                // immediately. This prevents Teams from retrying
                                // when LLM processing takes longer than ~15 seconds.
                                let account_id_owned = account_id.clone();
                                let teams_plugin_for_spawn = Arc::clone(&teams_plugin);
                                tokio::spawn(async move {
                                    let plugin = teams_plugin_for_spawn.read().await;
                                    if let Err(e) = plugin
                                        .ingest_verified_activity(&account_id_owned, payload)
                                        .await
                                    {
                                        tracing::warn!(
                                            account_id = account_id_owned,
                                            "Teams webhook processing failed: {e}"
                                        );
                                    }
                                });

                                (
                                    StatusCode::ACCEPTED,
                                    Json(serde_json::json!({ "ok": true })),
                                )
                                    .into_response()
                            },
                        }
                    }
                },
            ),
        );
    }
    #[cfg(feature = "slack")]
    {
        // Slack Events API webhook -- receives event callbacks.
        let slack_events_plugin = Arc::clone(&slack_webhook_plugin);
        let state_for_slack_events = Arc::clone(&state);
        app = app.route(
            "/api/channels/slack/{account_id}/events",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let plugin = Arc::clone(&slack_events_plugin);
                    let gw_state = Arc::clone(&state_for_slack_events);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let p = plugin.read().await;
                            p.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Slack account" })),
                            )
                                .into_response();
                        };

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                crate::channel_webhook_middleware::rejection_into_response(
                                    rejection,
                                )
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Dispatch to Slack plugin with verified body.
                                let result = {
                                    let p = plugin.read().await;
                                    p.ingest_verified_webhook(&account_id, &verified.body)
                                        .await
                                };
                                match result {
                                    Ok(Some(challenge)) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "challenge": challenge })),
                                    )
                                        .into_response(),
                                    Ok(None) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "ok": true })),
                                    )
                                        .into_response(),
                                    Err(e) => {
                                        let msg = e.to_string();
                                        if msg.contains("unknown") {
                                            (
                                                StatusCode::NOT_FOUND,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        } else {
                                            (
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        }
                                    },
                                }
                            },
                        }
                    }
                },
            ),
        );

        // Slack interaction webhook -- receives button click payloads.
        let slack_interact_plugin = Arc::clone(&slack_webhook_plugin);
        let state_for_slack_interact = Arc::clone(&state);
        app = app.route(
            "/api/channels/slack/{account_id}/interactions",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let plugin = Arc::clone(&slack_interact_plugin);
                    let gw_state = Arc::clone(&state_for_slack_interact);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let p = plugin.read().await;
                            p.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Slack account" })),
                            )
                                .into_response();
                        };

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                crate::channel_webhook_middleware::rejection_into_response(
                                    rejection,
                                )
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Dispatch to Slack plugin with verified body.
                                let result = {
                                    let p = plugin.read().await;
                                    p.ingest_verified_interaction_webhook(
                                        &account_id,
                                        &verified.body,
                                    )
                                    .await
                                };
                                match result {
                                    Ok(()) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "ok": true })),
                                    )
                                        .into_response(),
                                    Err(e) => (
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
                                    )
                                        .into_response(),
                                }
                            },
                        }
                    }
                },
            ),
        );

        // Slack slash command webhook -- receives /command payloads.
        let slack_cmd_plugin = Arc::clone(&slack_webhook_plugin);
        let state_for_slack_cmd = Arc::clone(&state);
        app = app.route(
            "/api/channels/slack/{account_id}/commands",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let plugin = Arc::clone(&slack_cmd_plugin);
                    let gw_state = Arc::clone(&state_for_slack_cmd);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let p = plugin.read().await;
                            p.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Slack account" })),
                            )
                                .into_response();
                        };

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                crate::channel_webhook_middleware::rejection_into_response(
                                    rejection,
                                )
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => {
                                // Slash commands display the response body in
                                // Slack, so return an empty 200 for deduped
                                // requests instead of JSON the user would see.
                                StatusCode::OK.into_response()
                            },
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Dispatch to Slack plugin with verified body.
                                let result = {
                                    let p = plugin.read().await;
                                    p.ingest_verified_command_webhook(
                                        &account_id,
                                        &verified.body,
                                    )
                                    .await
                                };
                                match result {
                                    Ok(response_text) => (
                                        StatusCode::OK,
                                        response_text,
                                    )
                                        .into_response(),
                                    Err(e) => (
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
                                    )
                                        .into_response(),
                                }
                            },
                        }
                    }
                },
            ),
        );
    }

    // -- Generic webhook ingress ------------------------------------------------
    {
        fn webhook_cors_headers(mut resp: axum::response::Response) -> axum::response::Response {
            use axum::http::HeaderValue;
            let h = resp.headers_mut();
            h.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
            h.insert(
                axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("POST, OPTIONS"),
            );
            h.insert(axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type, Authorization, X-Hub-Signature-256, X-GitHub-Event, X-GitHub-Delivery, X-Gitlab-Token, X-Gitlab-Event, Stripe-Signature, X-Webhook-Secret, X-Event-Type, X-Delivery-Id, Idempotency-Key, Linear-Signature, X-PagerDuty-Signature, Sentry-Hook-Signature"));
            h.insert(
                axum::http::header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("86400"),
            );
            resp
        }

        // OPTIONS preflight handler.
        app = app.route(
            "/api/webhooks/ingest/{public_id}",
            axum::routing::options(move |_: axum::extract::Path<String>| async move {
                webhook_cors_headers(StatusCode::NO_CONTENT.into_response())
            }),
        );

        let state_for_webhook_ingest = Arc::clone(&state);
        app = app.route(
            "/api/webhooks/ingest/{public_id}",
            axum::routing::post(
                move |axum::extract::Path(public_id): axum::extract::Path<String>,
                      ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let gw = Arc::clone(&state_for_webhook_ingest);
                    async move {
                        // Extract remote IP. Behind a proxy, trust forwarded
                        // headers; otherwise use the real TCP peer address.
                        let remote_ip = if gw.behind_proxy {
                            headers
                                .get("x-forwarded-for")
                                .and_then(|v| v.to_str().ok())
                                .and_then(|v| v.split(',').next())
                                .map(|s| s.trim().to_string())
                                .or_else(|| {
                                    headers
                                        .get("x-real-ip")
                                        .and_then(|v| v.to_str().ok())
                                        .map(|s| s.trim().to_string())
                                })
                                .or_else(|| Some(peer.ip().to_string()))
                        } else {
                            Some(peer.ip().to_string())
                        };

                        let resp = async {
                            let Some(store) = gw.webhook_store.get() else {
                                return (
                                    StatusCode::NOT_FOUND,
                                    Json(serde_json::json!({ "error": "webhooks not configured" })),
                                )
                                    .into_response();
                            };

                            // Look up webhook by public_id.
                            let webhook = match store.get_webhook_by_public_id(&public_id).await {
                                Ok(w) if w.enabled => w,
                                Ok(_) => {
                                    return (
                                        StatusCode::NOT_FOUND,
                                        Json(serde_json::json!({ "error": "webhook not found" })),
                                    )
                                        .into_response();
                                },
                                Err(_) => {
                                    return (
                                        StatusCode::NOT_FOUND,
                                        Json(serde_json::json!({ "error": "webhook not found" })),
                                    )
                                        .into_response();
                                },
                            };

                            #[allow(unused_mut)]
                            // Secret decryption mutates the webhook only when the vault feature is enabled.
                            let mut webhook = webhook;

                            #[cfg(feature = "vault")]
                            if let Err(error) = moltis_gateway::webhooks::decrypt_webhook_secrets(
                                &mut webhook,
                                gw.vault.as_ref(),
                            )
                            .await
                            {
                                tracing::warn!(
                                    public_id = %webhook.public_id,
                                    error = %error,
                                    "webhook secrets unavailable for runtime verification"
                                );
                                return (
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    Json(serde_json::json!({
                                        "error": "webhook secrets unavailable",
                                    })),
                                )
                                    .into_response();
                            }

                            // Check CIDR allowlist (before auth to avoid timing side-channels).
                            if !webhook.allowed_cidrs.is_empty() {
                                let allowed = match &remote_ip {
                                    Some(ip) => {
                                        if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
                                            webhook.allowed_cidrs.iter().any(|cidr| {
                                                cidr.parse::<ipnet::IpNet>()
                                                    .map(|net| net.contains(&addr))
                                                    .unwrap_or_else(|_| {
                                                        // Fall back to exact string match.
                                                        cidr == ip
                                                    })
                                            })
                                        } else {
                                            // IP couldn't be parsed -- no match.
                                            false
                                        }
                                    },
                                    None => false, // No IP available -- can't match allowlist.
                                };
                                if !allowed {
                                    return (
                                        StatusCode::FORBIDDEN,
                                        Json(serde_json::json!({ "error": "IP not in allowlist" })),
                                    )
                                        .into_response();
                                }
                            }

                            // Check body size limit.
                            if body.len() > webhook.max_body_bytes {
                                return (
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                    Json(serde_json::json!({
                                        "error": "payload too large",
                                        "maxBytes": webhook.max_body_bytes,
                                    })),
                                )
                                    .into_response();
                            }

                            // Verify authentication.
                            if let Err(e) = moltis_webhooks::auth::verify(
                                &webhook.auth_mode,
                                webhook.auth_config.as_ref(),
                                &headers,
                                &body,
                            ) {
                                tracing::warn!(
                                    webhook_id = webhook.id,
                                    public_id = %webhook.public_id,
                                    error = %e,
                                    "webhook auth verification failed"
                                );
                                return (
                                    StatusCode::UNAUTHORIZED,
                                    Json(serde_json::json!({ "error": "authentication failed" })),
                                )
                                    .into_response();
                            }

                            // Parse event type and delivery key from source profile.
                            let profile_registry =
                                moltis_webhooks::profiles::ProfileRegistry::new();
                            let profile = profile_registry.get(&webhook.source_profile);
                            let event_type =
                                profile.and_then(|p| p.parse_event_type(&headers, &body));
                            let delivery_key =
                                profile.and_then(|p| p.parse_delivery_key(&headers, &body));

                            // Check event filter.
                            if let Some(ref et) = event_type
                                && !webhook.event_filter.accepts(et)
                            {
                                return (
                                    StatusCode::OK,
                                    Json(serde_json::json!({
                                        "status": "filtered",
                                        "eventType": et,
                                    })),
                                )
                                    .into_response();
                            }

                            // Check rate limit.
                            if !gw
                                .webhook_rate_limiter
                                .check(webhook.id, webhook.rate_limit_per_minute)
                            {
                                return (
                                    StatusCode::TOO_MANY_REQUESTS,
                                    Json(serde_json::json!({ "error": "rate limited" })),
                                )
                                    .into_response();
                            }

                            // Dedup check.
                            if let Some(ref dk) = delivery_key {
                                match moltis_webhooks::dedup::check_duplicate(
                                    store.as_ref(),
                                    webhook.id,
                                    Some(dk.as_str()),
                                )
                                .await
                                {
                                    Ok(Some(existing_id)) => {
                                        return (
                                            StatusCode::OK,
                                            Json(serde_json::json!({
                                                "status": "deduplicated",
                                                "existingDeliveryId": existing_id,
                                            })),
                                        )
                                            .into_response();
                                    },
                                    Ok(None) => { /* new delivery, continue */ },
                                    Err(e) => {
                                        tracing::error!(
                                            webhook_id = webhook.id,
                                            error = %e,
                                            "dedup check failed"
                                        );
                                        // Continue despite dedup error -- better to
                                        // accept a potential duplicate than reject.
                                    },
                                }
                            }

                            // Build timestamp.
                            let received_at = time::OffsetDateTime::now_utc()
                                .format(&time::format_description::well_known::Rfc3339)
                                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into());

                            // Extract entity key.
                            let entity_key = if let (Some(p), Some(et)) = (profile, &event_type) {
                                let body_val: serde_json::Value =
                                    serde_json::from_slice(&body).unwrap_or_default();
                                p.entity_key(et, &body_val)
                            } else {
                                None
                            };

                            // Extract safe headers for audit logging.
                            let safe_headers =
                                moltis_webhooks::normalize::extract_safe_headers(&headers);
                            let headers_json = serde_json::to_string(&safe_headers).ok();

                            let content_type = headers
                                .get("content-type")
                                .and_then(|v| v.to_str().ok())
                                .map(String::from);

                            // Persist delivery.
                            let delivery = moltis_webhooks::store::NewDelivery {
                                webhook_id: webhook.id,
                                received_at: received_at.clone(),
                                status: moltis_webhooks::types::DeliveryStatus::Queued,
                                event_type: event_type.clone(),
                                entity_key,
                                delivery_key,
                                http_method: Some("POST".into()),
                                content_type,
                                remote_ip: remote_ip.clone(),
                                headers_json,
                                body_size: body.len(),
                                body_blob: Some(body.to_vec()),
                                rejection_reason: None,
                            };

                            let delivery_id = match store.insert_delivery(&delivery).await {
                                Ok(id) => id,
                                Err(e) => {
                                    tracing::error!(
                                        webhook_id = webhook.id,
                                        error = %e,
                                        "failed to persist webhook delivery"
                                    );
                                    return (
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        Json(serde_json::json!({
                                            "error": "failed to persist delivery"
                                        })),
                                    )
                                        .into_response();
                                },
                            };

                            // Update denormalized delivery count.
                            if let Err(e) = store
                                .increment_delivery_count(webhook.id, &received_at)
                                .await
                            {
                                tracing::warn!(
                                    webhook_id = webhook.id,
                                    error = %e,
                                    "failed to increment delivery count"
                                );
                            }

                            // Queue for async processing.
                            if let Some(tx) = gw.webhook_worker_tx.get()
                                && let Err(e) = tx.send(delivery_id).await
                            {
                                tracing::error!(
                                    delivery_id,
                                    error = %e,
                                    "failed to queue webhook delivery for processing"
                                );
                            }

                            (
                                StatusCode::ACCEPTED,
                                Json(serde_json::json!({
                                    "deliveryId": delivery_id,
                                    "status": "queued",
                                    "webhookId": webhook.public_id,
                                    "eventType": event_type,
                                    "receivedAt": received_at,
                                })),
                            )
                                .into_response()
                        }
                        .await;
                        webhook_cors_headers(resp)
                    }
                },
            ),
        );
    }

    let method_count = methods.method_names().len();

    super::runtime::finalize_prepared_gateway(FinalizeGatewayArgs {
        bind,
        port,
        tls_enabled_for_gateway,
        state,
        browser_for_lifecycle,
        browser_tool_for_warmup,
        sandbox_router,
        cron_service,
        log_buffer,
        config,
        data_dir,
        provider_summary,
        mcp_configured_count,
        method_count,
        openclaw_startup_status,
        setup_code_display,
        webauthn_registry,
        #[cfg(feature = "ngrok")]
        ngrok_controller,
        #[cfg(feature = "trusted-network")]
        audit_buffer_for_broadcast,
        #[cfg(feature = "trusted-network")]
        _proxy_shutdown_tx,
        #[cfg(feature = "tailscale")]
        tailscale_mode,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit,
        app,
    })
    .await
}
