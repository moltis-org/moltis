use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "voicecall.status",
        Box::new(|ctx| {
            Box::pin(async move {
                #[cfg(feature = "telephony")]
                {
                    let registry = ctx
                        .state
                        .services
                        .channel_registry
                        .as_ref()
                        .ok_or_else(|| ErrorShape::new("internal_error", "no channel registry"))?;

                    let plugin = registry.get("telephony");
                    let Some(plugin) = plugin else {
                        return Ok(serde_json::json!({
                            "configured": false,
                            "active_calls": []
                        }));
                    };

                    let guard = plugin.read().await;
                    let account_ids = guard.account_ids();
                    Ok(serde_json::json!({
                        "configured": true,
                        "accounts": account_ids
                    }))
                }
                #[cfg(not(feature = "telephony"))]
                {
                    let _ = ctx;
                    Ok(serde_json::json!({
                        "configured": false,
                        "feature_disabled": true
                    }))
                }
            })
        }),
    );

    reg.register(
        "voicecall.initiate",
        Box::new(|ctx| {
            Box::pin(async move {
                #[cfg(feature = "telephony")]
                {
                    let to = ctx.params["to"].as_str().ok_or_else(|| {
                        ErrorShape::new("invalid_params", "missing 'to' phone number")
                    })?;
                    let message = ctx.params["message"].as_str();
                    let _mode = ctx.params["mode"].as_str().unwrap_or("conversation");

                    Ok(serde_json::json!({
                        "status": "pending",
                        "to": to,
                        "message": message,
                        "note": "Full call initiation via RPC requires webhook URL configuration. Use the voice_call agent tool for automated calling."
                    }))
                }
                #[cfg(not(feature = "telephony"))]
                {
                    let _ = ctx;
                    Err(ErrorShape::new(
                        "feature_disabled",
                        "telephony feature not enabled",
                    ))
                }
            })
        }),
    );

    reg.register(
        "voicecall.end",
        Box::new(|ctx| {
            Box::pin(async move {
                let _call_id = ctx.params["call_id"]
                    .as_str()
                    .ok_or_else(|| ErrorShape::new("invalid_params", "missing 'call_id'"))?;

                Ok(serde_json::json!({
                    "status": "pending",
                    "note": "Call hangup via RPC will be connected to the call manager."
                }))
            })
        }),
    );
}
