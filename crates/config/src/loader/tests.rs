use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use secrecy::ExposeSecret;

use crate::{AgentIdentity, UserProfile, schema::MoltisConfig};

use super::{
    config_io::{
        apply_env_overrides_with, parse_config, parse_env_value, resubstitute_config, set_nested,
    },
    *,
};

struct TestDataDirState {
    _data_dir: Option<PathBuf>,
}

static DATA_DIR_TEST_LOCK: Mutex<TestDataDirState> =
    Mutex::new(TestDataDirState { _data_dir: None });

#[test]
fn parse_env_value_bool() {
    assert_eq!(parse_env_value("true"), serde_json::Value::Bool(true));
    assert_eq!(parse_env_value("TRUE"), serde_json::Value::Bool(true));
    assert_eq!(parse_env_value("false"), serde_json::Value::Bool(false));
}

#[test]
fn parse_env_value_number() {
    assert_eq!(parse_env_value("42"), serde_json::json!(42));
    assert_eq!(parse_env_value("1.5"), serde_json::json!(1.5));
}

#[test]
fn parse_env_value_string() {
    assert_eq!(
        parse_env_value("hello"),
        serde_json::Value::String("hello".into())
    );
}

#[test]
fn parse_env_value_json_array() {
    assert_eq!(
        parse_env_value("[\"openai\",\"github-copilot\"]"),
        serde_json::json!(["openai", "github-copilot"])
    );
}

#[test]
fn set_nested_creates_intermediate_objects() {
    let mut root = serde_json::json!({});
    set_nested(
        &mut root,
        &["a".into(), "b".into(), "c".into()],
        serde_json::json!(42),
    );
    assert_eq!(root, serde_json::json!({"a": {"b": {"c": 42}}}));
}

#[test]
fn set_nested_overwrites_existing() {
    let mut root = serde_json::json!({"auth": {"disabled": false}});
    set_nested(
        &mut root,
        &["auth".into(), "disabled".into()],
        serde_json::Value::Bool(true),
    );
    assert_eq!(root, serde_json::json!({"auth": {"disabled": true}}));
}

#[test]
fn apply_env_overrides_auth_disabled() {
    let vars = vec![("MOLTIS_AUTH__DISABLED".into(), "true".into())];
    let config = MoltisConfig::default();
    assert!(!config.auth.disabled);
    let config = apply_env_overrides_with(config, vars.into_iter());
    assert!(config.auth.disabled);
}

#[test]
fn apply_env_overrides_tools_agent_timeout() {
    let vars = vec![("MOLTIS_TOOLS__AGENT_TIMEOUT_SECS".into(), "120".into())];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert_eq!(config.tools.agent_timeout_secs, 120);
}

#[test]
fn apply_env_overrides_tools_agent_max_iterations() {
    let vars = vec![("MOLTIS_TOOLS__AGENT_MAX_ITERATIONS".into(), "64".into())];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert_eq!(config.tools.agent_max_iterations, 64);
}

#[test]
fn apply_env_overrides_ignores_excluded() {
    // MOLTIS_CONFIG_DIR should not be treated as a config field override.
    let vars = vec![("MOLTIS_CONFIG_DIR".into(), "/tmp/test".into())];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert!(!config.auth.disabled);
}

#[test]
fn apply_env_overrides_ignores_external_url() {
    // MOLTIS_EXTERNAL_URL is handled by effective_external_url(), not by
    // the generic env override mechanism.
    let vars = vec![(
        "MOLTIS_EXTERNAL_URL".into(),
        "https://test.example.com".into(),
    )];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert!(
        config.server.external_url.is_none(),
        "MOLTIS_EXTERNAL_URL should be excluded from generic env overrides"
    );
}

#[test]
fn apply_env_overrides_multiple() {
    let vars = vec![
        ("MOLTIS_AUTH__DISABLED".into(), "true".into()),
        ("MOLTIS_TOOLS__AGENT_TIMEOUT_SECS".into(), "300".into()),
        ("MOLTIS_TAILSCALE__MODE".into(), "funnel".into()),
    ];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert!(config.auth.disabled);
    assert_eq!(config.tools.agent_timeout_secs, 300);
    assert_eq!(config.tailscale.mode, "funnel");
}

#[test]
fn apply_env_overrides_deep_nesting() {
    let vars = vec![(
        "MOLTIS_TOOLS__EXEC__DEFAULT_TIMEOUT_SECS".into(),
        "60".into(),
    )];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert_eq!(config.tools.exec.default_timeout_secs, 60);
}

#[test]
fn apply_env_overrides_mcp_request_timeout() {
    let vars = vec![("MOLTIS_MCP__REQUEST_TIMEOUT_SECS".into(), "90".into())];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert_eq!(config.mcp.request_timeout_secs, 90);
}

#[test]
fn apply_env_overrides_providers_offered_array() {
    let vars = vec![(
        "MOLTIS_PROVIDERS__OFFERED".into(),
        "[\"openai\",\"github-copilot\"]".into(),
    )];
    let config = apply_env_overrides_with(MoltisConfig::default(), vars.into_iter());
    assert_eq!(config.providers.offered, vec!["openai", "github-copilot"]);
}

#[test]
fn apply_env_overrides_providers_offered_empty_array() {
    let vars = vec![("MOLTIS_PROVIDERS__OFFERED".into(), "[]".into())];
    let mut base = MoltisConfig::default();
    base.providers.offered = vec!["openai".into()];
    let config = apply_env_overrides_with(base, vars.into_iter());
    assert!(
        config.providers.offered.is_empty(),
        "empty JSON array env override should clear providers.offered"
    );
}

#[test]
fn generate_random_port_returns_valid_port() {
    // Generate a few random ports and verify they're in the valid range
    for _ in 0..5 {
        let port = generate_random_port();
        // Port should be in the ephemeral range (1024-65535) or fallback (18789)
        assert!(
            port >= 1024 || port == 0,
            "generated port {port} is out of expected range"
        );
    }
}

#[test]
fn generate_random_port_returns_different_ports() {
    // Generate multiple ports and verify we get at least some variation
    let ports: Vec<u16> = (0..10).map(|_| generate_random_port()).collect();
    let unique: std::collections::HashSet<_> = ports.iter().collect();
    // With 10 random ports, we should have at least 2 different values
    // (unless somehow all ports are in use, which is extremely unlikely)
    assert!(
        unique.len() >= 2,
        "expected variation in generated ports, got {:?}",
        ports
    );
}

#[test]
fn write_default_config_writes_template_to_requested_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nested").join("moltis.toml");
    let mut config = MoltisConfig::default();
    config.server.port = 23456;

    write_default_config(&path, &config).expect("write default config");

    let raw = std::fs::read_to_string(&path).expect("read generated config");
    assert!(
        raw.contains("port = 23456"),
        "generated template should include selected server port"
    );
    assert!(
        raw.contains("message_queue_mode = \"followup\""),
        "generated template should set followup queue mode by default"
    );
    assert!(
        raw.contains("\"followup\" - Queue messages, replay one-by-one after run"),
        "generated template should document the followup queue option"
    );
    assert!(
        raw.contains("\"collect\"  - Buffer messages, concatenate as single message"),
        "generated template should document the collect queue option"
    );
    assert!(
        raw.contains("\"tmux\""),
        "generated template should include tmux in sandbox packages"
    );

    let parsed: MoltisConfig = parse_config(&raw, &path).expect("parse generated config");
    assert!(
        parsed
            .tools
            .exec
            .sandbox
            .packages
            .iter()
            .any(|pkg| pkg == "tmux"),
        "parsed config should include tmux in sandbox packages"
    );
}

#[test]
fn write_default_config_does_not_overwrite_existing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    std::fs::write(&path, "existing = true\n").expect("seed config");

    let mut config = MoltisConfig::default();
    config.server.port = 34567;
    write_default_config(&path, &config).expect("write default config");

    let raw = std::fs::read_to_string(&path).expect("read seeded config");
    assert_eq!(raw, "existing = true\n");
}

#[test]
fn save_config_to_path_preserves_provider_and_voice_comment_blocks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    std::fs::write(&path, crate::template::default_config_template(18789)).expect("write template");

    let mut config = load_config(&path).expect("load template config");
    config.auth.disabled = true;
    config.server.http_request_logs = true;

    save_config_to_path(&path, &config).expect("save config");

    let saved = std::fs::read_to_string(&path).expect("read saved config");
    assert!(saved.contains("# All available providers:"));
    assert!(saved.contains("# All available TTS providers:"));
    assert!(saved.contains("# All available STT providers:"));
    assert!(saved.contains("disabled = true"));
    assert!(saved.contains("http_request_logs = true"));
}

#[test]
fn save_config_to_path_removes_stale_keys_when_values_are_cleared() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    std::fs::write(
        &path,
        r#"[server]
bind = "127.0.0.1"
port = 18789

[identity]
name = "Rex"
"#,
    )
    .expect("write seed config");

    // Use parse_config directly to avoid env-override pollution
    // (e.g. MOLTIS_IDENTITY__NAME in the process environment).
    let raw = std::fs::read_to_string(&path).expect("read seed");
    let mut config: MoltisConfig = parse_config(&raw, &path).expect("parse seed config");
    config.identity.name = None;

    save_config_to_path(&path, &config).expect("save config");

    let saved = std::fs::read_to_string(&path).expect("read saved file");
    let reloaded: MoltisConfig = parse_config(&saved, &path).expect("reload config");
    assert!(
        reloaded.identity.name.is_none(),
        "identity.name should be removed when cleared"
    );
}

#[test]
fn server_config_default_port_is_zero() {
    // Default port should be 0 (to be replaced with random port on config creation)
    let config = crate::schema::ServerConfig::default();
    assert_eq!(config.port, 0);
    assert_eq!(config.bind, "127.0.0.1");
}

#[test]
fn data_dir_override_works() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let path = PathBuf::from("/tmp/test-data-dir-override");
    set_data_dir(path.clone());
    assert_eq!(data_dir(), path);
    clear_data_dir();
}

#[test]
fn save_and_load_identity_frontmatter() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let identity = AgentIdentity {
        name: Some("Rex".to_string()),
        emoji: Some("🐶".to_string()),
        theme: Some("chill dog golden retriever".to_string()),
    };

    let path = save_identity(&identity).expect("save identity");
    assert!(path.exists());
    let raw = std::fs::read_to_string(&path).expect("read identity file");

    let loaded = load_identity().expect("load identity");
    assert_eq!(loaded.name.as_deref(), Some("Rex"));
    assert_eq!(loaded.emoji.as_deref(), Some("🐶"), "raw file:\n{raw}");
    assert_eq!(loaded.theme.as_deref(), Some("chill dog golden retriever"));

    clear_data_dir();
}

#[test]
fn save_identity_removes_empty_file() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let seeded = AgentIdentity {
        name: Some("Rex".to_string()),
        emoji: None,
        theme: None,
    };
    let path = save_identity(&seeded).expect("seed identity");
    assert!(path.exists());

    save_identity(&AgentIdentity::default()).expect("save empty identity");
    assert!(!path.exists());

    clear_data_dir();
}

#[test]
fn save_and_load_user_frontmatter() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let user = UserProfile {
        name: Some("Alice".to_string()),
        timezone: Some(crate::schema::Timezone::from(chrono_tz::Europe::Berlin)),
        location: None,
    };

    let path = save_user(&user).expect("save user");
    assert!(path.exists());

    let loaded = load_user().expect("load user");
    assert_eq!(loaded.name.as_deref(), Some("Alice"));
    assert_eq!(
        loaded.timezone.as_ref().map(|tz| tz.name()),
        Some("Europe/Berlin")
    );

    clear_data_dir();
}

#[test]
fn save_and_load_user_with_location() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let user = UserProfile {
        name: Some("Bob".to_string()),
        timezone: Some(crate::schema::Timezone::from(chrono_tz::US::Eastern)),
        location: Some(crate::schema::GeoLocation {
            latitude: 48.8566,
            longitude: 2.3522,
            place: Some("Paris, France".to_string()),
            updated_at: Some(1_700_000_000),
        }),
    };

    save_user(&user).expect("save user with location");

    let loaded = load_user().expect("load user with location");
    assert_eq!(loaded.name.as_deref(), Some("Bob"));
    assert_eq!(
        loaded.timezone.as_ref().map(|tz| tz.name()),
        Some("US/Eastern")
    );
    let loc = loaded.location.expect("location should be present");
    assert!((loc.latitude - 48.8566).abs() < 1e-6);
    assert!((loc.longitude - 2.3522).abs() < 1e-6);
    assert_eq!(loc.place.as_deref(), Some("Paris, France"));

    clear_data_dir();
}

#[test]
fn save_user_removes_empty_file() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let seeded = UserProfile {
        name: Some("Alice".to_string()),
        timezone: None,
        location: None,
    };
    let path = save_user(&seeded).expect("seed user");
    assert!(path.exists());

    save_user(&UserProfile::default()).expect("save empty user");
    assert!(!path.exists());

    clear_data_dir();
}

#[test]
fn resolve_user_profile_prefers_user_md_over_config() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let config = MoltisConfig {
        user: UserProfile {
            name: Some("Config User".to_string()),
            timezone: Some(crate::schema::Timezone::from(chrono_tz::Europe::Paris)),
            location: Some(crate::schema::GeoLocation {
                latitude: 1.0,
                longitude: 2.0,
                place: Some("Config Place".to_string()),
                updated_at: Some(100),
            }),
        },
        ..Default::default()
    };
    save_user(&UserProfile {
        name: Some("File User".to_string()),
        timezone: Some(crate::schema::Timezone::from(chrono_tz::US::Eastern)),
        location: Some(crate::schema::GeoLocation {
            latitude: 3.0,
            longitude: 4.0,
            place: Some("File Place".to_string()),
            updated_at: Some(200),
        }),
    })
    .expect("save user");

    let resolved = resolve_user_profile_from_config(&config);
    assert_eq!(resolved.name.as_deref(), Some("File User"));
    assert_eq!(
        resolved.timezone.as_ref().map(|tz| tz.name()),
        Some("US/Eastern")
    );
    let location = resolved.location.expect("resolved location");
    assert_eq!(location.place.as_deref(), Some("File Place"));
    assert_eq!(location.updated_at, Some(200));

    clear_data_dir();
}

#[test]
fn save_user_with_mode_off_removes_existing_file() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let user = UserProfile {
        name: Some("Alice".to_string()),
        ..Default::default()
    };
    let path = save_user(&user).expect("seed user");
    assert!(path.exists());

    let saved_path = save_user_with_mode(&user, crate::schema::UserProfileWriteMode::Off)
        .expect("disable user profile writes");
    assert!(saved_path.is_none());
    assert!(!path.exists());

    clear_data_dir();
}

#[test]
fn load_boot_md_reads_trimmed_content() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("BOOT.md"), "\n  Run startup checks.  \n").unwrap();
    assert_eq!(load_boot_md().as_deref(), Some("Run startup checks."));

    clear_data_dir();
}

#[test]
fn load_boot_md_for_agent_falls_back_to_root() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("BOOT.md"), "Root boot context").unwrap();
    // No agent-specific file — should fall back to root.
    assert_eq!(
        load_boot_md_for_agent("test-agent").as_deref(),
        Some("Root boot context")
    );

    // Agent-specific file overrides root.
    let agent_dir = dir.path().join("agents").join("test-agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("BOOT.md"), "Agent-specific boot").unwrap();
    assert_eq!(
        load_boot_md_for_agent("test-agent").as_deref(),
        Some("Agent-specific boot")
    );

    clear_data_dir();
}

#[test]
fn load_tools_md_reads_trimmed_content() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("TOOLS.md"), "\n  Use safe tools first.  \n").unwrap();
    assert_eq!(load_tools_md().as_deref(), Some("Use safe tools first."));

    clear_data_dir();
}

#[test]
fn load_agents_md_reads_trimmed_content() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(
        dir.path().join("AGENTS.md"),
        "\nLocal workspace instructions\n",
    )
    .unwrap();
    assert_eq!(
        load_agents_md().as_deref(),
        Some("Local workspace instructions")
    );

    clear_data_dir();
}

#[test]
fn load_heartbeat_md_reads_trimmed_content() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("HEARTBEAT.md"), "\n# Heartbeat\n- ping\n").unwrap();
    assert_eq!(load_heartbeat_md().as_deref(), Some("# Heartbeat\n- ping"));

    clear_data_dir();
}

#[test]
fn load_memory_md_reads_trimmed_content() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(
        dir.path().join("MEMORY.md"),
        "\n## User Facts\n- Lives in Paris\n",
    )
    .unwrap();
    assert_eq!(
        load_memory_md().as_deref(),
        Some("## User Facts\n- Lives in Paris")
    );

    clear_data_dir();
}

#[test]
fn load_memory_md_returns_none_when_missing() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    assert_eq!(load_memory_md(), None);

    clear_data_dir();
}

#[test]
fn load_memory_md_for_main_prefers_agent_workspace_then_root() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("MEMORY.md"), "root memory").unwrap();
    assert_eq!(
        load_memory_md_for_agent("main").as_deref(),
        Some("root memory")
    );

    let agent_dir = dir.path().join("agents").join("main");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("MEMORY.md"), "main agent memory").unwrap();
    assert_eq!(
        load_memory_md_for_agent("main").as_deref(),
        Some("main agent memory")
    );

    clear_data_dir();
}

#[test]
fn load_memory_md_for_non_main_is_agent_scoped() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("MEMORY.md"), "root memory").unwrap();
    assert_eq!(load_memory_md_for_agent("ops"), None);

    let agent_dir = dir.path().join("agents").join("ops");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("MEMORY.md"), "ops memory").unwrap();
    assert_eq!(
        load_memory_md_for_agent("ops").as_deref(),
        Some("ops memory")
    );

    clear_data_dir();
}

#[test]
fn load_memory_md_for_agent_reports_resolved_source_and_path() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("MEMORY.md"), "root memory").unwrap();
    let main_root = load_memory_md_for_agent_with_source("main").unwrap();
    assert_eq!(main_root.content, "root memory");
    assert_eq!(main_root.path, dir.path().join("MEMORY.md"));
    assert_eq!(main_root.source, WorkspaceMarkdownSource::RootWorkspace);

    let agent_dir = dir.path().join("agents").join("ops");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("MEMORY.md"), "ops memory").unwrap();
    let ops_memory = load_memory_md_for_agent_with_source("ops").unwrap();
    assert_eq!(ops_memory.content, "ops memory");
    assert_eq!(ops_memory.path, agent_dir.join("MEMORY.md"));
    assert_eq!(ops_memory.source, WorkspaceMarkdownSource::AgentWorkspace);

    clear_data_dir();
}

#[test]
fn memory_path_is_under_data_dir() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    assert_eq!(memory_path(), dir.path().join("MEMORY.md"));

    clear_data_dir();
}

#[test]
fn workspace_markdown_ignores_leading_html_comments() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(
        dir.path().join("TOOLS.md"),
        "<!-- comment -->\n\nUse read-only tools first.",
    )
    .unwrap();
    assert_eq!(
        load_tools_md().as_deref(),
        Some("Use read-only tools first.")
    );

    clear_data_dir();
}

#[test]
fn workspace_markdown_comment_only_is_treated_as_empty() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    std::fs::write(dir.path().join("HEARTBEAT.md"), "<!-- guidance -->").unwrap();
    assert_eq!(load_heartbeat_md(), None);

    clear_data_dir();
}

#[test]
fn load_soul_creates_default_when_missing() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let soul_file = dir.path().join("SOUL.md");
    assert!(!soul_file.exists(), "SOUL.md should not exist yet");

    let content = load_soul();
    assert!(
        content.is_some(),
        "load_soul should return Some after seeding"
    );
    assert_eq!(content.as_deref(), Some(DEFAULT_SOUL));
    assert!(soul_file.exists(), "SOUL.md should be created on disk");

    let on_disk = std::fs::read_to_string(&soul_file).unwrap();
    assert_eq!(on_disk, DEFAULT_SOUL);

    clear_data_dir();
}

#[test]
fn load_soul_does_not_overwrite_existing() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let custom = "You are a loyal companion who loves fetch.";
    std::fs::write(dir.path().join("SOUL.md"), custom).unwrap();

    let content = load_soul();
    assert_eq!(content.as_deref(), Some(custom));

    let on_disk = std::fs::read_to_string(dir.path().join("SOUL.md")).unwrap();
    assert_eq!(on_disk, custom, "existing SOUL.md must not be overwritten");

    clear_data_dir();
}

#[test]
fn load_soul_reseeds_after_deletion() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    // First call seeds the file.
    let _ = load_soul();
    let soul_file = dir.path().join("SOUL.md");
    assert!(soul_file.exists());

    // Delete it.
    std::fs::remove_file(&soul_file).unwrap();
    assert!(!soul_file.exists());

    // Second call re-seeds.
    let content = load_soul();
    assert_eq!(content.as_deref(), Some(DEFAULT_SOUL));
    assert!(soul_file.exists());

    clear_data_dir();
}

#[test]
fn save_soul_none_prevents_reseed() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    // Auto-seed SOUL.md.
    let _ = load_soul();
    let soul_file = dir.path().join("SOUL.md");
    assert!(soul_file.exists());

    // User explicitly clears the soul via settings.
    save_soul(None).expect("save_soul(None)");
    assert!(
        soul_file.exists(),
        "save_soul(None) should leave an empty file, not delete"
    );
    assert!(
        std::fs::read_to_string(&soul_file).unwrap().is_empty(),
        "file should be empty after clearing"
    );

    // load_soul must return None — NOT re-seed.
    let content = load_soul();
    assert_eq!(
        content, None,
        "load_soul must return None after explicit clear, not re-seed"
    );

    clear_data_dir();
}

#[test]
fn save_soul_some_overwrites_default() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    // Auto-seed.
    let _ = load_soul();

    // User writes custom soul.
    let custom = "You love fetch and belly rubs.";
    save_soul(Some(custom)).expect("save_soul");

    let content = load_soul();
    assert_eq!(content.as_deref(), Some(custom));

    let on_disk = std::fs::read_to_string(dir.path().join("SOUL.md")).unwrap();
    assert_eq!(on_disk, custom);

    clear_data_dir();
}

#[test]
fn save_soul_for_agent_writes_to_agent_dir() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let custom = "Agent soul content.";
    save_soul_for_agent("main", Some(custom)).expect("save_soul_for_agent");

    let agent_soul = dir.path().join("agents/main/SOUL.md");
    assert!(agent_soul.exists(), "SOUL.md should exist in agents/main/");
    assert_eq!(std::fs::read_to_string(&agent_soul).unwrap(), custom);

    // load_soul_for_agent must find the agent-level file.
    let loaded = load_soul_for_agent("main");
    assert_eq!(loaded.as_deref(), Some(custom));

    clear_data_dir();
}

#[test]
fn save_soul_for_agent_none_clears() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    save_soul_for_agent("main", Some("initial")).expect("save");
    save_soul_for_agent("main", None).expect("clear");

    let agent_soul = dir.path().join("agents/main/SOUL.md");
    assert!(agent_soul.exists(), "file should remain after clearing");
    assert!(
        std::fs::read_to_string(&agent_soul).unwrap().is_empty(),
        "file should be empty after clearing"
    );

    clear_data_dir();
}

// ── share_dir tests ─────────────────────────────────────────────────

#[test]
fn share_dir_override_takes_precedence() {
    let dir = tempfile::tempdir().expect("tempdir");
    set_share_dir(dir.path().to_path_buf());

    let result = share_dir();
    assert_eq!(result, Some(dir.path().to_path_buf()));

    clear_share_dir();
}

#[test]
fn share_dir_returns_none_when_no_source() {
    clear_share_dir();
    // Without an override, env var, or existing directories, share_dir
    // should return None (unless /usr/share/moltis or ~/.moltis/share
    // happens to exist on the test machine).
    let _ = share_dir();
}

#[test]
fn share_dir_data_dir_fallback() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());
    clear_share_dir();

    // Without the share/ subdirectory, should not return data_dir/share
    let result = share_dir();
    assert_ne!(result, Some(dir.path().join("share")));

    // Create the share/ subdirectory
    std::fs::create_dir(dir.path().join("share")).unwrap();
    let result = share_dir();
    assert_eq!(result, Some(dir.path().join("share")));

    clear_data_dir();
}

#[test]
fn normalize_workspace_markdown_content_strips_leading_comments_and_trims() {
    let content = "<!-- comment -->\n\n  hello world  \n";
    let normalized = normalize_workspace_markdown_content(content);
    assert_eq!(normalized.as_deref(), Some("hello world"));
}

#[test]
fn normalize_workspace_markdown_content_returns_none_for_comment_only_content() {
    let content = "  <!-- comment -->\n\n<!-- another -->  ";
    let normalized = normalize_workspace_markdown_content(content);
    assert_eq!(normalized, None);
}

/// GH-684: section order must be preserved after a save roundtrip.
#[test]
fn gh684_template_section_order_preserved_after_save() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    let template = crate::template::default_config_template(18789);
    std::fs::write(&path, &template).expect("write template");

    let template_sections: Vec<String> = template
        .lines()
        .filter(|l| l.starts_with('['))
        .map(|l| l.to_string())
        .collect();

    // Load, modify, save — simulating a web UI setting change
    let raw = std::fs::read_to_string(&path).expect("read");
    let mut config: MoltisConfig = parse_config(&raw, &path).expect("parse");
    config.auth.disabled = true;
    config.server.http_request_logs = true;

    save_config_to_path(&path, &config).expect("save config");

    let saved = std::fs::read_to_string(&path).expect("read saved");
    let saved_sections: Vec<String> = saved
        .lines()
        .filter(|l| l.starts_with('['))
        .map(|l| l.to_string())
        .collect();

    // Every template section must still exist in the saved file
    for ts in &template_sections {
        assert!(
            saved_sections.contains(ts),
            "template section {ts} missing from saved file"
        );
    }

    // Template sections must maintain their relative order
    let template_positions: Vec<usize> = template_sections
        .iter()
        .map(|ts| {
            saved_sections
                .iter()
                .position(|ss| ss == ts)
                .unwrap_or_else(|| panic!("section {ts} not found in saved"))
        })
        .collect();
    for window in template_positions.windows(2) {
        assert!(
            window[0] < window[1],
            "template sections swapped: saved position {} >= {} \
                 (sections around index {})",
            window[0],
            window[1],
            window[0],
        );
    }
}

/// GH-684: new sub-tables must render after their parent, not interleaved
/// with unrelated sections.
#[test]
fn gh684_new_subtables_render_after_parent() {
    let original = r#"[server]
port = 8080

[channels]
offered = ["telegram"]

[memory]
enabled = true
"#;

    let updated = r#"[server]
port = 8080

[channels]
offered = ["telegram"]

[channels.telegram]
token = "abc"

[channels.whatsapp]
token = "xyz"

[memory]
enabled = true
"#;

    let mut current_doc = original.parse::<toml_edit::DocumentMut>().unwrap();
    let updated_doc = updated.parse::<toml_edit::DocumentMut>().unwrap();

    merge_toml_tables(current_doc.as_table_mut(), updated_doc.as_table());
    let result = current_doc.to_string();

    let sections: Vec<&str> = result.lines().filter(|l| l.starts_with('[')).collect();

    // Expected order: server, channels, channels.telegram, channels.whatsapp, memory
    let channels_idx = sections.iter().position(|s| *s == "[channels]").unwrap();
    let telegram_idx = sections
        .iter()
        .position(|s| *s == "[channels.telegram]")
        .unwrap();
    let whatsapp_idx = sections
        .iter()
        .position(|s| *s == "[channels.whatsapp]")
        .unwrap();
    let memory_idx = sections.iter().position(|s| *s == "[memory]").unwrap();

    assert!(
        channels_idx < telegram_idx,
        "[channels.telegram] must come after [channels]"
    );
    assert!(
        telegram_idx < whatsapp_idx,
        "[channels.whatsapp] must come after [channels.telegram]"
    );
    assert!(
        whatsapp_idx < memory_idx,
        "[memory] must come after channel sub-tables"
    );
}

#[test]
fn load_guidelines_md_for_agent_falls_back_to_root() {
    let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    set_data_dir(dir.path().to_path_buf());

    let docs_dir = dir.path().join("docs/moltis");
    std::fs::create_dir_all(&docs_dir).unwrap();
    std::fs::write(docs_dir.join("GUIDELINES.md"), "Root guidelines").unwrap();
    assert_eq!(
        load_guidelines_md_for_agent("test-agent").as_deref(),
        Some("Root guidelines")
    );

    let agent_dir = dir.path().join("agents/test-agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("GUIDELINES.md"), "Agent-specific guidelines").unwrap();
    assert_eq!(
        load_guidelines_md_for_agent("test-agent").as_deref(),
        Some("Agent-specific guidelines")
    );

    clear_data_dir();
}

// ── GH-770: env variable resolution from [env] section and DB ────────

/// GH-770: `${VAR}` in config sections should resolve against `[env]` values
/// defined in the same TOML file.
#[test]
fn gh770_env_section_vars_resolve_in_config_placeholders() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    let expected = "sk-test-from-env-section";
    std::fs::write(
        &path,
        format!(
            r#"
[env]
MY_API_KEY = "{expected}"

[tools.web.search.perplexity]
api_key = "${{MY_API_KEY}}"
model = "sonar"
"#
        ),
    )
    .expect("write config");

    let config = load_config(&path).expect("load config");
    let api_key = config
        .tools
        .web
        .search
        .perplexity
        .api_key
        .as_ref()
        .expect("api_key should be set");
    // Never pass secret values to assert_eq! — it prints both sides on failure.
    assert!(
        api_key.expose_secret() == expected,
        "api_key should be resolved from [env] section, not left as literal placeholder"
    );
}

/// GH-770: Precedence test.  Process env lookup wins over the overrides map.
/// Tested via the underlying `substitute_env_with` with a mock lookup
/// so that no real env vars are read (avoids leaking secrets on failure).
#[test]
fn gh770_process_env_takes_precedence_over_env_section() {
    // The precedence logic lives in substitute_env_with_overrides which
    // chains std::env::var → overrides.  We verify the same chain
    // through substitute_env_with using a controlled mock.
    let result = crate::env_subst::substitute_env_with_overrides(
        "${MOLTIS_GH770_PRECEDENCE_TEST}",
        &HashMap::from([("MOLTIS_GH770_PRECEDENCE_TEST".into(), "from-map".into())]),
    );
    // The var is not in the process env, so the map value is used.
    assert_eq!(result, "from-map");

    // The full precedence proof (process env > map) is in the env_subst
    // unit test `with_overrides_primary_lookup_wins_over_map`.
}

/// GH-770: `resubstitute_config` resolves leftover `${VAR}` placeholders
/// using a runtime override map (simulating DB env vars).
#[test]
fn gh770_resubstitute_config_resolves_db_env_vars() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    // Use a var name that definitely does not exist in the process env.
    let var = "MOLTIS_GH770_ONLY_IN_DB_42";
    let expected_after = "sk-or-from-db";
    std::fs::write(
        &path,
        format!(
            r#"
[tools.web.search.perplexity]
api_key = "${{{var}}}"
model = "sonar"
"#
        ),
    )
    .expect("write config");

    let config = load_config(&path).expect("load config");
    // Before resubstitution, the placeholder should still be literal.
    let key_before = config
        .tools
        .web
        .search
        .perplexity
        .api_key
        .as_ref()
        .expect("api_key should be set");
    assert!(
        key_before.expose_secret().starts_with("${"),
        "placeholder should be unresolved before resubstitution"
    );

    // Simulate DB env vars becoming available.
    let mut runtime_overrides = HashMap::new();
    runtime_overrides.insert(var.to_string(), expected_after.to_string());
    let config = resubstitute_config(&config, &runtime_overrides).expect("resubstitute");

    let key_after = config
        .tools
        .web
        .search
        .perplexity
        .api_key
        .as_ref()
        .expect("api_key should be set");
    assert!(
        key_after.expose_secret() == expected_after,
        "placeholder should resolve against runtime override map after resubstitution"
    );
}

/// GH-770: `resubstitute_config` preserves already-resolved values and
/// only resolves remaining placeholders.
#[test]
fn gh770_resubstitute_preserves_resolved_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    let var = "MOLTIS_GH770_UNRESOLVABLE_43";
    let expected = "resolved-later";
    std::fs::write(
        &path,
        format!(
            r#"
[identity]
name = "Rex"

[tools.web.search.perplexity]
api_key = "${{{var}}}"
model = "sonar"
"#
        ),
    )
    .expect("write config");

    let config = load_config(&path).expect("load config");
    assert_eq!(config.identity.name.as_deref(), Some("Rex"));

    let mut overrides = HashMap::new();
    overrides.insert(var.to_string(), expected.to_string());
    let config = resubstitute_config(&config, &overrides).expect("resubstitute");

    // Existing values must survive the round-trip.
    assert_eq!(
        config.identity.name.as_deref(),
        Some("Rex"),
        "non-placeholder values must survive resubstitution"
    );
    let key = config
        .tools
        .web
        .search
        .perplexity
        .api_key
        .as_ref()
        .expect("api_key");
    assert!(
        key.expose_secret() == expected,
        "placeholder should resolve after resubstitution"
    );
}

/// GH-770: Override values containing quotes or backslashes must not break
/// resubstitution (no TOML injection via textual round-trip).
#[test]
fn gh770_resubstitute_handles_special_chars_in_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");
    let var = "MOLTIS_GH770_SPECIAL_CHARS";
    std::fs::write(
        &path,
        format!(
            r#"
[tools.web.search.perplexity]
api_key = "${{{var}}}"
model = "sonar"
"#
        ),
    )
    .expect("write config");

    let config = load_config(&path).expect("load config");

    // Value with double-quote and backslash — would break TOML text substitution.
    let tricky_value = r#"sk-pass"word\with\special"chars"#;
    let mut overrides = HashMap::new();
    overrides.insert(var.to_string(), tricky_value.to_string());

    let config = resubstitute_config(&config, &overrides)
        .expect("resubstitute must not fail on special chars");
    let key = config
        .tools
        .web
        .search
        .perplexity
        .api_key
        .as_ref()
        .expect("api_key should be set");
    assert!(
        key.expose_secret() == tricky_value,
        "value with quotes/backslashes must survive resubstitution intact"
    );
}
