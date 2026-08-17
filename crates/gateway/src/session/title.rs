//! Session title auto-generation.
//!
//! Uses a lightweight LLM call to produce a short descriptive title from
//! the first few messages. Runs in the background after the first assistant
//! response so it never blocks the chat flow.

use std::sync::Arc;

use {
    anyhow::{Context, Result},
    tracing::{debug, info, warn},
};

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
    let Err(e) = try_generate_title_if_needed(state, session_key).await else {
        return;
    };
    warn!(error = %e, session = %session_key, "auto-title: generation failed");
}

async fn try_generate_title_if_needed(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> Result<Option<String>> {
    let Some(session_metadata) = state.services.session_metadata.as_ref() else {
        return Ok(None);
    };
    let entry = match session_metadata.get(session_key).await {
        Some(e) => e,
        None => return Ok(None),
    };

    // Skip if the session already has a user-set label.
    if entry.label.is_some() {
        debug!(session = %session_key, "auto-title: session already has label, skipping");
        return Ok(entry.label);
    }

    generate_title_for_session(state, session_key).await
}

/// Unconditionally generate and persist a title for the session.
///
/// Used by both the auto-trigger (via [`generate_title_if_needed`]) and the
/// manual `/title` command / RPC endpoint.
pub(crate) async fn generate_title_for_session(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> Result<Option<String>> {
    let Some(session_store) = state.services.session_store.as_ref() else {
        return Ok(None);
    };
    let Some(session_metadata) = state.services.session_metadata.as_ref() else {
        return Ok(None);
    };

    let history = match session_store.read(session_key).await {
        Ok(h) if h.len() >= MIN_MESSAGES_FOR_TITLE => h,
        Ok(_) => {
            debug!("auto-title: too few messages, skipping");
            return Ok(None);
        },
        Err(e) => {
            warn!(error = %e, "auto-title: failed to read session history");
            return Err(e).context("failed to read session history");
        },
    };

    // Resolve a provider — prefer auxiliary.title_generation, fall back to session model.
    let session_model = session_metadata
        .get(session_key)
        .await
        .and_then(|e| e.model);
    let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
        let inner = state.inner.read().await;
        let Some(ref registry) = inner.llm_providers else {
            debug!("auto-title: no provider registry available");
            return Ok(None);
        };
        let reg = registry.read().await;

        // Try auxiliary title_generation model first.
        let auxiliary_model = state.config.auxiliary.title_generation.as_deref();
        let from_auxiliary = auxiliary_model.and_then(|id| reg.get(id));
        if auxiliary_model.is_some() && from_auxiliary.is_none() {
            let available: Vec<_> = reg.list_models().iter().map(|m| m.id.as_str()).collect();
            warn!(
                requested = auxiliary_model.unwrap_or_default(),
                available = ?available,
                "auto-title: configured auxiliary title_generation model is unavailable"
            );
        }

        // Fall back to the session's own model.
        let from_session = session_model.as_deref().and_then(|id| reg.get(id));

        match from_auxiliary.or(from_session).or_else(|| reg.first()) {
            Some(p) => p,
            None => {
                debug!("auto-title: no provider available, skipping");
                return Ok(None);
            },
        }
    };

    let chat_msgs = moltis_agents::model::values_to_chat_messages_with_tool_result_limit(
        &history,
        state.config.tools.max_tool_result_bytes,
    );
    let title = moltis_agents::title::generate_title(provider, &chat_msgs).await?;
    // Persist the title as the session label and read back the
    // entry atomically so the broadcast version is consistent.
    let entry = session_metadata
        .upsert(session_key, Some(title.clone()))
        .await
        .with_context(|| format!("failed to persist title for session {session_key}"))?;

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

    Ok(Some(title))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::{pin::Pin, sync::Arc};

    use {
        async_trait::async_trait,
        moltis_agents::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage},
        moltis_auth::{AuthMode, ResolvedAuth},
        moltis_providers::{ModelCapabilities, ModelInfo, ProviderRegistry},
        moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
        tokio::sync::RwLock,
        tokio_stream::Stream,
    };

    use {super::*, crate::services::GatewayServices};

    struct MockTitleProvider {
        result: Result<&'static str>,
    }

    #[async_trait]
    impl LlmProvider for MockTitleProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn id(&self) -> &str {
            "mock-title"
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> Result<CompletionResponse> {
            let text = match &self.result {
                Ok(title) => Some((*title).to_string()),
                Err(e) => anyhow::bail!(e.to_string()),
            };
            Ok(CompletionResponse {
                text,
                tool_calls: Vec::new(),
                usage: Usage::default(),
            })
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    async fn test_state(provider: Arc<dyn LlmProvider>) -> (Arc<GatewayState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_projects::run_migrations(&pool).await.unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        let session_metadata = Arc::new(SqliteSessionMetadata::new(pool));
        let services = GatewayServices::noop()
            .with_session_store(Arc::clone(&session_store))
            .with_session_metadata(Arc::clone(&session_metadata));
        let state = GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            services,
        );

        let mut registry = ProviderRegistry::empty();
        registry.register(
            ModelInfo {
                id: "mock-title".to_string(),
                provider: "mock".to_string(),
                display_name: "Mock Title".to_string(),
                created_at: None,
                recommended: false,
                capabilities: ModelCapabilities::infer("mock-title"),
            },
            provider,
        );
        state.inner.write().await.llm_providers = Some(Arc::new(RwLock::new(registry)));

        session_metadata.upsert("session:test", None).await.unwrap();
        session_store
            .append(
                "session:test",
                &serde_json::json!({"role": "user", "content": "How do I deploy Moltis?"}),
            )
            .await
            .unwrap();
        session_store
            .append(
                "session:test",
                &serde_json::json!({"role": "assistant", "content": "Use the Docker image."}),
            )
            .await
            .unwrap();
        (state, dir)
    }

    #[tokio::test]
    async fn generate_title_for_session_persists_label() {
        let (state, _dir) = test_state(Arc::new(MockTitleProvider {
            result: Ok("Docker Deployment"),
        }))
        .await;

        let title = generate_title_for_session(&state, "session:test")
            .await
            .unwrap();

        assert_eq!(title.as_deref(), Some("Docker Deployment"));
        let label = state
            .services
            .session_metadata
            .as_ref()
            .unwrap()
            .get("session:test")
            .await
            .and_then(|entry| entry.label);
        assert_eq!(label.as_deref(), Some("Docker Deployment"));
    }

    #[tokio::test]
    async fn generate_title_for_session_returns_provider_errors() {
        let (state, _dir) = test_state(Arc::new(MockTitleProvider {
            result: Err(anyhow::anyhow!("provider unavailable")),
        }))
        .await;

        let err = generate_title_for_session(&state, "session:test")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("provider unavailable"));
    }

    #[tokio::test]
    async fn generate_title_for_session_skip_keeps_existing_label() {
        let (state, _dir) = test_state(Arc::new(MockTitleProvider {
            result: Ok("Should Not Be Used"),
        }))
        .await;
        let metadata = state.services.session_metadata.as_ref().unwrap();
        metadata
            .upsert("session:short", Some("Existing Label".to_string()))
            .await
            .unwrap();

        let title = generate_title_for_session(&state, "session:short")
            .await
            .unwrap();

        assert_eq!(title, None);
        let label = metadata
            .get("session:short")
            .await
            .and_then(|entry| entry.label);
        assert_eq!(label.as_deref(), Some("Existing Label"));
    }
}
