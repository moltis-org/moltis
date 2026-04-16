//! Initialize the code index system.
//!
//! When the `qmd` feature is enabled and a QMD binary is available on the
//! system, creates a [`moltis_code_index::CodeIndex`] in full mode (discover,
//! filter, status, peek, and search all work).
//!
//! If QMD is unavailable or the feature is disabled, falls back to config-only
//! mode where search operations return [`BackendUnavailable`] gracefully.
//!
//! Per-project collection registration is deferred — the `QmdManager` starts
//! with empty collections. When `index_project()` is called, collections are
//! configured using [`backend_qmd::qmd_config_for_project`].

use std::sync::Arc;

use tracing::{info, warn};

/// Initialize the code index.
///
/// Checks QMD availability when the feature is enabled.
/// Falls back to config-only mode if QMD is absent.
pub(crate) async fn init_code_index(
    data_dir: &std::path::Path,
) -> Arc<moltis_code_index::CodeIndex> {
    let mut code_index_config = moltis_code_index::CodeIndexConfig::default();
    code_index_config.data_dir = Some(data_dir.join("code-index"));

    #[cfg(feature = "qmd")]
    {
        let qmd_config = moltis_qmd::QmdManagerConfig {
            command: "qmd".into(),
            collections: std::collections::HashMap::new(),
            max_results: 20,
            timeout_ms: 30_000,
            work_dir: data_dir.to_path_buf(),
            index_name: format!(
                "code-{}",
                super::helpers::sanitize_qmd_index_name(data_dir)
            ),
            env_overrides: std::collections::HashMap::new(),
        };
        let qmd = moltis_qmd::QmdManager::new(qmd_config);

        if qmd.is_available().await {
            info!(
                index = %qmd.index_name(),
                "code-index: QMD backend available, initializing in full mode"
            );
            return Arc::new(moltis_code_index::CodeIndex::new(
                code_index_config,
                qmd,
            ));
        }

        warn!(
            "code-index: QMD binary not found, falling back to config-only mode \
             (search unavailable until QMD is installed)"
        );
    }

    #[cfg(not(feature = "qmd"))]
    {
        info!(
            "code-index: initialized in config-only mode \
             (qmd feature disabled — search unavailable)"
        );
    }

    Arc::new(moltis_code_index::CodeIndex::config_only(code_index_config))
}
