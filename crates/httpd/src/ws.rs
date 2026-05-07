use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use {
    axum::extract::ws::{Message, WebSocket},
    futures::{SinkExt, stream::StreamExt},
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

use {
    moltis_gateway::auth::{AuthIdentity, AuthMethod},
    moltis_protocol::{
        ConnectParams, ConnectParamsV4, ErrorShape, EventFrame, Extensions, Features, GatewayFrame,
        HANDSHAKE_TIMEOUT_MS, HelloAuth, HelloOk, KNOWN_EVENTS, MAX_PAYLOAD_BYTES,
        PROTOCOL_VERSION, Policy, ResponseFrame, ServerInfo, error_codes, roles, scopes,
    },
};

use moltis_gateway::{
    auth,
    broadcast::{BroadcastOpts, broadcast},
    methods::{MethodContext, MethodRegistry},
    nodes::NodeSession,
    pairing::{
        PinPublicKeyResult, generate_challenge_nonce, public_key_fingerprint,
        verify_ed25519_challenge,
    },
    state::{ConnectedClient, GatewayState},
};

fn top_level_param_keys(params: &Option<serde_json::Value>) -> Vec<String> {
    params
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

/// Handle a single WebSocket connection through its full lifecycle:
/// handshake (with auth) → message loop → cleanup.
pub async fn handle_connection(
    socket: WebSocket,
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    remote_addr: SocketAddr,
    accept_language: Option<String>,
    remote_ip: Option<String>,
    header_identity: Option<AuthIdentity>,
    is_local: bool,
) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    let conn_remote_ip = remote_addr.ip().to_string();
    debug!(conn_id = %conn_id, remote_ip = %conn_remote_ip, "ws: new connection");

    let (mut ws_tx, mut ws_rx) = socket.split();
    // Bounded channel prevents unbounded memory growth from slow clients.
    let (client_tx, mut client_rx) = mpsc::channel::<String>(512);

    // Spawn write loop: forwards frames from the client_tx channel to the WebSocket.
    let write_conn_id = conn_id.clone();
    let write_handle = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                debug!(conn_id = %write_conn_id, "ws: write loop closed");
                break;
            }
        }
    });

    // ── Handshake phase ──────────────────────────────────────────────────

    let connect_result = match tokio::time::timeout(
        std::time::Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
        wait_for_connect(&mut ws_rx),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            warn!(conn_id = %conn_id, error = %e, "ws: handshake failed");
            graceful_writer_shutdown(client_tx, write_handle).await;
            return;
        },
        Err(_) => {
            warn!(conn_id = %conn_id, "ws: handshake timeout");
            graceful_writer_shutdown(client_tx, write_handle).await;
            return;
        },
    };

    let ConnectResult {
        request_id,
        params,
        is_v4,
    } = connect_result;

    if state.ws_request_logs {
        let connect_param_keys = serde_json::to_value(&params)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        info!(
            conn_id = %conn_id,
            request_id = %request_id,
            method = "connect",
            param_keys = ?connect_param_keys,
            "ws: received request frame"
        );
    }

    // Validate protocol version.
    if params.min_protocol > PROTOCOL_VERSION || params.max_protocol < PROTOCOL_VERSION {
        warn!(
            conn_id = %conn_id,
            min_protocol = params.min_protocol,
            max_protocol = params.max_protocol,
            server_protocol = PROTOCOL_VERSION,
            "ws: protocol mismatch"
        );
        let err = ResponseFrame::err(
            &request_id,
            ErrorShape::new(
                error_codes::PROTOCOL_ERROR,
                format!(
                    "protocol mismatch: server={}, client={}-{}",
                    PROTOCOL_VERSION, params.min_protocol, params.max_protocol
                ),
            ),
        );
        #[allow(clippy::unwrap_used)] // serializing known-valid struct
        let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
        graceful_writer_shutdown(client_tx, write_handle).await;
        return;
    }

    // ── Auth validation ──────────────────────────────────────────────────
    // SECURITY: Three-tier auth model (see docs/src/security.md):
    //
    // 1. Password set → always require credentials, any IP.
    // 2. No password + genuine local connection → full access (dev convenience).
    // 3. No password + remote/proxied → onboarding only.
    //
    // `is_local` is computed per-request by `is_local_connection()` using:
    //   - MOLTIS_BEHIND_PROXY env var (hard override)
    //   - Proxy header detection (X-Forwarded-For, X-Real-IP, etc.)
    //   - Host header loopback check
    //   - TCP source IP loopback check
    //
    // See CVE-2026-25253 for the analogous OpenClaw vulnerability.
    let mut authenticated = header_identity.is_some();
    // Scopes from API key verification (if any).
    // When authenticated via HTTP header (cookie/bearer), scopes come from
    // the AuthIdentity. API-key scopes are non-empty; password/loopback
    // scopes are empty (= full access).
    let mut api_key_scopes: Option<Vec<String>> = header_identity
        .filter(|id| id.method == AuthMethod::ApiKey)
        .map(|id| id.scopes);
    // Device token verification result (if any).
    let mut device_token_device_id: Option<String> = None;

    // Check Ed25519 public key first (new challenge-response auth for nodes).
    if !authenticated
        && let Some(ref pk_b64) = params.auth.as_ref().and_then(|a| a.public_key.clone())
        && let Some(ref store) = state.pairing_store
    {
        let fingerprint = match public_key_fingerprint(pk_b64) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                warn!(
                    conn_id = %conn_id,
                    error = %error,
                    "ws: rejecting malformed Ed25519 public key"
                );
                let err = ResponseFrame::err(
                    &request_id,
                    ErrorShape::new(error_codes::INVALID_REQUEST, "invalid Ed25519 public key"),
                );
                #[allow(clippy::unwrap_used)]
                let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                graceful_writer_shutdown(client_tx, write_handle).await;
                return;
            },
        };

        // Check if this key has been revoked.
        if store.is_public_key_revoked(pk_b64).await.unwrap_or(false) {
            warn!(
                conn_id = %conn_id,
                fingerprint = %fingerprint,
                "ws: Ed25519 public key is revoked"
            );
            let err = ResponseFrame::err(
                &request_id,
                ErrorShape::new(error_codes::FORBIDDEN, "public key has been revoked"),
            );
            #[allow(clippy::unwrap_used)]
            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
            graceful_writer_shutdown(client_tx, write_handle).await;
            return;
        }

        // TOFU key pinning: reject if this device_id has been revoked or has a different key.
        match store.check_key_pinning(&params.client.id, pk_b64).await {
            Ok(moltis_gateway::pairing::KeyPinningResult::Revoked) => {
                warn!(
                    conn_id = %conn_id,
                    device_id = %params.client.id,
                    fingerprint = %fingerprint,
                    "ws: device has been revoked — rejecting re-pair attempt"
                );
                let err = ResponseFrame::err(
                    &request_id,
                    ErrorShape::new(
                        error_codes::FORBIDDEN,
                        "device has been revoked — contact the gateway operator",
                    ),
                );
                #[allow(clippy::unwrap_used)]
                let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                graceful_writer_shutdown(client_tx, write_handle).await;
                return;
            },
            Ok(moltis_gateway::pairing::KeyPinningResult::Mismatch { expected }) => {
                let expected_fp =
                    public_key_fingerprint(&expected).unwrap_or_else(|_| "unknown".into());
                warn!(
                    conn_id = %conn_id,
                    device_id = %params.client.id,
                    expected_fingerprint = %expected_fp,
                    presented_fingerprint = %fingerprint,
                    "ws: TOFU key pinning violation — node presented a different key"
                );
                let err = ResponseFrame::err(
                    &request_id,
                    ErrorShape::new(
                        error_codes::FORBIDDEN,
                        format!(
                            "key mismatch: expected {expected_fp}, got {fingerprint} — \
                         re-key this node from the gateway UI if intentional"
                        ),
                    ),
                );
                #[allow(clippy::unwrap_used)]
                let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());

                // Broadcast security alert to operators.
                broadcast(
                    &state,
                    "node.security.key-mismatch",
                    serde_json::json!({
                        "deviceId": params.client.id,
                        "displayName": params.client.display_name,
                        "expectedFingerprint": expected_fp,
                        "presentedFingerprint": fingerprint,
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                graceful_writer_shutdown(client_tx, write_handle).await;
                return;
            },
            _ => {}, // Match or NoPinnedKey — proceed
        }

        match store.find_device_by_public_key(pk_b64).await {
            Ok(Some(device)) => {
                // Known key — send challenge, verify response.
                let (nonce_bytes, nonce_b64) = generate_challenge_nonce();
                let challenge_event = serde_json::json!({
                    "type": "event",
                    "event": "node.auth.challenge",
                    "payload": { "nonce": nonce_b64 }
                });
                #[allow(clippy::unwrap_used)]
                let _ = client_tx.try_send(serde_json::to_string(&challenge_event).unwrap());

                // Wait for challenge-response from node.
                // SECURITY: If the challenge fails for a known key, reject hard.
                // Do NOT fall through to token auth — a failed signature for a
                // known key means impersonation, not a migration scenario.
                match receive_challenge_response(&mut ws_rx, std::time::Duration::from_secs(10))
                    .await
                {
                    Ok(sig_b64) => match verify_ed25519_challenge(pk_b64, &nonce_bytes, &sig_b64) {
                        Ok(true) => {
                            authenticated = true;
                            device_token_device_id = Some(device.device_id.clone());
                            api_key_scopes = Some(vec![
                                "operator.read".into(),
                                "operator.write".into(),
                                "operator.approvals".into(),
                            ]);
                            info!(
                                conn_id = %conn_id,
                                device_id = %device.device_id,
                                fingerprint = %fingerprint,
                                "ws: authenticated via Ed25519 challenge-response"
                            );
                        },
                        Ok(false) | Err(_) => {
                            warn!(
                                conn_id = %conn_id,
                                fingerprint = %fingerprint,
                                device_id = %device.device_id,
                                "ws: Ed25519 challenge failed for known key — possible impersonation"
                            );
                            let err = ResponseFrame::err(
                                &request_id,
                                ErrorShape::new(
                                    error_codes::FORBIDDEN,
                                    "challenge-response verification failed",
                                ),
                            );
                            match serde_json::to_string(&err) {
                                Ok(serialized) => {
                                    let _ = client_tx.try_send(serialized);
                                },
                                Err(e) => {
                                    warn!(conn_id = %conn_id, error = %e, "ws: failed to serialize challenge-response error");
                                },
                            }
                            graceful_writer_shutdown(client_tx, write_handle).await;
                            return;
                        },
                    },
                    Err(e) => {
                        warn!(
                            conn_id = %conn_id,
                            error = %e,
                            fingerprint = %fingerprint,
                            "ws: challenge-response failed for known key"
                        );
                        let err = ResponseFrame::err(
                            &request_id,
                            ErrorShape::new(error_codes::FORBIDDEN, "challenge-response failed"),
                        );
                        #[allow(clippy::unwrap_used)]
                        let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                        graceful_writer_shutdown(client_tx, write_handle).await;
                        return;
                    },
                }
            },
            Ok(None) => {
                // Unknown key. If a device_token is also present, skip the
                // pairing flow entirely — fall through to token auth, which
                // will pin the key during verification (upgrade-auth path).
                let has_device_token = params
                    .auth
                    .as_ref()
                    .and_then(|a| a.device_token.as_ref())
                    .is_some_and(|t| !t.is_empty());

                if has_device_token {
                    debug!(
                        conn_id = %conn_id,
                        fingerprint = %fingerprint,
                        "ws: unknown key with device token — deferring to token auth for key pinning"
                    );
                // Fall through to device_token auth block below.
                } else if !state
                    .node_pairing_enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    info!(
                        conn_id = %conn_id,
                        fingerprint = %fingerprint,
                        "ws: node pairing is disabled — rejecting unknown key"
                    );
                    let err = ResponseFrame::err(
                        &request_id,
                        ErrorShape::new(
                            error_codes::FORBIDDEN,
                            "node pairing is disabled — enable it from the gateway UI or CLI",
                        ),
                    );
                    #[allow(clippy::unwrap_used)]
                    let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                    graceful_writer_shutdown(client_tx, write_handle).await;
                    return;
                } else {
                    // Create pair request and hold connection for approval.
                    let display_name = params.client.display_name.as_deref();
                    let platform = &params.client.platform;
                    let device_id = &params.client.id;

                    match store
                        .request_pair(device_id, display_name, platform, Some(pk_b64))
                        .await
                    {
                        Ok(pair_req) => {
                            info!(
                                conn_id = %conn_id,
                                pair_id = %pair_req.id,
                                fingerprint = %fingerprint,
                                "ws: Ed25519 pairing request created, waiting for approval"
                            );

                            // Notify operators about the pending pair request.
                            broadcast(
                                &state,
                                "node.pair.requested",
                                serde_json::json!({
                                    "id": pair_req.id,
                                    "deviceId": device_id,
                                    "displayName": display_name,
                                    "platform": platform,
                                    "fingerprint": fingerprint,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;

                            // Tell the node it's pending.
                            let pending_event = serde_json::json!({
                                "type": "event",
                                "event": "node.pair.pending",
                                "payload": {
                                    "pairId": pair_req.id,
                                    "fingerprint": fingerprint,
                                }
                            });
                            #[allow(clippy::unwrap_used)]
                            let _ =
                                client_tx.try_send(serde_json::to_string(&pending_event).unwrap());

                            // Hold connection until approved/rejected/expired (5 min).
                            match wait_for_pair_approval(
                                store,
                                &pair_req.id,
                                std::time::Duration::from_secs(300),
                            )
                            .await
                            {
                                PairOutcome::Approved => {
                                    // Now run the challenge-response.
                                    let (nonce_bytes, nonce_b64) = generate_challenge_nonce();
                                    let challenge_event = serde_json::json!({
                                        "type": "event",
                                        "event": "node.auth.challenge",
                                        "payload": { "nonce": nonce_b64 }
                                    });
                                    #[allow(clippy::unwrap_used)]
                                    let _ = client_tx
                                        .try_send(serde_json::to_string(&challenge_event).unwrap());

                                    match receive_challenge_response(
                                        &mut ws_rx,
                                        std::time::Duration::from_secs(10),
                                    )
                                    .await
                                    {
                                        Ok(sig_b64) => {
                                            match verify_ed25519_challenge(
                                                pk_b64,
                                                &nonce_bytes,
                                                &sig_b64,
                                            ) {
                                                Ok(true) => {
                                                    authenticated = true;
                                                    device_token_device_id =
                                                        Some(device_id.to_string());
                                                    api_key_scopes = Some(vec![
                                                        "operator.read".into(),
                                                        "operator.write".into(),
                                                        "operator.approvals".into(),
                                                    ]);
                                                    info!(
                                                        conn_id = %conn_id,
                                                        device_id = %device_id,
                                                        fingerprint = %fingerprint,
                                                        "ws: authenticated via Ed25519 after pairing approval"
                                                    );
                                                },
                                                Ok(false) | Err(_) => {
                                                    warn!(conn_id = %conn_id, "ws: Ed25519 post-approval verification failed");
                                                    let err = ResponseFrame::err(
                                                        &request_id,
                                                        ErrorShape::new(
                                                            error_codes::FORBIDDEN,
                                                            "challenge-response verification failed after approval",
                                                        ),
                                                    );
                                                    #[allow(clippy::unwrap_used)]
                                                    let _ = client_tx.try_send(
                                                        serde_json::to_string(&err).unwrap(),
                                                    );
                                                    graceful_writer_shutdown(
                                                        client_tx,
                                                        write_handle,
                                                    )
                                                    .await;
                                                    return;
                                                },
                                            }
                                        },
                                        Err(e) => {
                                            warn!(conn_id = %conn_id, error = %e, "ws: post-approval challenge-response failed");
                                            let err = ResponseFrame::err(
                                                &request_id,
                                                ErrorShape::new(
                                                    error_codes::FORBIDDEN,
                                                    "challenge-response failed after approval",
                                                ),
                                            );
                                            #[allow(clippy::unwrap_used)]
                                            let _ = client_tx
                                                .try_send(serde_json::to_string(&err).unwrap());
                                            graceful_writer_shutdown(client_tx, write_handle).await;
                                            return;
                                        },
                                    }
                                },
                                PairOutcome::Rejected => {
                                    warn!(conn_id = %conn_id, "ws: pairing request rejected by operator");
                                    let err = ResponseFrame::err(
                                        &request_id,
                                        ErrorShape::new(
                                            error_codes::FORBIDDEN,
                                            "pairing request rejected",
                                        ),
                                    );
                                    #[allow(clippy::unwrap_used)]
                                    let _ =
                                        client_tx.try_send(serde_json::to_string(&err).unwrap());
                                    graceful_writer_shutdown(client_tx, write_handle).await;
                                    return;
                                },
                                PairOutcome::Expired => {
                                    warn!(conn_id = %conn_id, "ws: pairing request expired");
                                    let err = ResponseFrame::err(
                                        &request_id,
                                        ErrorShape::new(
                                            error_codes::TIMEOUT,
                                            "pairing request expired",
                                        ),
                                    );
                                    #[allow(clippy::unwrap_used)]
                                    let _ =
                                        client_tx.try_send(serde_json::to_string(&err).unwrap());
                                    graceful_writer_shutdown(client_tx, write_handle).await;
                                    return;
                                },
                            }
                        },
                        Err(e) => {
                            warn!(conn_id = %conn_id, error = %e, "ws: failed to create pair request");
                            let err = ResponseFrame::err(
                                &request_id,
                                ErrorShape::new(
                                    error_codes::INTERNAL,
                                    "failed to create pairing request",
                                ),
                            );
                            match serde_json::to_string(&err) {
                                Ok(serialized) => {
                                    let _ = client_tx.try_send(serialized);
                                },
                                Err(e) => {
                                    warn!(conn_id = %conn_id, error = %e, "ws: failed to serialize pair request error");
                                },
                            }
                            graceful_writer_shutdown(client_tx, write_handle).await;
                            return;
                        },
                    }
                } // close else (pairing flow)
            },
            Err(e) => {
                warn!(conn_id = %conn_id, error = %e, "ws: public key lookup failed");
            },
        }
    }

    // Check device token (used by paired nodes, legacy path).
    if !authenticated
        && let Some(ref dt) = params.auth.as_ref().and_then(|a| a.device_token.clone())
        && let Some(ref store) = state.pairing_store
    {
        if params
            .auth
            .as_ref()
            .and_then(|a| a.public_key.as_ref())
            .is_some()
        {
            debug!(conn_id = %conn_id, "ws: device token auth is deprecated — use Ed25519 key-based auth");
        }
        match store.verify_device_token(dt).await {
            Ok(Some(verification)) => {
                // If the node also presented a public key, pin it for future
                // key-based auth (migration from token to Ed25519).
                if let Some(ref pk) = params.auth.as_ref().and_then(|a| a.public_key.clone()) {
                    let fp = public_key_fingerprint(pk).unwrap_or_else(|_| "unknown".into());
                    match store.pin_public_key(&verification.device_id, pk).await {
                        Ok(PinPublicKeyResult::Pinned) => {
                            info!(
                                conn_id = %conn_id,
                                device_id = %verification.device_id,
                                fingerprint = %fp,
                                "ws: pinned Ed25519 public key during token auth (migration)"
                            );
                        },
                        Ok(PinPublicKeyResult::AlreadyPinned) => {
                            debug!(
                                conn_id = %conn_id,
                                device_id = %verification.device_id,
                                fingerprint = %fp,
                                "ws: Ed25519 public key already pinned during token auth"
                            );
                        },
                        Ok(PinPublicKeyResult::Mismatch { expected }) => {
                            let expected_fp = public_key_fingerprint(&expected)
                                .unwrap_or_else(|_| "unknown".into());
                            warn!(
                                conn_id = %conn_id,
                                device_id = %verification.device_id,
                                expected_fingerprint = %expected_fp,
                                presented_fingerprint = %fp,
                                "ws: refusing token auth with mismatched Ed25519 public key"
                            );
                            let err = ResponseFrame::err(
                                &request_id,
                                ErrorShape::new(
                                    error_codes::FORBIDDEN,
                                    format!(
                                        "key mismatch: expected {expected_fp}, got {fp} — \
                                         re-key this node from the gateway UI if intentional"
                                    ),
                                ),
                            );
                            #[allow(clippy::unwrap_used)]
                            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                            graceful_writer_shutdown(client_tx, write_handle).await;
                            return;
                        },
                        Ok(PinPublicKeyResult::Revoked | PinPublicKeyResult::DeviceNotFound) => {
                            warn!(
                                conn_id = %conn_id,
                                device_id = %verification.device_id,
                                fingerprint = %fp,
                                "ws: refusing token auth because device is not active for key pinning"
                            );
                            let err = ResponseFrame::err(
                                &request_id,
                                ErrorShape::new(
                                    error_codes::FORBIDDEN,
                                    "device is not active for key pinning",
                                ),
                            );
                            #[allow(clippy::unwrap_used)]
                            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                            graceful_writer_shutdown(client_tx, write_handle).await;
                            return;
                        },
                        Err(e) => {
                            warn!(conn_id = %conn_id, error = %e, "ws: failed to pin public key during token auth");
                            let err = ResponseFrame::err(
                                &request_id,
                                ErrorShape::new(
                                    error_codes::INTERNAL,
                                    "failed to pin public key during token auth",
                                ),
                            );
                            #[allow(clippy::unwrap_used)]
                            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                            graceful_writer_shutdown(client_tx, write_handle).await;
                            return;
                        },
                    }
                }

                authenticated = true;
                api_key_scopes = Some(verification.scopes.clone());
                device_token_device_id = Some(verification.device_id.clone());

                info!(
                    conn_id = %conn_id,
                    device_id = %verification.device_id,
                    "ws: authenticated via device token"
                );
            },
            Ok(None) => {
                debug!(conn_id = %conn_id, "ws: device token not found or revoked");
            },
            Err(e) => {
                debug!(conn_id = %conn_id, error = %e, "ws: device token verification failed");
            },
        }
    }

    if !authenticated && let Some(ref cred_store) = state.credential_store {
        if cred_store.is_setup_complete() {
            // Check API key.
            if let Some(ref api_key) = params.auth.as_ref().and_then(|a| a.api_key.clone())
                && let Ok(Some(verification)) = cred_store.verify_api_key(api_key).await
            {
                authenticated = true;
                // Store the scopes from the API key (empty = no access)
                api_key_scopes = Some(verification.scopes);
            }
            // Check password against DB hash.
            if !authenticated
                && let Some(ref pw) = params.auth.as_ref().and_then(|a| a.password.clone())
                && cred_store.verify_password(pw).await.unwrap_or(false)
            {
                authenticated = true;
            }
        } else {
            // Setup not complete yet — only allow local connections.
            // Remote connections must go through the onboarding/setup flow.
            if is_local {
                authenticated = true;
            }
        }
    }

    // Fall back to legacy env-var auth if credential store didn't authenticate.
    if !authenticated {
        let has_legacy_auth = state.auth.token.is_some() || state.auth.password.is_some();
        if has_legacy_auth {
            let provided_token = params.auth.as_ref().and_then(|a| a.token.as_deref());
            let provided_password = params.auth.as_ref().and_then(|a| a.password.as_deref());
            let auth_result = auth::authorize_connect(
                &state.auth,
                provided_token,
                provided_password,
                Some(&conn_remote_ip),
            );
            if auth_result.ok {
                authenticated = true;
            }
        } else if state.credential_store.is_none() {
            // No auth configured at all — grant access (backward compat).
            authenticated = true;
        }
    }

    if !authenticated {
        let setup_complete = state
            .credential_store
            .as_ref()
            .is_some_and(|store| store.is_setup_complete());
        let has_api_key = params
            .auth
            .as_ref()
            .and_then(|auth| auth.api_key.as_ref())
            .is_some();
        let has_password = params
            .auth
            .as_ref()
            .and_then(|auth| auth.password.as_ref())
            .is_some();
        let has_token = params
            .auth
            .as_ref()
            .and_then(|auth| auth.token.as_ref())
            .is_some();
        warn!(
            conn_id = %conn_id,
            is_local,
            authenticated,
            setup_complete,
            has_api_key,
            has_password,
            has_token,
            "ws: auth failed"
        );
        let err = ResponseFrame::err(
            &request_id,
            ErrorShape::new(error_codes::UNAUTHORIZED, "authentication failed"),
        );
        #[allow(clippy::unwrap_used)] // serializing known-valid struct
        let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
        graceful_writer_shutdown(client_tx, write_handle).await;
        return;
    }

    // Device-token-authenticated connections default to "node" role.
    let role = if device_token_device_id.is_some() {
        params.role.clone().unwrap_or_else(|| roles::NODE.into())
    } else {
        params
            .role
            .clone()
            .unwrap_or_else(|| roles::OPERATOR.into())
    };

    // Determine scopes based on auth method.
    // API keys MUST declare scopes explicitly — empty scopes means no access.
    // Non-API-key auth (password, local, legacy) gets full access.
    let scopes = match api_key_scopes {
        Some(key_scopes) if !key_scopes.is_empty() => key_scopes,
        Some(_empty) => {
            // API key with no scopes → reject (least-privilege).
            warn!(conn_id = %conn_id, "ws: API key has no scopes, denying access");
            let err = ResponseFrame::err(
                &request_id,
                ErrorShape::new(
                    error_codes::FORBIDDEN,
                    "API key has no scopes — specify at least one scope when creating the key",
                ),
            );
            #[allow(clippy::unwrap_used)] // serializing known-valid struct
            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
            graceful_writer_shutdown(client_tx, write_handle).await;
            return;
        },
        None => {
            // Non-API-key auth (password, local, legacy) → full access.
            vec![
                scopes::ADMIN.into(),
                scopes::READ.into(),
                scopes::WRITE.into(),
                scopes::APPROVALS.into(),
                scopes::PAIRING.into(),
            ]
        },
    };

    // Build HelloOk with auth info.
    let hello_auth = HelloAuth {
        device_token: String::new(),
        role: role.clone(),
        scopes: scopes.clone(),
        issued_at_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        ),
    };

    let hello = HelloOk {
        r#type: "hello-ok".into(),
        protocol: PROTOCOL_VERSION,
        server: ServerInfo {
            version: state.version.clone(),
            commit: None,
            host: Some(state.hostname.clone()),
            conn_id: conn_id.clone(),
        },
        features: Features {
            methods: methods.method_names(),
            events: KNOWN_EVENTS.iter().map(|s| (*s).into()).collect(),
        },
        snapshot: serde_json::json!({}),
        canvas_host_url: None,
        auth: Some(hello_auth),
        policy: Policy::default(),
        extensions: Extensions::new(),
    };
    #[allow(clippy::unwrap_used)] // serializing known-valid struct
    let hello_val = serde_json::to_value(&hello).unwrap();
    let resp = ResponseFrame::ok(&request_id, hello_val);
    #[allow(clippy::unwrap_used)] // serializing known-valid struct
    let _ = client_tx.try_send(serde_json::to_string(&resp).unwrap());

    debug!(
        conn_id = %conn_id,
        client_id = %params.client.id,
        client_version = %params.client.version,
        role = %role,
        "ws: handshake complete"
    );

    // Register the client with server-resolved scopes so broadcast guards work.
    let now = std::time::Instant::now();
    let mut resolved_params = params.clone();
    resolved_params.scopes = Some(scopes.clone());
    resolved_params.role = Some(role.clone());
    let browser_timezone = params.timezone.clone();

    // Auto-persist browser timezone to USER.md on first connect (one-time).
    // Runs in spawn_blocking because discover_and_load / resolve_user_profile
    // do synchronous file I/O that must not block the async WS task.
    if let Some(ref tz_str) = browser_timezone
        && let Ok(tz) = tz_str.parse::<chrono_tz::Tz>()
    {
        let tz_conn_id = conn_id.clone();
        let tz_val = tz;
        let tz_display = tz_str.clone();
        tokio::task::spawn_blocking(move || {
            let write_mode = moltis_config::discover_and_load()
                .memory
                .user_profile_write_mode;
            let existing_user = moltis_config::resolve_user_profile();
            if existing_user.timezone.as_ref().is_none() && write_mode.allows_auto_write() {
                let mut user = existing_user;
                user.timezone = Some(moltis_config::Timezone::from(tz_val));
                if let Err(e) = moltis_config::save_user_with_mode(&user, write_mode) {
                    warn!(conn_id = %tz_conn_id, error = %e, "ws: failed to auto-persist timezone");
                } else {
                    info!(conn_id = %tz_conn_id, timezone = %tz_display, "ws: auto-persisted browser timezone to USER.md");
                }
            }
        });
    }

    // v3 clients default to wildcard subscriptions (all events).
    // v4 clients default to empty subscriptions (must explicitly subscribe).
    let subscriptions = if is_v4 {
        Some(std::collections::HashSet::new())
    } else {
        None
    };

    let client = ConnectedClient {
        conn_id: conn_id.clone(),
        connect_params: resolved_params,
        sender: client_tx.clone(),
        connected_at: now,
        last_activity_ms: std::sync::atomic::AtomicU64::new(0),
        accept_language,
        remote_ip,
        timezone: browser_timezone,
        subscriptions,
        joined_channels: std::collections::HashSet::new(),
        negotiated_protocol: PROTOCOL_VERSION,
    };
    state.register_client(client).await;

    #[cfg(feature = "metrics")]
    {
        moltis_metrics::counter!(moltis_metrics::websocket::CONNECTIONS_TOTAL).increment(1);
        moltis_metrics::gauge!(moltis_metrics::websocket::CONNECTIONS_ACTIVE).increment(1.0);
    }

    // If node role, register in node registry.
    if role == roles::NODE {
        let caps = params.caps.clone().unwrap_or_default();
        let commands = params.commands.clone().unwrap_or_default();
        let permissions: HashMap<String, bool> = params
            .permissions
            .as_ref()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                    .collect()
            })
            .unwrap_or_default();

        let node = NodeSession {
            node_id: params.client.id.clone(),
            conn_id: conn_id.clone(),
            display_name: params.client.display_name.clone(),
            platform: params.client.platform.clone(),
            version: params.client.version.clone(),
            capabilities: caps,
            commands,
            permissions,
            path_env: params.path_env.clone(),
            remote_ip: Some(conn_remote_ip.clone()),
            connected_at: now,
            mem_total: None,
            mem_available: None,
            cpu_count: None,
            cpu_usage: None,
            uptime_secs: None,
            services: Vec::new(),
            last_telemetry: None,
            disk_total: None,
            disk_available: None,
            runtimes: Vec::new(),
            providers: Vec::new(),
        };
        state.inner.write().await.nodes.register(node);
        state
            .node_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        info!(conn_id = %conn_id, node_id = %params.client.id, "node registered");

        // Broadcast presence change.
        broadcast(
            &state,
            "presence",
            serde_json::json!({
                "type": "node.connected",
                "nodeId": params.client.id,
                "platform": params.client.platform,
            }),
            BroadcastOpts::default(),
        )
        .await;

        // Query provider discovery in the background (best-effort).
        if params
            .commands
            .as_ref()
            .is_some_and(|cmds| cmds.iter().any(|c| c == "system.providers"))
        {
            let prov_state = Arc::clone(&state);
            let prov_node_id = params.client.id.clone();
            tokio::spawn(async move {
                match moltis_gateway::node_exec::query_node_providers(&prov_state, &prov_node_id)
                    .await
                {
                    Ok(providers) => {
                        let mut inner = prov_state.inner.write().await;
                        if let Some(n) = inner.nodes.get_mut(&prov_node_id) {
                            n.providers = providers;
                        }
                    },
                    Err(e) => {
                        debug!(node_id = %prov_node_id, error = %e, "provider discovery failed")
                    },
                }
            });
        }
    }

    // ── Message loop ─────────────────────────────────────────────────────

    while let Some(msg) = ws_rx.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                debug!(conn_id = %conn_id, error = %e, "ws: read error");
                break;
            },
        };

        // Enforce payload size limit.
        if text.len() > MAX_PAYLOAD_BYTES {
            warn!(conn_id = %conn_id, size = text.len(), "ws: payload too large");
            let err = EventFrame::new(
                "error",
                serde_json::json!({ "code": error_codes::PAYLOAD_TOO_LARGE, "message": "payload too large", "maxBytes": MAX_PAYLOAD_BYTES }),
                state.next_seq(),
            );
            #[allow(clippy::unwrap_used)] // serializing known-valid struct
            let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
            continue;
        }

        let frame: GatewayFrame = match serde_json::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                warn!(conn_id = %conn_id, error = %e, "ws: invalid frame");
                let err = EventFrame::new(
                    "error",
                    serde_json::json!({ "message": "invalid frame" }),
                    state.next_seq(),
                );
                #[allow(clippy::unwrap_used)] // serializing known-valid struct
                let _ = client_tx.try_send(serde_json::to_string(&err).unwrap());
                continue;
            },
        };

        // Touch activity timestamp (lock-free atomic, no write lock needed).
        if let Some(client) = state.client_registry.read().await.clients.get(&conn_id) {
            client.touch();
        }

        match frame {
            GatewayFrame::Request(req) => {
                if state.ws_request_logs {
                    info!(
                        conn_id = %conn_id,
                        request_id = %req.id,
                        method = %req.method,
                        param_keys = ?top_level_param_keys(&req.params),
                        "ws: received request frame"
                    );
                }
                let ctx = MethodContext {
                    request_id: req.id.clone(),
                    method: req.method.clone(),
                    params: req.params.unwrap_or(serde_json::Value::Null),
                    client_conn_id: conn_id.clone(),
                    client_role: role.clone(),
                    client_scopes: scopes.clone(),
                    state: Arc::clone(&state),
                    channel: req.channel,
                };
                // Spawn RPC dispatch so the read loop is never blocked by slow
                // lock acquisition or I/O inside a handler. The response is sent
                // back via the client_tx channel (same path as before).
                let rpc_tx = client_tx.clone();
                let rpc_conn_id = conn_id.clone();
                let rpc_method = req.method.clone();
                let rpc_request_id = req.id.clone();
                let rpc_methods = Arc::clone(&methods);
                let rpc_state = Arc::clone(&state);
                tokio::spawn(async move {
                    let response = rpc_methods.dispatch(ctx).await;
                    if rpc_state.ws_request_logs {
                        info!(
                            conn_id = %rpc_conn_id,
                            request_id = %rpc_request_id,
                            method = %rpc_method,
                            ok = response.ok,
                            "ws: sent response frame"
                        );
                    }
                    #[allow(clippy::unwrap_used)] // serializing known-valid struct
                    let response_json = serde_json::to_string(&response).unwrap();
                    let response_len = response_json.len();
                    if rpc_tx.send(response_json).await.is_err() {
                        warn!(
                            conn_id = %rpc_conn_id,
                            request_id = %rpc_request_id,
                            method = %rpc_method,
                            response_len,
                            "ws: failed to queue response frame; client writer is closed"
                        );
                    }
                });
            },
            GatewayFrame::Response(res) => {
                // v4 bidirectional RPC: client responding to a server-initiated request.
                let pending = state
                    .inner
                    .write()
                    .await
                    .pending_client_requests
                    .remove(&res.id);
                if let Some(req) = pending {
                    let result = if res.ok {
                        Ok(res.payload.unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(res.error.unwrap_or_else(|| {
                            ErrorShape::new(
                                error_codes::INTERNAL,
                                "client returned error without details",
                            )
                        }))
                    };
                    let _ = req.sender.send(result);
                } else {
                    debug!(conn_id = %conn_id, id = %res.id, "ws: response for unknown request");
                }
            },
            _ => {
                debug!(conn_id = %conn_id, "ws: ignoring non-request frame");
            },
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────

    // Unregister node if applicable.
    let removed_node = state.inner.write().await.nodes.unregister_by_conn(&conn_id);
    if let Some(node) = &removed_node {
        state
            .node_count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        info!(conn_id = %conn_id, node_id = %node.node_id, "node unregistered");
        broadcast(
            &state,
            "presence",
            serde_json::json!({
                "type": "node.disconnected",
                "nodeId": node.node_id,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }

    let duration = state
        .remove_client(&conn_id)
        .await
        .map(|c| c.connected_at.elapsed())
        .unwrap_or_default();

    #[cfg(feature = "metrics")]
    moltis_metrics::gauge!(moltis_metrics::websocket::CONNECTIONS_ACTIVE).decrement(1.0);

    debug!(
        conn_id = %conn_id,
        duration_secs = duration.as_secs(),
        "ws: connection closed"
    );

    drop(client_tx);
    write_handle.abort();
}

/// Result of parsing connect params: includes whether v4 format was used.
struct ConnectResult {
    request_id: String,
    params: ConnectParams,
    is_v4: bool,
}

// ── Ed25519 challenge-response helpers ──────────────────────────────────────

enum PairOutcome {
    Approved,
    Rejected,
    Expired,
}

/// Read WS frames from the node until we get a `node.auth.challenge-response`
/// request containing a `signature` field. Control frames and unrelated text
/// frames are skipped so they don't abort the handshake.
async fn receive_challenge_response(
    ws_rx: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
    timeout: std::time::Duration,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("challenge-response timeout".into());
        }
        match tokio::time::timeout(remaining, ws_rx.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let frame: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(frame) => frame,
                    Err(_) => continue,
                };
                let method = frame.get("method").and_then(|v| v.as_str()).unwrap_or("");
                if method != "node.auth.challenge-response" {
                    continue;
                }
                return frame
                    .get("params")
                    .and_then(|p| p.get("signature"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .ok_or_else(|| "challenge-response missing signature".into());
            },
            // Skip control frames (ping/pong/binary) — keep waiting for text.
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(_))) => return Err("unexpected binary message".into()),
            Ok(Some(Err(e))) => return Err(format!("websocket error: {e}")),
            Ok(None) => return Err("connection closed".into()),
            Err(_) => return Err("challenge-response timeout".into()),
        }
    }
}

/// Poll the pair request status until it's resolved or expired.
async fn wait_for_pair_approval(
    store: &moltis_gateway::pairing::PairingStore,
    pair_id: &str,
    timeout: std::time::Duration,
) -> PairOutcome {
    use moltis_gateway::pairing::PairStatus;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));

    loop {
        interval.tick().await;

        if tokio::time::Instant::now() >= deadline {
            return PairOutcome::Expired;
        }

        // Query the pair request status from the DB.
        match store.get_pair_status(pair_id).await {
            Ok(Some(PairStatus::Approved)) => return PairOutcome::Approved,
            Ok(Some(PairStatus::Rejected)) => return PairOutcome::Rejected,
            Ok(Some(PairStatus::Expired)) => return PairOutcome::Expired,
            Ok(Some(PairStatus::Pending)) => continue,
            Ok(None) => return PairOutcome::Expired, // deleted
            Err(_) => continue,                      // retry on DB error
        }
    }
}

async fn graceful_writer_shutdown(
    client_tx: mpsc::Sender<String>,
    write_handle: tokio::task::JoinHandle<()>,
) {
    drop(client_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), write_handle).await;
}

/// Wait for the first `connect` request frame. Tries v4 format first, falls back to v3.
async fn wait_for_connect(
    rx: &mut futures::stream::SplitStream<WebSocket>,
) -> crate::error::Result<ConnectResult> {
    while let Some(msg) = rx.next().await {
        let text = match msg.map_err(|e| crate::Error::Protocol(e.to_string()))? {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => {
                return Err(crate::Error::Protocol(
                    "connection closed before handshake".into(),
                ));
            },
            _ => continue,
        };

        let frame: GatewayFrame =
            serde_json::from_str(&text).map_err(|e| crate::Error::Protocol(e.to_string()))?;
        match frame {
            GatewayFrame::Request(req) => {
                if req.method != "connect" {
                    return Err(crate::Error::Protocol(format!(
                        "first message must be 'connect', got '{}'",
                        req.method
                    )));
                }
                let raw = req.params.unwrap_or(serde_json::Value::Null);

                // Try v4 format first (has `protocol` object instead of flat fields).
                if let Ok(v4) = serde_json::from_value::<ConnectParamsV4>(raw.clone()) {
                    return Ok(ConnectResult {
                        request_id: req.id,
                        params: v4.into_connect_params(),
                        is_v4: true,
                    });
                }

                // Fall back to v3 flat format.
                let params: ConnectParams = serde_json::from_value(raw)
                    .map_err(|e| crate::Error::Protocol(e.to_string()))?;
                return Ok(ConnectResult {
                    request_id: req.id,
                    params,
                    is_v4: false,
                });
            },
            _ => {
                return Err(crate::Error::Protocol(
                    "first message must be a request frame".into(),
                ));
            },
        }
    }
    Err(crate::Error::Protocol(
        "connection closed before handshake".into(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn receive_challenge_response_skips_unrelated_text_frames() {
        let frames = vec![
            Ok(Message::Text("node-status-ping".into())),
            Ok(Message::Text(
                serde_json::json!({
                    "type": "event",
                    "event": "node.telemetry",
                    "payload": {}
                })
                .to_string()
                .into(),
            )),
            Ok(Message::Text(
                serde_json::json!({
                    "type": "req",
                    "id": "not-auth",
                    "method": "node.invoke.result",
                    "params": {}
                })
                .to_string()
                .into(),
            )),
            Ok(Message::Text(
                serde_json::json!({
                    "type": "req",
                    "id": "auth-response",
                    "method": "node.auth.challenge-response",
                    "params": {
                        "signature": "signed-nonce",
                    }
                })
                .to_string()
                .into(),
            )),
        ];
        let mut stream = futures::stream::iter(frames);

        let signature = receive_challenge_response(&mut stream, std::time::Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(signature, "signed-nonce");
    }
}
