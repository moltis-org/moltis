//! MCP server management subcommands.

use {
    clap::{Args, Subcommand},
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum McpCommand {
    /// Manage MCP repositories through the running gateway.
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
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

#[derive(Subcommand)]
pub enum RepoCommand {
    List,
    Preview(RepoSourceArgs),
    Add(RepoAddArgs),
    Update {
        #[arg(long)]
        id: String,
        #[arg(long)]
        apply: bool,
    },
    Rollback {
        #[arg(long)]
        id: String,
    },
    Remove {
        #[arg(long)]
        id: String,
    },
    Approve(RepoApproveArgs),
}

#[derive(Args, Clone)]
#[command(group(clap::ArgGroup::new("source").required(true).multiple(false).args(["url", "ssh", "local"])))]
pub struct RepoSourceArgs {
    #[arg(long)]
    alias: String,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    ssh: Option<String>,
    #[arg(long)]
    local: Option<std::path::PathBuf>,
    #[arg(long)]
    private: bool,
    #[arg(long)]
    https_credential_id: Option<i64>,
    #[arg(long)]
    ssh_target_id: Option<i64>,
    #[arg(long = "ref", default_value = "HEAD")]
    requested_ref: String,
    #[arg(long)]
    id: Option<String>,
}

#[derive(Args)]
pub struct RepoAddArgs {
    #[command(flatten)]
    source: RepoSourceArgs,
    #[arg(long)]
    approve_all: bool,
    #[arg(long)]
    enable: bool,
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("approval").required(true).multiple(false).args(["all", "server"])))]
pub struct RepoApproveArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    all: bool,
    #[arg(long, action = clap::ArgAction::Append)]
    server: Vec<String>,
    #[arg(long)]
    enable: bool,
}

fn parse_env_pair(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected KEY=VALUE, got: {s}"))?;
    Ok((k.to_string(), v.to_string()))
}

pub async fn run(client: &mut CtlClient, cmd: McpCommand) -> anyhow::Result<Value> {
    match cmd {
        McpCommand::Repo { command } => run_repo(client, command).await,
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

async fn run_repo(client: &mut CtlClient, command: RepoCommand) -> anyhow::Result<Value> {
    match command {
        RepoCommand::List => client
            .call("mcp.repositories.list", json!({}))
            .await
            .map_err(Into::into),
        RepoCommand::Preview(source) => client
            .call("mcp.repositories.preview", source_params(&source))
            .await
            .map_err(Into::into),
        RepoCommand::Add(args) => {
            if args.enable && !args.approve_all {
                anyhow::bail!("--enable requires --approve-all");
            }
            let preview = client
                .call("mcp.repositories.preview", source_params(&args.source))
                .await?;
            let commit = preview
                .get("commit")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("preview response has no commit"))?;
            let candidates = expected_candidates(&preview)?;
            let mut install = source_params(&args.source);
            let object = install
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("invalid repository request"))?;
            let repository_id = preview
                .pointer("/repository/id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("preview response has no repository id"))?;
            object.insert("id".into(), json!(repository_id));
            object.insert("expectedCommit".into(), json!(commit));
            object.insert(
                "selection".into(),
                json!({ "mode": "all", "candidates": candidates }),
            );
            let installed = client.call("mcp.repositories.install", install).await?;
            if !args.approve_all {
                return Ok(installed);
            }
            let id = installed
                .pointer("/repository/id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("install response has no repository id"))?;
            client
                .call(
                    "mcp.managed.approve",
                    json!({
                        "id": id, "expectedCommit": commit,
                        "selection": { "mode": "all", "candidates": candidates },
                        "enable": args.enable,
                    }),
                )
                .await
                .map_err(Into::into)
        },
        RepoCommand::Update { id, apply: false } => client
            .call("mcp.repositories.update.preview", json!({ "id": id }))
            .await
            .map_err(Into::into),
        RepoCommand::Update { id, apply: true } => {
            let preview = client
                .call("mcp.repositories.update.preview", json!({ "id": id }))
                .await?;
            let commit = preview
                .get("commit")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("preview response has no commit"))?;
            client.call("mcp.repositories.update.apply", json!({ "id": id, "expectedCommit": commit, "candidates": expected_candidates(&preview)? })).await.map_err(Into::into)
        },
        RepoCommand::Rollback { id } => {
            let list = client.call("mcp.repositories.list", json!({})).await?;
            let previous = list
                .get("repositories")
                .and_then(Value::as_array)
                .and_then(|repositories| {
                    repositories.iter().find(|entry| {
                        entry.pointer("/repository/id").and_then(Value::as_str) == Some(id.as_str())
                    })
                })
                .and_then(|entry| entry.get("previousCommit"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("repository has no previous commit"))?;
            client
                .call(
                    "mcp.repositories.rollback",
                    json!({ "id": id, "expectedCommit": previous }),
                )
                .await
                .map_err(Into::into)
        },
        RepoCommand::Remove { id } => client
            .call("mcp.repositories.remove", json!({ "id": id }))
            .await
            .map_err(Into::into),
        RepoCommand::Approve(args) => {
            let preview = client
                .call("mcp.repositories.update.preview", json!({ "id": args.id }))
                .await?;
            let commit = preview
                .get("commit")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("preview response has no commit"))?;
            let candidates = approval_candidates(&preview, args.all, &args.server)?;
            client.call("mcp.managed.approve", json!({ "id": args.id, "expectedCommit": commit, "selection": { "mode": if args.all { "all" } else { "selected" }, "candidates": candidates }, "enable": args.enable })).await.map_err(Into::into)
        },
    }
}

fn source_params(args: &RepoSourceArgs) -> Value {
    let source = if let Some(url) = &args.url {
        json!({ "kind": "https", "url": url, "private": args.private })
    } else if let Some(remote) = &args.ssh {
        json!({ "kind": "ssh", "remote": remote })
    } else {
        json!({ "kind": "local", "path": args.local })
    };
    json!({
        "id": args.id, "alias": args.alias, "source": source, "ref": args.requested_ref,
        "httpsCredentialId": args.https_credential_id, "sshTargetId": args.ssh_target_id,
    })
}

fn expected_candidates(preview: &Value) -> anyhow::Result<Vec<Value>> {
    preview.get("candidates").and_then(Value::as_array).ok_or_else(|| anyhow::anyhow!("preview response has no candidates"))?.iter().map(|candidate| {
        Ok(json!({
            "identity": candidate.get("identity").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("candidate has no identity"))?,
            "digest": candidate.get("digest").and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("candidate has no digest"))?,
        }))
    }).collect()
}

fn approval_candidates(
    preview: &Value,
    all: bool,
    selected_names: &[String],
) -> anyhow::Result<Vec<Value>> {
    let candidates = preview
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("preview response has no candidates"))?;
    let mut selected = candidates
        .iter()
        .filter(|candidate| {
            all || candidate
                .get("runtimeName")
                .and_then(Value::as_str)
                .is_some_and(|name| selected_names.iter().any(|selected| selected == name))
        })
        .map(|candidate| {
            Ok(json!({
                "identity": candidate
                    .get("identity")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("candidate has no identity"))?,
                "digest": candidate
                    .get("digest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("candidate has no digest"))?,
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if selected.is_empty() {
        anyhow::bail!("approval selection matched no managed servers");
    }
    selected.sort_by_key(Value::to_string);
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use {clap::Parser, serde_json::json};

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: super::McpCommand,
    }

    #[test]
    fn parses_repo_add_as_one_command() {
        assert!(
            TestCli::try_parse_from([
                "test",
                "repo",
                "add",
                "--alias",
                "demo",
                "--url",
                "https://example.com/repo.git",
                "--approve-all",
                "--enable"
            ])
            .is_ok()
        );
    }

    #[test]
    fn approval_candidates_use_update_preview_contract() {
        let preview = json!({
            "commit": "0123456789abcdef",
            "candidates": [
                {
                    "runtimeName": "tools__one",
                    "identity": "manifest:.mcp.json:one",
                    "digest": "digest-one"
                },
                {
                    "runtimeName": "tools__two",
                    "identity": "manifest:.mcp.json:two",
                    "digest": "digest-two"
                }
            ]
        });

        let candidates =
            super::approval_candidates(&preview, false, &["tools__two".to_string()]).unwrap();

        assert_eq!(candidates, vec![json!({
            "identity": "manifest:.mcp.json:two",
            "digest": "digest-two"
        })]);
    }
}
