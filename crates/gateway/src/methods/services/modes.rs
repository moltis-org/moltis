use super::*;

fn sorted_mode_presets(
    config: &moltis_config::MoltisConfig,
) -> Vec<(String, moltis_config::ModePreset)> {
    let mut modes: Vec<(String, moltis_config::ModePreset)> = config
        .modes
        .presets
        .iter()
        .filter(|(_, preset)| !preset.prompt.trim().is_empty())
        .map(|(id, preset)| (id.clone(), preset.clone()))
        .collect();
    modes.sort_by(|(left_id, left), (right_id, right)| {
        let left_name = left.name.as_deref().unwrap_or(left_id);
        let right_name = right.name.as_deref().unwrap_or(right_id);
        left_name
            .to_lowercase()
            .cmp(&right_name.to_lowercase())
            .then_with(|| left_id.cmp(right_id))
    });
    modes
}

fn parse_mode_id_param(params: &serde_json::Value) -> Option<String> {
    params
        .get("mode_id")
        .or_else(|| params.get("modeId"))
        .or_else(|| params.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn session_key_for_mode_set(ctx: &MethodContext) -> Result<String, ErrorShape> {
    if let Some(session_key) = ctx
        .params
        .get("session_key")
        .or_else(|| ctx.params.get("sessionKey"))
        .or_else(|| ctx.params.get("key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(session_key.to_string());
    }
    active_session_key_for_ctx(ctx).await.ok_or_else(|| {
        ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing 'session_key' parameter",
        )
    })
}

pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "modes.list",
        Box::new(|_ctx| {
            Box::pin(async move {
                let config = moltis_config::discover_and_load();
                let modes = sorted_mode_presets(&config)
                    .into_iter()
                    .map(|(id, preset)| {
                        serde_json::json!({
                            "id": id,
                            "name": preset.name.unwrap_or_default(),
                            "description": preset.description.unwrap_or_default(),
                            "prompt": preset.prompt,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::json!({ "modes": modes }))
            })
        }),
    );

    reg.register(
        "modes.set_session",
        Box::new(|ctx| {
            Box::pin(async move {
                let session_key = session_key_for_mode_set(&ctx).await?;
                let config = moltis_config::discover_and_load();
                let mode_id = parse_mode_id_param(&ctx.params);
                if let Some(ref id) = mode_id
                    && !config.modes.presets.contains_key(id)
                {
                    return Err(ErrorShape::new(
                        error_codes::INVALID_REQUEST,
                        format!("mode '{id}' not found"),
                    ));
                }

                let Some(ref meta) = ctx.state.services.session_metadata else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "session metadata not available",
                    ));
                };
                meta.upsert(&session_key, None)
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                meta.set_mode_id(&session_key, mode_id.as_deref())
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;

                broadcast(
                    &ctx.state,
                    "session",
                    serde_json::json!({
                        "kind": "patched",
                        "sessionKey": session_key,
                    }),
                    BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;

                Ok(serde_json::json!({ "ok": true, "mode_id": mode_id }))
            })
        }),
    );
}
