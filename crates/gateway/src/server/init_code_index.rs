//! Initialize the code index system.
//!
//! Creates a [`moltis_code_index::CodeIndex`] in config-only mode (discover, filter,
//! status, and peek work; search returns "backend unavailable" gracefully).
//!
//! When the `qmd` feature is enabled and a QMD backend becomes available, the gateway
//! can upgrade to a full `CodeIndex::new()` with search support. That wiring is
//! deferred to avoid coupling code-index initialization to QMD's async availability
//! check during startup.

use std::sync::Arc;

use tracing::info;

/// Initialize the code index in config-only mode.
///
/// Always succeeds — config-only mode requires no external backend.
/// The resulting `CodeIndex` supports discover, filter, status, and peek.
/// Search operations will return `BackendUnavailable` gracefully.
pub(crate) fn init_code_index() -> Arc<moltis_code_index::CodeIndex> {
    let mut code_index_config = moltis_code_index::CodeIndexConfig::default();
    code_index_config.data_dir = Some(moltis_config::data_dir().join("code-index"));

    let index = moltis_code_index::CodeIndex::config_only(code_index_config);
    info!("code-index: initialized in config-only mode (search unavailable until QMD is wired)");
    Arc::new(index)
}