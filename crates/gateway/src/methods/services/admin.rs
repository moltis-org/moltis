use super::*;

#[cfg(feature = "voice")]
use crate::methods::voice;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Update
    reg.register(
        "update.run",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .update
                    .run(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Onboarding / Wizard
    reg.register(
        "wizard.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.next",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_next(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.cancel",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_cancel()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Web login
    reg.register(
        "web.login.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .web_login
                    .start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "web.login.wait",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .web_login
                    .wait(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Projects ────────────────────────────────────────────────────

    reg.register(
        "projects.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.upsert",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .upsert(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.delete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .delete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .detect(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.complete_path",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .complete_path(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.context",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .context(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Voice Config ───────────────────────────────────────────────
    #[cfg(feature = "voice")]
    {
        reg.register(
                "voice.config.get",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        let config = moltis_config::discover_and_load();
                        Ok(serde_json::json!({
                            "tts": {
                                "enabled": config.voice.tts.enabled,
                                "provider": config.voice.tts.provider,
                                "elevenlabs_configured": config.voice.tts.elevenlabs.api_key.is_some(),
                                "openai_configured": config.voice.tts.openai.api_key.is_some(),
                            },
                            "stt": {
                                "enabled": config.voice.stt.enabled,
                                "provider": config.voice.stt.provider,
                                "whisper_configured": config.voice.stt.whisper.api_key.is_some(),
                                "groq_configured": config.voice.stt.groq.api_key.is_some(),
                                "deepgram_configured": config.voice.stt.deepgram.api_key.is_some(),
                                "google_configured": config.voice.stt.google.api_key.is_some(),
                                "elevenlabs_configured": config.voice.stt.elevenlabs.api_key.is_some(),
                                "whisper_cli_configured": config.voice.stt.whisper_cli.model_path.is_some(),
                                "sherpa_onnx_configured": config.voice.stt.sherpa_onnx.model_dir.is_some(),
                            },
                        }))
                    })
                }),
            );
        // Comprehensive provider listing with availability detection
        reg.register(
            "voice.providers.all",
            Box::new(|_ctx| {
                Box::pin(async move {
                    let config = moltis_config::discover_and_load();
                    let providers = voice::detect_voice_providers(&config).await;
                    Ok(serde_json::json!(providers))
                })
            }),
        );
        reg.register(
            "voice.elevenlabs.catalog",
            Box::new(|_ctx| {
                Box::pin(async move {
                    let config = moltis_config::discover_and_load();
                    Ok(voice::fetch_elevenlabs_catalog(&config).await)
                })
            }),
        );
        // Enable/disable a voice provider (updates config file)
        reg.register(
            "voice.provider.toggle",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;
                    let enabled = ctx
                        .params
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing enabled")
                        })?;
                    let provider_type = ctx
                        .params
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("stt");

                    voice::toggle_voice_provider(provider, enabled, provider_type).map_err(
                        |e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to toggle provider: {}", e),
                            )
                        },
                    )?;

                    // Broadcast change
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "enabled": enabled }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider, "enabled": enabled }))
                })
            }),
        );
        reg.register(
            "voice.override.session.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let session_key = ctx
                        .params
                        .get("sessionKey")
                        .or_else(|| ctx.params.get("session_key"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                        })?
                        .to_string();

                    let override_cfg = crate::state::TtsRuntimeOverride {
                        provider: ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        voice_id: ctx
                            .params
                            .get("voiceId")
                            .or_else(|| ctx.params.get("voice_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        model: ctx
                            .params
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    };

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_session_overrides
                        .insert(session_key.clone(), override_cfg.clone());

                    Ok(serde_json::to_value(override_cfg).unwrap_or_else(
                        |_| serde_json::json!({ "ok": true, "sessionKey": session_key }),
                    ))
                })
            }),
        );
        reg.register(
            "voice.override.session.clear",
            Box::new(|ctx| {
                Box::pin(async move {
                    let session_key = ctx
                        .params
                        .get("sessionKey")
                        .or_else(|| ctx.params.get("session_key"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                        })?
                        .to_string();

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_session_overrides
                        .remove(&session_key);
                    Ok(serde_json::json!({ "ok": true, "sessionKey": session_key }))
                })
            }),
        );
        reg.register(
            "voice.override.channel.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let channel_type = ctx
                        .params
                        .get("channelType")
                        .or_else(|| ctx.params.get("channel_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("telegram");
                    let account_id = ctx
                        .params
                        .get("accountId")
                        .or_else(|| ctx.params.get("account_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                        })?;

                    let key = format!("{}:{}", channel_type, account_id);
                    let override_cfg = crate::state::TtsRuntimeOverride {
                        provider: ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        voice_id: ctx
                            .params
                            .get("voiceId")
                            .or_else(|| ctx.params.get("voice_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        model: ctx
                            .params
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    };

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_channel_overrides
                        .insert(key.clone(), override_cfg.clone());

                    Ok(serde_json::json!({ "ok": true, "key": key, "override": override_cfg }))
                })
            }),
        );
        reg.register(
            "voice.override.channel.clear",
            Box::new(|ctx| {
                Box::pin(async move {
                    let channel_type = ctx
                        .params
                        .get("channelType")
                        .or_else(|| ctx.params.get("channel_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("telegram");
                    let account_id = ctx
                        .params
                        .get("accountId")
                        .or_else(|| ctx.params.get("account_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                        })?;

                    let key = format!("{}:{}", channel_type, account_id);
                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_channel_overrides
                        .remove(&key);
                    Ok(serde_json::json!({ "ok": true, "key": key }))
                })
            }),
        );
        reg.register(
            "voice.config.save_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    use secrecy::Secret;

                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;
                    let api_key = ctx
                        .params
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing api_key")
                        })?;

                    moltis_config::update_config(|cfg| {
                        match provider {
                            // TTS providers
                            "elevenlabs" => {
                                // ElevenLabs shares key between TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.tts.elevenlabs.api_key = Some(key.clone());
                                cfg.voice.stt.elevenlabs.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both TTS and STT with ElevenLabs
                                cfg.voice.tts.provider = "elevenlabs".to_string();
                                cfg.voice.tts.enabled = true;
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::ElevenLabs);
                                cfg.voice.stt.enabled = true;
                            },
                            "openai" | "openai-tts" => {
                                cfg.voice.tts.openai.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.tts.provider = "openai".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            "google-tts" => {
                                // Google API key is shared - set both TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.tts.google.api_key = Some(key.clone());
                                cfg.voice.stt.google.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both TTS and STT with Google
                                cfg.voice.tts.provider = "google".to_string();
                                cfg.voice.tts.enabled = true;
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Google);
                                cfg.voice.stt.enabled = true;
                            },
                            // STT providers
                            "whisper" => {
                                cfg.voice.stt.whisper.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Whisper);
                                cfg.voice.stt.enabled = true;
                            },
                            "groq" => {
                                cfg.voice.stt.groq.api_key = Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Groq);
                                cfg.voice.stt.enabled = true;
                            },
                            "deepgram" => {
                                cfg.voice.stt.deepgram.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Deepgram);
                                cfg.voice.stt.enabled = true;
                            },
                            "google" => {
                                // Google STT key - also set TTS since they share the same key
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.stt.google.api_key = Some(key.clone());
                                cfg.voice.tts.google.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both STT and TTS with Google
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Google);
                                cfg.voice.stt.enabled = true;
                                cfg.voice.tts.provider = "google".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            "mistral" => {
                                cfg.voice.stt.mistral.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::Mistral);
                                cfg.voice.stt.enabled = true;
                            },
                            "elevenlabs-stt" => {
                                // ElevenLabs shares key between TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.stt.elevenlabs.api_key = Some(key.clone());
                                cfg.voice.tts.elevenlabs.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both STT and TTS with ElevenLabs
                                cfg.voice.stt.provider =
                                    Some(moltis_config::VoiceSttProvider::ElevenLabs);
                                cfg.voice.stt.enabled = true;
                                cfg.voice.tts.provider = "elevenlabs".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            _ => {},
                        }

                        voice::apply_voice_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| {
                        ErrorShape::new(error_codes::UNAVAILABLE, format!("failed to save: {}", e))
                    })?;

                    // Broadcast voice config change event
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.save_settings",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;

                    moltis_config::update_config(|cfg| {
                        voice::apply_voice_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| {
                        ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            format!("failed to save settings: {}", e),
                        )
                    })?;

                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "settings": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.remove_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;

                    moltis_config::update_config(|cfg| match provider {
                        // TTS providers
                        "elevenlabs" => {
                            cfg.voice.tts.elevenlabs.api_key = None;
                        },
                        "openai" => {
                            cfg.voice.tts.openai.api_key = None;
                        },
                        // STT providers
                        "whisper" => {
                            cfg.voice.stt.whisper.api_key = None;
                        },
                        "groq" => {
                            cfg.voice.stt.groq.api_key = None;
                        },
                        "deepgram" => {
                            cfg.voice.stt.deepgram.api_key = None;
                        },
                        "google" => {
                            cfg.voice.stt.google.api_key = None;
                        },
                        "mistral" => {
                            cfg.voice.stt.mistral.api_key = None;
                        },
                        "elevenlabs-stt" => {
                            cfg.voice.stt.elevenlabs.api_key = None;
                        },
                        _ => {},
                    })
                    .map_err(|e| {
                        ErrorShape::new(error_codes::UNAVAILABLE, format!("failed to save: {}", e))
                    })?;

                    // Broadcast voice config change event
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "removed": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.voxtral_requirements",
            Box::new(|_ctx| {
                Box::pin(async move {
                    // Detect OS and architecture
                    let os = std::env::consts::OS;
                    let arch = std::env::consts::ARCH;

                    // Check Python version
                    let python_info = voice::check_python_version().await;

                    // Check CUDA availability
                    let cuda_info = voice::check_cuda_availability().await;

                    // Determine compatibility
                    let (compatible, reasons) =
                        voice::check_voxtral_compatibility(os, arch, &python_info, &cuda_info);

                    Ok(serde_json::json!({
                        "os": os,
                        "arch": arch,
                        "python": python_info,
                        "cuda": cuda_info,
                        "compatible": compatible,
                        "reasons": reasons,
                    }))
                })
            }),
        );
    }

    #[cfg(feature = "graphql")]
    {
        reg.register(
            "graphql.config.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    Ok(serde_json::json!({
                        "enabled": ctx.state.is_graphql_enabled(),
                    }))
                })
            }),
        );
        reg.register(
            "graphql.config.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let enabled = ctx
                        .params
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing enabled")
                        })?;

                    ctx.state.set_graphql_enabled(enabled);

                    let mut persisted = true;
                    if let Err(error) = moltis_config::update_config(|cfg| {
                        cfg.graphql.enabled = enabled;
                    }) {
                        persisted = false;
                        tracing::warn!(%error, enabled, "failed to persist graphql config");
                    }

                    Ok(serde_json::json!({
                        "ok": true,
                        "enabled": enabled,
                        "persisted": persisted,
                    }))
                })
            }),
        );
    }

    // ── Memory ─────────────────────────────────────────────────────

    reg.register(
        "memory.status",
        Box::new(|ctx| {
            Box::pin(async move {
                if let Some(ref mm) = ctx.state.memory_manager {
                    match mm.status().await {
                        Ok(status) => Ok(serde_json::json!({
                            "available": true,
                            "backend": mm.backend_name(),
                            "total_files": status.total_files,
                            "total_chunks": status.total_chunks,
                            "db_size": status.db_size_bytes,
                            "db_size_display": status.db_size_display(),
                            "embedding_model": status.embedding_model,
                            "has_embeddings": mm.has_embeddings(),
                        })),
                        Err(e) => Ok(serde_json::json!({
                            "available": false,
                            "error": e.to_string(),
                        })),
                    }
                } else {
                    Ok(serde_json::json!({
                        "available": false,
                        "error": "Memory system not initialized",
                    }))
                }
            })
        }),
    );

    reg.register(
        "memory.config.get",
        Box::new(|_ctx| {
            Box::pin(async move {
                // Read memory config from the config file
                let config = moltis_config::discover_and_load();
                let memory = &config.memory;
                let chat = &config.chat;
                Ok(serde_json::json!({
                    "style": match memory.style {
                        moltis_config::MemoryStyle::Hybrid => "hybrid",
                        moltis_config::MemoryStyle::PromptOnly => "prompt-only",
                        moltis_config::MemoryStyle::SearchOnly => "search-only",
                        moltis_config::MemoryStyle::Off => "off",
                    },
                    "agent_write_mode": match memory.agent_write_mode {
                        moltis_config::AgentMemoryWriteMode::Hybrid => "hybrid",
                        moltis_config::AgentMemoryWriteMode::PromptOnly => "prompt-only",
                        moltis_config::AgentMemoryWriteMode::SearchOnly => "search-only",
                        moltis_config::AgentMemoryWriteMode::Off => "off",
                    },
                    "user_profile_write_mode": match memory.user_profile_write_mode {
                        moltis_config::UserProfileWriteMode::ExplicitAndAuto => "explicit-and-auto",
                        moltis_config::UserProfileWriteMode::ExplicitOnly => "explicit-only",
                        moltis_config::UserProfileWriteMode::Off => "off",
                    },
                    "backend": match memory.backend {
                        moltis_config::MemoryBackend::Builtin => "builtin",
                        moltis_config::MemoryBackend::Qmd => "qmd",
                    },
                    "provider": match memory.provider {
                        Some(moltis_config::MemoryProvider::Local) => "local",
                        Some(moltis_config::MemoryProvider::Ollama) => "ollama",
                        Some(moltis_config::MemoryProvider::OpenAi) => "openai",
                        Some(moltis_config::MemoryProvider::Custom) => "custom",
                        None => "auto",
                    },
                    "citations": match memory.citations {
                        moltis_config::MemoryCitationsMode::On => "on",
                        moltis_config::MemoryCitationsMode::Off => "off",
                        moltis_config::MemoryCitationsMode::Auto => "auto",
                    },
                    "disable_rag": memory.disable_rag,
                    "llm_reranking": memory.llm_reranking,
                    "search_merge_strategy": match memory.search_merge_strategy {
                        moltis_config::MemorySearchMergeStrategy::Rrf => "rrf",
                        moltis_config::MemorySearchMergeStrategy::Linear => "linear",
                    },
                    "session_export": match memory.session_export {
                        moltis_config::SessionExportMode::Off => "off",
                        moltis_config::SessionExportMode::OnNewOrReset => "on-new-or-reset",
                    },
                    "prompt_memory_mode": match chat.prompt_memory_mode {
                        moltis_config::PromptMemoryMode::LiveReload => "live-reload",
                        moltis_config::PromptMemoryMode::FrozenAtSessionStart => "frozen-at-session-start",
                    },
                    "qmd_feature_enabled": cfg!(feature = "qmd"),
                    "enable_prefetch": memory.enable_prefetch,
                    "prefetch_limit": memory.prefetch_limit,
                    "auto_extract_interval": memory.auto_extract_interval,
                    "enable_session_summary": memory.enable_session_summary,
                    "enable_self_improvement": config.skills.enable_self_improvement,
                }))
            })
        }),
    );

    reg.register(
        "memory.config.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let current_config = moltis_config::discover_and_load();
                let current_memory = current_config.memory;
                let current_chat = current_config.chat;
                let style = ctx.params.get("style").and_then(|v| v.as_str()).unwrap_or(
                    match current_memory.style {
                        moltis_config::MemoryStyle::Hybrid => "hybrid",
                        moltis_config::MemoryStyle::PromptOnly => "prompt-only",
                        moltis_config::MemoryStyle::SearchOnly => "search-only",
                        moltis_config::MemoryStyle::Off => "off",
                    },
                );
                let backend = ctx
                    .params
                    .get("backend")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.backend {
                        moltis_config::MemoryBackend::Builtin => "builtin",
                        moltis_config::MemoryBackend::Qmd => "qmd",
                    });
                let agent_write_mode = ctx
                    .params
                    .get("agent_write_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.agent_write_mode {
                        moltis_config::AgentMemoryWriteMode::Hybrid => "hybrid",
                        moltis_config::AgentMemoryWriteMode::PromptOnly => "prompt-only",
                        moltis_config::AgentMemoryWriteMode::SearchOnly => "search-only",
                        moltis_config::AgentMemoryWriteMode::Off => "off",
                    });
                let user_profile_write_mode = ctx
                    .params
                    .get("user_profile_write_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.user_profile_write_mode {
                        moltis_config::UserProfileWriteMode::ExplicitAndAuto => "explicit-and-auto",
                        moltis_config::UserProfileWriteMode::ExplicitOnly => "explicit-only",
                        moltis_config::UserProfileWriteMode::Off => "off",
                    });
                let citations = ctx
                    .params
                    .get("citations")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.citations {
                        moltis_config::MemoryCitationsMode::On => "on",
                        moltis_config::MemoryCitationsMode::Off => "off",
                        moltis_config::MemoryCitationsMode::Auto => "auto",
                    });
                let provider = ctx
                    .params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.provider {
                        Some(moltis_config::MemoryProvider::Local) => "local",
                        Some(moltis_config::MemoryProvider::Ollama) => "ollama",
                        Some(moltis_config::MemoryProvider::OpenAi) => "openai",
                        Some(moltis_config::MemoryProvider::Custom) => "custom",
                        None => "auto",
                    });
                let llm_reranking = ctx
                    .params
                    .get("llm_reranking")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current_memory.llm_reranking);
                let search_merge_strategy = ctx
                    .params
                    .get("search_merge_strategy")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_memory.search_merge_strategy {
                        moltis_config::MemorySearchMergeStrategy::Rrf => "rrf",
                        moltis_config::MemorySearchMergeStrategy::Linear => "linear",
                    });
                let style_value = parse_memory_style(style)?;
                let backend_value = parse_memory_backend(backend)?;
                let agent_write_mode_value = parse_agent_memory_write_mode(agent_write_mode)?;
                let user_profile_write_mode_value =
                    parse_user_profile_write_mode(user_profile_write_mode)?;
                let citations_value = parse_memory_citations_mode(citations)?;
                let provider_value = parse_memory_provider(provider)?;
                let search_merge_strategy_value =
                    parse_memory_search_merge_strategy(search_merge_strategy)?;
                let disable_rag = ctx.params.get("disable_rag").and_then(|v| v.as_bool());
                let session_export = match ctx.params.get("session_export") {
                    Some(value) => parse_session_export_mode(value)?,
                    None => current_memory.session_export,
                };
                let prompt_memory_mode = ctx
                    .params
                    .get("prompt_memory_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or(match current_chat.prompt_memory_mode {
                        moltis_config::PromptMemoryMode::LiveReload => "live-reload",
                        moltis_config::PromptMemoryMode::FrozenAtSessionStart => {
                            "frozen-at-session-start"
                        },
                    });
                let prompt_memory_mode_value = parse_prompt_memory_mode(prompt_memory_mode)?;
                let mut effective_disable_rag = current_memory.disable_rag;
                let enable_prefetch = ctx
                    .params
                    .get("enable_prefetch")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current_memory.enable_prefetch);
                let prefetch_limit = ctx
                    .params
                    .get("prefetch_limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(current_memory.prefetch_limit);
                let auto_extract_interval = ctx
                    .params
                    .get("auto_extract_interval")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(current_memory.auto_extract_interval);
                let enable_session_summary = ctx
                    .params
                    .get("enable_session_summary")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current_memory.enable_session_summary);
                let enable_self_improvement = ctx
                    .params
                    .get("enable_self_improvement")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(current_config.skills.enable_self_improvement);
                if let Err(e) = moltis_config::update_config(|cfg| {
                    cfg.memory.style = style_value;
                    cfg.memory.agent_write_mode = agent_write_mode_value;
                    cfg.memory.user_profile_write_mode = user_profile_write_mode_value;
                    cfg.memory.backend = backend_value;
                    cfg.memory.provider = provider_value;
                    cfg.memory.citations = citations_value;
                    cfg.memory.llm_reranking = llm_reranking;
                    cfg.memory.search_merge_strategy = search_merge_strategy_value;
                    if let Some(value) = disable_rag {
                        cfg.memory.disable_rag = value;
                    }
                    cfg.memory.session_export = session_export;
                    cfg.memory.enable_prefetch = enable_prefetch;
                    cfg.memory.prefetch_limit = prefetch_limit;
                    cfg.memory.auto_extract_interval = auto_extract_interval;
                    cfg.memory.enable_session_summary = enable_session_summary;
                    cfg.skills.enable_self_improvement = enable_self_improvement;
                    cfg.chat.prompt_memory_mode = prompt_memory_mode_value;
                    effective_disable_rag = cfg.memory.disable_rag;
                }) {
                    tracing::warn!(error = %e, "failed to persist memory config");
                }

                Ok(serde_json::json!({
                    "style": style,
                    "agent_write_mode": agent_write_mode,
                    "user_profile_write_mode": user_profile_write_mode,
                    "backend": backend,
                    "provider": provider,
                    "citations": citations,
                    "disable_rag": effective_disable_rag,
                    "llm_reranking": llm_reranking,
                    "search_merge_strategy": search_merge_strategy,
                    "session_export": match session_export {
                        moltis_config::SessionExportMode::Off => "off",
                        moltis_config::SessionExportMode::OnNewOrReset => "on-new-or-reset",
                    },
                    "prompt_memory_mode": prompt_memory_mode,
                    "enable_prefetch": enable_prefetch,
                    "prefetch_limit": prefetch_limit,
                    "auto_extract_interval": auto_extract_interval,
                    "enable_session_summary": enable_session_summary,
                    "enable_self_improvement": enable_self_improvement,
                }))
            })
        }),
    );

    // QMD status check
    reg.register(
        "memory.qmd.status",
        Box::new(|_ctx| {
            Box::pin(async move {
                #[cfg(feature = "qmd")]
                {
                    use moltis_qmd::{QmdManager, QmdManagerConfig};

                    let config = moltis_config::discover_and_load();
                    let qmd_config = QmdManagerConfig {
                        command: config
                            .memory
                            .qmd
                            .command
                            .clone()
                            .unwrap_or_else(|| "qmd".into()),
                        collections: std::collections::HashMap::new(),
                        max_results: config.memory.qmd.max_results.unwrap_or(10),
                        timeout_ms: config.memory.qmd.timeout_ms.unwrap_or(30_000),
                        work_dir: moltis_config::data_dir(),
                        index_name: "moltis-status".into(),
                        env_overrides: std::collections::HashMap::new(),
                    };

                    let manager = QmdManager::new(qmd_config);
                    let status = manager.status().await;

                    Ok(serde_json::json!({
                        "feature_enabled": true,
                        "available": status.available,
                        "version": status.version,
                        "error": status.error,
                    }))
                }

                #[cfg(not(feature = "qmd"))]
                {
                    Ok(serde_json::json!({
                        "feature_enabled": false,
                        "available": false,
                        "error": "QMD feature not enabled. Rebuild with --features qmd",
                    }))
                }
            })
        }),
    );

    // ── Hooks methods ────────────────────────────────────────────────

    // hooks.list — return discovered hooks with live stats.
    reg.register(
        "hooks.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let mut list = inner.discovered_hooks.clone();

                // Enrich with live stats from the registry.
                if let Some(ref registry) = inner.hook_registry {
                    for hook in &mut list {
                        if let Some(stats) = registry.handler_stats(&hook.name) {
                            let calls = stats.call_count.load(std::sync::atomic::Ordering::Relaxed);
                            let failures = stats
                                .failure_count
                                .load(std::sync::atomic::Ordering::Relaxed);
                            let total_us = stats
                                .total_latency_us
                                .load(std::sync::atomic::Ordering::Relaxed);
                            hook.call_count = calls;
                            hook.failure_count = failures;
                            hook.avg_latency_ms = total_us.checked_div(calls).unwrap_or(0) / 1000;
                        }
                    }
                }

                Ok(serde_json::json!({ "hooks": list }))
            })
        }),
    );

    // hooks.enable — re-enable a previously disabled hook.
    reg.register(
        "hooks.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;

                ctx.state.inner.write().await.disabled_hooks.remove(name);

                // Persist disabled hooks list.
                persist_disabled_hooks(&ctx.state).await;

                // Rebuild hooks.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.disable — disable a hook without removing its files.
    reg.register(
        "hooks.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;

                ctx.state
                    .inner
                    .write()
                    .await
                    .disabled_hooks
                    .insert(name.to_string());

                // Persist disabled hooks list.
                persist_disabled_hooks(&ctx.state).await;

                // Rebuild hooks.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.save — write HOOK.md content back to disk.
    reg.register(
        "hooks.save",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;
                let content = ctx
                    .params
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing content")
                    })?;

                // Find the hook's source path.
                let source_path = {
                    let inner = ctx.state.inner.read().await;
                    inner
                        .discovered_hooks
                        .iter()
                        .find(|h| h.name == name)
                        .map(|h| h.source_path.clone())
                };

                let source_path = source_path.ok_or_else(|| {
                    ErrorShape::new(error_codes::INVALID_REQUEST, "hook not found")
                })?;

                // Write the content to HOOK.md.
                let hook_md_path = PathBuf::from(&source_path).join("HOOK.md");
                std::fs::write(&hook_md_path, content).map_err(|e| {
                    ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        format!("failed to write HOOK.md: {e}"),
                    )
                })?;

                // Reload hooks to pick up the changes.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.reload — re-run discovery and rebuild the registry.
    reg.register(
        "hooks.reload",
        Box::new(|ctx| {
            Box::pin(async move {
                reload_hooks(&ctx.state).await;
                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // ── OpenClaw import ─────────────────────────────────────────────────

    reg.register(
        "openclaw.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.scan",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_scan()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Logs ────────────────────────────────────────────────────────────────

    reg.register(
        "logs.tail",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .tail(ctx.params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .list(ctx.params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.ack",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .ack()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}
