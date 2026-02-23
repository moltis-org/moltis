use moltis_protocol::{ErrorShape, error_codes};

use crate::broadcast::{BroadcastOpts, broadcast};

use super::MethodRegistry;

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

                let req = ctx.state.inner.write().await.pairing.request_pair(
                    device_id,
                    display_name,
                    platform,
                    public_key,
                );

                // Broadcast pair request to operators with pairing scope.
                broadcast(
                    &ctx.state,
                    "node.pair.requested",
                    serde_json::json!({
                        "id": req.id,
                        "deviceId": req.device_id,
                        "displayName": req.display_name,
                        "platform": req.platform,
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({
                    "id": req.id,
                    "nonce": req.nonce,
                }))
            })
        }),
    );

    // node.pair.list
    reg.register(
        "node.pair.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let list: Vec<_> = inner
                    .pairing
                    .list_pending()
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.id,
                            "deviceId": r.device_id,
                            "displayName": r.display_name,
                            "platform": r.platform,
                        })
                    })
                    .collect();
                Ok(serde_json::json!(list))
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
                let token = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .pairing
                    .approve(pair_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

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
                    "deviceToken": token.token,
                    "scopes": token.scopes,
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
                ctx.state
                    .inner
                    .write()
                    .await
                    .pairing
                    .reject(pair_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

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

    // node.pair.verify (placeholder — signature verification)
    reg.register(
        "node.pair.verify",
        Box::new(|_ctx| Box::pin(async move { Ok(serde_json::json!({ "verified": true })) })),
    );

    // device.pair.list
    reg.register(
        "device.pair.list",
        Box::new(|ctx| {
            Box::pin(async move {
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
                let token = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .pairing
                    .approve(pair_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

                broadcast(
                    &ctx.state,
                    "device.pair.resolved",
                    serde_json::json!({
                        "id": pair_id, "status": "approved",
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                Ok(serde_json::json!({ "deviceToken": token.token, "scopes": token.scopes }))
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
                ctx.state
                    .inner
                    .write()
                    .await
                    .pairing
                    .reject(pair_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;

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
                let token = ctx
                    .state
                    .inner
                    .write()
                    .await
                    .pairing
                    .rotate_token(device_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;
                Ok(serde_json::json!({ "deviceToken": token.token, "scopes": token.scopes }))
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
                ctx.state
                    .inner
                    .write()
                    .await
                    .pairing
                    .revoke_token(device_id)
                    .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, e))?;
                Ok(serde_json::json!({}))
            })
        }),
    );
}
