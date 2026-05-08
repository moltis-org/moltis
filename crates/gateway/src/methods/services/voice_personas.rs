use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "voice.personas.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Ok(serde_json::json!({ "personas": [], "active": null }));
                };
                let personas = store.list().await.map_err(ErrorShape::from)?;
                let active = personas.iter().find(|p| p.is_active).map(|p| &p.persona.id);
                Ok(serde_json::json!({
                    "personas": personas,
                    "active": active,
                }))
            })
        }),
    );

    reg.register(
        "voice.personas.get",
        Box::new(|ctx| {
            Box::pin(async move {
                let id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing 'id' parameter")
                    })?;
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "voice personas not available",
                    ));
                };
                let persona = store
                    .get(id)
                    .await
                    .map_err(ErrorShape::from)?
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "persona not found")
                    })?;
                serde_json::to_value(&persona)
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );

    reg.register(
        "voice.personas.create",
        Box::new(|ctx| {
            Box::pin(async move {
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "voice personas not available",
                    ));
                };
                let params: crate::voice_persona::CreateVoicePersonaParams =
                    serde_json::from_value(ctx.params).map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                let persona = store.create(params).await.map_err(ErrorShape::from)?;
                serde_json::to_value(&persona)
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );

    reg.register(
        "voice.personas.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing 'id' parameter")
                    })?
                    .to_string();
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "voice personas not available",
                    ));
                };
                let params: crate::voice_persona::UpdateVoicePersonaParams =
                    serde_json::from_value(ctx.params).map_err(|e| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                    })?;
                let persona = store.update(&id, params).await.map_err(ErrorShape::from)?;
                serde_json::to_value(&persona)
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );

    reg.register(
        "voice.personas.delete",
        Box::new(|ctx| {
            Box::pin(async move {
                let id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing 'id' parameter")
                    })?;
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "voice personas not available",
                    ));
                };
                store.delete(id).await.map_err(ErrorShape::from)?;
                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    reg.register(
        "voice.personas.set_active",
        Box::new(|ctx| {
            Box::pin(async move {
                let Some(ref store) = ctx.state.services.voice_persona_store else {
                    return Err(ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        "voice personas not available",
                    ));
                };
                // Accept "id" param, or null/"none"/"off" to deactivate.
                let id = ctx
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty() && *s != "none" && *s != "off");
                let result = store.set_active(id).await.map_err(ErrorShape::from)?;
                Ok(serde_json::json!({
                    "ok": true,
                    "active": result.map(|r| r.persona.id),
                }))
            })
        }),
    );
}
