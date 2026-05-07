use super::*;

#[cfg(feature = "telephony")]
use moltis_channels::ChannelPlugin as _;

pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "voicecall.status",
        Box::new(|ctx| {
            Box::pin(async move {
                #[cfg(feature = "telephony")]
                {
                    let plugin_arc =
                        ctx.state
                            .services
                            .telephony_plugin
                            .as_ref()
                            .ok_or_else(|| {
                                ErrorShape::new("not_configured", "telephony not configured")
                            })?;

                    let plugin = plugin_arc.read().await;
                    let account_ids = plugin.account_ids();
                    let mut accounts = Vec::new();
                    for aid in &account_ids {
                        let config = plugin.account_config_json(aid);
                        let active_calls: Vec<serde_json::Value> = plugin
                            .call_manager(aid)
                            .and_then(|mgr| {
                                mgr.try_read().ok().map(|m| {
                                    m.active_calls()
                                        .into_iter()
                                        .map(|c| {
                                            serde_json::json!({
                                                "call_id": c.call_id,
                                                "state": c.state,
                                                "from": c.from,
                                                "to": c.to,
                                                "direction": c.direction,
                                                "mode": c.mode,
                                            })
                                        })
                                        .collect()
                                })
                            })
                            .unwrap_or_default();

                        accounts.push(serde_json::json!({
                            "account_id": aid,
                            "config": config,
                            "active_calls": active_calls,
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
                #[cfg(feature = "telephony")]
                {
                    let to = ctx.params["to"].as_str().ok_or_else(|| {
                        ErrorShape::new("invalid_params", "missing 'to' phone number")
                    })?;
                    if !to.starts_with('+') {
                        return Err(ErrorShape::new(
                            "invalid_params",
                            "phone number must be in E.164 format (start with +)",
                        ));
                    }
                    let message = ctx.params["message"].as_str();
                    let mode_str = ctx.params["mode"].as_str().unwrap_or("conversation");
                    let mode = match mode_str {
                        "notify" => moltis_telephony::types::CallMode::Notify,
                        _ => moltis_telephony::types::CallMode::Conversation,
                    };
                    let target_account = ctx.params["account_id"].as_str().map(String::from);

                    let plugin_arc =
                        ctx.state
                            .services
                            .telephony_plugin
                            .as_ref()
                            .ok_or_else(|| {
                                ErrorShape::new("not_configured", "telephony not configured")
                            })?;

                    let plugin = plugin_arc.read().await;
                    let account_ids = plugin.account_ids();
                    let account_id = target_account
                        .as_deref()
                        .or(account_ids.first().map(|s| s.as_str()))
                        .ok_or_else(|| {
                            ErrorShape::new("not_configured", "no telephony accounts available")
                        })?
                        .to_string();

                    let from_number = plugin.caller_number(&account_id).ok_or_else(|| {
                        ErrorShape::new("not_configured", "no from_number for account")
                    })?;
                    if from_number.is_empty() {
                        return Err(ErrorShape::new(
                            "not_configured",
                            "from_number is empty for this account",
                        ));
                    }

                    let mgr = plugin.call_manager(&account_id).ok_or_else(|| {
                        ErrorShape::new("not_configured", "no call manager for account")
                    })?;

                    let webhook_base = plugin
                        .account_config_json(&account_id)
                        .and_then(|config| {
                            config["webhook_url"]
                                .as_str()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned)
                        })
                        .or_else(|| ctx.state.config.server.effective_external_url())
                        .ok_or_else(|| {
                            ErrorShape::new(
                                "not_configured",
                                "phone webhook_url or server.external_url is required to initiate calls",
                            )
                        })?;

                    let status_url =
                        format!("{}/api/channels/telephony/{account_id}/status", webhook_base.trim_end_matches('/'));
                    let answer_url =
                        format!("{}/api/channels/telephony/{account_id}/answer", webhook_base.trim_end_matches('/'));

                    let manager = mgr.read().await;
                    let call_id = manager
                        .initiate(
                            &from_number,
                            to,
                            mode,
                            message,
                            &account_id,
                            &status_url,
                            &answer_url,
                        )
                        .await
                        .map_err(|e| ErrorShape::new("call_failed", e.to_string()))?;

                    Ok(serde_json::json!({
                        "status": "initiated",
                        "call_id": call_id,
                        "from": from_number,
                        "to": to,
                        "mode": mode_str,
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
                #[cfg(feature = "telephony")]
                {
                    let call_id = ctx.params["call_id"]
                        .as_str()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing 'call_id'"))?;
                    let target_account = ctx.params["account_id"].as_str().map(String::from);

                    let plugin_arc =
                        ctx.state
                            .services
                            .telephony_plugin
                            .as_ref()
                            .ok_or_else(|| {
                                ErrorShape::new("not_configured", "telephony not configured")
                            })?;

                    let plugin = plugin_arc.read().await;
                    let account_ids = plugin.account_ids();
                    let account_id = target_account
                        .as_deref()
                        .or(account_ids.first().map(|s| s.as_str()))
                        .ok_or_else(|| {
                            ErrorShape::new("not_configured", "no telephony accounts available")
                        })?
                        .to_string();

                    let mgr = plugin.call_manager(&account_id).ok_or_else(|| {
                        ErrorShape::new("not_configured", "no call manager for account")
                    })?;

                    let manager = mgr.read().await;
                    manager
                        .hangup(call_id)
                        .await
                        .map_err(|e| ErrorShape::new("hangup_failed", e.to_string()))?;

                    Ok(serde_json::json!({
                        "status": "ended",
                        "call_id": call_id,
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

    // ── Phone provider management ─────────────────────────────────
    #[cfg(feature = "telephony")]
    {
        use crate::methods::phone;

        reg.register(
            "phone.providers.all",
            Box::new(|ctx| {
                Box::pin(async move {
                    let result = phone::detect_phone_providers(&ctx.state.config);
                    Ok(result)
                })
            }),
        );

        reg.register(
            "phone.provider.toggle",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx.params["provider"]
                        .as_str()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing provider"))?;
                    let enabled = ctx.params["enabled"]
                        .as_bool()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing enabled"))?;

                    phone::toggle_phone_provider(provider, enabled)
                        .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;
                    phone::reload_running_phone_account(&ctx.state)
                        .await
                        .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;

                    broadcast(
                        &ctx.state,
                        "phone.config.changed",
                        serde_json::json!({ "provider": provider, "enabled": enabled }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        reg.register(
            "phone.config.save_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx.params["provider"]
                        .as_str()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing provider"))?;
                    let primary_credential =
                        ctx.params["account_sid"].as_str().unwrap_or("").trim();
                    let secondary_credential =
                        ctx.params["auth_token"].as_str().unwrap_or("").trim();
                    if !matches!(provider, "twilio" | "telnyx" | "plivo") {
                        return Err(ErrorShape::new("invalid_params", "unknown phone provider"));
                    }
                    if primary_credential.is_empty() || secondary_credential.is_empty() {
                        return Err(ErrorShape::new(
                            "invalid_params",
                            "both phone provider credential fields are required",
                        ));
                    }

                    let store = crate::provider_setup::KeyStore::new();
                    let store_key = phone::phone_key_store_name(provider);
                    store
                        .save_config(
                            &store_key,
                            Some(primary_credential.to_string()),
                            Some(secondary_credential.to_string()),
                            None,
                        )
                        .map_err(|e| ErrorShape::new("storage_error", e.to_string()))?;

                    moltis_config::update_config(|cfg| {
                        cfg.phone.enabled = true;
                        cfg.phone.provider = provider.to_string();
                        phone::clear_inline_phone_credentials(cfg, provider);
                        phone::apply_phone_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;
                    phone::reload_running_phone_account(&ctx.state)
                        .await
                        .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;

                    broadcast(
                        &ctx.state,
                        "phone.config.changed",
                        serde_json::json!({ "provider": provider, "settings": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        reg.register(
            "phone.config.save_settings",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx.params["provider"]
                        .as_str()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing provider"))?;

                    moltis_config::update_config(|cfg| {
                        phone::apply_phone_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;
                    phone::reload_running_phone_account(&ctx.state)
                        .await
                        .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;

                    broadcast(
                        &ctx.state,
                        "phone.config.changed",
                        serde_json::json!({ "provider": provider, "settings": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );

        reg.register(
            "phone.config.remove_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx.params["provider"]
                        .as_str()
                        .ok_or_else(|| ErrorShape::new("invalid_params", "missing provider"))?;

                    let store = crate::provider_setup::KeyStore::new();
                    let store_key = phone::phone_key_store_name(provider);
                    store
                        .remove(&store_key)
                        .map_err(|e| ErrorShape::new("storage_error", e.to_string()))?;

                    moltis_config::update_config(|cfg| {
                        phone::clear_inline_phone_credentials(cfg, provider);
                        if cfg.phone.provider == provider {
                            cfg.phone.enabled = false;
                            cfg.phone.provider = String::new();
                        }
                    })
                    .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;
                    phone::reload_running_phone_account(&ctx.state)
                        .await
                        .map_err(|e| ErrorShape::new("config_error", e.to_string()))?;

                    broadcast(
                        &ctx.state,
                        "phone.config.changed",
                        serde_json::json!({ "provider": provider, "removed": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );
    }
}
