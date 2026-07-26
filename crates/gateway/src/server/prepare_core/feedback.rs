//! Installing the reaction-feedback service during startup.
//!
//! Split out of `post_state` so that file stays inside the size limit; the
//! wiring is self-contained.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::state::GatewayState;

/// Install reply/trace correlation and start the retention prune.
///
/// Needs the database pool, which does not exist at state construction, so it
/// runs here rather than in `GatewayState::new`.
pub(super) fn install_feedback(state: &Arc<GatewayState>, db_pool: &SqlitePool) {
    let links: Arc<dyn moltis_channels::trace_link::TraceLinkStore> = Arc::new(
        crate::trace_link_store::SqliteTraceLinkStore::new(db_pool.clone()),
    );
    state.feedback.apply(
        links,
        &state.config.instrumentation.feedback,
        Some(state.config.instrumentation.environment.clone()),
    );

    // Links accumulate one row per delivered reply; drop the ones too old
    // to attribute a reaction to.
    let feedback = Arc::clone(&state.feedback);
    let retention_days = state.config.instrumentation.feedback.link_retention_days;
    tokio::spawn(async move {
        let removed = feedback.prune(retention_days).await;
        if removed > 0 {
            tracing::debug!(removed, "pruned expired trace links");
        }
    });
}
