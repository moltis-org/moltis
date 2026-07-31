use moltis_protocol::{ErrorShape, error_codes};

use crate::broadcast::{BroadcastOpts, broadcast};

use super::MethodRegistry;

/// Helper to get the pairing store, falling back to in-memory state.
fn get_pairing_store(
    state: &crate::state::GatewayState,
) -> Option<&std::sync::Arc<crate::pairing::PairingStore>> {
    state.pairing_store.as_ref()
}

#[derive(serde::Deserialize)]
struct PairVerifyParams {
    id: String,
    signature: String,
}

async fn find_pending_pair_request(
    state: &crate::state::GatewayState,
    id: &str,
) -> Result<crate::pairing::PairRequest, ErrorShape> {
    let request = if let Some(store) = get_pairing_store(state) {
        store
            .list_pending()
            .await
            .map_err(|error| ErrorShape::new(error_codes::INTERNAL, error.to_string()))?
            .into_iter()
            .find(|request| request.id == id)
    } else {
        state
            .inner
            .read()
            .await
            .pairing
            .list_pending()
            .into_iter()
            .find(|request| request.id == id)
            .cloned()
    };

    request.ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "pair request not found"))
}

async fn verify_pairing_challenge(
    state: &crate::state::GatewayState,
    params: serde_json::Value,
) -> Result<serde_json::Value, ErrorShape> {
    let params: PairVerifyParams = serde_json::from_value(params).map_err(|error| {
        ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!("invalid node.pair.verify params: {error}"),
        )
    })?;
    let request = find_pending_pair_request(state, &params.id).await?;
    let public_key = request.public_key.as_deref().ok_or_else(|| {
        ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "pair request has no public key",
        )
    })?;
    let verified = crate::pairing::verify_ed25519_challenge(
        public_key,
        request.nonce.as_bytes(),
        &params.signature,
    )
    .map_err(|error| ErrorShape::new(error_codes::INVALID_REQUEST, error))?;

    Ok(serde_json::json!({ "verified": verified }))
}

pub(super) fn register(reg: &mut MethodRegistry) {
    // node.pair.request
    reg.register(
        "node.pair.request",
        Box::new(|ctx| {
            Box::pin(async move {
                let device_id = ctx
                    .params
                    .get("deviceId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                    })?;
                let display_name = ctx.params.get("displayName").and_then(|v| v.as_str());
                let platform = ctx
                    .params
                    .get("platform")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let public_key = ctx.params.get("publicKey").and_then(|v| v.as_str());

                let (id, nonce, device_id_out, display_name_out, platform_out) =
                    if let Some(store) = get_pairing_store(&ctx.state) {
                        let req = store
                            .request_pair(device_id, display_name, platform, public_key)
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::INTERNAL, e.to_string()))?;
                        (
                            req.id,
                            req.nonce,
                            req.device_id,
                            req.display_name,
                            req.platform,
                        )
                    } else {
                        let req = ctx.state.inner.write().await.pairing.request_pair(
                            device_id,
                            display_name,
                            platform,
                            public_key,
                        );
                        (
                            req.id,
                            req.nonce,
                            req.device_id,
                            req.display_name,
                            req.platform,
                        )
                    };

                broadcast(
                    &ctx.state,
                    "node.pair.requested",
                    serde_json::json!({
                        "id": id,
                        "deviceId": device_id_out,
                        "displayName": display_name_out,
                        "platform": platform_out,
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({
                    "id": id,
                    "nonce": nonce,
                }))
            })
        }),
    );

    // node.pair.list
    reg.register(
        "node.pair.list",
        Box::new(|ctx| {
            Box::pin(async move {
                if let Some(store) = get_pairing_store(&ctx.state) {
                    let pending = store
                        .list_pending()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::INTERNAL, e.to_string()))?;
                    let list: Vec<_> = pending
                        .iter()
                        .map(|r| {
                            let fp = r
                                .public_key
                                .as_deref()
                                .and_then(|pk| crate::pairing::public_key_fingerprint(pk).ok());
                            serde_json::json!({
                                "id": r.id,
                                "deviceId": r.device_id,
                                "displayName": r.display_name,
                                "platform": r.platform,
                                "fingerprint": fp,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                } else {
                    let inner = ctx.state.inner.read().await;
                    let list: Vec<_> = inner
                        .pairing
                        .list_pending()
                        .iter()
                        .map(|r| {
                            let fp = r
                                .public_key
                                .as_deref()
                                .and_then(|pk| crate::pairing::public_key_fingerprint(pk).ok());
                            serde_json::json!({
                                "id": r.id,
                                "deviceId": r.device_id,
                                "displayName": r.display_name,
                                "platform": r.platform,
                                "fingerprint": fp,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                }
            })
        }),
    );

    // node.pair.approve
    reg.register(
        "node.pair.approve",
        Box::new(|ctx| {
            Box::pin(async move {
                let pair_id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing id"))?;

                let (token_str, scopes) = if let Some(store) = get_pairing_store(&ctx.state) {
                    let token = store.approve(pair_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                    (token.token, token.scopes)
                } else {
                    let token = ctx
                        .state
                        .inner
                        .write()
                        .await
                        .pairing
                        .approve(pair_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                    (token.token, token.scopes)
                };

                broadcast(
                    &ctx.state,
                    "node.pair.resolved",
                    serde_json::json!({
                        "id": pair_id, "status": "approved",
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({
                    "deviceToken": token_str,
                    "scopes": scopes,
                }))
            })
        }),
    );

    // node.pair.reject
    reg.register(
        "node.pair.reject",
        Box::new(|ctx| {
            Box::pin(async move {
                let pair_id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing id"))?;

                if let Some(store) = get_pairing_store(&ctx.state) {
                    store.reject(pair_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                } else {
                    ctx.state
                        .inner
                        .write()
                        .await
                        .pairing
                        .reject(pair_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                }

                broadcast(
                    &ctx.state,
                    "node.pair.resolved",
                    serde_json::json!({
                        "id": pair_id, "status": "rejected",
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({}))
            })
        }),
    );

    // node.pair.verify
    reg.register(
        "node.pair.verify",
        Box::new(|ctx| {
            Box::pin(async move { verify_pairing_challenge(&ctx.state, ctx.params).await })
        }),
    );

    // node.pairing.enable — open the gate for new node pairing requests.
    reg.register(
        "node.pairing.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .node_pairing_enabled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                tracing::info!("node pairing enabled");
                Ok(serde_json::json!({ "enabled": true }))
            })
        }),
    );

    // node.pairing.disable — close the gate.
    reg.register(
        "node.pairing.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .node_pairing_enabled
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                tracing::info!("node pairing disabled");
                Ok(serde_json::json!({ "enabled": false }))
            })
        }),
    );

    // node.pairing.status — check if pairing is enabled.
    reg.register(
        "node.pairing.status",
        Box::new(|ctx| {
            Box::pin(async move {
                let enabled = ctx
                    .state
                    .node_pairing_enabled
                    .load(std::sync::atomic::Ordering::Relaxed);
                Ok(serde_json::json!({ "enabled": enabled }))
            })
        }),
    );

    // device.pair.list
    reg.register(
        "device.pair.list",
        Box::new(|ctx| {
            Box::pin(async move {
                if let Some(store) = get_pairing_store(&ctx.state) {
                    let devices = store
                        .list_devices()
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::INTERNAL, e.to_string()))?;
                    let list: Vec<_> = devices
                        .iter()
                        .map(|d| {
                            let fp = d
                                .public_key
                                .as_deref()
                                .and_then(|pk| crate::pairing::public_key_fingerprint(pk).ok());
                            serde_json::json!({
                                "deviceId": d.device_id,
                                "displayName": d.display_name,
                                "platform": d.platform,
                                "publicKey": d.public_key,
                                "fingerprint": fp,
                                "createdAt": d.created_at,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                } else {
                    let inner = ctx.state.inner.read().await;
                    let list: Vec<_> = inner
                        .pairing
                        .list_devices()
                        .iter()
                        .map(|d| {
                            serde_json::json!({
                                "deviceId": d.device_id,
                                "scopes": d.scopes,
                                "issuedAtMs": d.issued_at_ms,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                }
            })
        }),
    );

    // device.pair.approve (alias for node.pair.approve)
    reg.register(
        "device.pair.approve",
        Box::new(|ctx| {
            Box::pin(async move {
                let pair_id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing id"))?;

                let (token_str, scopes) = if let Some(store) = get_pairing_store(&ctx.state) {
                    let token = store.approve(pair_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                    (token.token, token.scopes)
                } else {
                    let token = ctx
                        .state
                        .inner
                        .write()
                        .await
                        .pairing
                        .approve(pair_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                    (token.token, token.scopes)
                };

                broadcast(
                    &ctx.state,
                    "device.pair.resolved",
                    serde_json::json!({
                        "id": pair_id, "status": "approved",
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({ "deviceToken": token_str, "scopes": scopes }))
            })
        }),
    );

    // device.pair.reject
    reg.register(
        "device.pair.reject",
        Box::new(|ctx| {
            Box::pin(async move {
                let pair_id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing id"))?;

                if let Some(store) = get_pairing_store(&ctx.state) {
                    store.reject(pair_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                } else {
                    ctx.state
                        .inner
                        .write()
                        .await
                        .pairing
                        .reject(pair_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                }

                broadcast(
                    &ctx.state,
                    "device.pair.resolved",
                    serde_json::json!({
                        "id": pair_id, "status": "rejected",
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({}))
            })
        }),
    );

    // device.token.create — pre-authorize a device and issue a token directly
    reg.register(
        "device.token.create",
        Box::new(|ctx| {
            Box::pin(async move {
                let display_name = ctx.params.get("displayName").and_then(|v| v.as_str());
                let platform = ctx
                    .params
                    .get("platform")
                    .and_then(|v| v.as_str())
                    .unwrap_or("remote");

                let (token_str, device_id, scopes) =
                    if let Some(store) = get_pairing_store(&ctx.state) {
                        let token = store
                            .create_device_token(display_name, platform)
                            .await
                            .map_err(|e| ErrorShape::new(error_codes::INTERNAL, e.to_string()))?;
                        (token.token, token.device_id, token.scopes)
                    } else {
                        let token = ctx
                            .state
                            .inner
                            .write()
                            .await
                            .pairing
                            .create_device_token(display_name, platform);
                        (token.token, token.device_id, token.scopes)
                    };

                Ok(serde_json::json!({
                    "deviceToken": token_str,
                    "deviceId": device_id,
                    "scopes": scopes,
                }))
            })
        }),
    );

    // device.token.rotate
    reg.register(
        "device.token.rotate",
        Box::new(|ctx| {
            Box::pin(async move {
                let device_id = ctx
                    .params
                    .get("deviceId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                    })?;

                let (token_str, scopes) = if let Some(store) = get_pairing_store(&ctx.state) {
                    let token = store.rotate_token(device_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                    (token.token, token.scopes)
                } else {
                    let token = ctx
                        .state
                        .inner
                        .write()
                        .await
                        .pairing
                        .rotate_token(device_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                    (token.token, token.scopes)
                };

                Ok(serde_json::json!({ "deviceToken": token_str, "scopes": scopes }))
            })
        }),
    );

    // device.token.revoke
    reg.register(
        "device.token.revoke",
        Box::new(|ctx| {
            Box::pin(async move {
                let device_id = ctx
                    .params
                    .get("deviceId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing deviceId")
                    })?;

                if let Some(store) = get_pairing_store(&ctx.state) {
                    store.revoke_token(device_id).await.map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                } else {
                    ctx.state
                        .inner
                        .write()
                        .await
                        .pairing
                        .revoke_token(device_id)
                        .map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                }

                Ok(serde_json::json!({}))
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            auth::{AuthMode, ResolvedAuth},
            methods::MethodContext,
            pairing::{PairRequest, PairingStore},
            services::GatewayServices,
            state::GatewayState,
        },
        base64::Engine,
        ed25519_dalek::{Signer, SigningKey},
        moltis_protocol::ResponseFrame,
    };

    fn test_state() -> std::sync::Arc<GatewayState> {
        GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            GatewayServices::noop(),
        )
    }

    fn test_state_with_pairing_store(
        pairing_store: std::sync::Arc<PairingStore>,
    ) -> std::sync::Arc<GatewayState> {
        let mut state = test_state();
        let Some(unique_state) = std::sync::Arc::get_mut(&mut state) else {
            panic!("test state should be uniquely owned");
        };
        unique_state.pairing_store = Some(pairing_store);
        state
    }

    fn public_key(signing_key: &SigningKey) -> String {
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes())
    }

    fn signature(nonce: &str, signing_key: &SigningKey) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(signing_key.sign(nonce.as_bytes()).to_bytes())
    }

    fn signed_params(request: &PairRequest, signing_key: &SigningKey) -> serde_json::Value {
        serde_json::json!({
            "id": request.id,
            "signature": signature(&request.nonce, signing_key),
        })
    }

    async fn request_pair(
        state: &GatewayState,
        device_id: &str,
        public_key: Option<&str>,
    ) -> PairRequest {
        state.inner.write().await.pairing.request_pair(
            device_id,
            Some("Test node"),
            "test",
            public_key,
        )
    }

    async fn dispatch_pair_verify(
        state: std::sync::Arc<GatewayState>,
        params: serde_json::Value,
    ) -> ResponseFrame {
        MethodRegistry::new()
            .dispatch(MethodContext {
                request_id: String::from("test"),
                method: String::from("node.pair.verify"),
                params,
                client_conn_id: String::from("conn-1"),
                client_role: String::from("operator"),
                client_scopes: vec![String::from("operator.pairing")],
                state,
                channel: None,
            })
            .await
    }

    fn assert_invalid_request(response: &ResponseFrame) {
        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::INVALID_REQUEST)
        );
    }

    #[tokio::test]
    async fn pair_verify_accepts_valid_signature() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = public_key(&signing_key);
        let request = request_pair(&state, "valid-node", Some(&public_key)).await;

        let response = dispatch_pair_verify(state, signed_params(&request, &signing_key)).await;

        assert!(response.ok);
        assert_eq!(
            response.payload,
            Some(serde_json::json!({ "verified": true }))
        );
    }

    #[tokio::test]
    async fn pair_verify_accepts_valid_signature_from_sqlite_store() -> anyhow::Result<()> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
        crate::run_migrations(&pool).await?;
        let pairing_store = std::sync::Arc::new(PairingStore::new(pool));
        let state = test_state_with_pairing_store(std::sync::Arc::clone(&pairing_store));
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = public_key(&signing_key);
        let request = pairing_store
            .request_pair(
                "sqlite-valid-node",
                Some("SQLite test node"),
                "test",
                Some(&public_key),
            )
            .await?;
        let pending = pairing_store.list_pending().await?;
        assert_eq!(pending.len(), 1);
        let persisted = &pending[0];
        assert_eq!(persisted.device_id, "sqlite-valid-node");
        assert_eq!(persisted.display_name.as_deref(), Some("SQLite test node"));
        assert_eq!(persisted.platform, "test");
        assert_eq!(persisted.public_key.as_deref(), Some(public_key.as_str()));
        assert_eq!(persisted.nonce, request.nonce);

        let response = dispatch_pair_verify(state, signed_params(&request, &signing_key)).await;

        assert!(response.ok);
        assert_eq!(
            response.payload,
            Some(serde_json::json!({ "verified": true }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn pair_verify_rejects_expired_request_from_sqlite_store() -> anyhow::Result<()> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
        crate::run_migrations(&pool).await?;
        let pairing_store = std::sync::Arc::new(PairingStore::new(pool.clone()));
        let state = test_state_with_pairing_store(std::sync::Arc::clone(&pairing_store));
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = public_key(&signing_key);
        let request = pairing_store
            .request_pair(
                "sqlite-expired-node",
                Some("Expired SQLite test node"),
                "test",
                Some(&public_key),
            )
            .await?;
        sqlx::query("UPDATE pair_requests SET expires_at = ? WHERE id = ?")
            .bind("2000-01-01T00:00:00Z")
            .bind(&request.id)
            .execute(&pool)
            .await?;

        let response = dispatch_pair_verify(state, signed_params(&request, &signing_key)).await;

        assert_invalid_request(&response);
        Ok(())
    }

    #[tokio::test]
    async fn pair_verify_returns_false_for_wrong_signature_and_cross_request_nonce() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let wrong_signing_key = SigningKey::from_bytes(&[8; 32]);
        let public_key = public_key(&signing_key);
        let first_request = request_pair(&state, "first-node", Some(&public_key)).await;
        let second_request = request_pair(&state, "second-node", Some(&public_key)).await;

        let wrong_signature_response = dispatch_pair_verify(
            std::sync::Arc::clone(&state),
            signed_params(&first_request, &wrong_signing_key),
        )
        .await;
        let cross_request_response = dispatch_pair_verify(
            state,
            serde_json::json!({
                "id": second_request.id,
                "signature": signature(&first_request.nonce, &signing_key),
            }),
        )
        .await;

        assert!(wrong_signature_response.ok);
        assert_eq!(
            wrong_signature_response.payload,
            Some(serde_json::json!({ "verified": false }))
        );
        assert!(cross_request_response.ok);
        assert_eq!(
            cross_request_response.payload,
            Some(serde_json::json!({ "verified": false }))
        );
    }

    #[tokio::test]
    async fn pair_verify_fails_closed_when_params_are_missing() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = public_key(&signing_key);
        let request = request_pair(&state, "missing-param-node", Some(&public_key)).await;

        for field in ["id", "signature"] {
            let mut params = signed_params(&request, &signing_key);
            if let Some(params) = params.as_object_mut() {
                params.remove(field);
            }

            assert_invalid_request(
                &dispatch_pair_verify(std::sync::Arc::clone(&state), params).await,
            );
        }
    }

    #[tokio::test]
    async fn pair_verify_fails_closed_when_params_are_malformed() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = public_key(&signing_key);
        let request = request_pair(&state, "malformed-param-node", Some(&public_key)).await;
        let malformed_params = [
            serde_json::json!({ "id": 42, "signature": signature(&request.nonce, &signing_key) }),
            serde_json::json!({ "id": request.id, "signature": "not base64" }),
            serde_json::json!({
                "id": request.id,
                "signature": base64::engine::general_purpose::STANDARD.encode([0; 63]),
            }),
        ];

        for params in malformed_params {
            assert_invalid_request(
                &dispatch_pair_verify(std::sync::Arc::clone(&state), params).await,
            );
        }
    }

    #[tokio::test]
    async fn pair_verify_rejects_unknown_request() {
        let state = test_state();
        let signature = base64::engine::general_purpose::STANDARD.encode([0; 64]);

        let response = dispatch_pair_verify(
            state,
            serde_json::json!({ "id": "unknown", "signature": signature }),
        )
        .await;

        assert_invalid_request(&response);
    }

    #[tokio::test]
    async fn pair_verify_rejects_request_without_public_key() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let request = request_pair(&state, "no-key-node", None).await;

        let response = dispatch_pair_verify(state, signed_params(&request, &signing_key)).await;

        assert_invalid_request(&response);
    }

    #[tokio::test]
    async fn pair_verify_rejects_request_with_malformed_public_key() {
        let state = test_state();
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let request = request_pair(&state, "bad-key-node", Some("not base64")).await;

        let response = dispatch_pair_verify(state, signed_params(&request, &signing_key)).await;

        assert_invalid_request(&response);
    }
}
