use {
    super::*,
    crate::{AgentRuntimeLimitSource, AgentRuntimeLimits, schema::SandboxMountAccess},
};

#[test]
fn agent_runtime_limits_use_global_fallbacks() {
    let config: MoltisConfig = toml::from_str(
        r#"
[tools]
agent_timeout_secs = 120
agent_max_iterations = 11

[agents.presets.quick]
model = "openai/gpt-5.2"
"#,
    )
    .unwrap();

    let limits = config.agent_runtime_limits("quick");
    assert_eq!(limits.timeout_secs, 120);
    assert_eq!(limits.timeout_source, AgentRuntimeLimitSource::GlobalTools);
    assert_eq!(limits.max_iterations, 11);
    assert_eq!(
        limits.max_iterations_source,
        AgentRuntimeLimitSource::GlobalTools
    );
}

#[test]
fn agent_runtime_limits_use_partial_preset_overrides() {
    let config: MoltisConfig = toml::from_str(
        r#"
[tools]
agent_timeout_secs = 120
agent_max_iterations = 11

[agents.presets.quick]
timeout_secs = 5
"#,
    )
    .unwrap();

    let limits = config.agent_runtime_limits("quick");
    assert_eq!(limits.timeout_secs, 5);
    assert_eq!(limits.timeout_source, AgentRuntimeLimitSource::AgentPreset);
    assert_eq!(limits.max_iterations, 11);
    assert_eq!(
        limits.max_iterations_source,
        AgentRuntimeLimitSource::GlobalTools
    );
}

#[test]
fn spawned_agent_runtime_limits_preserve_default_no_timeout() {
    let config: MoltisConfig = toml::from_str(
        r#"
[agents.presets.quick]
max_iterations = 7
"#,
    )
    .unwrap();

    let preset = config.agents.get_preset("quick");
    let limits = AgentRuntimeLimits::resolve_for_spawned_agent(&config.tools, preset);
    assert_eq!(limits.timeout_secs, 0);
    assert_eq!(limits.max_iterations, 7);
}

#[test]
fn spawned_agent_runtime_limits_require_preset_timeout() {
    let config: MoltisConfig = toml::from_str(
        r#"
[tools]
agent_timeout_secs = 1800

[agents.presets.deep]
max_iterations = 80
"#,
    )
    .unwrap();

    let preset = config.agents.get_preset("deep");
    let limits = AgentRuntimeLimits::resolve_for_spawned_agent(&config.tools, preset);
    assert_eq!(limits.timeout_secs, 0);
    assert_eq!(limits.timeout_source, AgentRuntimeLimitSource::GlobalTools);
    assert_eq!(limits.max_iterations, 80);
}

#[test]
fn spawned_agent_runtime_limits_use_preset_timeout() {
    let config: MoltisConfig = toml::from_str(
        r#"
[tools]
agent_timeout_secs = 1800

[agents.presets.deep]
timeout_secs = 600
max_iterations = 80
"#,
    )
    .unwrap();

    let preset = config.agents.get_preset("deep");
    let limits = AgentRuntimeLimits::resolve_for_spawned_agent(&config.tools, preset);
    assert_eq!(limits.timeout_secs, 600);
    assert_eq!(limits.timeout_source, AgentRuntimeLimitSource::AgentPreset);
    assert_eq!(limits.max_iterations, 80);
}

#[test]
fn preset_max_iterations_must_be_positive() {
    let result = validate_toml_str(
        r#"
[agents.presets.quick]
max_iterations = 0
"#,
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            && diagnostic.category == "invalid-value"
            && diagnostic.path == "agents.presets.quick.max_iterations"
    }));
}

#[test]
fn reasoning_effort_valid_values_no_error() {
    for effort in &[
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "extra-high",
        "max",
    ] {
        let toml = format!(
            r#"
            [agents.presets.thinker]
            model = "claude-opus-4-5-20251101"
            reasoning_effort = "{effort}"
            "#
        );
        let result = validate_toml_str(&toml);
        let errors: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.path.contains("reasoning_effort") && d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "effort={effort} should be valid: {errors:?}"
        );
    }
}

#[test]
fn reasoning_effort_invalid_value_reports_type_error() {
    let toml = r#"
    [agents.presets.thinker]
    model = "claude-opus-4-5-20251101"
    reasoning_effort = "extreme"
    "#;
    let result = validate_toml_str(toml);
    let error = result
        .diagnostics
        .iter()
        .find(|d| d.category == "type-error" && d.severity == Severity::Error);
    assert!(
        error.is_some(),
        "invalid reasoning_effort should produce type error: {:?}",
        result.diagnostics
    );
}

#[test]
fn reasoning_effort_recognized_in_schema() {
    let toml = r#"
    [agents.presets.thinker]
    reasoning_effort = "high"
    "#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.message.contains("reasoning_effort"));
    assert!(
        unknown.is_none(),
        "reasoning_effort should be a recognized field, got: {:?}",
        result.diagnostics
    );
}

fn find_preset_silent_policy_warning(result: &ValidationResult) -> Option<&Diagnostic> {
    result.diagnostics.iter().find(|d| {
        d.category == "security" && d.path == "agents.presets" && d.message.contains("spawn_agent")
    })
}

#[test]
fn preset_tools_deny_without_main_policy_warns() {
    let toml = r#"
[agents]
default_preset = "full"

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser", "web_fetch"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert_eq!(warning.severity, Severity::Warning);
    assert!(
        warning.message.contains("\"full\""),
        "expected preset name in message: {}",
        warning.message
    );
    assert!(
        warning.message.contains("[tools.policy]"),
        "expected pointer to [tools.policy] in message: {}",
        warning.message
    );
}

#[test]
fn preset_tools_allow_without_main_policy_also_warns() {
    let toml = r#"
[agents.presets.research]
[agents.presets.research.tools]
allow = ["web_search", "web_fetch"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert!(warning.message.contains("\"research\""));
}

#[test]
fn preset_tools_deny_with_main_policy_deny_does_not_warn() {
    let toml = r#"
[tools.policy]
deny = ["exec"]

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy] is non-empty, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn preset_tools_deny_with_main_policy_allow_does_not_warn() {
    let toml = r#"
[tools.policy]
allow = ["web_search"]

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy] has allow list, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn preset_tools_deny_with_main_policy_profile_does_not_warn() {
    let toml = r#"
[tools.policy]
profile = "default"

[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when [tools.policy.profile] is set, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn empty_preset_tools_does_not_warn() {
    let toml = r#"
[agents]
default_preset = "basic"

[agents.presets.basic]
model = "openai/gpt-5.2"
"#;
    let result = validate_toml_str(toml);
    assert!(
        find_preset_silent_policy_warning(&result).is_none(),
        "should not warn when presets declare no tool policy, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn multiple_offending_presets_are_rolled_up() {
    let toml = r#"
[agents.presets.full]
[agents.presets.full.tools]
deny = ["browser"]

[agents.presets.minimal]
[agents.presets.minimal.tools]
allow = ["web_search"]
"#;
    let result = validate_toml_str(toml);
    let warning = find_preset_silent_policy_warning(&result).unwrap_or_else(|| {
        panic!(
            "expected silent-policy warning, got: {:?}",
            result.diagnostics
        )
    });
    assert!(
        warning.message.contains("\"full\"") && warning.message.contains("\"minimal\""),
        "expected both preset names in single rolled-up warning: {}",
        warning.message
    );
    // And only one such diagnostic should be emitted.
    let count = result
        .diagnostics
        .iter()
        .filter(|d| d.category == "security" && d.path == "agents.presets")
        .count();
    assert_eq!(count, 1, "expected exactly one rolled-up warning");
}

#[test]
fn external_agents_known_kinds_not_warned() {
    let toml = r#"
[external_agents]
enabled = true

[external_agents.agents.claude-code]
binary = "claude"
models = ["claude-opus-4-8", "claude-sonnet-4-6"]
efforts = ["high", "xhigh"]

[external_agents.agents.codex]
binary = "codex"
models = ["gpt-5.5", "gpt-5.4"]
efforts = ["medium", "high", "xhigh"]

[external_agents.agents.acp]
binary = "/path/to/acp-agent"
args = ["--stdio"]

[external_agents.agents.acp-copilot]
binary = "copilot"
args = ["--acp"]

[external_agents.agents.acp-codex]
binary = "codex-acp"

[external_agents.agents.acp-claude]
binary = "claude-agent-acp"

[external_agents.agents.acp-pi]
binary = "pi-acp"

[external_agents.agents.acp-opencode]
binary = "opencode"
args = ["acp"]

[external_agents.agents.acp-gemini]
binary = "gemini"
args = ["--experimental-acp"]

[external_agents.agents.acp-augment]
binary = "auggie"
args = ["--acp"]

[external_agents.agents.acp-kiro]
binary = "kiro-cli"
args = ["acp"]

[external_agents.agents.acp-openclaw]
binary = "openclaw"
args = ["acp"]

[external_agents.agents.acp-openhands]
binary = "openhands"
args = ["acp"]

[external_agents.agents.acp-kimi]
binary = "kimi"
args = ["acp"]

[external_agents.agents.acp-minimax-code]
binary = "mcode"
args = ["acp"]

[external_agents.agents.acp-stakpak]
binary = "stakpak"
args = ["acp"]

[external_agents.agents.acp-fast-agent]
binary = "fast-agent-acp"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path.starts_with("external_agents.agents.") && d.category == "unknown-field");
    assert!(
        warning.is_none(),
        "known external agent kinds should not warn, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn external_agents_unknown_kind_warned_with_suggestion() {
    let toml = r#"
[external_agents]
enabled = true

[external_agents.agents.claude_code]
binary = "claude"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "external_agents.agents.claude_code" && d.category == "unknown-field");
    assert!(
        warning.is_some(),
        "unknown external agent kind should produce warning, got: {:?}",
        result.diagnostics
    );
    let warning = match warning {
        Some(warning) => warning,
        None => unreachable!("assert above guarantees warning exists"),
    };
    assert!(
        warning.message.contains("Did you mean \"claude-code\"?"),
        "expected typo suggestion in warning, got: {:?}",
        warning
    );
}

fn mount_diagnostics(toml: &str) -> Vec<Diagnostic> {
    validate_toml_str(toml).diagnostics
}

fn errors_mentioning(diagnostics: &[Diagnostic], needle: &str) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter(|d| d.message.contains(needle) || d.path.contains(needle))
        .map(|d| format!("{}: {}", d.path, d.message))
        .collect()
}

#[test]
fn preset_sandbox_mounts_parse_clean() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "rw"

[[agents.presets.walter.sandbox.mounts]]
source = "/srv/notes"
target = "/srv/notes"
"#;
    let config: MoltisConfig = toml::from_str(toml).unwrap();
    let mounts = &config.agents.presets["walter"].sandbox.mounts;
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[0].source, "/srv/vault");
    assert_eq!(mounts[0].access, SandboxMountAccess::Rw);
    // access defaults to read-only when absent
    assert_eq!(mounts[1].access, SandboxMountAccess::Ro);

    let diagnostics = mount_diagnostics(toml);
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    let unknown = diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("mounts"));
    assert!(
        unknown.is_none(),
        "mounts must be a known key, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_mount_unknown_key_is_an_error() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
acces = "rw"
"#;
    let diagnostics = mount_diagnostics(toml);
    let found = diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && d.path.contains("acces"));
    assert!(
        found,
        "unknown key inside a mount table must be an error, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_mount_bad_access_is_an_error() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "readwrite"
"#;
    let diagnostics = mount_diagnostics(toml);
    let found = diagnostics.iter().any(|d| {
        d.severity == Severity::Error
            && d.category != "unknown-field"
            && d.message.contains("readwrite")
    });
    assert!(
        found,
        "an unrecognised access value must be an error naming the value, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_mount_relative_source_is_an_error() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "vault"
target = "/srv/vault"
access = "rw"
"#;
    let diagnostics = mount_diagnostics(toml);
    let errors = errors_mentioning(&diagnostics, "absolute");
    assert!(
        !errors.is_empty(),
        "a relative source must be an error, got: {diagnostics:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("walter")),
        "the diagnostic must name the preset, got: {errors:?}"
    );
}

#[test]
fn preset_sandbox_mounts_sharing_a_target_are_an_error() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/shared"
access = "rw"

[[agents.presets.walter.sandbox.mounts]]
source = "/srv/notes"
target = "/srv/shared"
access = "ro"
"#;
    let diagnostics = mount_diagnostics(toml);
    let errors = errors_mentioning(&diagnostics, "share the target");
    assert!(
        !errors.is_empty(),
        "two mounts sharing a target must be an error, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_force_is_the_only_thing_that_forces_the_sandbox() {
    // The whole semantics in one place: `force` decides, and the container
    // arguments next to it do not. `sandbox_forced` is the single seat every
    // layer asks, so this pins what the UI, the write path and the router all
    // get told.
    let forced = r#"
[agents.presets.walter.sandbox]
force = true
"#;
    let config: MoltisConfig = toml::from_str(forced).unwrap();
    assert!(
        config.agents.sandbox_forced(Some("walter")),
        "force = true must force the sandbox"
    );

    let configured = r#"
[agents.presets.walter.sandbox]
run_as = "1000:1000"

[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "ro"
"#;
    let config: MoltisConfig = toml::from_str(configured).unwrap();
    assert!(
        !config.agents.presets["walter"].sandbox.mounts.is_empty(),
        "the mounts must still be read"
    );
    assert!(
        !config.agents.sandbox_forced(Some("walter")),
        "mounts and a run_as configure the sandbox, they must not force it on"
    );

    let diagnostics = mount_diagnostics(forced);
    let unknown = diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("force"));
    assert!(
        unknown.is_none(),
        "force must be a known key, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_mode_off_with_force_is_an_error() {
    // mode = "off" plus force = true is a contradiction: one says never
    // sandbox this agent, the other says never run it outside a sandbox. The
    // runtime resolves it by forcing the sandbox on, so `moltis config check`
    // has to say so before that happens.
    let toml = r#"
[agents.presets.walter.sandbox]
mode = "off"
force = true
"#;
    let diagnostics = mount_diagnostics(toml);
    let errors = errors_mentioning(&diagnostics, "sandbox.force");
    assert!(
        !errors.is_empty(),
        "mode off next to force must be an error, got: {diagnostics:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("walter")),
        "the diagnostic must name the preset, got: {errors:?}"
    );
}

#[test]
fn preset_sandbox_mode_off_with_mounts_stays_clean() {
    // The inverse of the old rule, and the new semantics in one test: mounts
    // and a run_as configure the sandbox, they never demand one, so they do
    // not contradict `mode = "off"` - they are simply inert there.
    let toml = r#"
[agents.presets.walter.sandbox]
mode = "off"
run_as = "1000:1000"

[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "ro"
"#;
    let errors = errors_mentioning(&mount_diagnostics(toml), "sandbox.force");
    assert!(
        errors.is_empty(),
        "mounts and run_as without force must not contradict mode off, got: {errors:?}"
    );
}

#[test]
fn preset_sandbox_mode_off_alone_stays_clean() {
    // Nothing to contradict: an agent may still ask for no sandbox at all.
    let toml = r#"
[agents.presets.walter.sandbox]
mode = "off"
"#;
    let errors = errors_mentioning(&mount_diagnostics(toml), "sandbox.force");
    assert!(
        errors.is_empty(),
        "mode off on its own must stay clean, got: {errors:?}"
    );
}

#[test]
fn preset_sandbox_rw_mount_warns_naming_agent_and_path() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "rw"
"#;
    let diagnostics = mount_diagnostics(toml);
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .filter(|d| {
            (d.message.contains("walter") || d.path.contains("walter"))
                && d.message.contains("/srv/vault")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        warnings.len(),
        1,
        "expected exactly one rw warning naming walter and the path, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_ro_mount_produces_no_rw_warning() {
    let toml = r#"
[[agents.presets.walter.sandbox.mounts]]
source = "/srv/vault"
target = "/srv/vault"
access = "ro"
"#;
    let diagnostics = mount_diagnostics(toml);
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .filter(|d| {
            (d.message.contains("walter") || d.path.contains("walter"))
                && d.message.contains("/srv/vault")
        })
        .collect::<Vec<_>>();
    assert!(
        warnings.is_empty(),
        "a read-only mount must not warn, got: {warnings:?}"
    );
}

#[test]
fn config_template_documents_the_preset_mount_example() {
    let template = crate::template::default_config_template(18789);
    let header = "# [[agents.presets.kids.sandbox.mounts]]";
    let start = template
        .find(header)
        .unwrap_or_else(|| panic!("template must carry the mount example: {template}"));
    for field in ["source = ", "target = ", "access = "] {
        let line = template[start..]
            .lines()
            .skip(1)
            .take_while(|line| line.starts_with('#'))
            .find(|line| line.contains(field));
        assert!(
            line.is_some(),
            "mount example must document {field:?} right after the header"
        );
    }
}

#[test]
fn preset_sandbox_run_as_parses_and_validates_clean() {
    let toml = r#"
[agents.presets.walter.sandbox]
run_as = "1000:1000"
"#;
    let config: MoltisConfig = toml::from_str(toml).unwrap();
    assert_eq!(
        config.agents.presets["walter"].sandbox.run_as.as_deref(),
        Some("1000:1000")
    );

    let diagnostics = mount_diagnostics(toml);
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    let unknown = diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("run_as"));
    assert!(
        unknown.is_none(),
        "run_as must be a known key, got: {diagnostics:?}"
    );
}

#[test]
fn preset_sandbox_run_as_malformed_is_an_error() {
    // Every shape that is not exactly two non-empty numeric parts. None of
    // them may degrade to "no run_as", which would mean root.
    for value in ["1000", "1000:1000:1000", "1000:", ":1000", "walter:walter"] {
        let toml = format!(
            r#"
[agents.presets.walter.sandbox]
run_as = "{value}"
"#
        );
        let diagnostics = mount_diagnostics(&toml);
        let errors = errors_mentioning(&diagnostics, "run_as");
        assert!(
            !errors.is_empty(),
            "run_as {value:?} must be an error, got: {diagnostics:?}"
        );
        assert!(
            errors.iter().any(|e| e.contains("walter")),
            "the diagnostic must name the preset, got: {errors:?}"
        );
    }
}

#[test]
fn preset_sandbox_run_as_root_uid_is_an_error() {
    // A run_as that silently meant root would be worse than no field at all.
    let toml = r#"
[agents.presets.walter.sandbox]
run_as = "0:0"
"#;
    let diagnostics = mount_diagnostics(toml);
    let errors = errors_mentioning(&diagnostics, "uid 0");
    assert!(
        !errors.is_empty(),
        "a run_as of 0:0 must be an error, got: {diagnostics:?}"
    );
}
