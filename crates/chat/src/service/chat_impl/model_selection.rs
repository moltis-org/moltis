use std::sync::Arc;

use {
    moltis_agents::model::{LlmProvider, ReasoningEffort},
    moltis_providers::ProviderRegistry,
};

pub(super) fn resolve_primary(
    registry: &ProviderRegistry,
    model_id: Option<&str>,
    stream_only: bool,
    reasoning_default: Option<ReasoningEffort>,
) -> Result<Arc<dyn LlmProvider>, String> {
    // A bare explicit or saved model is an intentional Off selection.
    if let Some(id) = model_id {
        return registry.get(id).ok_or_else(|| {
            let available: Vec<_> = registry
                .list_models()
                .iter()
                .map(|m| m.id.clone())
                .collect();
            format!("model '{}' not found. available: {:?}", id, available)
        });
    }
    let provider = if stream_only {
        registry.first()
    } else {
        registry.first_with_tools()
    }
    .ok_or_else(|| "no LLM providers configured".to_string())?;
    Ok(reasoning_default
        .and_then(|effort| Arc::clone(&provider).with_reasoning_effort(effort))
        .unwrap_or(provider))
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_agents::model::{ChatMessage, CompletionResponse, StreamEvent},
        moltis_providers::{ModelCapabilities, ModelInfo},
        std::pin::Pin,
        tokio_stream::Stream,
    };

    #[derive(Clone)]
    struct TestProvider {
        tools: bool,
        reasoning: bool,
        effort: Option<ReasoningEffort>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn id(&self) -> &str {
            "model"
        }

        fn supports_tools(&self) -> bool {
            self.tools
        }

        fn reasoning_effort(&self) -> Option<ReasoningEffort> {
            self.effort
        }

        fn with_reasoning_effort(
            self: Arc<Self>,
            effort: ReasoningEffort,
        ) -> Option<Arc<dyn LlmProvider>> {
            self.reasoning.then(|| {
                Arc::new(Self {
                    effort: Some(effort),
                    ..(*self).clone()
                }) as Arc<dyn LlmProvider>
            })
        }

        async fn complete(
            &self,
            _: &[ChatMessage],
            _: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            anyhow::bail!("selection tests must not call the provider")
        }

        fn stream(
            &self,
            _: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            panic!("selection tests must not stream")
        }
    }

    fn registry(reasoning: bool) -> ProviderRegistry {
        let mut registry = ProviderRegistry::empty();
        for (id, tools) in [("plain", false), ("tools", true)] {
            registry.register(
                ModelInfo {
                    id: id.into(),
                    provider: "test".into(),
                    display_name: id.into(),
                    created_at: None,
                    recommended: false,
                    capabilities: ModelCapabilities::infer(id),
                },
                Arc::new(TestProvider {
                    tools,
                    reasoning,
                    effort: None,
                }),
            );
        }
        registry
    }

    #[test]
    fn reasoning_default_only_applies_to_unselected_fallback() -> Result<(), String> {
        for stream_only in [false, true] {
            for supported in [false, true] {
                let registry = registry(supported);
                for default in [None, Some(ReasoningEffort::High)] {
                    let provider = resolve_primary(&registry, None, stream_only, default)?;
                    assert_eq!(provider.supports_tools(), !stream_only);
                    assert_eq!(provider.reasoning_effort(), default.filter(|_| supported));
                    let restored = registry
                        .get(provider.id())
                        .ok_or("persisted model missing")?;
                    assert_eq!(restored.id(), provider.id());
                    assert_eq!(restored.reasoning_effort(), provider.reasoning_effort());
                    // Explicit and session selections share this path, including bare Off IDs.
                    for selected in ["test::plain", "test::plain@reasoning-low"] {
                        let provider =
                            resolve_primary(&registry, Some(selected), stream_only, default)?;
                        let expected =
                            (supported && selected.contains('@')).then_some(ReasoningEffort::Low);
                        assert_eq!(provider.reasoning_effort(), expected);
                        assert!(!provider.supports_tools());
                    }
                }
                assert!(
                    resolve_primary(
                        &registry,
                        Some("missing"),
                        stream_only,
                        Some(ReasoningEffort::High)
                    )
                    .is_err()
                );
            }
        }
        assert!(
            resolve_primary(
                &ProviderRegistry::empty(),
                None,
                false,
                Some(ReasoningEffort::High)
            )
            .is_err()
        );
        Ok(())
    }
}
