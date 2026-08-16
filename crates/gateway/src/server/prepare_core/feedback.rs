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
    tokio::spawn(async move {
        let prune_interval = std::time::Duration::try_from(time::Duration::days(1))
            .unwrap_or(std::time::Duration::MAX);
        loop {
            let removed = feedback.prune().await;
            if removed > 0 {
                tracing::debug!(removed, "pruned expired trace links");
            }
            tokio::time::sleep(prune_interval).await;
        }
    });
}
