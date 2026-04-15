use secrecy::ExposeSecret;

use super::*;

#[test]
fn geolocation_display_with_place() {
    let loc = GeoLocation {
        latitude: 37.759,
        longitude: -122.433,
        place: Some("Noe Valley, San Francisco, CA".to_string()),
        updated_at: None,
    };
    assert_eq!(loc.to_string(), "Noe Valley, San Francisco, CA");
}

#[test]
fn geolocation_display_without_place() {
    let loc = GeoLocation {
        latitude: 37.759,
        longitude: -122.433,
        place: None,
        updated_at: None,
    };
    assert_eq!(loc.to_string(), "37.759,-122.433");
}

#[test]
fn geolocation_serde_backward_compat() {
    // Old JSON without `place` field should deserialize fine.
    let json = r#"{"latitude":48.8566,"longitude":2.3522,"updated_at":1700000000}"#;
    let loc: GeoLocation = serde_json::from_str(json).unwrap();
    assert!((loc.latitude - 48.8566).abs() < 1e-6);
    assert!(loc.place.is_none());
}

#[test]
fn geolocation_serde_with_place() {
    let loc = GeoLocation {
        latitude: 48.8566,
        longitude: 2.3522,
        place: Some("Paris, France".to_string()),
        updated_at: Some(1_700_000_000),
    };
    let json = serde_json::to_string(&loc).unwrap();
    let parsed: GeoLocation = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.place.as_deref(), Some("Paris, France"));
}

#[test]
fn geolocation_now_stores_place() {
    let loc = GeoLocation::now(37.0, -122.0, Some("San Francisco".to_string()));
    assert_eq!(loc.place.as_deref(), Some("San Francisco"));
    assert!(loc.updated_at.is_some());
}

#[test]
fn skills_config_sidecar_files_default_disabled() {
    let toml = r#"
[skills]
enabled = true
"#;
    let parsed: MoltisConfig = toml::from_str(toml).unwrap();
    assert!(!parsed.skills.enable_agent_sidecar_files);
}

#[test]
fn env_section_parses() {
    let toml = r#"
[env]
BRAVE_API_KEY = "test-key"
OPENROUTER_API_KEY = "sk-or-test"
"#;
    let config: MoltisConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.env.len(), 2);
    assert_eq!(config.env.get("BRAVE_API_KEY").unwrap(), "test-key");
    assert_eq!(config.env.get("OPENROUTER_API_KEY").unwrap(), "sk-or-test");
}

#[test]
fn env_section_defaults_to_empty() {
    let config: MoltisConfig = toml::from_str("").unwrap();
    assert!(config.env.is_empty());
}

#[test]
fn agents_config_defaults_empty() {
    let config: MoltisConfig = toml::from_str("").unwrap();
    assert!(config.agents.default_preset.is_none());
    assert!(config.agents.presets.is_empty());
}

#[test]
fn mcp_config_defaults_request_timeout() {
    let config: MoltisConfig = toml::from_str("").unwrap();
    assert_eq!(config.mcp.request_timeout_secs, 30);
}

#[test]
fn mcp_server_entry_parses_request_timeout_override() {
    let config: MoltisConfig = toml::from_str(
        r#"
[mcp.servers.memory]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-memory"]
request_timeout_secs = 75
"#,
    )
    .unwrap();
    assert_eq!(
        config
            .mcp
            .servers
            .get("memory")
            .and_then(|entry| entry.request_timeout_secs),
        Some(75)
    );
}

#[test]
fn agents_config_parses_presets() {
    let toml = r#"
[agents]
default_preset = "research"

[agents.presets.research]
model = "openai/gpt-5.2"
delegate_only = false
system_prompt_suffix = "Focus on evidence."
max_iterations = 10
timeout_secs = 120

[agents.presets.research.identity]
name = "scout"
emoji = "🔍"
theme = "thorough"

[agents.presets.research.tools]
allow = ["web_search", "web_fetch"]
deny = ["exec"]
"#;
    let config: MoltisConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.agents.default_preset.as_deref(), Some("research"));
    let preset = config.agents.get_preset("research").unwrap();
    assert_eq!(preset.model.as_deref(), Some("openai/gpt-5.2"));
    assert_eq!(preset.tools.allow.len(), 2);
    assert_eq!(preset.tools.deny, vec!["exec".to_string()]);
    assert!(!preset.delegate_only);
    assert_eq!(
        preset.system_prompt_suffix.as_deref(),
        Some("Focus on evidence.")
    );
    assert_eq!(preset.identity.name.as_deref(), Some("scout"));
    assert_eq!(preset.identity.emoji.as_deref(), Some("🔍"));
    assert_eq!(preset.identity.theme.as_deref(), Some("thorough"));
    assert_eq!(preset.max_iterations, Some(10));
    assert_eq!(preset.timeout_secs, Some(120));
}

#[test]
fn chat_config_default_queue_mode_is_followup() {
    let cfg = ChatConfig::default();
    assert_eq!(cfg.message_queue_mode, MessageQueueMode::Followup);
}

#[test]
fn chat_config_toml_missing_queue_mode_defaults_to_followup() {
    let cfg: ChatConfig = toml::from_str("").unwrap();
    assert_eq!(cfg.message_queue_mode, MessageQueueMode::Followup);
}

#[test]
fn chat_config_default_prompt_memory_mode_is_live_reload() {
    let cfg = ChatConfig::default();
    assert_eq!(cfg.prompt_memory_mode, PromptMemoryMode::LiveReload);
}

#[test]
fn memory_config_default_style_is_hybrid() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.style, MemoryStyle::Hybrid);
}

#[test]
fn memory_config_default_agent_write_mode_is_hybrid() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.agent_write_mode, AgentMemoryWriteMode::Hybrid);
}

#[test]
fn memory_config_default_user_profile_write_mode_is_explicit_and_auto() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(
        cfg.user_profile_write_mode,
        UserProfileWriteMode::ExplicitAndAuto
    );
}

#[test]
fn memory_config_default_backend_is_builtin() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.backend, MemoryBackend::Builtin);
}

#[test]
fn memory_config_default_citations_is_auto() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.citations, MemoryCitationsMode::Auto);
}

#[test]
fn memory_config_default_search_merge_strategy_is_rrf() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.search_merge_strategy, MemorySearchMergeStrategy::Rrf);
}

#[test]
fn memory_config_default_session_export_mode_is_on_new_or_reset() {
    let cfg = MemoryEmbeddingConfig::default();
    assert_eq!(cfg.session_export, SessionExportMode::OnNewOrReset);
}

#[test]
fn memory_config_toml_parses_style() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("style = \"search-only\"").unwrap();
    assert_eq!(cfg.style, MemoryStyle::SearchOnly);
}

#[test]
fn memory_config_toml_parses_agent_write_mode() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("agent_write_mode = \"prompt-only\"").unwrap();
    assert_eq!(cfg.agent_write_mode, AgentMemoryWriteMode::PromptOnly);
}

#[test]
fn memory_config_toml_parses_user_profile_write_mode() {
    let cfg: MemoryEmbeddingConfig =
        toml::from_str("user_profile_write_mode = \"explicit-only\"").unwrap();
    assert_eq!(
        cfg.user_profile_write_mode,
        UserProfileWriteMode::ExplicitOnly
    );
}

#[test]
fn memory_config_toml_parses_backend() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("backend = \"qmd\"").unwrap();
    assert_eq!(cfg.backend, MemoryBackend::Qmd);
}

#[test]
fn memory_config_toml_parses_provider() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("provider = \"openai\"").unwrap();
    assert_eq!(cfg.provider, Some(MemoryProvider::OpenAi));
}

#[test]
fn memory_config_toml_parses_citations() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("citations = \"on\"").unwrap();
    assert_eq!(cfg.citations, MemoryCitationsMode::On);
}

#[test]
fn memory_config_toml_parses_search_merge_strategy() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("search_merge_strategy = \"linear\"").unwrap();
    assert_eq!(cfg.search_merge_strategy, MemorySearchMergeStrategy::Linear);
}

#[test]
fn memory_config_toml_parses_session_export_mode() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("session_export = \"off\"").unwrap();
    assert_eq!(cfg.session_export, SessionExportMode::Off);
}

#[test]
fn memory_config_toml_accepts_legacy_bool_session_export() {
    let cfg: MemoryEmbeddingConfig = toml::from_str("session_export = false").unwrap();
    assert_eq!(cfg.session_export, SessionExportMode::Off);

    let cfg: MemoryEmbeddingConfig = toml::from_str("session_export = true").unwrap();
    assert_eq!(cfg.session_export, SessionExportMode::OnNewOrReset);
}

#[test]
fn chat_config_toml_parses_prompt_memory_mode() {
    let cfg: ChatConfig =
        toml::from_str("prompt_memory_mode = \"frozen-at-session-start\"").unwrap();
    assert_eq!(
        cfg.prompt_memory_mode,
        PromptMemoryMode::FrozenAtSessionStart
    );
}

#[test]
fn chat_config_workspace_file_limit_defaults_to_32000() {
    let cfg = ChatConfig::default();
    assert_eq!(cfg.workspace_file_max_chars, 32_000);
}

#[test]
fn chat_config_toml_parses_workspace_file_limit() {
    let cfg: ChatConfig = toml::from_str("workspace_file_max_chars = 12345").unwrap();
    assert_eq!(cfg.workspace_file_max_chars, 12_345);
}

#[test]
fn providers_config_local_alias_maps_local_llm_to_local() {
    let mut config = ProvidersConfig::default();
    config.providers.insert("local-llm".into(), ProviderEntry {
        enabled: false,
        ..ProviderEntry::default()
    });

    assert!(!config.is_enabled("local"));
    assert!(!config.is_enabled("local-llm"));
    assert!(config.get("local").is_some());
}

#[test]
fn providers_config_local_alias_prefers_exact_key() {
    let mut config = ProvidersConfig::default();
    config.providers.insert("local".into(), ProviderEntry {
        enabled: false,
        ..ProviderEntry::default()
    });
    config.providers.insert("local-llm".into(), ProviderEntry {
        enabled: true,
        ..ProviderEntry::default()
    });

    assert!(!config.is_enabled("local"));
    assert!(config.is_enabled("local-llm"));
}

#[test]
fn providers_config_offered_controls_enablement() {
    let config = ProvidersConfig {
        offered: vec!["openai".into()],
        ..ProvidersConfig::default()
    };
    assert!(config.is_enabled("openai"));
    assert!(!config.is_enabled("anthropic"));
}

#[test]
fn providers_config_offered_handles_local_alias() {
    let config = ProvidersConfig {
        offered: vec!["local-llm".into()],
        ..ProvidersConfig::default()
    };
    assert!(config.is_enabled("local"));
    assert!(config.is_enabled("local-llm"));
}

#[test]
fn providers_config_enabled_flag_still_applies_with_offered_allowlist() {
    let mut config = ProvidersConfig {
        offered: vec!["openai".into()],
        ..ProvidersConfig::default()
    };
    config.providers.insert("openai".into(), ProviderEntry {
        enabled: false,
        ..ProviderEntry::default()
    });
    assert!(!config.is_enabled("openai"));
}

#[test]
fn provider_entry_defaults_fetch_models_enabled() {
    let entry = ProviderEntry::default();
    assert!(entry.fetch_models);
    assert!(entry.models.is_empty());
}

#[test]
fn channels_config_defaults_offered() {
    let config = ChannelsConfig::default();
    assert_eq!(config.offered, vec![
        "telegram".to_string(),
        "whatsapp".to_string(),
        "msteams".to_string(),
        "discord".to_string(),
        "slack".to_string(),
        "matrix".to_string(),
        "nostr".to_string(),
    ]);
}

#[test]
fn channels_config_empty_toml_defaults_offered() {
    let config: ChannelsConfig = toml::from_str("").unwrap();
    assert_eq!(config.offered, vec![
        "telegram".to_string(),
        "whatsapp".to_string(),
        "msteams".to_string(),
        "discord".to_string(),
        "slack".to_string(),
        "matrix".to_string(),
        "nostr".to_string(),
    ]);
}

#[test]
fn channels_config_explicit_offered() {
    let config: ChannelsConfig = toml::from_str(r#"offered = ["telegram", "msteams"]"#).unwrap();
    assert_eq!(config.offered, vec![
        "telegram".to_string(),
        "msteams".to_string()
    ]);
}

#[test]
fn channels_slack_is_named_field_not_extra() {
    let toml_str = r#"
[slack.my-bot]
token = "xoxb-test"
"#;
    let config: ChannelsConfig = toml::from_str(toml_str).unwrap();
    assert!(
        config.slack.contains_key("my-bot"),
        "slack should be in named field"
    );
    assert!(
        !config.extra.contains_key("slack"),
        "slack should not appear in extra"
    );
}

#[test]
fn channels_all_channel_configs_includes_slack() {
    let mut config = ChannelsConfig::default();
    config
        .slack
        .insert("bot1".into(), serde_json::json!({"token": "xoxb-test"}));
    let all = config.all_channel_configs();
    let slack_entry = all.iter().find(|(ct, _)| *ct == "slack");
    assert!(
        slack_entry.is_some(),
        "all_channel_configs should include slack"
    );
    assert!(slack_entry.unwrap().1.contains_key("bot1"));
}

#[test]
fn sandbox_defaults_include_go_runtime() {
    let sandbox = SandboxConfig::default();
    assert!(sandbox.packages.iter().any(|pkg| pkg == "golang-go"));
    assert_eq!(sandbox.home_persistence, HomePersistenceConfig::Shared);
    assert!(sandbox.host_data_dir.is_none());
    assert!(sandbox.wasm_tool_limits.is_none());
}

#[test]
fn wasm_tool_limits_config_defaults() {
    let limits = WasmToolLimitsConfig::default();
    assert_eq!(limits.default_memory, 16 * 1024 * 1024);
    assert_eq!(limits.default_fuel, 1_000_000);
    assert!(limits.tool_overrides.contains_key("calc"));
}

#[test]
fn sandbox_wasm_tool_limits_deserialize() {
    let config: SandboxConfig = toml::from_str(
        r#"
mode = "all"
scope = "session"
workspace_mount = "ro"
host_data_dir = "/host/moltis-data"

[wasm_tool_limits]
default_memory = 2048
default_fuel = 5000

[wasm_tool_limits.tool_overrides.calc]
fuel = 100
memory = 300
"#,
    )
    .unwrap();

    assert_eq!(config.host_data_dir.as_deref(), Some("/host/moltis-data"));
    let limits = config.wasm_tool_limits.unwrap();
    assert_eq!(limits.default_memory, 2048);
    assert_eq!(limits.default_fuel, 5000);
    assert_eq!(
        limits
            .tool_overrides
            .get("calc")
            .and_then(|override_cfg| override_cfg.fuel),
        Some(100)
    );
}

#[test]
fn browserless_api_version_deserialize_v2() {
    let config: BrowserConfig = toml::from_str(
        r#"
browserless_api_version = "v2"
"#,
    )
    .unwrap();
    assert_eq!(config.browserless_api_version, BrowserlessApiVersion::V2);
}

#[test]
fn browserless_api_version_rejects_non_lowercase_variants() {
    let parsed: Result<BrowserConfig, _> = toml::from_str(
        r#"
browserless_api_version = "V2"
"#,
    );
    assert!(
        parsed.is_err(),
        "uppercase value should fail serde enum deserialization"
    );
}

#[test]
fn tool_mode_serde_round_trip() {
    for (variant, expected_str) in [
        (ToolMode::Auto, r#""auto""#),
        (ToolMode::Native, r#""native""#),
        (ToolMode::Text, r#""text""#),
        (ToolMode::Off, r#""off""#),
    ] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected_str, "serialize {variant:?}");
        let parsed: ToolMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, variant, "deserialize {expected_str}");
    }
}

#[test]
fn tool_mode_toml_round_trip() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Wrapper {
        mode: ToolMode,
    }

    for variant in [
        ToolMode::Auto,
        ToolMode::Native,
        ToolMode::Text,
        ToolMode::Off,
    ] {
        let w = Wrapper { mode: variant };
        let toml_str = toml::to_string(&w).unwrap();
        let parsed: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.mode, variant, "toml round-trip {variant:?}");
    }
}

#[test]
fn tool_mode_default_is_auto() {
    assert_eq!(ToolMode::default(), ToolMode::Auto);
}

#[test]
fn provider_entry_tool_mode_defaults_to_auto() {
    let entry = ProviderEntry::default();
    assert_eq!(entry.tool_mode, ToolMode::Auto);
}

#[test]
fn provider_entry_tool_mode_skipped_when_default() {
    let entry = ProviderEntry::default();
    let toml_str = toml::to_string(&entry).unwrap();
    assert!(
        !toml_str.contains("tool_mode"),
        "tool_mode should be skipped when default: {toml_str}"
    );
}

#[test]
fn provider_entry_tool_mode_persisted_when_non_default() {
    let entry = ProviderEntry {
        tool_mode: ToolMode::Text,
        ..ProviderEntry::default()
    };
    let toml_str = toml::to_string(&entry).unwrap();
    assert!(
        toml_str.contains("tool_mode"),
        "tool_mode should be present when non-default: {toml_str}"
    );
    let parsed: ProviderEntry = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.tool_mode, ToolMode::Text);
}

#[test]
fn provider_entry_url_alias_maps_to_base_url() {
    let entry: ProviderEntry = toml::from_str(
        r#"
enabled = true
url = "http://192.168.0.9:11434"
"#,
    )
    .unwrap();

    assert_eq!(entry.base_url.as_deref(), Some("http://192.168.0.9:11434"));
}

#[test]
fn memory_embedding_legacy_aliases_map_to_current_fields() {
    let config: MoltisConfig = toml::from_str(
        r#"
[memory]
embedding_provider = "custom"
embedding_base_url = "http://moltis-embeddings:7997/v1"
embedding_model = "intfloat/multilingual-e5-small"
embedding_api_key = "secret-key"
"#,
    )
    .unwrap();

    assert_eq!(config.memory.provider, Some(MemoryProvider::Custom));
    assert_eq!(
        config.memory.base_url.as_deref(),
        Some("http://moltis-embeddings:7997/v1")
    );
    assert_eq!(
        config.memory.model.as_deref(),
        Some("intfloat/multilingual-e5-small")
    );
    assert_eq!(
        config
            .memory
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .map(String::as_str),
        Some("secret-key")
    );
}

#[test]
fn full_config_with_tool_mode() {
    let toml_str = r#"
[providers.ollama]
enabled = true
tool_mode = "text"

[providers.anthropic]
enabled = true
tool_mode = "native"
"#;
    let config: MoltisConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(
        config.providers.get("ollama").unwrap().tool_mode,
        ToolMode::Text
    );
    assert_eq!(
        config.providers.get("anthropic").unwrap().tool_mode,
        ToolMode::Native
    );
}

#[test]
fn wire_api_serde_roundtrip() {
    assert_eq!(
        serde_json::to_string(&WireApi::ChatCompletions).unwrap(),
        "\"chat-completions\""
    );
    assert_eq!(
        serde_json::to_string(&WireApi::Responses).unwrap(),
        "\"responses\""
    );
    assert_eq!(
        serde_json::from_str::<WireApi>("\"chat-completions\"").unwrap(),
        WireApi::ChatCompletions
    );
    assert_eq!(
        serde_json::from_str::<WireApi>("\"responses\"").unwrap(),
        WireApi::Responses
    );
}

#[test]
fn wire_api_default_is_chat_completions() {
    assert_eq!(WireApi::default(), WireApi::ChatCompletions);
}

#[test]
fn provider_entry_wire_api_from_toml() {
    let toml_str = r#"
[providers.custom-mn]
enabled = true
base_url = "https://gmn.example.com/v1"
wire_api = "responses"
models = ["gpt-5.3-codex"]
"#;
    let config: MoltisConfig = toml::from_str(toml_str).unwrap();
    let entry = config.providers.get("custom-mn").unwrap();
    assert_eq!(entry.wire_api, WireApi::Responses);
}

#[test]
fn provider_entry_wire_api_defaults_to_chat_completions() {
    let toml_str = r#"
[providers.openai]
enabled = true
"#;
    let config: MoltisConfig = toml::from_str(toml_str).unwrap();
    let entry = config.providers.get("openai").unwrap();
    assert_eq!(entry.wire_api, WireApi::ChatCompletions);
}

#[test]
fn provider_entry_wire_api_skip_serializing_default() {
    let entry = ProviderEntry::default();
    let serialized = toml::to_string(&entry).unwrap();
    assert!(
        !serialized.contains("wire_api"),
        "default wire_api should be skipped in serialization"
    );
}

#[test]
fn provider_entry_wire_api_serializes_responses() {
    let entry = ProviderEntry {
        wire_api: WireApi::Responses,
        ..Default::default()
    };
    let serialized = toml::to_string(&entry).unwrap();
    assert!(
        serialized.contains("wire_api = \"responses\""),
        "non-default wire_api should be serialized"
    );
}

#[test]
fn terminal_enabled_defaults_to_true() {
    let cfg: MoltisConfig = toml::from_str("").unwrap();
    assert!(cfg.server.terminal_enabled);
    // Note: is_terminal_enabled() is NOT tested here because it reads
    // the MOLTIS_TERMINAL_DISABLED env var, which may be set in CI.
}

#[test]
fn terminal_enabled_parsed_from_config() {
    let cfg: MoltisConfig = toml::from_str("[server]\nterminal_enabled = false\n").unwrap();
    assert!(!cfg.server.terminal_enabled);
}

#[test]
fn terminal_disabled_via_config_reflects_in_helper() {
    let cfg: MoltisConfig = toml::from_str("[server]\nterminal_enabled = false\n").unwrap();
    // When the env var is not set, the helper returns the config value.
    // (We cannot test the env-var override here because workspace lints
    // deny unsafe code, and `std::env::set_var` is unsafe.)
    assert!(!cfg.server.is_terminal_enabled());
}

#[test]
fn voice_openai_and_whisper_base_url_parse_from_toml() {
    let toml_str = r#"
[voice.tts.openai]
base_url = "http://127.0.0.1:8003/v1"

[voice.stt.whisper]
base_url = "http://127.0.0.1:8001/v1"
"#;
    let config: MoltisConfig = toml::from_str(toml_str).unwrap();

    assert_eq!(
        config.voice.tts.openai.base_url.as_deref(),
        Some("http://127.0.0.1:8003/v1")
    );
    assert_eq!(
        config.voice.stt.whisper.base_url.as_deref(),
        Some("http://127.0.0.1:8001/v1")
    );
}
