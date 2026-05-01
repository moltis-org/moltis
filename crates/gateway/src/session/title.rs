//! Session title auto-generation.
//!
//! Uses a lightweight LLM call to produce a short descriptive title from
//! the first few messages. Runs in the background after the first assistant
//! response so it never blocks the chat flow.

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

/// Minimum number of messages before title generation fires (1 user + 1 assistant).
const MIN_MESSAGES_FOR_TITLE: usize = 2;

/// Generate and persist a session title if the session has no label yet.
///
/// Intended to be called from a background task after the first assistant
/// response. No-ops silently when:
/// - the session already has a label
/// - there are too few messages
/// - no provider is available
pub(crate) async fn generate_title_if_needed(state: &Arc<GatewayState>, session_key: &str) {
    let Some(session_metadata) = state.services.session_metadata.as_ref() else {
        return;
    };
    let entry = match session_metadata.get(session_key).await {
        Some(e) => e,
        None => return,
    };

    // Skip if the session already has a user-set label.
    if entry.label.is_some() {
        debug!(session = %session_key, "auto-title: session already has label, skipping");
        return;
    }

    generate_title_for_session(state, session_key).await;
}

/// Unconditionally generate and persist a title for the session.
///
/// Used by both the auto-trigger (via [`generate_title_if_needed`]) and the
/// manual `/title` command / RPC endpoint.
pub(crate) async fn generate_title_for_session(state: &Arc<GatewayState>, session_key: &str) {
    let Some(session_store) = state.services.session_store.as_ref() else {
        return;
    };
    let Some(session_metadata) = state.services.session_metadata.as_ref() else {
        return;
    };

    let history = match session_store.read(session_key).await {
        Ok(h) if h.len() >= MIN_MESSAGES_FOR_TITLE => h,
        Ok(_) => {
            debug!("auto-title: too few messages, skipping");
            return;
        },
        Err(e) => {
            warn!(error = %e, "auto-title: failed to read session history");
            return;
        },
    };

    // Resolve a provider — prefer auxiliary.title_generation, fall back to session model.
    let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
        let inner = state.inner.read().await;
        let Some(ref registry) = inner.llm_providers else {
            debug!("auto-title: no provider registry available");
            return;
        };
        let reg = registry.read().await;

        // Try auxiliary title_generation model first.
        let auxiliary_model = state.config.auxiliary.title_generation.as_deref();
        let from_auxiliary = auxiliary_model.and_then(|id| reg.get(id));

        // Fall back to the session's own model.
        let entry = session_metadata.get(session_key).await;
        let session_model = entry.and_then(|e| e.model);
        let from_session = session_model.and_then(|id| reg.get(&id));

        match from_auxiliary.or(from_session).or_else(|| reg.first()) {
            Some(p) => p,
            None => {
                debug!("auto-title: no provider available, skipping");
                return;
            },
        }
    };

    let chat_msgs = moltis_agents::model::values_to_chat_messages(&history);
    match moltis_agents::title::generate_title(provider, &chat_msgs).await {
        Ok(title) => {
            // Persist the title as the session label and read back the
            // entry atomically so the broadcast version is consistent.
            let entry = match session_metadata
                .upsert(session_key, Some(title.clone()))
                .await
            {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, session = %session_key, "auto-title: failed to persist title");
                    return;
                },
            };

            info!(session = %session_key, title = %title, "auto-title: set session title");

            broadcast(
                state,
                "session",
                serde_json::json!({
                    "kind": "patched",
                    "sessionKey": session_key,
                    "version": entry.version,
                    "label": title,
                }),
                BroadcastOpts::default(),
            )
            .await;
        },
        Err(e) => {
            warn!(error = %e, session = %session_key, "auto-title: generation failed");
        },
    }
}
