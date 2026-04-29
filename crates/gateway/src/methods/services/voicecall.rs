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
                            "accounts": []
                        }));
                    };

                    let guard = plugin.read().await;
                    let account_ids = guard.account_ids();

                    let mut accounts = Vec::new();
                    for aid in &account_ids {
                        let config = guard.account_config_json(aid);
                        accounts.push(serde_json::json!({
                            "account_id": aid,
                            "config": config,
                        }));
                    }

                    Ok(serde_json::json!({
                        "configured": true,
                        "accounts": accounts
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
                let to = ctx.params["to"].as_str().ok_or_else(|| {
                    ErrorShape::new("invalid_params", "missing 'to' phone number")
                })?;
                let message = ctx.params["message"].as_str();

                if !to.starts_with('+') {
                    return Err(ErrorShape::new(
                        "invalid_params",
                        "phone number must be in E.164 format (start with +)",
                    ));
                }

                Ok(serde_json::json!({
                    "status": "accepted",
                    "to": to,
                    "message": message,
                    "hint": "Use the voice_call agent tool for full call lifecycle management."
                }))
            })
        }),
    );

    reg.register(
        "voicecall.end",
        Box::new(|ctx| {
            Box::pin(async move {
                let call_id = ctx.params["call_id"]
                    .as_str()
                    .ok_or_else(|| ErrorShape::new("invalid_params", "missing 'call_id'"))?;

                Ok(serde_json::json!({
                    "status": "accepted",
                    "call_id": call_id,
                    "hint": "Use the voice_call agent tool for full call lifecycle management."
                }))
            })
        }),
    );
}
