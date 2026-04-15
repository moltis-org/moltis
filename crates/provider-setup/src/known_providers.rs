/// Known provider definitions and auth type enumeration.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    ApiKey,
    Oauth,
    Local,
}

impl AuthType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api-key",
            Self::Oauth => "oauth",
            Self::Local => "local",
        }
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

/// Known provider definitions used to populate the "available providers" list.
pub struct KnownProvider {
    pub name: &'static str,
    pub display_name: &'static str,
    pub auth_type: AuthType,
    pub env_key: Option<&'static str>,
    /// Default base URL for this provider (for OpenAI-compatible providers).
    pub default_base_url: Option<&'static str>,
    /// Whether this provider requires a model to be specified.
    pub requires_model: bool,
    /// Whether the API key is optional (e.g. Ollama and LM Studio run locally
    /// without auth).
    pub key_optional: bool,
    /// Whether this provider only runs locally (binds to localhost) and should
    /// be hidden from cloud deployments. Separate from `key_optional` because a
    /// remote provider could legitimately support unauthenticated access without
    /// binding to localhost.
    pub local_only: bool,
}

impl KnownProvider {
    /// Returns true if this provider is local-only — runs on the user's
    /// machine and isn't reachable from cloud deployments. Used by cloud-mode
    /// filters to hide providers that bind to localhost.
    #[must_use]
    pub fn is_local_only(&self) -> bool {
        self.auth_type == AuthType::Local || self.local_only
    }
}

/// Build the known providers list at runtime, including local-llm if enabled.
pub fn known_providers() -> Vec<KnownProvider> {
    let providers = vec![
        KnownProvider {
            name: "anthropic",
            display_name: "Anthropic",
            auth_type: AuthType::ApiKey,
            env_key: Some("ANTHROPIC_API_KEY"),
            default_base_url: Some("https://api.anthropic.com"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "openai",
            display_name: "OpenAI",
            auth_type: AuthType::ApiKey,
            env_key: Some("OPENAI_API_KEY"),
            default_base_url: Some("https://api.openai.com/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "gemini",
            display_name: "Google Gemini",
            auth_type: AuthType::ApiKey,
            env_key: Some("GEMINI_API_KEY"),
            default_base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "groq",
            display_name: "Groq",
            auth_type: AuthType::ApiKey,
            env_key: Some("GROQ_API_KEY"),
            default_base_url: Some("https://api.groq.com/openai/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "xai",
            display_name: "xAI (Grok)",
            auth_type: AuthType::ApiKey,
            env_key: Some("XAI_API_KEY"),
            default_base_url: Some("https://api.x.ai/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "deepseek",
            display_name: "DeepSeek",
            auth_type: AuthType::ApiKey,
            env_key: Some("DEEPSEEK_API_KEY"),
            default_base_url: Some("https://api.deepseek.com"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "fireworks",
            display_name: "Fireworks",
            auth_type: AuthType::ApiKey,
            env_key: Some("FIREWORKS_API_KEY"),
            default_base_url: Some("https://api.fireworks.ai/inference/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "mistral",
            display_name: "Mistral",
            auth_type: AuthType::ApiKey,
            env_key: Some("MISTRAL_API_KEY"),
            default_base_url: Some("https://api.mistral.ai/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "openrouter",
            display_name: "OpenRouter",
            auth_type: AuthType::ApiKey,
            env_key: Some("OPENROUTER_API_KEY"),
            default_base_url: Some("https://openrouter.ai/api/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "cerebras",
            display_name: "Cerebras",
            auth_type: AuthType::ApiKey,
            env_key: Some("CEREBRAS_API_KEY"),
            default_base_url: Some("https://api.cerebras.ai/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "minimax",
            display_name: "MiniMax",
            auth_type: AuthType::ApiKey,
            env_key: Some("MINIMAX_API_KEY"),
            default_base_url: Some("https://api.minimax.io/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "moonshot",
            display_name: "Moonshot",
            auth_type: AuthType::ApiKey,
            env_key: Some("MOONSHOT_API_KEY"),
            default_base_url: Some("https://api.moonshot.cn/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "zai",
            display_name: "Z.AI",
            auth_type: AuthType::ApiKey,
            env_key: Some("Z_API_KEY"),
            default_base_url: Some("https://api.z.ai/api/paas/v4"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "zai-code",
            display_name: "Z.AI (Coding Plan)",
            auth_type: AuthType::ApiKey,
            env_key: Some("Z_CODE_API_KEY"),
            default_base_url: Some("https://api.z.ai/api/coding/paas/v4"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "venice",
            display_name: "Venice",
            auth_type: AuthType::ApiKey,
            env_key: Some("VENICE_API_KEY"),
            default_base_url: Some("https://api.venice.ai/api/v1"),
            requires_model: true,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "ollama",
            display_name: "Ollama",
            auth_type: AuthType::ApiKey,
            env_key: Some("OLLAMA_API_KEY"),
            default_base_url: Some("http://localhost:11434"),
            requires_model: false,
            key_optional: true,
            local_only: true,
        },
        KnownProvider {
            name: "lmstudio",
            display_name: "LM Studio",
            auth_type: AuthType::ApiKey,
            env_key: Some("LMSTUDIO_API_KEY"),
            default_base_url: Some("http://127.0.0.1:1234/v1"),
            requires_model: false,
            key_optional: true,
            local_only: true,
        },
        KnownProvider {
            name: "openai-codex",
            display_name: "OpenAI Codex",
            auth_type: AuthType::Oauth,
            env_key: None,
            default_base_url: None,
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "github-copilot",
            display_name: "GitHub Copilot",
            auth_type: AuthType::Oauth,
            env_key: None,
            default_base_url: None,
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
        KnownProvider {
            name: "kimi-code",
            display_name: "Kimi Code",
            auth_type: AuthType::ApiKey,
            env_key: Some("KIMI_API_KEY"),
            default_base_url: Some("https://api.kimi.com/coding/v1"),
            requires_model: false,
            key_optional: false,
            local_only: false,
        },
    ];

    // Add local-llm provider when the local-llm feature is enabled
    #[cfg(feature = "local-llm")]
    let providers = {
        let mut p = providers;
        p.push(KnownProvider {
            name: "local-llm",
            display_name: "Local LLM (Offline)",
            auth_type: AuthType::Local,
            env_key: None,
            default_base_url: None,
            requires_model: true,
            key_optional: false,
            local_only: true,
        });
        p
    };

    providers
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_have_valid_auth_types() {
        for p in known_providers() {
            assert!(
                p.auth_type == AuthType::ApiKey
                    || p.auth_type == AuthType::Oauth
                    || p.auth_type == AuthType::Local,
                "invalid auth type for {}: {}",
                p.name,
                p.auth_type
            );
        }
    }

    #[test]
    fn api_key_providers_have_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::ApiKey {
                assert!(
                    p.env_key.is_some(),
                    "api-key provider {} missing env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn oauth_providers_have_no_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::Oauth {
                assert!(
                    p.env_key.is_none(),
                    "oauth provider {} should not have env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn local_providers_have_no_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::Local {
                assert!(
                    p.env_key.is_none(),
                    "local provider {} should not have env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn known_provider_names_unique() {
        let providers = known_providers();
        let mut names: Vec<&str> = providers.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), providers.len());
    }

    #[test]
    fn known_providers_include_new_providers() {
        let providers = known_providers();
        let names: Vec<&str> = providers.iter().map(|p| p.name).collect();
        // All new OpenAI-compatible providers
        assert!(names.contains(&"mistral"), "missing mistral");
        assert!(names.contains(&"openrouter"), "missing openrouter");
        assert!(names.contains(&"cerebras"), "missing cerebras");
        assert!(names.contains(&"minimax"), "missing minimax");
        assert!(names.contains(&"moonshot"), "missing moonshot");
        assert!(names.contains(&"zai"), "missing zai");
        assert!(names.contains(&"zai-code"), "missing zai-code");
        assert!(names.contains(&"kimi-code"), "missing kimi-code");
        assert!(names.contains(&"venice"), "missing venice");
        assert!(names.contains(&"ollama"), "missing ollama");
        assert!(names.contains(&"lmstudio"), "missing lmstudio");
        // OAuth providers
        assert!(names.contains(&"github-copilot"), "missing github-copilot");
    }

    #[test]
    fn lmstudio_is_local_only_with_optional_key() {
        let providers = known_providers();
        let lmstudio = providers
            .iter()
            .find(|p| p.name == "lmstudio")
            .expect("lmstudio not in known_providers");
        assert_eq!(lmstudio.auth_type, AuthType::ApiKey);
        assert!(
            lmstudio.key_optional,
            "lmstudio runs locally and must not require an API key"
        );
        assert!(
            lmstudio.is_local_only(),
            "lmstudio must be filtered out of cloud deployments"
        );
        assert_eq!(lmstudio.env_key, Some("LMSTUDIO_API_KEY"));
        assert_eq!(
            lmstudio.default_base_url,
            Some("http://127.0.0.1:1234/v1"),
            "lmstudio default base URL must match LM Studio's default server port"
        );
    }

    #[test]
    fn is_local_only_is_superset_of_legacy_check() {
        for p in known_providers() {
            let legacy = p.auth_type == AuthType::Local || p.name == "ollama";
            let typed = p.is_local_only();
            assert!(
                typed || !legacy,
                "{}: legacy says local but typed disagrees",
                p.name
            );
        }
    }

    #[test]
    fn github_copilot_is_oauth_provider() {
        let providers = known_providers();
        let copilot = providers
            .iter()
            .find(|p| p.name == "github-copilot")
            .expect("github-copilot not in known_providers");
        assert_eq!(copilot.auth_type, AuthType::Oauth);
        assert!(copilot.env_key.is_none());
    }

    #[test]
    fn new_api_key_providers_have_correct_env_keys() {
        let expected = [
            ("mistral", "MISTRAL_API_KEY"),
            ("openrouter", "OPENROUTER_API_KEY"),
            ("cerebras", "CEREBRAS_API_KEY"),
            ("minimax", "MINIMAX_API_KEY"),
            ("moonshot", "MOONSHOT_API_KEY"),
            ("zai", "Z_API_KEY"),
            ("zai-code", "Z_CODE_API_KEY"),
            ("kimi-code", "KIMI_API_KEY"),
            ("venice", "VENICE_API_KEY"),
            ("ollama", "OLLAMA_API_KEY"),
            ("lmstudio", "LMSTUDIO_API_KEY"),
        ];
        let providers = known_providers();
        for (name, env_key) in expected {
            let provider = providers
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("missing provider: {name}"));
            assert_eq!(provider.env_key, Some(env_key), "wrong env_key for {name}");
            assert_eq!(provider.auth_type, AuthType::ApiKey);
        }
    }
}
