//! OAuth flow management — `oauth_start`, `oauth_complete`, `oauth_status`,
//! and device-flow implementations.

use std::sync::Arc;

use {serde_json::Value, tracing::info};

use {
    moltis_oauth::{
        CallbackServer, OAuthFlow, callback_port, device_flow, load_oauth_config,
        normalize_loopback_redirect,
    },
    moltis_providers::ProviderRegistry,
    moltis_service_traits::{ServiceError, ServiceResult},
};

use {
    super::{LiveProviderSetupService, support::PendingOAuthFlow},
    crate::{
        config_helpers::set_provider_enabled_in_config,
        oauth::{
            build_provider_headers, build_verification_uri_complete, has_oauth_tokens,
            normalize_loaded_redirect_uri,
        },
    },
};

impl LiveProviderSetupService {
    /// Start a device-flow OAuth for providers like GitHub Copilot.
    /// Returns `{ "userCode": "...", "verificationUri": "..." }` for the UI to display.
    async fn oauth_start_device_flow(
        &self,
        provider_name: String,
        oauth_config: moltis_oauth::OAuthConfig,
    ) -> ServiceResult {
        let client = reqwest::Client::new();
        let extra_headers = build_provider_headers(&provider_name);
        let device_resp = device_flow::request_device_code_with_headers(
            &client,
            &oauth_config,
            extra_headers.as_ref(),
        )
        .await
        .map_err(ServiceError::message)?;

        let user_code = device_resp.user_code.clone();
        let verification_uri = device_resp.verification_uri.clone();
        let verification_uri_complete = build_verification_uri_complete(
            &provider_name,
            &verification_uri,
            &user_code,
            device_resp.verification_uri_complete.clone(),
        );
        let device_code = device_resp.device_code.clone();
        let interval = device_resp.interval;

        // Spawn background task to poll for the token
        let token_store = self.token_store.clone();
        let registry = Arc::clone(&self.registry);
        let config = self.effective_config();
        let env_overrides = self.env_overrides.clone();
        let poll_headers = extra_headers.clone();
        tokio::spawn(async move {
            let poll_extra = poll_headers.as_ref();
            match device_flow::poll_for_token_with_headers(
                &client,
                &oauth_config,
                &device_code,
                interval,
                poll_extra,
            )
            .await
            {
                Ok(tokens) => {
                    if let Err(e) = token_store.save(&provider_name, &tokens) {
                        tracing::error!(
                            provider = %provider_name,
                            error = %e,
                            "failed to save device-flow OAuth tokens"
                        );
                        return;
                    }
                    let new_registry = ProviderRegistry::from_env_with_config_and_overrides(
                        &config,
                        &env_overrides,
                    );
                    let provider_summary = new_registry.provider_summary();
                    let model_count = new_registry.list_models().len();
                    let mut reg = registry.write().await;
                    *reg = new_registry;
                    info!(
                        provider = %provider_name,
                        provider_summary = %provider_summary,
                        models = model_count,
                        "device-flow OAuth complete, rebuilt provider registry"
                    );
                },
                Err(e) => {
                    tracing::error!(
                        provider = %provider_name,
                        error = %e,
                        "device-flow OAuth polling failed"
                    );
                },
            }
        });

        Ok(serde_json::json!({
            "deviceFlow": true,
            "userCode": user_code,
            "verificationUri": verification_uri,
            "verificationUriComplete": verification_uri_complete,
        }))
    }

    pub(super) async fn oauth_start_inner(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?
            .to_string();

        // RFC 8252 S7.3/S8.3: loopback redirect URIs must use `http`.
        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(normalize_loopback_redirect);

        let mut oauth_config = load_oauth_config(&provider_name)
            .ok_or_else(|| format!("no OAuth config for provider: {provider_name}"))?;

        normalize_loaded_redirect_uri(&mut oauth_config);

        // User explicitly initiated OAuth for this provider; ensure it is enabled.
        set_provider_enabled_in_config(&provider_name, true)?;
        self.set_provider_enabled_in_memory(&provider_name, true);

        // If tokens already exist, skip launching a fresh OAuth flow.
        if has_oauth_tokens(&provider_name, &self.token_store) {
            let effective = self.effective_config();
            let new_registry = self.build_registry(&effective);
            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            let mut reg = self.registry.write().await;
            *reg = new_registry;
            info!(
                provider = %provider_name,
                provider_summary = %provider_summary,
                models = model_count,
                "oauth start skipped because provider already has tokens; rebuilt provider registry"
            );
            return Ok(serde_json::json!({
                "alreadyAuthenticated": true,
            }));
        }

        if oauth_config.device_flow {
            return self
                .oauth_start_device_flow(provider_name, oauth_config)
                .await;
        }

        let has_registered_redirect = !oauth_config.redirect_uri.is_empty();
        let use_server_callback = redirect_uri.is_some() && !has_registered_redirect;
        if !has_registered_redirect && let Some(uri) = redirect_uri {
            oauth_config.redirect_uri = uri;
        }

        let port = callback_port(&oauth_config);
        let oauth_config_for_pending = oauth_config.clone();
        let flow = OAuthFlow::new(oauth_config);
        let auth_req = flow.start().map_err(ServiceError::message)?;

        let auth_url = auth_req.url.clone();
        let verifier = auth_req.pkce.verifier.clone();
        let expected_state = auth_req.state.clone();

        let pending = PendingOAuthFlow {
            provider_name: provider_name.clone(),
            oauth_config: oauth_config_for_pending,
            verifier: verifier.clone(),
        };
        self.pending_oauth
            .write()
            .await
            .insert(expected_state.clone(), pending);

        if use_server_callback {
            return Ok(serde_json::json!({
                "authUrl": auth_url,
            }));
        }

        // Spawn background task to wait for the callback and exchange the code
        let token_store = self.token_store.clone();
        let registry = Arc::clone(&self.registry);
        let config = self.effective_config();
        let env_overrides = self.env_overrides.clone();
        let bind_addr = self.callback_bind_addr.clone();
        let pending_oauth = Arc::clone(&self.pending_oauth);
        let callback_state = expected_state.clone();
        tokio::spawn(async move {
            match CallbackServer::wait_for_code(port, callback_state, &bind_addr).await {
                Ok(code) => {
                    let state_is_pending = pending_oauth
                        .write()
                        .await
                        .remove(&expected_state)
                        .is_some();
                    if !state_is_pending {
                        tracing::debug!(
                            provider = %provider_name,
                            "OAuth callback received after flow was already completed manually"
                        );
                        return;
                    }

                    match flow.exchange(&code, &verifier).await {
                        Ok(tokens) => {
                            if let Err(e) = token_store.save(&provider_name, &tokens) {
                                tracing::error!(
                                    provider = %provider_name,
                                    error = %e,
                                    "failed to save OAuth tokens"
                                );
                                return;
                            }
                            // Rebuild registry with new tokens
                            let new_registry = ProviderRegistry::from_env_with_config_and_overrides(
                                &config,
                                &env_overrides,
                            );
                            let provider_summary = new_registry.provider_summary();
                            let model_count = new_registry.list_models().len();
                            let mut reg = registry.write().await;
                            *reg = new_registry;
                            info!(
                                provider = %provider_name,
                                provider_summary = %provider_summary,
                                models = model_count,
                                "OAuth flow complete, rebuilt provider registry"
                            );
                        },
                        Err(e) => {
                            tracing::error!(
                                provider = %provider_name,
                                error = %e,
                                "OAuth token exchange failed"
                            );
                        },
                    }
                },
                Err(e) => {
                    // Ignore callback timeout/noise after successful manual completion.
                    if pending_oauth.read().await.get(&expected_state).is_none() {
                        tracing::debug!(
                            provider = %provider_name,
                            error = %e,
                            "OAuth callback wait ended after flow was completed elsewhere"
                        );
                        return;
                    }
                    tracing::error!(
                        provider = %provider_name,
                        error = %e,
                        "OAuth callback failed"
                    );
                },
            }
        });

        Ok(serde_json::json!({
            "authUrl": auth_url,
        }))
    }

    pub(super) async fn oauth_complete_inner(&self, params: Value) -> ServiceResult {
        let parsed_callback = params
            .get("callback")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(moltis_oauth::parse_callback_input)
            .transpose()
            .map_err(ServiceError::message)?;

        let code = params
            .get("code")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| parsed_callback.as_ref().map(|parsed| parsed.code.clone()))
            .ok_or_else(|| "missing 'code' parameter".to_string())?;
        let state = params
            .get("state")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| parsed_callback.as_ref().map(|parsed| parsed.state.clone()))
            .ok_or_else(|| "missing 'state' parameter".to_string())?;
        let requested_provider = params
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let pending = {
            let mut pending_oauth = self.pending_oauth.write().await;
            let pending = pending_oauth
                .get(&state)
                .cloned()
                .ok_or_else(|| "unknown or expired OAuth state".to_string())?;

            if let Some(provider) = requested_provider.as_deref()
                && provider != pending.provider_name
            {
                return Err(ServiceError::message(format!(
                    "provider mismatch for OAuth state: expected '{}', got '{}'",
                    pending.provider_name, provider
                )));
            }

            pending_oauth
                .remove(&state)
                .ok_or_else(|| "unknown or expired OAuth state".to_string())?
        };

        let flow = OAuthFlow::new(pending.oauth_config);
        let tokens = flow
            .exchange(&code, &pending.verifier)
            .await
            .map_err(ServiceError::message)?;

        self.token_store
            .save(&pending.provider_name, &tokens)
            .map_err(ServiceError::message)?;
        set_provider_enabled_in_config(&pending.provider_name, true)?;
        self.set_provider_enabled_in_memory(&pending.provider_name, true);

        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = %pending.provider_name,
            provider_summary = %provider_summary,
            models = model_count,
            "OAuth callback complete, rebuilt provider registry"
        );

        Ok(serde_json::json!({
            "ok": true,
            "provider": pending.provider_name,
        }))
    }

    pub(super) async fn oauth_status_inner(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let has_tokens = has_oauth_tokens(provider_name, &self.token_store);
        Ok(serde_json::json!({
            "provider": provider_name,
            "authenticated": has_tokens,
        }))
    }
}
