use std::{collections::HashMap, path::Path as FsPath, sync::Arc};

#[cfg(feature = "local-embeddings")]
use std::path::PathBuf;

use {
    secrecy::ExposeSecret,
    tracing::{info, warn},
};

use super::helpers::{ensure_ollama_model, env_value_with_overrides};

/// Initialize the memory system (embedding providers, sync, watchers).
///
/// Returns `Some(runtime)` when the memory system is available, or `None`
/// when no embedding provider could be resolved or the database could not
/// be opened.
pub(crate) async fn init_memory_system(
    config: &moltis_config::MoltisConfig,
    data_dir: &FsPath,
    effective_providers: &moltis_config::schema::ProvidersConfig,
    runtime_env_overrides: &HashMap<String, String>,
    db_pool_max_connections: u32,
) -> Option<moltis_memory::runtime::DynMemoryRuntime> {
    // Build embedding provider(s) for the fallback chain.
    let mut embedding_providers: Vec<(
        String,
        Box<dyn moltis_memory::embeddings::EmbeddingProvider>,
    )> = Vec::new();

    let mem_cfg = &config.memory;

    if mem_cfg.disable_rag {
        info!("memory: RAG disabled via memory.disable_rag=true, using keyword-only search");
    } else {
        // 1. If user explicitly configured an embedding provider, use it.
        if let Some(provider) = mem_cfg.provider {
            match provider {
                moltis_config::MemoryProvider::Local => {
                    #[cfg(feature = "local-embeddings")]
                    {
                        let cache_dir = mem_cfg
                            .base_url
                            .as_ref()
                            .map(PathBuf::from)
                            .unwrap_or_else(
                                moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::default_cache_dir,
                            );
                        match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::ensure_model(
                            cache_dir,
                        )
                        .await
                        {
                            Ok(path) => {
                                match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::new(
                                    path,
                                ) {
                                    Ok(p) => embedding_providers.push(("local-gguf".into(), Box::new(p))),
                                    Err(e) => warn!("memory: failed to load local GGUF model: {e}"),
                                }
                            },
                            Err(e) => warn!("memory: failed to ensure local model: {e}"),
                        }
                    }
                    #[cfg(not(feature = "local-embeddings"))]
                    warn!(
                        "memory: 'local' embedding provider requires the 'local-embeddings' feature"
                    );
                },
                moltis_config::MemoryProvider::Ollama
                | moltis_config::MemoryProvider::Custom
                | moltis_config::MemoryProvider::OpenAi => {
                    let base_url = mem_cfg.base_url.clone().unwrap_or_else(|| match provider {
                        moltis_config::MemoryProvider::Ollama => "http://localhost:11434".into(),
                        _ => "https://api.openai.com".into(),
                    });
                    if provider == moltis_config::MemoryProvider::Ollama {
                        let model = mem_cfg.model.as_deref().unwrap_or("nomic-embed-text");
                        ensure_ollama_model(&base_url, model).await;
                    }
                    let api_key = mem_cfg
                        .api_key
                        .as_ref()
                        .map(|k| k.expose_secret().clone())
                        .or_else(|| {
                            env_value_with_overrides(runtime_env_overrides, "OPENAI_API_KEY")
                        })
                        .unwrap_or_default();
                    let mut e =
                        moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                    if base_url != "https://api.openai.com" {
                        e = e.with_base_url(base_url);
                    }
                    if let Some(ref model) = mem_cfg.model {
                        e = e.with_model(model.clone(), 1536);
                    }
                    let provider_name = match provider {
                        moltis_config::MemoryProvider::Ollama => "ollama",
                        moltis_config::MemoryProvider::Custom => "custom",
                        moltis_config::MemoryProvider::OpenAi => "openai",
                        moltis_config::MemoryProvider::Local => "local",
                    };
                    embedding_providers.push((provider_name.to_owned(), Box::new(e)));
                },
            }
        }

        // 2. Auto-detect: try Ollama health check.
        if embedding_providers.is_empty() {
            let ollama_ok = reqwest::Client::new()
                .get("http://localhost:11434/api/tags")
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
                .is_ok();
            if ollama_ok {
                ensure_ollama_model("http://localhost:11434", "nomic-embed-text").await;
                let e =
                    moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(String::new())
                        .with_base_url("http://localhost:11434".into())
                        .with_model("nomic-embed-text".into(), 768);
                embedding_providers.push(("ollama".into(), Box::new(e)));
                info!("memory: detected Ollama at localhost:11434");
            }
        }

        // 3. Auto-detect: try remote API-key providers.
        const EMBEDDING_CANDIDATES: &[(&str, &str, &str)] = &[
            ("openai", "OPENAI_API_KEY", "https://api.openai.com"),
            ("mistral", "MISTRAL_API_KEY", "https://api.mistral.ai/v1"),
            (
                "openrouter",
                "OPENROUTER_API_KEY",
                "https://openrouter.ai/api/v1",
            ),
            ("groq", "GROQ_API_KEY", "https://api.groq.com/openai"),
            ("xai", "XAI_API_KEY", "https://api.x.ai"),
            ("deepseek", "DEEPSEEK_API_KEY", "https://api.deepseek.com"),
            ("cerebras", "CEREBRAS_API_KEY", "https://api.cerebras.ai/v1"),
            ("minimax", "MINIMAX_API_KEY", "https://api.minimax.io/v1"),
            ("moonshot", "MOONSHOT_API_KEY", "https://api.moonshot.ai/v1"),
            ("venice", "VENICE_API_KEY", "https://api.venice.ai/api/v1"),
        ];

        for (config_name, env_key, default_base) in EMBEDDING_CANDIDATES {
            let key = effective_providers
                .get(config_name)
                .and_then(|e| e.api_key.as_ref().map(|k| k.expose_secret().clone()))
                .or_else(|| env_value_with_overrides(runtime_env_overrides, env_key))
                .filter(|k| !k.is_empty());
            if let Some(api_key) = key {
                let base = effective_providers
                    .get(config_name)
                    .and_then(|e| e.base_url.clone())
                    .unwrap_or_else(|| default_base.to_string());
                let mut e = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                if base != "https://api.openai.com" {
                    e = e.with_base_url(base);
                }
                embedding_providers.push((config_name.to_string(), Box::new(e)));
            }
        }
    }

    // Build the final embedder: fallback chain, single provider, or keyword-only.
    let embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>> =
        if mem_cfg.disable_rag {
            None
        } else if embedding_providers.is_empty() {
            info!("memory: no embedding provider found, using keyword-only search");
            None
        } else {
            let names: Vec<&str> = embedding_providers
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            if embedding_providers.len() == 1 {
                if let Some((name, provider)) = embedding_providers.into_iter().next() {
                    info!(provider = %name, "memory: using single embedding provider");
                    Some(provider)
                } else {
                    None
                }
            } else {
                info!(providers = ?names, active = names[0], "memory: fallback chain configured");
                Some(Box::new(
                    moltis_memory::embeddings_fallback::FallbackEmbeddingProvider::new(
                        embedding_providers,
                    ),
                ))
            }
        };

    let memory_db_path = data_dir.join("memory.db");
    let memory_pool_result = {
        use {
            sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
            std::str::FromStr,
        };
        let options =
            match SqliteConnectOptions::from_str(&format!("sqlite:{}", memory_db_path.display())) {
                Ok(options) => options,
                Err(error) => {
                    tracing::warn!(
                        path = %memory_db_path.display(),
                        error = %error,
                        "memory: invalid memory database path"
                    );
                    return None;
                },
            }
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));
        sqlx::pool::PoolOptions::new()
            .max_connections(db_pool_max_connections)
            .connect_with(options)
            .await
    };
    match memory_pool_result {
        Ok(memory_pool) => {
            if let Err(e) = moltis_memory::schema::run_migrations(&memory_pool).await {
                tracing::warn!("memory migration failed: {e}");
                None
            } else {
                build_memory_runtime(mem_cfg, data_dir, embedder, memory_pool).await
            }
        },
        Err(e) => {
            tracing::warn!("memory: failed to open memory.db: {e}");
            None
        },
    }
}

/// Build the memory runtime, start initial sync, file watchers, and periodic syncs.
async fn build_memory_runtime(
    mem_cfg: &moltis_config::schema::MemoryEmbeddingConfig,
    data_dir: &FsPath,
    embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>>,
    memory_pool: sqlx::SqlitePool,
) -> Option<moltis_memory::runtime::DynMemoryRuntime> {
    let data_memory_file = data_dir.join("MEMORY.md");
    let data_memory_file_lower = data_dir.join("memory.md");
    let data_memory_sub = data_dir.join("memory");
    let agents_root = data_dir.join("agents");

    if let Err(error) = std::fs::create_dir_all(&data_memory_sub) {
        tracing::warn!(
            path = %data_memory_sub.display(),
            error = %error,
            "memory: failed to create memory directory"
        );
    }
    if let Err(error) = std::fs::create_dir_all(&agents_root) {
        tracing::warn!(
            path = %agents_root.display(),
            error = %error,
            "memory: failed to create agents directory"
        );
    }

    let memory_runtime_config = moltis_memory::config::MemoryConfig {
        db_path: data_dir.join("memory.db").to_string_lossy().into(),
        data_dir: Some(data_dir.to_path_buf()),
        memory_dirs: vec![
            data_memory_file,
            data_memory_file_lower,
            data_memory_sub,
            agents_root,
        ],
        citations: match mem_cfg.citations {
            moltis_config::MemoryCitationsMode::On => moltis_memory::config::CitationMode::On,
            moltis_config::MemoryCitationsMode::Off => moltis_memory::config::CitationMode::Off,
            moltis_config::MemoryCitationsMode::Auto => moltis_memory::config::CitationMode::Auto,
        },
        llm_reranking: mem_cfg.llm_reranking,
        merge_strategy: match mem_cfg.search_merge_strategy {
            moltis_config::MemorySearchMergeStrategy::Rrf => {
                moltis_memory::config::MergeStrategy::Rrf
            },
            moltis_config::MemorySearchMergeStrategy::Linear => {
                moltis_memory::config::MergeStrategy::Linear
            },
        },
        ..Default::default()
    };

    let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(
        memory_pool,
    ));
    let memory_dirs_for_watch = memory_runtime_config.memory_dirs.clone();
    let builtin_manager = Arc::new(if let Some(embedder) = embedder {
        moltis_memory::manager::MemoryManager::new(memory_runtime_config, store, embedder)
    } else {
        moltis_memory::manager::MemoryManager::keyword_only(memory_runtime_config, store)
    });
    let manager: moltis_memory::runtime::DynMemoryRuntime = match mem_cfg.backend {
        moltis_config::MemoryBackend::Builtin => builtin_manager.clone(),
        moltis_config::MemoryBackend::Qmd => {
            #[cfg(feature = "qmd")]
            {
                let qmd_manager =
                    Arc::new(moltis_qmd::QmdManager::new(moltis_qmd::QmdManagerConfig {
                        command: mem_cfg.qmd.command.clone().unwrap_or_else(|| "qmd".into()),
                        collections: super::helpers::build_qmd_collections(data_dir, &mem_cfg.qmd),
                        max_results: mem_cfg.qmd.max_results.unwrap_or(20),
                        timeout_ms: mem_cfg.qmd.timeout_ms.unwrap_or(30_000),
                        work_dir: data_dir.to_path_buf(),
                        index_name: super::helpers::sanitize_qmd_index_name(data_dir),
                        env_overrides: HashMap::new(),
                    }));

                if qmd_manager.is_available().await {
                    info!(
                        index = %qmd_manager.index_name(),
                        collections = qmd_manager.collections().len(),
                        "memory: using QMD backend"
                    );
                    Arc::new(moltis_qmd::QmdMemoryRuntime::new(
                        qmd_manager,
                        builtin_manager.clone(),
                        mem_cfg.disable_rag,
                    ))
                } else {
                    warn!(
                        "memory: QMD backend requested but qmd is unavailable, falling back to builtin memory"
                    );
                    builtin_manager.clone()
                }
            }

            #[cfg(not(feature = "qmd"))]
            {
                warn!(
                    "memory: QMD backend requested but the gateway was built without the qmd feature, falling back to builtin memory"
                );
                builtin_manager.clone()
            }
        },
    };

    // Initial sync + periodic re-sync (15min with watcher, 5min without).
    let sync_manager = Arc::clone(&manager);
    tokio::spawn(async move {
        match sync_manager.sync().await {
            Ok(report) => {
                info!(
                    updated = report.files_updated,
                    unchanged = report.files_unchanged,
                    removed = report.files_removed,
                    errors = report.errors,
                    cache_hits = report.cache_hits,
                    cache_misses = report.cache_misses,
                    "memory: initial sync complete"
                );
                match sync_manager.status().await {
                    Ok(status) => info!(
                        files = status.total_files,
                        chunks = status.total_chunks,
                        db_size = %status.db_size_display(),
                        model = %status.embedding_model,
                        "memory: status"
                    ),
                    Err(e) => tracing::warn!("memory: failed to get status: {e}"),
                }
            },
            Err(e) => tracing::warn!("memory: initial sync failed: {e}"),
        }

        // Start file watcher for real-time sync (if feature enabled).
        #[cfg(feature = "file-watcher")]
        {
            let watcher_manager = Arc::clone(&sync_manager);
            let watch_specs = moltis_memory::watcher::build_watch_specs(&memory_dirs_for_watch);
            match moltis_memory::watcher::MemoryFileWatcher::start(watch_specs) {
                Ok((_watcher, mut rx)) => {
                    info!("memory: file watcher started");
                    tokio::spawn(async move {
                        while let Some(event) = rx.recv().await {
                            let path = match &event {
                                moltis_memory::watcher::WatchEvent::Created(p)
                                | moltis_memory::watcher::WatchEvent::Modified(p) => {
                                    Some(p.clone())
                                },
                                moltis_memory::watcher::WatchEvent::Removed(p) => {
                                    if let Err(e) = watcher_manager.sync().await {
                                        tracing::warn!(
                                            path = %p.display(),
                                            error = %e,
                                            "memory: watcher sync (removal) failed"
                                        );
                                    }
                                    None
                                },
                            };
                            if let Some(path) = path
                                && let Err(e) = watcher_manager.sync_path(&path).await
                            {
                                tracing::warn!(
                                    path = %path.display(),
                                    error = %e,
                                    "memory: watcher sync_path failed"
                                );
                            }
                        }
                    });
                },
                Err(e) => {
                    tracing::warn!("memory: failed to start file watcher: {e}");
                },
            }
        }

        // Periodic full sync as safety net (longer interval with watcher).
        #[cfg(feature = "file-watcher")]
        let interval_secs = 900; // 15 minutes
        #[cfg(not(feature = "file-watcher"))]
        let interval_secs = 300; // 5 minutes

        #[cfg(not(feature = "file-watcher"))]
        let _ = memory_dirs_for_watch;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            if let Err(e) = sync_manager.sync().await {
                tracing::warn!("memory: periodic sync failed: {e}");
            }
        }
    });

    info!(
        backend = manager.backend_name(),
        embeddings = manager.has_embeddings(),
        "memory system initialized"
    );
    Some(manager)
}
