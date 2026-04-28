//! MCP server management subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum McpCommand {
    /// List all configured MCP servers.
    List,
    /// Show detailed status for a server.
    Status {
        /// Server name.
        #[arg(long)]
        name: String,
    },
    /// List tools exposed by a server.
    Tools {
        /// Server name.
        #[arg(long)]
        name: String,
    },
    /// Add a new MCP server.
    Add {
        /// Server name.
        #[arg(long)]
        name: String,
        /// Command to run (stdio transport).
        #[arg(long)]
        command: Option<String>,
        /// Command arguments (comma-separated or repeated).
        #[arg(long, value_delimiter = ',')]
        args: Vec<String>,
        /// Transport type: stdio, sse, streamable-http.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// URL for remote transports.
        #[arg(long)]
        url: Option<String>,
        /// Environment variables (KEY=VALUE, repeated).
        #[arg(long = "env", value_parser = parse_env_pair)]
        env_vars: Vec<(String, String)>,
        /// Human-readable display name.
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Remove an MCP server.
    Remove {
        /// Server name.
        #[arg(long)]
        name: String,
    },
    /// Update an existing MCP server.
    Update {
        /// Server name.
        #[arg(long)]
        name: String,
        /// New command.
        #[arg(long)]
        command: Option<String>,
        /// New arguments (comma-separated).
        #[arg(long, value_delimiter = ',')]
        args: Option<Vec<String>>,
        /// New URL.
        #[arg(long)]
        url: Option<String>,
        /// Environment variables (KEY=VALUE, repeated).
        #[arg(long = "env", value_parser = parse_env_pair)]
        env_vars: Vec<(String, String)>,
    },
    /// Enable a disabled server.
    Enable {
        /// Server name.
        #[arg(long)]
        name: String,
    },
    /// Disable a server without removing it.
    Disable {
        /// Server name.
        #[arg(long)]
        name: String,
    },
    /// Restart a running server.
    Restart {
        /// Server name.
        #[arg(long)]
        name: String,
    },
}

fn parse_env_pair(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected KEY=VALUE, got: {s}"))?;
    Ok((k.to_string(), v.to_string()))
}

pub async fn run(client: &mut CtlClient, cmd: McpCommand) -> anyhow::Result<Value> {
    match cmd {
        McpCommand::List => client
            .call("mcp.list", Value::Null)
            .await
            .map_err(Into::into),
        McpCommand::Status { name } => client
            .call("mcp.status", json!({ "name": name }))
            .await
            .map_err(Into::into),
        McpCommand::Tools { name } => client
            .call("mcp.tools", json!({ "name": name }))
            .await
            .map_err(Into::into),
        McpCommand::Add {
            name,
            command,
            args,
            transport,
            url,
            env_vars,
            display_name,
        } => {
            let mut params = json!({
                "name": name,
                "transport": transport,
            });
            let obj = params.as_object_mut().unwrap_or_else(|| unreachable!());
            if let Some(cmd) = command {
                obj.insert("command".into(), json!(cmd));
            }
            if !args.is_empty() {
                obj.insert("args".into(), json!(args));
            }
            if let Some(u) = url {
                obj.insert("url".into(), json!(u));
            }
            if !env_vars.is_empty() {
                let env: serde_json::Map<String, Value> =
                    env_vars.into_iter().map(|(k, v)| (k, json!(v))).collect();
                obj.insert("env".into(), Value::Object(env));
            }
            if let Some(dn) = display_name {
                obj.insert("display_name".into(), json!(dn));
            }
            client.call("mcp.add", params).await.map_err(Into::into)
        },
        McpCommand::Remove { name } => client
            .call("mcp.remove", json!({ "name": name }))
            .await
            .map_err(Into::into),
        McpCommand::Update {
            name,
            command,
            args,
            url,
            env_vars,
        } => {
            let mut params = json!({ "name": name });
            let obj = params.as_object_mut().unwrap_or_else(|| unreachable!());
            if let Some(cmd) = command {
                obj.insert("command".into(), json!(cmd));
            }
            if let Some(a) = args {
                obj.insert("args".into(), json!(a));
            }
            if let Some(u) = url {
                obj.insert("url".into(), json!(u));
            }
            if !env_vars.is_empty() {
                let env: serde_json::Map<String, Value> =
                    env_vars.into_iter().map(|(k, v)| (k, json!(v))).collect();
                obj.insert("env".into(), Value::Object(env));
            }
            client.call("mcp.update", params).await.map_err(Into::into)
        },
        McpCommand::Enable { name } => client
            .call("mcp.enable", json!({ "name": name }))
            .await
            .map_err(Into::into),
        McpCommand::Disable { name } => client
            .call("mcp.disable", json!({ "name": name }))
            .await
            .map_err(Into::into),
        McpCommand::Restart { name } => client
            .call("mcp.restart", json!({ "name": name }))
            .await
            .map_err(Into::into),
    }
}
