//! Static model catalogs and OpenAI-compatible provider definitions.

/// Known Anthropic Claude models (model_id, display_name).
/// Current models listed first, then legacy models.
pub(crate) const ANTHROPIC_MODELS: &[(&str, &str)] = &[
    // Anthropic currently documents Claude 4.6 using alias IDs rather than
    // dated snapshot IDs. Register the documented aliases instead of
    // inventing snapshot suffixes that the API rejects.
    ("claude-opus-4-6", "Claude Opus 4.6"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("claude-opus-4-5-20251101", "Claude Opus 4.5"),
    ("claude-sonnet-4-5-20250929", "Claude Sonnet 4.5"),
    ("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
    ("claude-opus-4-1-20250805", "Claude Opus 4.1"),
    ("claude-sonnet-4-20250514", "Claude Sonnet 4"),
    ("claude-opus-4-20250514", "Claude Opus 4"),
    ("claude-3-7-sonnet-20250219", "Claude 3.7 Sonnet"),
    ("claude-3-haiku-20240307", "Claude 3 Haiku"),
];

/// Known Mistral models.
pub(crate) const MISTRAL_MODELS: &[(&str, &str)] = &[
    ("mistral-large-latest", "Mistral Large"),
    ("codestral-latest", "Codestral"),
];

/// Known Cerebras models.
pub(crate) const CEREBRAS_MODELS: &[(&str, &str)] =
    &[("llama-4-scout-17b-16e-instruct", "Llama 4 Scout (Cerebras)")];

/// Known MiniMax models.
/// See: <https://platform.minimax.io/docs/api-reference/text-anthropic-api>
pub(crate) const MINIMAX_MODELS: &[(&str, &str)] = &[
    ("MiniMax-M2.7", "MiniMax M2.7"),
    ("MiniMax-M2.7-highspeed", "MiniMax M2.7 Highspeed"),
    ("MiniMax-M2.5", "MiniMax M2.5"),
    ("MiniMax-M2.5-highspeed", "MiniMax M2.5 Highspeed"),
    ("MiniMax-M2.1", "MiniMax M2.1"),
    ("MiniMax-M2.1-highspeed", "MiniMax M2.1 Highspeed"),
    ("MiniMax-M2", "MiniMax M2"),
];

/// Known Z.AI (Zhipu) models.
/// See: <https://docs.z.ai/api-reference/llm/chat-completion>
pub(crate) const ZAI_MODELS: &[(&str, &str)] = &[
    ("glm-5", "GLM-5"),
    ("glm-4.7", "GLM-4.7"),
    ("glm-4.7-flash", "GLM-4.7 Flash"),
    ("glm-4.7-flashx", "GLM-4.7 FlashX"),
    ("glm-4.6", "GLM-4.6"),
    ("glm-4.6v", "GLM-4.6V (Vision)"),
    ("glm-4.6v-flash", "GLM-4.6V Flash"),
    ("glm-4.5", "GLM-4.5"),
    ("glm-4.5-air", "GLM-4.5 Air"),
    ("glm-4.5-airx", "GLM-4.5 AirX"),
    ("glm-4.5-flash", "GLM-4.5 Flash"),
    ("glm-4.5v", "GLM-4.5V (Vision)"),
    ("glm-4-32b-0414-128k", "GLM-4 32B 128K"),
];

/// Known Fireworks models.
pub(crate) const FIREWORKS_MODELS: &[(&str, &str)] = &[
    (
        "accounts/fireworks/routers/kimi-k2p5-turbo",
        "Kimi K2.5 Turbo",
    ),
    ("accounts/fireworks/models/deepseek-v3p2", "DeepSeek V3p2"),
    (
        "accounts/fireworks/models/qwen3-235b-a22b-instruct-2507",
        "Qwen3 235B A22B Instruct",
    ),
    (
        "accounts/fireworks/models/llama-v3p1-405b-instruct",
        "Llama 3.1 405B Instruct",
    ),
    (
        "accounts/fireworks/models/llama-v3p1-70b-instruct",
        "Llama 3.1 70B Instruct",
    ),
    (
        "accounts/fireworks/models/qwen3-coder-480b-a35b-instruct",
        "Qwen3 Coder 480B A35B",
    ),
    (
        "accounts/fireworks/models/kimi-k2-instruct-0905",
        "Kimi K2 Instruct",
    ),
];

/// Known Alibaba Cloud Coding Plan models.
/// See: <https://www.alibabacloud.com/help/en/model-studio/coding-plan>
pub(crate) const ALIBABA_CODING_MODELS: &[(&str, &str)] = &[
    ("qwen3.6-plus", "Qwen 3.6 Plus"),
    ("kimi-k2.5", "Kimi K2.5"),
    ("glm-5", "GLM-5"),
    ("MiniMax-M2.5", "MiniMax M2.5"),
    ("qwen3.5-plus", "Qwen 3.5 Plus"),
    ("qwen3-max-2026-01-23", "Qwen3 Max"),
    ("qwen3-coder-next", "Qwen3 Coder Next"),
    ("qwen3-coder-plus", "Qwen3 Coder Plus"),
    ("glm-4.7", "GLM-4.7"),
];

/// Known DeepSeek models.
pub(crate) const DEEPSEEK_MODELS: &[(&str, &str)] = &[
    ("deepseek-chat", "DeepSeek Chat"),
    ("deepseek-reasoner", "DeepSeek Reasoner"),
];

/// Known Moonshot models.
pub(crate) const MOONSHOT_MODELS: &[(&str, &str)] = &[("kimi-k2.5", "Kimi K2.5")];

/// Known Google Gemini models.
/// See: <https://ai.google.dev/gemini-api/docs/models>
pub(crate) const GEMINI_MODELS: &[(&str, &str)] = &[
    ("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
    (
        "gemini-3.1-flash-lite-preview",
        "Gemini 3.1 Flash-Lite Preview",
    ),
    ("gemini-3-flash-preview", "Gemini 3 Flash Preview"),
    ("gemini-2.5-flash-preview-05-20", "Gemini 2.5 Flash Preview"),
    ("gemini-2.5-pro-preview-05-06", "Gemini 2.5 Pro Preview"),
    ("gemini-2.0-flash", "Gemini 2.0 Flash"),
    ("gemini-2.0-flash-lite", "Gemini 2.0 Flash Lite"),
];

/// OpenAI-compatible provider definition for table-driven registration.
pub(crate) struct OpenAiCompatDef {
    pub(crate) config_name: &'static str,
    pub(crate) env_key: &'static str,
    pub(crate) env_base_url_key: &'static str,
    pub(crate) default_base_url: &'static str,
    pub(crate) models: &'static [(&'static str, &'static str)],
    /// Whether to attempt `/models` discovery by default. Providers whose API
    /// does not expose a models endpoint (e.g. MiniMax returns 404) should set
    /// this to `false` so the static catalog is used without a noisy warning.
    /// Users can still override via `fetch_models = true` in config.
    pub(crate) supports_model_discovery: bool,
    /// When `false`, a dummy API key (the provider name) is used if none is
    /// configured. Intended for local servers that don't authenticate.
    pub(crate) requires_api_key: bool,
    /// Local-only providers (Ollama, LM Studio) are skipped unless the user
    /// has an explicit `[providers.<name>]` entry, a `_BASE_URL` env var, or
    /// configured models. This avoids probing localhost when nothing is running.
    /// Also ensures model discovery is always attempted (never short-circuited
    /// by the empty-catalog heuristic).
    pub(crate) local_only: bool,
}

pub(crate) const OPENAI_COMPAT_PROVIDERS: &[OpenAiCompatDef] = &[
    OpenAiCompatDef {
        config_name: "mistral",
        env_key: "MISTRAL_API_KEY",
        env_base_url_key: "MISTRAL_BASE_URL",
        default_base_url: "https://api.mistral.ai/v1",
        models: MISTRAL_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "openrouter",
        env_key: "OPENROUTER_API_KEY",
        env_base_url_key: "OPENROUTER_BASE_URL",
        default_base_url: "https://openrouter.ai/api/v1",
        models: &[],
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "cerebras",
        env_key: "CEREBRAS_API_KEY",
        env_base_url_key: "CEREBRAS_BASE_URL",
        default_base_url: "https://api.cerebras.ai/v1",
        models: CEREBRAS_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "minimax",
        env_key: "MINIMAX_API_KEY",
        env_base_url_key: "MINIMAX_BASE_URL",
        default_base_url: "https://api.minimax.io/v1",
        models: MINIMAX_MODELS,
        // MiniMax API does not expose a /models endpoint (returns 404).
        supports_model_discovery: false,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "moonshot",
        env_key: "MOONSHOT_API_KEY",
        env_base_url_key: "MOONSHOT_BASE_URL",
        default_base_url: "https://api.moonshot.ai/v1",
        models: MOONSHOT_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "zai",
        env_key: "Z_API_KEY",
        env_base_url_key: "Z_BASE_URL",
        default_base_url: "https://api.z.ai/api/paas/v4",
        models: ZAI_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "zai-code",
        env_key: "Z_CODE_API_KEY",
        env_base_url_key: "Z_CODE_BASE_URL",
        default_base_url: "https://api.z.ai/api/coding/paas/v4",
        models: ZAI_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "venice",
        env_key: "VENICE_API_KEY",
        env_base_url_key: "VENICE_BASE_URL",
        default_base_url: "https://api.venice.ai/api/v1",
        models: &[],
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "deepseek",
        env_key: "DEEPSEEK_API_KEY",
        env_base_url_key: "DEEPSEEK_BASE_URL",
        default_base_url: "https://api.deepseek.com",
        models: DEEPSEEK_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "fireworks",
        env_key: "FIREWORKS_API_KEY",
        env_base_url_key: "FIREWORKS_BASE_URL",
        default_base_url: "https://api.fireworks.ai/inference/v1",
        models: FIREWORKS_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "ollama",
        env_key: "OLLAMA_API_KEY",
        env_base_url_key: "OLLAMA_BASE_URL",
        default_base_url: "http://localhost:11434/v1",
        models: &[],
        supports_model_discovery: true,
        requires_api_key: false,
        local_only: true,
    },
    OpenAiCompatDef {
        config_name: "lmstudio",
        env_key: "LMSTUDIO_API_KEY",
        env_base_url_key: "LMSTUDIO_BASE_URL",
        default_base_url: "http://127.0.0.1:1234/v1",
        models: &[],
        supports_model_discovery: true,
        requires_api_key: false,
        local_only: true,
    },
    OpenAiCompatDef {
        config_name: "alibaba-coding",
        env_key: "ALIBABA_CODING_API_KEY",
        env_base_url_key: "ALIBABA_CODING_BASE_URL",
        default_base_url: "https://coding-intl.dashscope.aliyuncs.com/v1",
        models: ALIBABA_CODING_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
    OpenAiCompatDef {
        config_name: "gemini",
        env_key: "GEMINI_API_KEY",
        env_base_url_key: "GEMINI_BASE_URL",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        models: GEMINI_MODELS,
        supports_model_discovery: true,
        requires_api_key: true,
        local_only: false,
    },
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, crate::catalog_to_discovered};

    #[test]
    fn model_lists_not_empty() {
        assert!(!ANTHROPIC_MODELS.is_empty());
        assert!(!crate::openai::default_model_catalog().is_empty());
        assert!(!MISTRAL_MODELS.is_empty());
        assert!(!CEREBRAS_MODELS.is_empty());
        assert!(!MINIMAX_MODELS.is_empty());
        assert!(!ZAI_MODELS.is_empty());
        assert!(!MOONSHOT_MODELS.is_empty());
        assert!(!GEMINI_MODELS.is_empty());
    }

    #[test]
    fn model_lists_have_unique_ids() {
        let openai_models = crate::openai::default_model_catalog();
        let mut openai_ids: Vec<&str> = openai_models.iter().map(|m| m.id.as_str()).collect();
        openai_ids.sort();
        openai_ids.dedup();
        assert_eq!(
            openai_ids.len(),
            openai_models.len(),
            "duplicate OpenAI model IDs found"
        );

        for models in [
            ANTHROPIC_MODELS,
            MISTRAL_MODELS,
            CEREBRAS_MODELS,
            MINIMAX_MODELS,
            ZAI_MODELS,
            MOONSHOT_MODELS,
            GEMINI_MODELS,
        ] {
            let mut ids: Vec<&str> = models.iter().map(|(id, _)| *id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), models.len(), "duplicate model IDs found");
        }
    }

    #[test]
    fn anthropic_catalog_uses_documented_claude_46_aliases() {
        let anthropic_ids: Vec<&str> = ANTHROPIC_MODELS.iter().map(|(id, _)| *id).collect();

        assert!(anthropic_ids.contains(&"claude-opus-4-6"));
        assert!(anthropic_ids.contains(&"claude-sonnet-4-6"));
        assert!(!anthropic_ids.contains(&"claude-opus-4-6-20260301"));
        assert!(!anthropic_ids.contains(&"claude-sonnet-4-6-20260301"));
        assert!(!anthropic_ids.contains(&"claude-haiku-4-6-20260301"));
    }

    #[test]
    fn openai_compat_providers_have_unique_names() {
        let mut names: Vec<&str> = OPENAI_COMPAT_PROVIDERS
            .iter()
            .map(|d| d.config_name)
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), OPENAI_COMPAT_PROVIDERS.len());
    }

    #[test]
    fn openai_compat_providers_have_valid_urls() {
        for def in OPENAI_COMPAT_PROVIDERS {
            assert!(
                def.default_base_url.starts_with("http://")
                    || def.default_base_url.starts_with("https://"),
                "{}: invalid base URL: {}",
                def.config_name,
                def.default_base_url
            );
        }
    }

    #[test]
    fn openai_compat_providers_env_keys_not_empty() {
        for def in OPENAI_COMPAT_PROVIDERS {
            assert!(
                !def.env_key.is_empty(),
                "{}: env_key is empty",
                def.config_name
            );
            assert!(
                !def.env_base_url_key.is_empty(),
                "{}: env_base_url_key is empty",
                def.config_name
            );
        }
    }

    #[test]
    fn alibaba_coding_provider_exists() {
        let alibaba = OPENAI_COMPAT_PROVIDERS
            .iter()
            .find(|d| d.config_name == "alibaba-coding")
            .expect("alibaba-coding entry must exist");
        assert_eq!(alibaba.env_key, "ALIBABA_CODING_API_KEY");
        assert_eq!(
            alibaba.default_base_url,
            "https://coding-intl.dashscope.aliyuncs.com/v1"
        );
        assert!(alibaba.requires_api_key);
        assert!(!alibaba.local_only);
        assert!(alibaba.supports_model_discovery);
    }

    #[test]
    fn alibaba_coding_models_no_duplicates() {
        let mut ids: Vec<&str> = ALIBABA_CODING_MODELS.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            ALIBABA_CODING_MODELS.len(),
            "duplicate model IDs"
        );
    }

    #[test]
    fn alibaba_coding_models_have_recommended() {
        let discovered = catalog_to_discovered(ALIBABA_CODING_MODELS, 4);
        let recommended_count = discovered.iter().filter(|m| m.recommended).count();
        assert_eq!(recommended_count, 4);
    }

    #[test]
    fn ollama_default_base_url_uses_localhost() {
        let ollama = OPENAI_COMPAT_PROVIDERS
            .iter()
            .find(|d| d.config_name == "ollama")
            .expect("ollama entry must exist");
        assert!(
            ollama.default_base_url.contains("localhost"),
            "expected 'localhost' in Ollama default_base_url, got: {}",
            ollama.default_base_url,
        );
    }
}
