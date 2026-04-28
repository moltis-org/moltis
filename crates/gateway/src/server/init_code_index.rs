//! Initialize the code index system.
//!
//! When the `qmd` feature is enabled and a QMD binary is available on the
//! system, creates a [`moltis_code_index::CodeIndex`] in full mode (discover,
//! filter, status, peek, and search all work).
//!
//! When the `code-index-builtin` feature is enabled, creates a builtin
//! SQLite+FTS5 backend for local code indexing.
//!
//! If neither feature is enabled (or QMD is unavailable), falls back to
//! config-only mode where search operations return [`BackendUnavailable`]
//! gracefully.
//!
//! Per-project collection registration is deferred — the `QmdManager` starts
//! with empty collections. When `index_project()` is called, collections are
//! configured using [`backend_qmd::qmd_config_for_project`].

use std::sync::Arc;

use tracing::{info, warn};

/// Initialize the code index.
///
/// Reads `[code_index]` from the loaded `MoltisConfig`. Falls back to
/// `CodeIndexConfig::default()` when the section is absent or empty.
///
/// Checks QMD availability when the feature is enabled.
/// Falls back to config-only mode if QMD is absent.
pub(crate) async fn init_code_index(
    data_dir: &std::path::Path,
    config: &moltis_config::MoltisConfig,
) -> Arc<moltis_code_index::CodeIndex> {
    // Build CodeIndexConfig from TOML, then overlay data_dir.
    let mut code_index_config = moltis_code_index::CodeIndexConfig::from(&config.code_index);
    // TOML data_dir overrides the default; if not set, use data_dir/code-index.
    if code_index_config.data_dir.is_none() {
        code_index_config.data_dir = Some(data_dir.join("code-index"));
    }

    if !config.code_index.enabled {
        info!("code-index: disabled via [code_index].enabled = false");
        return Arc::new(moltis_code_index::CodeIndex::config_only(code_index_config));
    }

    #[cfg(feature = "qmd")]
    {
        let qmd_config = moltis_qmd::QmdManagerConfig {
            command: "qmd".into(),
            collections: std::collections::HashMap::new(),
            max_results: 20,
            timeout_ms: 30_000,
            work_dir: data_dir.to_path_buf(),
            index_name: format!("code-{}", super::helpers::sanitize_qmd_index_name(data_dir)),
            env_overrides: std::collections::HashMap::new(),
        };
        let qmd = moltis_qmd::QmdManager::new(qmd_config);

        if qmd.is_available().await {
            info!(
                index = %qmd.index_name(),
                "code-index: QMD backend available, initializing in full mode"
            );
            return Arc::new(moltis_code_index::CodeIndex::new(code_index_config, qmd));
        }

        #[cfg(feature = "code-index-builtin")]
        info!("code-index: QMD binary not found, trying builtin backend");

        #[cfg(not(feature = "code-index-builtin"))]
        warn!(
            "code-index: QMD binary not found, falling back to config-only mode \
             (search unavailable until QMD is installed)"
        );
    }

    #[cfg(feature = "code-index-builtin")]
    {
        let default_index_root = data_dir.join("code-index");
        let index_root = code_index_config
            .data_dir
            .as_deref()
            .unwrap_or(default_index_root.as_path());
        let db_path = index_root.join("index.db");
        match moltis_code_index::store_sqlite::SqliteCodeIndexStore::new(&db_path).await {
            Ok(store) => {
                info!(path = %db_path.display(), "code-index: builtin SQLite backend initialized");
                return Arc::new(moltis_code_index::CodeIndex::new_builtin(
                    code_index_config,
                    Box::new(store),
                    None,
                ));
            },
            Err(e) => {
                warn!(
                    path = %db_path.display(),
                    error = %e,
                    "code-index: failed to initialize builtin backend, falling back to config-only"
                );
            },
        }
    }

    #[cfg(not(any(feature = "qmd", feature = "code-index-builtin")))]
    {
        info!(
            "code-index: initialized in config-only mode \
             (qmd feature disabled — search unavailable)"
        );
    }

    Arc::new(moltis_code_index::CodeIndex::config_only(code_index_config))
}
