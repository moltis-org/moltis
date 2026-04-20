//! Session-end memory summary.
//!
//! Before a session is cleared (`sessions.reset`), runs an LLM-powered
//! silent turn to summarize what was accomplished and save it to memory.
//! Gated by `[memory] enable_session_summary = true` (default: true).

use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::state::GatewayState;

/// Run the session-end memory summary if the config flag is enabled and
/// the session has enough messages to be worth summarizing.
pub(crate) async fn run_session_summary_if_enabled(state: &Arc<GatewayState>, session_key: &str) {
    let config = &state.config;
    if !config.memory.enable_session_summary {
        return;
    }

    let write_mode = config.memory.agent_write_mode;
    if !moltis_chat::memory_write_mode_allows_save(write_mode) {
        debug!("session summary: agent memory writes disabled, skipping");
        return;
    }

    let Some(session_store) = state.services.session_store.as_ref() else {
        return;
    };
    let history = match session_store.read(session_key).await {
        Ok(h) if h.len() >= 4 => h, // at least 2 turns
        Ok(_) => {
            debug!("session summary: too few messages, skipping");
            return;
        },
        Err(e) => {
            warn!(error = %e, "session summary: failed to read session history");
            return;
        },
    };

    let Some(mm) = state.memory_manager.as_ref() else {
        return;
    };

    // Read session metadata once for both provider resolution and agent ID.
    let session_entry = if let Some(ref meta) = state.services.session_metadata {
        meta.get(session_key).await
    } else {
        None
    };

    // Resolve a provider for the summary LLM call.
    let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
        let inner = state.inner.read().await;
        let Some(ref registry) = inner.llm_providers else {
            return;
        };
        let reg = registry.read().await;
        let session_model = session_entry.as_ref().and_then(|e| e.model.clone());
        let resolved = session_model
            .and_then(|id| reg.get(&id))
            .or_else(|| reg.first());
        match resolved {
            Some(p) => p,
            None => {
                debug!("session summary: no provider available, skipping");
                return;
            },
        }
    };

    let agent_id = session_entry
        .and_then(|e| e.agent_id)
        .unwrap_or_else(|| "main".to_string());

    let chat_msgs = moltis_agents::model::values_to_chat_messages(&history);
    let writer: Arc<dyn moltis_agents::memory_writer::MemoryWriter> = Arc::new(
        moltis_chat::AgentScopedMemoryWriter::new(Arc::clone(mm), agent_id, write_mode),
    );

    match moltis_agents::silent_turn::run_silent_memory_turn_with_prompt(
        provider,
        &chat_msgs,
        writer,
        moltis_agents::silent_turn::SilentTurnPrompt::SessionSummary,
    )
    .await
    {
        Ok(paths) if !paths.is_empty() => {
            info!(
                files = paths.len(),
                session = %session_key,
                "session-end summary: wrote memory files"
            );
        },
        Ok(_) => {},
        Err(e) => {
            warn!(error = %e, "session-end summary failed");
        },
    }
}
