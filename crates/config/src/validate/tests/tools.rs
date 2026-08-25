use super::*;

#[test]
fn sandbox_mode_off_warned() {
    let toml = r#"
[tools.exec.sandbox]
mode = "off"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.sandbox.mode");
    assert!(warning.is_some(), "expected warning for sandbox mode off");
}

#[test]
fn port_zero_info() {
    let toml = r#"
[server]
port = 0
"#;
    let result = validate_toml_str(toml);
    let info = result
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Info && d.path == "server.port");
    assert!(info.is_some(), "expected info for port 0");
}

#[test]
fn unknown_sandbox_backend_warned() {
    let toml = r#"
[tools.exec.sandbox]
backend = "lxc"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.sandbox.backend");
    assert!(
        warning.is_some(),
        "expected warning for unknown sandbox backend"
    );
}

#[test]
fn podman_sandbox_backend_accepted() {
    let toml = r#"
[tools.exec.sandbox]
backend = "podman"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.sandbox.backend");
    assert!(
        warning.is_none(),
        "podman should be accepted as a valid sandbox backend"
    );
}

#[test]
fn podman_escape_hatch_fields_accepted_and_warned() {
    let toml = r#"
[tools.exec.sandbox]
backend = "podman"
allow_nested_podman = true
"#;
    let result = validate_toml_str(toml);

    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.category != "unknown-field"),
        "podman escape hatch fields should be accepted"
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.path == "tools.exec.sandbox.allow_nested_podman")
    );
    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error)
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn host_podman_escape_hatch_rejected_off_linux() {
    let toml = r#"
[tools.exec.sandbox]
backend = "podman"
allow_host_podman = true
"#;
    let result = validate_toml_str(toml);

    assert!(result.diagnostics.iter().any(|d| {
        d.severity == Severity::Error && d.path == "tools.exec.sandbox.allow_host_podman"
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn host_podman_escape_hatch_accepted_on_linux() {
    let toml = r#"
[tools.exec.sandbox]
backend = "podman"
allow_host_podman = true
"#;
    let result = validate_toml_str(toml);

    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error)
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.path == "tools.exec.sandbox.allow_host_podman")
    );
}

#[test]
fn podman_escape_hatches_require_podman_backend() {
    let toml = r#"
[tools.exec.sandbox]
backend = "docker"
allow_nested_podman = true
"#;
    let result = validate_toml_str(toml);

    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| { d.severity == Severity::Error && d.path == "tools.exec.sandbox.backend" })
    );
}

#[test]
fn podman_escape_hatches_are_mutually_exclusive() {
    let toml = r#"
[tools.exec.sandbox]
backend = "podman"
allow_host_podman = true
allow_nested_podman = true
"#;
    let result = validate_toml_str(toml);

    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| { d.severity == Severity::Error && d.path == "tools.exec.sandbox" })
    );
}

#[test]
fn managed_files_mount_is_recognized_by_schema_validation() {
    let result = validate_toml_str(
        r#"
[tools.exec.sandbox]
managed_files_mount = "rw"
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.path != "tools.exec.sandbox.managed_files_mount"),
        "managed_files_mount should be a recognized typed field"
    );
}

#[test]
fn coder_url_accepts_https_and_local_loopback_http() {
    for url in [
        "https://coder.example.com",
        "https://coder.example.com/base/path",
        "http://localhost:3000",
        "http://127.0.0.1:3000",
        "http://127.42.0.9",
        "http://[::1]:3000",
    ] {
        let result = validate_toml_str(&format!("[tools.exec.sandbox]\ncoder_url = {url:?}\n"));
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.path != "tools.exec.sandbox.coder_url"),
            "expected {url:?} to be accepted: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn coder_url_rejects_insecure_or_ambiguous_urls() {
    for url in [
        "http://coder.example.com",
        "http://192.168.1.10",
        "ftp://coder.example.com",
        "https://user@coder.example.com",
        "https://@coder.example.com",
        "https://coder.example.com?token=secret",
        "https://coder.example.com/#settings",
        "coder.example.com",
        "https://",
    ] {
        let result = validate_toml_str(&format!("[tools.exec.sandbox]\ncoder_url = {url:?}\n"));
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity == Severity::Error
                    && diagnostic.path == "tools.exec.sandbox.coder_url"
            }),
            "expected {url:?} to be rejected: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn explicit_coder_backend_requires_core_configuration() {
    let result = validate_toml_str(
        r#"
[tools.exec.sandbox]
backend = "coder"
"#,
    );

    for path in [
        "tools.exec.sandbox.coder_url",
        "tools.exec.sandbox.coder_token",
        "tools.exec.sandbox",
    ] {
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error && diagnostic.path == path),
            "expected missing Coder field diagnostic at {path}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn explicit_coder_backend_accepts_template_id_or_name() {
    for template in [
        "coder_template_id = \"template-id\"",
        "coder_template_name = \"devbox\"",
    ] {
        let result = validate_toml_str(&format!(
            r#"
[tools.exec.sandbox]
backend = "coder"
coder_url = "https://coder.example.com"
coder_token = "token"
{template}
"#
        ));
        assert!(
            !result.has_errors(),
            "expected valid Coder config: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn explicit_coder_backend_rejects_whitespace_token() {
    let result = validate_toml_str(
        r#"
[tools.exec.sandbox]
backend = "coder"
coder_url = "https://coder.example.com"
coder_token = "   "
coder_template_name = "devbox"
"#,
    );

    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            && diagnostic.path == "tools.exec.sandbox.coder_token"
    }));
}

#[test]
fn coder_ttl_rejects_negative_values_and_accepts_zero() {
    let negative = validate_toml_str(
        r#"
[tools.exec.sandbox]
coder_ttl_ms = -1
"#,
    );
    assert!(negative.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            && diagnostic.path == "tools.exec.sandbox.coder_ttl_ms"
    }));

    let zero = validate_toml_str(
        r#"
[tools.exec.sandbox]
coder_ttl_ms = 0
"#,
    );
    assert!(zero.diagnostics.iter().all(|diagnostic| {
        diagnostic.path != "tools.exec.sandbox.coder_ttl_ms"
            || diagnostic.severity != Severity::Error
    }));
}

#[test]
fn coder_static_diagnostics_allow_unresolved_environment_placeholders() {
    let result = validate_toml_str(
        r#"
[tools.exec.sandbox]
backend = "coder"
coder_url = "${CODER_URL}"
coder_token = "${CODER_SESSION_TOKEN}"
coder_template_name = "${CODER_TEMPLATE_NAME}"
"#,
    );

    assert!(
        !result.has_errors(),
        "unresolved environment placeholders should be deferred: {:?}",
        result.diagnostics
    );
}

#[test]
fn unknown_security_level_warned() {
    let toml = r#"
[tools.exec]
security_level = "paranoid"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.security_level");
    assert!(
        warning.is_some(),
        "expected warning for unknown security level"
    );
}

#[test]
fn ssh_exec_host_accepted() {
    let toml = r#"
[tools.exec]
host = "ssh"
ssh_target = "deploy@example"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.host");
    assert!(
        warning.is_none(),
        "ssh should be accepted as a valid exec host"
    );
}

#[test]
fn ssh_exec_host_without_target_warned() {
    let toml = r#"
[tools.exec]
host = "ssh"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tools.exec.ssh_target");
    assert!(warning.is_some(), "expected warning for missing ssh target");
}

#[test]
fn browser_obscura_fields_accepted() {
    let toml = r#"
[tools.browser]
obscura_path = "/usr/local/bin/obscura"
obscura_stealth = false
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "tools.browser.obscura_path");
    assert!(
        unknown.is_none(),
        "obscura_path should be accepted as a browser config field, got: {:?}",
        result.diagnostics
    );
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "tools.browser.obscura_stealth");
    assert!(
        unknown.is_none(),
        "obscura_stealth should be accepted as a browser config field, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn browser_lightpanda_path_accepted() {
    let toml = r#"
[tools.browser]
lightpanda_path = "/usr/local/bin/lightpanda"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path == "tools.browser.lightpanda_path");
    assert!(
        unknown.is_none(),
        "lightpanda_path should be accepted as a browser config field, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn tools_agent_max_iterations_must_be_positive() {
    let toml = r#"
[tools]
agent_max_iterations = 0
"#;
    let result = validate_toml_str(toml);
    let invalid = result.diagnostics.iter().find(|d| {
        d.path == "tools.agent_max_iterations"
            && d.severity == Severity::Error
            && d.category == "invalid-value"
    });
    assert!(
        invalid.is_some(),
        "expected tools.agent_max_iterations invalid-value error, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn mcp_request_timeout_must_be_positive() {
    let toml = r#"
[mcp]
request_timeout_secs = 0
"#;
    let result = validate_toml_str(toml);
    let invalid = result.diagnostics.iter().find(|d| {
        d.path == "mcp.request_timeout_secs"
            && d.severity == Severity::Error
            && d.category == "invalid-value"
    });
    assert!(
        invalid.is_some(),
        "expected mcp.request_timeout_secs invalid-value error, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn mcp_server_request_timeout_override_must_be_positive() {
    let toml = r#"
[mcp.servers.memory]
command = "npx"
request_timeout_secs = 0
"#;
    let result = validate_toml_str(toml);
    let invalid = result.diagnostics.iter().find(|d| {
        d.path == "mcp.servers.memory.request_timeout_secs"
            && d.severity == Severity::Error
            && d.category == "invalid-value"
    });
    assert!(
        invalid.is_some(),
        "expected mcp server request_timeout_secs invalid-value error, got: {:?}",
        result.diagnostics
    );
}
