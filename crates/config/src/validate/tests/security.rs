use super::*;

#[test]
fn auth_disabled_non_localhost_warned() {
    let toml = r#"
[server]
bind = "0.0.0.0"

[auth]
disabled = true
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.category == "security" && d.path == "auth");
    assert!(
        warning.is_some(),
        "expected security warning for auth disabled + non-localhost"
    );
}

#[test]
fn auth_disabled_localhost_not_warned() {
    let toml = r#"
[server]
bind = "127.0.0.1"

[auth]
disabled = true
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.category == "security" && d.path == "auth");
    assert!(
        warning.is_none(),
        "should not warn about auth disabled on localhost"
    );
}

#[test]
fn tls_cert_without_key_is_error() {
    let toml = r#"
[tls]
cert_path = "/path/to/cert.pem"
"#;
    let result = validate_toml_str(toml);
    let error = result.diagnostics.iter().find(|d| {
        d.severity == Severity::Error && d.path == "tls" && d.message.contains("key_path")
    });
    assert!(
        error.is_some(),
        "expected error for cert_path without key_path, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn tls_key_without_cert_is_error() {
    let toml = r#"
[tls]
key_path = "/path/to/key.pem"
"#;
    let result = validate_toml_str(toml);
    let error = result.diagnostics.iter().find(|d| {
        d.severity == Severity::Error && d.path == "tls" && d.message.contains("cert_path")
    });
    assert!(
        error.is_some(),
        "expected error for key_path without cert_path"
    );
}

#[test]
fn unknown_tailscale_mode_warned() {
    let toml = r#"
[tailscale]
mode = "tunnel"
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.path == "tailscale.mode");
    assert!(
        warning.is_some(),
        "expected warning for unknown tailscale mode 'tunnel'"
    );
}

#[test]
fn ngrok_fields_are_recognized() {
    let toml = r#"
[ngrok]
enabled = true
authtoken = "secret"
domain = "team-gateway.ngrok.app"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.starts_with("ngrok."));
    assert!(
        unknown.is_none(),
        "ngrok fields should be recognized, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn tls_disabled_non_localhost_warned() {
    let toml = r#"
[server]
bind = "0.0.0.0"

[tls]
enabled = false
"#;
    let result = validate_toml_str(toml);
    let warning = result
        .diagnostics
        .iter()
        .find(|d| d.category == "security" && d.path == "tls");
    assert!(
        warning.is_some(),
        "expected security warning for TLS disabled + non-localhost"
    );
}
