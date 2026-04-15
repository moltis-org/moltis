//! Config, identity, soul, memory, and environment variable FFI exports.

use std::ffi::c_char;

use crate::{
    callbacks::emit_log,
    helpers::{
        config_dir_string, data_dir_string, encode_error, encode_json, format_bytes,
        parse_ffi_request, record_call, record_error, trace_call, vault_status_string,
        with_ffi_boundary,
    },
    state::BRIDGE,
    types::*,
};

// ── Config FFI ───────────────────────────────────────────────────────────

/// Returns the full `MoltisConfig` as JSON together with `config_dir` and
/// `data_dir` paths. Swift uses this to populate all settings panels.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_config() -> *mut c_char {
    record_call("moltis_get_config");
    trace_call("moltis_get_config");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_get_config called");
        let config = moltis_config::discover_and_load();
        let config_value = match serde_json::to_value(&config) {
            Ok(v) => v,
            Err(e) => return encode_error("serialization_error", &e.to_string()),
        };
        let response = GetConfigResponse {
            config: config_value,
            config_dir: config_dir_string(),
            data_dir: data_dir_string(),
        };
        emit_log("INFO", "bridge", "Config loaded for settings");
        encode_json(&response)
    })
}

/// Accepts a full `MoltisConfig` JSON and saves it via `save_config()`.
/// The TOML writer preserves existing comments in the config file.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_config(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_config");
    trace_call("moltis_save_config");

    with_ffi_boundary(|| {
        let config = match parse_ffi_request::<moltis_config::MoltisConfig>(
            "moltis_save_config",
            request_json,
        ) {
            Ok(c) => c,
            Err(e) => return e,
        };

        emit_log("INFO", "bridge.config", "Saving full config from settings");
        match moltis_config::save_config(&config) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Config saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log("ERROR", "bridge.config", &format!("Save failed: {e}"));
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

// ── Memory FFI ───────────────────────────────────────────────────────────

/// Returns memory status (counts + db size) for the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_status() -> *mut c_char {
    record_call("moltis_memory_status");
    trace_call("moltis_memory_status");

    with_ffi_boundary(|| {
        use {sqlx::sqlite::SqliteConnectOptions, std::str::FromStr};

        let config = moltis_config::discover_and_load();
        let embedding_model = config
            .memory
            .model
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        let has_embeddings = !config.memory.disable_rag;

        let db_path = moltis_config::data_dir().join("memory.db");
        let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

        if !db_path.exists() {
            let response = MemoryStatusResponse {
                available: false,
                total_files: 0,
                total_chunks: 0,
                db_size,
                db_size_display: format_bytes(db_size),
                embedding_model,
                has_embeddings,
                error: Some("memory.db not found".to_owned()),
            };
            return encode_json(&response);
        }

        let options = match SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
        {
            Ok(opts) => opts.create_if_missing(false).read_only(true),
            Err(error) => {
                let response = MemoryStatusResponse {
                    available: false,
                    total_files: 0,
                    total_chunks: 0,
                    db_size,
                    db_size_display: format_bytes(db_size),
                    embedding_model,
                    has_embeddings,
                    error: Some(format!("invalid sqlite path: {error}")),
                };
                return encode_json(&response);
            },
        };

        let pool = match BRIDGE
            .runtime
            .block_on(sqlx::SqlitePool::connect_with(options))
        {
            Ok(pool) => pool,
            Err(error) => {
                let response = MemoryStatusResponse {
                    available: false,
                    total_files: 0,
                    total_chunks: 0,
                    db_size,
                    db_size_display: format_bytes(db_size),
                    embedding_model,
                    has_embeddings,
                    error: Some(format!("failed to open memory.db: {error}")),
                };
                return encode_json(&response);
            },
        };

        let (total_files, total_chunks) = BRIDGE.runtime.block_on(async {
            let has_files_table: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
            let has_chunks_table: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
            )
            .fetch_one(&pool)
            .await
            .unwrap_or(0);

            let files: i64 = if has_files_table > 0 {
                sqlx::query_scalar("SELECT COUNT(*) FROM files")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0)
            } else {
                0
            };
            let chunks: i64 = if has_chunks_table > 0 {
                sqlx::query_scalar("SELECT COUNT(*) FROM chunks")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0)
            } else {
                0
            };

            let files_count: usize = files.max(0).try_into().unwrap_or(0);
            let chunk_count: usize = chunks.max(0).try_into().unwrap_or(0);
            (files_count, chunk_count)
        });
        BRIDGE.runtime.block_on(pool.close());

        let response = MemoryStatusResponse {
            available: true,
            total_files,
            total_chunks,
            db_size,
            db_size_display: format_bytes(db_size),
            embedding_model,
            has_embeddings,
            error: None,
        };
        encode_json(&response)
    })
}

/// Returns memory configuration fields used by the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_config_get() -> *mut c_char {
    record_call("moltis_memory_config_get");
    trace_call("moltis_memory_config_get");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let memory = config.memory;
        let chat = config.chat;
        let response = MemoryConfigResponse {
            style: match memory.style {
                moltis_config::MemoryStyle::Hybrid => "hybrid".to_owned(),
                moltis_config::MemoryStyle::PromptOnly => "prompt-only".to_owned(),
                moltis_config::MemoryStyle::SearchOnly => "search-only".to_owned(),
                moltis_config::MemoryStyle::Off => "off".to_owned(),
            },
            agent_write_mode: match memory.agent_write_mode {
                moltis_config::AgentMemoryWriteMode::Hybrid => "hybrid".to_owned(),
                moltis_config::AgentMemoryWriteMode::PromptOnly => "prompt-only".to_owned(),
                moltis_config::AgentMemoryWriteMode::SearchOnly => "search-only".to_owned(),
                moltis_config::AgentMemoryWriteMode::Off => "off".to_owned(),
            },
            user_profile_write_mode: match memory.user_profile_write_mode {
                moltis_config::UserProfileWriteMode::ExplicitAndAuto => {
                    "explicit-and-auto".to_owned()
                },
                moltis_config::UserProfileWriteMode::ExplicitOnly => "explicit-only".to_owned(),
                moltis_config::UserProfileWriteMode::Off => "off".to_owned(),
            },
            backend: match memory.backend {
                moltis_config::MemoryBackend::Builtin => "builtin".to_owned(),
                moltis_config::MemoryBackend::Qmd => "qmd".to_owned(),
            },
            provider: match memory.provider {
                Some(moltis_config::MemoryProvider::Local) => "local".to_owned(),
                Some(moltis_config::MemoryProvider::Ollama) => "ollama".to_owned(),
                Some(moltis_config::MemoryProvider::OpenAi) => "openai".to_owned(),
                Some(moltis_config::MemoryProvider::Custom) => "custom".to_owned(),
                None => "auto".to_owned(),
            },
            citations: match memory.citations {
                moltis_config::MemoryCitationsMode::On => "on".to_owned(),
                moltis_config::MemoryCitationsMode::Off => "off".to_owned(),
                moltis_config::MemoryCitationsMode::Auto => "auto".to_owned(),
            },
            disable_rag: memory.disable_rag,
            llm_reranking: memory.llm_reranking,
            search_merge_strategy: match memory.search_merge_strategy {
                moltis_config::MemorySearchMergeStrategy::Rrf => "rrf".to_owned(),
                moltis_config::MemorySearchMergeStrategy::Linear => "linear".to_owned(),
            },
            session_export: match memory.session_export {
                moltis_config::SessionExportMode::Off => "off".to_owned(),
                moltis_config::SessionExportMode::OnNewOrReset => "on-new-or-reset".to_owned(),
            },
            prompt_memory_mode: match chat.prompt_memory_mode {
                moltis_config::PromptMemoryMode::LiveReload => "live-reload".to_owned(),
                moltis_config::PromptMemoryMode::FrozenAtSessionStart => {
                    "frozen-at-session-start".to_owned()
                },
            },
            qmd_feature_enabled: cfg!(feature = "qmd"),
        };
        encode_json(&response)
    })
}

/// Updates memory configuration fields used by the settings panel.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_config_update(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_memory_config_update");
    trace_call("moltis_memory_config_update");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<MemoryConfigUpdateRequest>(
            "moltis_memory_config_update",
            request_json,
        ) {
            Ok(request) => request,
            Err(e) => return e,
        };

        let current_config = moltis_config::discover_and_load();
        let current = current_config.memory;
        let current_chat = current_config.chat;
        let style = request.style.unwrap_or_else(|| match current.style {
            moltis_config::MemoryStyle::Hybrid => "hybrid".to_owned(),
            moltis_config::MemoryStyle::PromptOnly => "prompt-only".to_owned(),
            moltis_config::MemoryStyle::SearchOnly => "search-only".to_owned(),
            moltis_config::MemoryStyle::Off => "off".to_owned(),
        });
        let agent_write_mode =
            request
                .agent_write_mode
                .unwrap_or_else(|| match current.agent_write_mode {
                    moltis_config::AgentMemoryWriteMode::Hybrid => "hybrid".to_owned(),
                    moltis_config::AgentMemoryWriteMode::PromptOnly => "prompt-only".to_owned(),
                    moltis_config::AgentMemoryWriteMode::SearchOnly => "search-only".to_owned(),
                    moltis_config::AgentMemoryWriteMode::Off => "off".to_owned(),
                });
        let user_profile_write_mode = request.user_profile_write_mode.unwrap_or_else(|| {
            match current.user_profile_write_mode {
                moltis_config::UserProfileWriteMode::ExplicitAndAuto => {
                    "explicit-and-auto".to_owned()
                },
                moltis_config::UserProfileWriteMode::ExplicitOnly => "explicit-only".to_owned(),
                moltis_config::UserProfileWriteMode::Off => "off".to_owned(),
            }
        });
        let backend = request.backend.unwrap_or_else(|| match current.backend {
            moltis_config::MemoryBackend::Builtin => "builtin".to_owned(),
            moltis_config::MemoryBackend::Qmd => "qmd".to_owned(),
        });
        let provider = request.provider.unwrap_or_else(|| match current.provider {
            Some(moltis_config::MemoryProvider::Local) => "local".to_owned(),
            Some(moltis_config::MemoryProvider::Ollama) => "ollama".to_owned(),
            Some(moltis_config::MemoryProvider::OpenAi) => "openai".to_owned(),
            Some(moltis_config::MemoryProvider::Custom) => "custom".to_owned(),
            None => "auto".to_owned(),
        });
        let citations = request
            .citations
            .unwrap_or_else(|| match current.citations {
                moltis_config::MemoryCitationsMode::On => "on".to_owned(),
                moltis_config::MemoryCitationsMode::Off => "off".to_owned(),
                moltis_config::MemoryCitationsMode::Auto => "auto".to_owned(),
            });
        let llm_reranking = request.llm_reranking.unwrap_or(current.llm_reranking);
        let search_merge_strategy = request
            .search_merge_strategy
            .unwrap_or_else(|| match current.search_merge_strategy {
                moltis_config::MemorySearchMergeStrategy::Rrf => "rrf".to_owned(),
                moltis_config::MemorySearchMergeStrategy::Linear => "linear".to_owned(),
            });
        let session_export = request.session_export.map_or_else(
            || match current.session_export {
                moltis_config::SessionExportMode::Off => "off".to_owned(),
                moltis_config::SessionExportMode::OnNewOrReset => "on-new-or-reset".to_owned(),
            },
            |value| match value {
                SessionExportUpdateValue::Mode(mode) => mode,
                SessionExportUpdateValue::LegacyBool(enabled) => {
                    if enabled {
                        "on-new-or-reset".to_owned()
                    } else {
                        "off".to_owned()
                    }
                },
            },
        );
        let prompt_memory_mode =
            request
                .prompt_memory_mode
                .unwrap_or_else(|| match current_chat.prompt_memory_mode {
                    moltis_config::PromptMemoryMode::LiveReload => "live-reload".to_owned(),
                    moltis_config::PromptMemoryMode::FrozenAtSessionStart => {
                        "frozen-at-session-start".to_owned()
                    },
                });
        let mut disable_rag = current.disable_rag;

        let style_value = match style.as_str() {
            "prompt-only" => moltis_config::MemoryStyle::PromptOnly,
            "search-only" => moltis_config::MemoryStyle::SearchOnly,
            "off" => moltis_config::MemoryStyle::Off,
            _ => moltis_config::MemoryStyle::Hybrid,
        };
        let agent_write_mode_value = match agent_write_mode.as_str() {
            "prompt-only" => moltis_config::AgentMemoryWriteMode::PromptOnly,
            "search-only" => moltis_config::AgentMemoryWriteMode::SearchOnly,
            "off" => moltis_config::AgentMemoryWriteMode::Off,
            _ => moltis_config::AgentMemoryWriteMode::Hybrid,
        };
        let user_profile_write_mode_value = match user_profile_write_mode.as_str() {
            "explicit-only" => moltis_config::UserProfileWriteMode::ExplicitOnly,
            "off" => moltis_config::UserProfileWriteMode::Off,
            _ => moltis_config::UserProfileWriteMode::ExplicitAndAuto,
        };
        let backend_value = match backend.as_str() {
            "qmd" => moltis_config::MemoryBackend::Qmd,
            _ => moltis_config::MemoryBackend::Builtin,
        };
        let provider_value = match provider.as_str() {
            "local" => Some(moltis_config::MemoryProvider::Local),
            "ollama" => Some(moltis_config::MemoryProvider::Ollama),
            "openai" => Some(moltis_config::MemoryProvider::OpenAi),
            "custom" => Some(moltis_config::MemoryProvider::Custom),
            _ => None,
        };
        let citations_value = match citations.as_str() {
            "on" => moltis_config::MemoryCitationsMode::On,
            "off" => moltis_config::MemoryCitationsMode::Off,
            _ => moltis_config::MemoryCitationsMode::Auto,
        };
        let search_merge_strategy_value = match search_merge_strategy.as_str() {
            "linear" => moltis_config::MemorySearchMergeStrategy::Linear,
            _ => moltis_config::MemorySearchMergeStrategy::Rrf,
        };
        let session_export_value = match session_export.as_str() {
            "off" => moltis_config::SessionExportMode::Off,
            _ => moltis_config::SessionExportMode::OnNewOrReset,
        };
        let prompt_memory_mode_value = match prompt_memory_mode.as_str() {
            "frozen-at-session-start" => moltis_config::PromptMemoryMode::FrozenAtSessionStart,
            _ => moltis_config::PromptMemoryMode::LiveReload,
        };

        if let Err(error) = moltis_config::update_config(|cfg| {
            cfg.memory.style = style_value;
            cfg.memory.agent_write_mode = agent_write_mode_value;
            cfg.memory.user_profile_write_mode = user_profile_write_mode_value;
            cfg.memory.backend = backend_value;
            cfg.memory.provider = provider_value;
            cfg.memory.citations = citations_value;
            cfg.memory.llm_reranking = llm_reranking;
            cfg.memory.search_merge_strategy = search_merge_strategy_value;
            if let Some(value) = request.disable_rag {
                cfg.memory.disable_rag = value;
            }
            cfg.memory.session_export = session_export_value;
            cfg.chat.prompt_memory_mode = prompt_memory_mode_value;
            disable_rag = cfg.memory.disable_rag;
        }) {
            record_error("moltis_memory_config_update", "save_failed");
            return encode_error("save_failed", &error.to_string());
        }

        let response = MemoryConfigResponse {
            style,
            agent_write_mode,
            user_profile_write_mode,
            backend,
            provider,
            citations,
            disable_rag,
            llm_reranking,
            search_merge_strategy,
            session_export,
            prompt_memory_mode,
            qmd_feature_enabled: cfg!(feature = "qmd"),
        };
        encode_json(&response)
    })
}

/// Returns QMD availability (binary detection + optional version).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_memory_qmd_status() -> *mut c_char {
    record_call("moltis_memory_qmd_status");
    trace_call("moltis_memory_qmd_status");

    with_ffi_boundary(|| {
        if !cfg!(feature = "qmd") {
            let response = MemoryQmdStatusResponse {
                feature_enabled: false,
                available: false,
                version: None,
                error: Some("QMD feature is disabled in this build".to_owned()),
            };
            return encode_json(&response);
        }

        let command = moltis_config::discover_and_load()
            .memory
            .qmd
            .command
            .unwrap_or_else(|| "qmd".to_owned());

        let output = std::process::Command::new(&command)
            .arg("--version")
            .output();

        let response = match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                let resolved_version = if version.is_empty() {
                    None
                } else {
                    Some(version)
                };
                MemoryQmdStatusResponse {
                    feature_enabled: true,
                    available: true,
                    version: resolved_version,
                    error: None,
                }
            },
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                let detail = if stderr.is_empty() {
                    format!("{command} --version exited with status {}", out.status)
                } else {
                    stderr
                };
                MemoryQmdStatusResponse {
                    feature_enabled: true,
                    available: false,
                    version: None,
                    error: Some(detail),
                }
            },
            Err(error) => MemoryQmdStatusResponse {
                feature_enabled: true,
                available: false,
                version: None,
                error: Some(error.to_string()),
            },
        };

        encode_json(&response)
    })
}

// ── Soul / Identity / User profile FFI ───────────────────────────────────

/// Returns the soul text from `SOUL.md`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_soul() -> *mut c_char {
    record_call("moltis_get_soul");
    trace_call("moltis_get_soul");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_get_soul called");
        let soul = moltis_config::load_soul_for_agent("main");
        encode_json(&GetSoulResponse { soul })
    })
}

/// Saves soul text to `SOUL.md`. Pass `{"soul": null}` to clear.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_soul(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_soul");
    trace_call("moltis_save_soul");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveSoulRequest>("moltis_save_soul", request_json) {
            Ok(r) => r,
            Err(e) => return e,
        };

        emit_log("INFO", "bridge.config", "Saving soul from settings");
        match moltis_config::save_soul_for_agent("main", request.soul.as_deref()) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Soul saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log("ERROR", "bridge.config", &format!("Soul save failed: {e}"));
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Saves identity (name, emoji, theme) to `IDENTITY.md`.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_identity(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_identity");
    trace_call("moltis_save_identity");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<SaveIdentityRequest>("moltis_save_identity", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        let identity = moltis_config::AgentIdentity {
            name: request.name,
            emoji: request.emoji,
            theme: request.theme,
        };

        emit_log("INFO", "bridge.config", "Saving identity from settings");
        match moltis_config::save_identity_for_agent("main", &identity) {
            Ok(path) => {
                emit_log(
                    "INFO",
                    "bridge.config",
                    &format!("Identity saved to {}", path.display()),
                );
                encode_json(&OkResponse { ok: true })
            },
            Err(e) => {
                emit_log(
                    "ERROR",
                    "bridge.config",
                    &format!("Identity save failed: {e}"),
                );
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

/// Saves user profile (name) to config, and mirrors it to `USER.md` when enabled.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_user_profile(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_user_profile");
    trace_call("moltis_save_user_profile");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveUserProfileRequest>(
            "moltis_save_user_profile",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let user = moltis_config::UserProfile {
            name: request.name,
            ..Default::default()
        };

        emit_log("INFO", "bridge.config", "Saving user profile from settings");
        match moltis_config::update_config(|cfg| {
            cfg.user.name = user.name.clone();
        }) {
            Ok(_) => match moltis_config::save_user_with_mode(
                &user,
                moltis_config::discover_and_load()
                    .memory
                    .user_profile_write_mode,
            ) {
                Ok(path) => {
                    let destination = path
                        .as_ref()
                        .map(|value| value.display().to_string())
                        .unwrap_or_else(|| "moltis.toml only".to_string());
                    emit_log(
                        "INFO",
                        "bridge.config",
                        &format!("User profile saved to {destination}"),
                    );
                    encode_json(&OkResponse { ok: true })
                },
                Err(e) => {
                    emit_log(
                        "ERROR",
                        "bridge.config",
                        &format!("User profile save failed: {e}"),
                    );
                    encode_error("save_failed", &e.to_string())
                },
            },
            Err(e) => {
                emit_log(
                    "ERROR",
                    "bridge.config",
                    &format!("User profile config save failed: {e}"),
                );
                encode_error("save_failed", &e.to_string())
            },
        }
    })
}

// ── Environment variables FFI ────────────────────────────────────────────

/// Returns runtime environment variables from the credential store.
/// Values are never returned, only metadata (id/key/timestamps/encrypted).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_list_env_vars() -> *mut c_char {
    record_call("moltis_list_env_vars");
    trace_call("moltis_list_env_vars");

    with_ffi_boundary(|| {
        let env_vars = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.list_env_vars())
        {
            Ok(vars) => vars,
            Err(e) => {
                record_error("moltis_list_env_vars", "ENV_LIST_FAILED");
                return encode_error("ENV_LIST_FAILED", &e.to_string());
            },
        };

        encode_json(&ListEnvVarsResponse {
            env_vars,
            vault_status: vault_status_string(),
        })
    })
}

/// Set (upsert) an environment variable in the credential store.
/// Uses vault encryption automatically when the vault is unsealed.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_set_env_var(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_set_env_var");
    trace_call("moltis_set_env_var");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<SetEnvVarRequest>("moltis_set_env_var", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        let key = request.key.trim();
        if key.is_empty() {
            record_error("moltis_set_env_var", "ENV_KEY_REQUIRED");
            return encode_error("ENV_KEY_REQUIRED", "key is required");
        }
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            record_error("moltis_set_env_var", "ENV_KEY_INVALID");
            return encode_error(
                "ENV_KEY_INVALID",
                "key must contain only letters, digits, and underscores",
            );
        }

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.set_env_var(key, &request.value))
        {
            Ok(_) => encode_json(&OkResponse { ok: true }),
            Err(e) => {
                record_error("moltis_set_env_var", "ENV_SET_FAILED");
                encode_error("ENV_SET_FAILED", &e.to_string())
            },
        }
    })
}

/// Delete an environment variable by ID.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_delete_env_var(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_delete_env_var");
    trace_call("moltis_delete_env_var");

    with_ffi_boundary(|| {
        let request =
            match parse_ffi_request::<DeleteEnvVarRequest>("moltis_delete_env_var", request_json) {
                Ok(r) => r,
                Err(e) => return e,
            };

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.delete_env_var(request.id))
        {
            Ok(_) => encode_json(&OkResponse { ok: true }),
            Err(e) => {
                record_error("moltis_delete_env_var", "ENV_DELETE_FAILED");
                encode_error("ENV_DELETE_FAILED", &e.to_string())
            },
        }
    })
}

// ── Tests ────────────────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;

    fn text_from_ptr(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "ffi returned null pointer");
        let owned = unsafe { CString::from_raw(ptr) };
        match owned.into_string() {
            Ok(text) => text,
            Err(error) => panic!("failed to decode UTF-8 from ffi pointer: {error}"),
        }
    }

    fn json_from_ptr(ptr: *mut c_char) -> Value {
        let text = text_from_ptr(ptr);
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(error) => panic!("failed to parse ffi json payload: {error}; payload={text}"),
        }
    }

    #[test]
    fn get_config_returns_config_and_paths() {
        let payload = json_from_ptr(moltis_get_config());

        assert!(
            payload.get("config").is_some(),
            "get_config should return a 'config' field"
        );
        assert!(
            payload.get("config_dir").and_then(Value::as_str).is_some(),
            "get_config should return config_dir"
        );
        assert!(
            payload.get("data_dir").and_then(Value::as_str).is_some(),
            "get_config should return data_dir"
        );

        // The config should be an object with expected top-level keys.
        let config = payload.get("config").unwrap_or_else(|| panic!("no config"));
        assert!(
            config.get("server").is_some(),
            "config should have a 'server' section"
        );
    }

    #[test]
    fn save_config_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_config(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_config_returns_error_for_invalid_json() {
        let bad = CString::new("not valid json").unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_save_config(bad.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "invalid_json");
    }

    #[test]
    fn memory_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_status());

        assert!(
            payload.get("available").and_then(Value::as_bool).is_some(),
            "memory_status should return available"
        );
        assert!(
            payload.get("total_files").and_then(Value::as_u64).is_some(),
            "memory_status should return total_files"
        );
        assert!(
            payload
                .get("total_chunks")
                .and_then(Value::as_u64)
                .is_some(),
            "memory_status should return total_chunks"
        );
        assert!(
            payload
                .get("db_size_display")
                .and_then(Value::as_str)
                .is_some(),
            "memory_status should return db_size_display"
        );
    }

    #[test]
    fn memory_config_get_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_config_get());

        assert!(
            payload.get("style").and_then(Value::as_str).is_some(),
            "memory_config_get should return style"
        );
        assert!(
            payload
                .get("agent_write_mode")
                .and_then(Value::as_str)
                .is_some(),
            "memory_config_get should return agent_write_mode"
        );
        assert!(
            payload
                .get("user_profile_write_mode")
                .and_then(Value::as_str)
                .is_some(),
            "memory_config_get should return user_profile_write_mode"
        );
        assert!(
            payload.get("backend").and_then(Value::as_str).is_some(),
            "memory_config_get should return backend"
        );
        assert!(
            payload.get("provider").and_then(Value::as_str).is_some(),
            "memory_config_get should return provider"
        );
        assert!(
            payload.get("citations").and_then(Value::as_str).is_some(),
            "memory_config_get should return citations"
        );
        assert!(
            payload
                .get("search_merge_strategy")
                .and_then(Value::as_str)
                .is_some(),
            "memory_config_get should return search_merge_strategy"
        );
        assert!(
            payload
                .get("disable_rag")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_config_get should return disable_rag"
        );
        assert!(
            payload
                .get("llm_reranking")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_config_get should return llm_reranking"
        );
        assert!(
            payload
                .get("prompt_memory_mode")
                .and_then(Value::as_str)
                .is_some(),
            "memory_config_get should return prompt_memory_mode"
        );
    }

    #[test]
    fn memory_config_update_returns_error_for_null() {
        let payload = json_from_ptr(moltis_memory_config_update(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn memory_config_update_round_trip() {
        let request = serde_json::json!({
            "style": "prompt-only",
            "agent_write_mode": "hybrid",
            "user_profile_write_mode": "explicit-only",
            "backend": "builtin",
            "provider": "ollama",
            "citations": "auto",
            "llm_reranking": false,
            "search_merge_strategy": "linear",
            "session_export": "off",
            "prompt_memory_mode": "frozen-at-session-start"
        })
        .to_string();
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_memory_config_update(c_request.as_ptr()));

        assert_eq!(
            payload.get("style").and_then(Value::as_str),
            Some("prompt-only"),
        );
        assert_eq!(
            payload.get("agent_write_mode").and_then(Value::as_str),
            Some("hybrid"),
        );
        assert_eq!(
            payload
                .get("user_profile_write_mode")
                .and_then(Value::as_str),
            Some("explicit-only"),
        );
        assert_eq!(
            payload.get("backend").and_then(Value::as_str),
            Some("builtin"),
        );
        assert_eq!(
            payload.get("provider").and_then(Value::as_str),
            Some("ollama"),
        );
        assert_eq!(
            payload.get("citations").and_then(Value::as_str),
            Some("auto")
        );
        assert_eq!(
            payload.get("search_merge_strategy").and_then(Value::as_str),
            Some("linear")
        );
        assert_eq!(
            payload.get("session_export").and_then(Value::as_str),
            Some("off")
        );
        assert_eq!(
            payload.get("prompt_memory_mode").and_then(Value::as_str),
            Some("frozen-at-session-start")
        );
    }

    #[test]
    fn memory_qmd_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_memory_qmd_status());

        assert!(
            payload
                .get("feature_enabled")
                .and_then(Value::as_bool)
                .is_some(),
            "memory_qmd_status should return feature_enabled"
        );
        assert!(
            payload.get("available").and_then(Value::as_bool).is_some(),
            "memory_qmd_status should return available"
        );
    }

    #[test]
    fn get_soul_returns_soul_field() {
        let payload = json_from_ptr(moltis_get_soul());

        // soul field should exist (may be null or a string)
        assert!(
            payload.get("soul").is_some(),
            "get_soul should return a 'soul' field"
        );
    }

    #[test]
    fn save_soul_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_soul(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_identity_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_identity(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn save_user_profile_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_user_profile(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn list_env_vars_returns_env_vars_and_vault_status() {
        let payload = json_from_ptr(moltis_list_env_vars());

        assert!(
            payload.get("env_vars").and_then(Value::as_array).is_some(),
            "list_env_vars should return env_vars array"
        );
        assert!(
            payload
                .get("vault_status")
                .and_then(Value::as_str)
                .is_some(),
            "list_env_vars should return vault_status"
        );
    }

    #[test]
    fn set_env_var_returns_error_for_null() {
        let payload = json_from_ptr(moltis_set_env_var(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn set_env_var_rejects_invalid_key() {
        let request = r#"{"key":"BAD-KEY","value":"secret"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_set_env_var(c_request.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "ENV_KEY_INVALID");
    }

    #[test]
    fn delete_env_var_returns_error_for_null() {
        let payload = json_from_ptr(moltis_delete_env_var(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn set_and_delete_env_var_round_trip() {
        let key = format!("MACOS_TEST_{}", uuid::Uuid::new_v4().simple());
        let set_request = serde_json::json!({
            "key": key,
            "value": "secret-value"
        })
        .to_string();
        let c_set_request = CString::new(set_request).unwrap_or_else(|e| panic!("{e}"));
        let set_payload = json_from_ptr(moltis_set_env_var(c_set_request.as_ptr()));
        assert_eq!(set_payload.get("ok").and_then(Value::as_bool), Some(true));

        let list_payload = json_from_ptr(moltis_list_env_vars());
        let env_vars = list_payload
            .get("env_vars")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("env_vars missing"));
        let item = env_vars
            .iter()
            .find(|entry| entry.get("key").and_then(Value::as_str) == Some(key.as_str()))
            .unwrap_or_else(|| panic!("saved env var should appear in list"));
        let id = item
            .get("id")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("env var id should be present"));

        let delete_request = serde_json::json!({ "id": id }).to_string();
        let c_delete_request = CString::new(delete_request).unwrap_or_else(|e| panic!("{e}"));
        let delete_payload = json_from_ptr(moltis_delete_env_var(c_delete_request.as_ptr()));
        assert_eq!(
            delete_payload.get("ok").and_then(Value::as_bool),
            Some(true)
        );
    }
}
