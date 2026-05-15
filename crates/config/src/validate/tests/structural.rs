use super::*;

#[test]
fn mcp_server_entries_validated() {
    let toml = r#"
[mcp.servers.myserver]
command = "node"
args = ["server.js"]
enabled = true
transport = "stdio"
unknwon_field = true
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("myserver"));
    assert!(
        unknown.is_some(),
        "expected unknown-field in MCP server entry, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn hooks_array_entries_validated() {
    let toml = r#"
[[hooks.hooks]]
name = "test"
command = "echo test"
events = ["startup"]
unknwon = "value"
"#;
    let result = validate_toml_str(toml);
    let unknown = result
        .diagnostics
        .iter()
        .find(|d| d.category == "unknown-field" && d.path.contains("hooks.hooks"));
    assert!(
        unknown.is_some(),
        "expected unknown-field in hooks entry, got: {:?}",
        result.diagnostics
    );
}

#[test]
fn mcp_oauth_client_secret_is_known_field() {
    let toml = r#"
[mcp.servers.hubspot]
url = "https://mcp.hubspot.com"
transport = "streamable-http"

[mcp.servers.hubspot.oauth]
client_id = "client-id"
client_secret = "client-secret"
auth_url = "https://mcp.hubspot.com/oauth/authorize/user"
token_url = "https://mcp.hubspot.com/oauth/v3/token"
"#;
    let result = validate_toml_str(toml);
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.category == "unknown-field"
                && d.path == "mcp.servers.hubspot.oauth.client_secret"),
        "client_secret should be accepted, got: {:?}",
        result.diagnostics
    );
}
