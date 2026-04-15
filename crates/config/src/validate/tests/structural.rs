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
