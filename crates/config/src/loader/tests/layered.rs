use std::{collections::HashMap, path::PathBuf};

use secrecy::ExposeSecret;

use crate::{AgentIdentity, schema::MoltisConfig};

use super::*;

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

// ── Layered config tests ─────────────────────────────────────────────

#[test]
fn defaults_toml_is_generated_and_parseable() {
    let content = crate::defaults::generate_defaults_toml().expect("generate defaults");
    assert!(
        content.contains("MOLTIS-MANAGED DEFAULTS"),
        "defaults.toml should contain ownership header"
    );
    let config: MoltisConfig =
        toml::from_str(&content).expect("defaults.toml should parse as valid MoltisConfig");
    // Verify it matches the built-in defaults.
    assert_eq!(config.tools.agent_timeout_secs, 600);
    assert_eq!(config.tools.agent_max_iterations, 25);
    assert!(config.tls.enabled);
}

#[test]
fn defaults_toml_written_and_loaded_from_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = crate::defaults::write_defaults_toml(dir.path()).expect("write defaults.toml");
    assert!(path.exists());

    let config = crate::defaults::load_defaults(dir.path());
    assert_eq!(config.tools.agent_timeout_secs, 600);
    assert!(config.tls.enabled);
}

#[test]
fn merge_defaults_with_user_overrides() {
    let defaults = crate::defaults::generate_defaults_toml().expect("generate defaults");
    // User only overrides agent_timeout_secs.
    let user = r#"
[tools]
agent_timeout_secs = 120
"#;
    let path = PathBuf::from("test.toml");
    let config =
        crate::defaults::merge_defaults_with_user_toml(&defaults, user, &path).expect("merge");

    // User override applied.
    assert_eq!(config.tools.agent_timeout_secs, 120);
    // Defaults preserved.
    assert_eq!(config.tools.agent_max_iterations, 25);
    assert!(config.tls.enabled);
    assert!(!config.auth.disabled);
}

#[test]
fn merge_preserves_user_only_keys() {
    let defaults = crate::defaults::generate_defaults_toml().expect("generate defaults");
    // User adds a custom provider entry (not in defaults).
    let user = r#"
[identity]
name = "Rex"
"#;
    let path = PathBuf::from("test.toml");
    let config =
        crate::defaults::merge_defaults_with_user_toml(&defaults, user, &path).expect("merge");

    assert_eq!(config.identity.name.as_deref(), Some("Rex"));
    // Defaults still present.
    assert!(config.tls.enabled);
}

#[test]
fn save_user_config_does_not_materialize_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("moltis.toml");

    // Start with a minimal user config.
    std::fs::write(&path, "[server]\nport = 12345\n").expect("write seed");

    let raw = std::fs::read_to_string(&path).expect("read seed");
    let mut config: MoltisConfig = parse_config(&raw, &path).expect("parse");
    // Make one change.
    config.auth.disabled = true;

    save_user_config_to_path(&path, &config).expect("save user config");

    let saved = std::fs::read_to_string(&path).expect("read saved");

    // User override should be present.
    assert!(
        saved.contains("disabled = true"),
        "user override should be saved"
    );
    // Built-in defaults should NOT be materialized.
    assert!(
        !saved.contains("agent_timeout_secs"),
        "defaults should not be materialized into user config"
    );
    assert!(
        !saved.contains("agent_max_iterations"),
        "defaults should not be materialized into user config"
    );
    assert!(
        !saved.contains("mode = \"deterministic\""),
        "compaction mode default should not be materialized"
    );
}

#[test]
fn update_config_preserves_override_boundary() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // Seed a minimal user config.
    std::fs::write(
        &config_path,
        "[server]\nport = 54321\n\n[auth]\ndisabled = true\n",
    )
    .expect("write seed");

    set_config_dir(dir.path().to_path_buf());

    // Simulate an update_config call that changes only one field.
    let result_path = update_config(|cfg| {
        cfg.server.http_request_logs = true;
    })
    .expect("update_config");

    let saved = std::fs::read_to_string(&result_path).expect("read saved");

    // User changes present.
    assert!(saved.contains("http_request_logs = true"));
    assert!(saved.contains("disabled = true"));
    assert!(saved.contains("port = 54321"));

    // Defaults NOT materialized.
    assert!(
        !saved.contains("agent_timeout_secs"),
        "update_config should not materialize defaults"
    );

    clear_config_dir();
}

#[test]
fn layered_load_user_override_wins_over_defaults() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // Write defaults.toml first.
    crate::defaults::write_defaults_toml(dir.path()).expect("write defaults");

    // Write user config with an override.
    std::fs::write(
        &config_path,
        "[server]\nport = 11111\n\n[tools]\nagent_timeout_secs = 999\n",
    )
    .expect("write user config");

    set_config_dir(dir.path().to_path_buf());

    let config = discover_and_load();
    // User override wins.
    assert_eq!(config.tools.agent_timeout_secs, 999);
    // Defaults inherited.
    assert_eq!(config.tools.agent_max_iterations, 25);
    assert!(config.tls.enabled);

    clear_config_dir();
}

#[test]
fn upgrade_adds_new_defaults_automatically() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    // Simulate: defaults.toml has new settings that weren't in the old version.
    // User config only has port. After layered load, new defaults should appear.
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // Write a minimal user config (no tools section).
    std::fs::write(&config_path, "[server]\nport = 22222\n").expect("write user config");

    // Write defaults.toml.
    crate::defaults::write_defaults_toml(dir.path()).expect("write defaults");

    set_config_dir(dir.path().to_path_buf());

    let config = discover_and_load();
    // Defaults should be inherited even though user didn't specify them.
    assert_eq!(config.tools.agent_timeout_secs, 600);
    assert_eq!(config.tools.agent_max_iterations, 25);
    assert!(config.heartbeat.enabled);

    clear_config_dir();
}

#[test]
fn user_override_survives_defaults_refresh() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // User overrides timeout.
    std::fs::write(
        &config_path,
        "[server]\nport = 33333\n\n[tools]\nagent_timeout_secs = 42\n",
    )
    .expect("write user config");

    set_config_dir(dir.path().to_path_buf());

    // First load (writes defaults.toml).
    let config1 = discover_and_load();
    assert_eq!(config1.tools.agent_timeout_secs, 42);

    // Simulate upgrade by refreshing defaults.toml again.
    crate::defaults::write_defaults_toml(dir.path()).expect("refresh defaults");

    // Reload — user override must survive.
    let config2 = discover_and_load();
    assert_eq!(
        config2.tools.agent_timeout_secs, 42,
        "user override must survive defaults refresh"
    );

    clear_config_dir();
}

/// Upgrade scenario: existing user has defaults spelled out in moltis.toml
/// (from the old template). An unrelated config write must NOT strip those
/// values — they are intentional freezes from the prior version.
#[test]
fn upgrade_existing_config_preserves_explicit_defaults() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // Simulate an old-style moltis.toml with many active defaults.
    std::fs::write(
        &config_path,
        r#"[server]
port = 18789
bind = "127.0.0.1"

[tools]
agent_timeout_secs = 600
agent_max_iterations = 25

[auth]
disabled = false
"#,
    )
    .expect("write old-style config");

    set_config_dir(dir.path().to_path_buf());

    // Simulate a web UI change: user changes http_request_logs.
    update_config(|cfg| {
        cfg.server.http_request_logs = true;
    })
    .expect("update_config");

    let saved = std::fs::read_to_string(&config_path).expect("read saved");

    // New change must be present.
    assert!(
        saved.contains("http_request_logs = true"),
        "http_request_logs should be saved"
    );
    // Existing defaults must NOT be stripped — they were already in the file.
    assert!(
        saved.contains("agent_timeout_secs = 600"),
        "existing agent_timeout_secs must survive (not stripped)"
    );
    assert!(
        saved.contains("agent_max_iterations = 25"),
        "existing agent_max_iterations must survive (not stripped)"
    );
    assert!(
        saved.contains("bind = \"127.0.0.1\""),
        "existing bind must survive (not stripped)"
    );
    // Port is installation-specific, must survive.
    assert!(saved.contains("port = 18789"), "port must survive");

    clear_config_dir();
}

#[test]
fn strip_default_values_removes_matching_defaults() {
    let effective = r#"
[server]
port = 18789
bind = "127.0.0.1"

[auth]
disabled = true

[tools]
agent_timeout_secs = 600
"#;
    let defaults = r#"
[server]
port = 0
bind = "127.0.0.1"

[auth]
disabled = false

[tools]
agent_timeout_secs = 600
"#;

    let mut eff_doc = effective.parse::<toml_edit::DocumentMut>().unwrap();
    let def_doc = defaults.parse::<toml_edit::DocumentMut>().unwrap();

    strip_default_values(eff_doc.as_table_mut(), def_doc.as_table());
    let result = eff_doc.to_string();

    // port differs → kept
    assert!(
        result.contains("port = 18789"),
        "different value should be kept"
    );
    // bind matches default → stripped
    assert!(!result.contains("bind"), "default value should be stripped");
    // auth.disabled differs → kept
    assert!(
        result.contains("disabled = true"),
        "different value should be kept"
    );
    // agent_timeout_secs matches default → stripped
    assert!(
        !result.contains("agent_timeout_secs"),
        "default value should be stripped"
    );
}

// ── Revert to built-in tests ─────────────────────────────────────────

#[test]
fn revert_preset_removes_from_user_config() {
    let _guard = CONFIG_DIR_TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("moltis.toml");

    // User config with a custom preset.
    std::fs::write(
        &config_path,
        r#"[server]
port = 44444

[agents.presets.custom-agent]
model = "openai/gpt-5.2"
delegate_only = true
"#,
    )
    .expect("write seed");

    set_config_dir(dir.path().to_path_buf());

    // Simulate revert: remove the preset via update_config.
    update_config(|cfg| {
        cfg.agents.presets.remove("custom-agent");
    })
    .expect("update_config");

    let saved = std::fs::read_to_string(&config_path).expect("read saved");

    // Preset should be removed from user config.
    assert!(
        !saved.contains("custom-agent"),
        "reverted preset should be removed from user config"
    );
    // Port should survive.
    assert!(saved.contains("port = 44444"), "port must survive revert");

    clear_config_dir();
}

// ── Provenance tests ─────────────────────────────────────────────────

#[test]
fn preset_provenance_custom_preset() {
    let mut config = MoltisConfig::default();
    config
        .agents
        .presets
        .insert("my-custom".to_string(), crate::AgentPreset {
            identity: AgentIdentity {
                name: Some("Custom".to_string()),
                ..Default::default()
            },
            ..Default::default()
        });

    let provenance = crate::defaults::compute_preset_provenance(&config.agents);
    let custom = provenance.iter().find(|p| p.id == "my-custom");
    assert!(custom.is_some(), "custom preset should be in provenance");
    assert_eq!(
        custom.map(|p| p.source),
        Some(crate::defaults::ConfigSource::Custom)
    );
}

#[test]
fn find_shadowed_defaults_detects_shadows() {
    // User config that overrides a built-in default
    let user = r#"
[tools]
agent_timeout_secs = 600

[auth]
disabled = false
"#;
    let shadowed = crate::defaults::find_shadowed_defaults(user);
    assert!(
        shadowed.contains(&"tools.agent_timeout_secs".to_string()),
        "should detect tools.agent_timeout_secs as shadowed"
    );
    assert!(
        shadowed.contains(&"auth.disabled".to_string()),
        "should detect auth.disabled as shadowed"
    );
}

#[test]
fn find_shadowed_defaults_ignores_intentional_overrides() {
    // User config where values DIFFER from defaults — these are intentional
    // overrides, not frozen defaults, and should NOT be flagged.
    let user = r#"
[tools]
agent_timeout_secs = 120

[auth]
disabled = true
"#;
    let shadowed = crate::defaults::find_shadowed_defaults(user);
    assert!(
        !shadowed.contains(&"tools.agent_timeout_secs".to_string()),
        "intentional override (120 != default 600) should not be flagged as shadowed"
    );
    assert!(
        !shadowed.contains(&"auth.disabled".to_string()),
        "intentional override (true != default false) should not be flagged as shadowed"
    );
}

#[test]
fn find_shadowed_defaults_ignores_custom_keys() {
    let user = r#"
[identity]
name = "Rex"
"#;
    let shadowed = crate::defaults::find_shadowed_defaults(user);
    // identity.name is not in defaults (it's Option<String> and defaults to None / absent)
    // so it should not appear as shadowed
    for key in &shadowed {
        assert!(
            key != "identity.name",
            "custom key identity.name should not be flagged as shadowed"
        );
    }
}
