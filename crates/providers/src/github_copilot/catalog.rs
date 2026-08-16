/// Current GitHub Copilot model IDs. Live `/models` discovery is preferred;
/// this catalog is used when discovery is unavailable.
pub(super) const COPILOT_MODELS: &[CopilotCatalogModel] = &[
    chat("gpt-4o", "GPT-4o (Copilot)"),
    chat("gpt-4.1", "GPT-4.1 (Copilot)"),
    chat("gpt-4.1-mini", "GPT-4.1 Mini (Copilot)"),
    chat("gpt-4.1-nano", "GPT-4.1 Nano (Copilot)"),
    chat("gpt-5-mini", "GPT-5 mini (Copilot)"),
    responses("gpt-5.3-codex", "GPT-5.3-Codex (Copilot)"),
    responses("gpt-5.4", "GPT-5.4 (Copilot)"),
    responses("gpt-5.4-mini", "GPT-5.4 mini (Copilot)"),
    responses("gpt-5.4-nano", "GPT-5.4 nano (Copilot)"),
    responses("gpt-5.4-pro", "GPT-5.4 Pro (Copilot)"),
    responses("gpt-5.5", "GPT-5.5 (Copilot)"),
    responses("gpt-5.6-luna", "GPT-5.6 Luna (Copilot)"),
    responses("gpt-5.6-sol", "GPT-5.6 Sol (Copilot)"),
    responses("gpt-5.6-terra", "GPT-5.6 Terra (Copilot)"),
    responses("gpt-5.2-pro", "GPT-5.2 Pro (Copilot)"),
    chat("o1", "o1 (Copilot)"),
    chat("o1-mini", "o1-mini (Copilot)"),
    chat("o3-mini", "o3-mini (Copilot)"),
    chat("claude-fable-5", "Claude Fable 5 (Copilot)"),
    chat("claude-haiku-4.5", "Claude Haiku 4.5 (Copilot)"),
    chat("claude-opus-4.5", "Claude Opus 4.5 (Copilot)"),
    chat("claude-opus-4.6", "Claude Opus 4.6 (Copilot)"),
    chat("claude-opus-4.7", "Claude Opus 4.7 (Copilot)"),
    chat("claude-opus-4.8", "Claude Opus 4.8 (Copilot)"),
    chat("claude-sonnet-4", "Claude Sonnet 4 (Copilot)"),
    chat("claude-sonnet-4.5", "Claude Sonnet 4.5 (Copilot)"),
    chat("claude-sonnet-4.6", "Claude Sonnet 4.6 (Copilot)"),
    chat("claude-sonnet-5", "Claude Sonnet 5 (Copilot)"),
    chat("gemini-2.0-flash", "Gemini 2.0 Flash (Copilot)"),
    chat("gemini-2.5-pro", "Gemini 2.5 Pro (Copilot)"),
    chat("gemini-3-flash-preview", "Gemini 3 Flash (Copilot)"),
    chat("gemini-3.1-pro-preview", "Gemini 3.1 Pro (Copilot)"),
    chat("gemini-3.5-flash", "Gemini 3.5 Flash (Copilot)"),
    chat("mai-code-1-flash-picker", "MAI-Code-1-Flash (Copilot)"),
    chat("raptor-mini", "Raptor mini (Copilot)"),
    chat("kimi-k2.7-code", "Kimi K2.7 Code (Copilot)"),
];

pub(super) struct CopilotCatalogModel {
    id: &'static str,
    display_name: &'static str,
    requires_responses_api: bool,
}

const fn chat(id: &'static str, display_name: &'static str) -> CopilotCatalogModel {
    CopilotCatalogModel {
        id,
        display_name,
        requires_responses_api: false,
    }
}

const fn responses(id: &'static str, display_name: &'static str) -> CopilotCatalogModel {
    CopilotCatalogModel {
        id,
        display_name,
        requires_responses_api: true,
    }
}

pub fn default_model_catalog() -> Vec<super::super::DiscoveredModel> {
    COPILOT_MODELS
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let mut capabilities = super::super::ModelCapabilities::infer(model.id);
            capabilities.requires_responses_api = model.requires_responses_api;
            if let Some(context_window) =
                super::super::model_capabilities::context_window_fallback_for_model(
                    super::super::model_capabilities::ContextWindowFallbackScope::GitHubCopilot,
                    model.id,
                )
            {
                capabilities.context_window = context_window;
            }
            super::super::DiscoveredModel::new(model.id, model.display_name)
                .with_recommended(index < 3)
                .with_capabilities(capabilities)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_current_copilot_model_ids() {
        for expected in [
            "gpt-5.6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "claude-opus-4.8",
            "claude-sonnet-5",
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "mai-code-1-flash-picker",
            "kimi-k2.7-code",
        ] {
            assert!(
                COPILOT_MODELS.iter().any(|model| model.id == expected),
                "missing current Copilot model ID: {expected}"
            );
        }
    }

    #[test]
    fn catalog_marks_responses_api_models_as_capability() {
        let catalog = default_model_catalog();
        let Some(luna) = catalog.iter().find(|model| model.id == "gpt-5.6-luna") else {
            panic!("missing gpt-5.6-luna model");
        };
        let Some(luna_capabilities) = luna.capabilities else {
            panic!("missing gpt-5.6-luna capabilities");
        };
        assert!(luna_capabilities.requires_responses_api);

        let Some(mini) = catalog.iter().find(|model| model.id == "gpt-5-mini") else {
            panic!("missing gpt-5-mini model");
        };
        let Some(mini_capabilities) = mini.capabilities else {
            panic!("missing gpt-5-mini capabilities");
        };
        assert!(!mini_capabilities.requires_responses_api);
    }

    #[test]
    fn catalog_marks_copilot_extended_context_models_as_capability() {
        let catalog = default_model_catalog();
        let Some(opus) = catalog.iter().find(|model| model.id == "claude-opus-4.8") else {
            panic!("missing claude-opus-4.8 model");
        };
        let Some(opus_capabilities) = opus.capabilities else {
            panic!("missing claude-opus-4.8 capabilities");
        };
        assert_eq!(opus_capabilities.context_window, 1_000_000);

        let Some(haiku) = catalog.iter().find(|model| model.id == "claude-haiku-4.5") else {
            panic!("missing claude-haiku-4.5 model");
        };
        let Some(haiku_capabilities) = haiku.capabilities else {
            panic!("missing claude-haiku-4.5 capabilities");
        };
        assert_eq!(haiku_capabilities.context_window, 200_000);
    }
}
