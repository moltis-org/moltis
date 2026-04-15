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
