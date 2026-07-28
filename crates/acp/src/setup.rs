use std::{collections::HashSet, fmt, path::PathBuf};

use agent_client_protocol as acp;

const MAX_MCP_SERVERS: usize = 16;
const MAX_MCP_ARGS: usize = 256;
const MAX_MCP_ENV_VARS: usize = 256;
const MAX_MCP_SETUP_BYTES: usize = 1024 * 1024;

/// Validated setup supplied by an ACP client for a new or loaded session.
#[derive(Clone)]
pub struct SessionSetup {
    cwd: PathBuf,
    mcp_servers: Vec<acp::McpServer>,
}

impl SessionSetup {
    pub async fn new(cwd: PathBuf, mcp_servers: Vec<acp::McpServer>) -> acp::Result<Self> {
        if !cwd.is_absolute() {
            return Err(acp::Error::invalid_params().data("session cwd must be an absolute path"));
        }
        let cwd = tokio::fs::canonicalize(&cwd).await.map_err(|error| {
            acp::Error::invalid_params().data(format!("invalid session cwd: {error}"))
        })?;
        if !cwd.is_dir() {
            return Err(acp::Error::invalid_params().data("session cwd must be a directory"));
        }

        if mcp_servers.len() > MAX_MCP_SERVERS {
            return Err(acp::Error::invalid_params()
                .data(format!("at most {MAX_MCP_SERVERS} MCP servers are allowed")));
        }

        let mut names = HashSet::new();
        let mut setup_bytes = 0usize;
        for server in &mcp_servers {
            let acp::McpServer::Stdio(server) = server else {
                return Err(
                    acp::Error::invalid_params().data("only stdio MCP servers are supported")
                );
            };
            let name = server.name.trim();
            if name.is_empty() {
                return Err(acp::Error::invalid_params().data("MCP server name cannot be empty"));
            }
            if !names.insert(name.to_string()) {
                return Err(
                    acp::Error::invalid_params().data(format!("duplicate MCP server name {name}"))
                );
            }
            if !server.command.is_absolute() {
                return Err(acp::Error::invalid_params().data(format!(
                    "MCP server {name} command must be an absolute path"
                )));
            }
            if server.args.len() > MAX_MCP_ARGS {
                return Err(acp::Error::invalid_params().data(format!(
                    "MCP server {name} has more than {MAX_MCP_ARGS} arguments"
                )));
            }
            if server.env.len() > MAX_MCP_ENV_VARS {
                return Err(acp::Error::invalid_params().data(format!(
                    "MCP server {name} has more than {MAX_MCP_ENV_VARS} environment variables"
                )));
            }
            setup_bytes = setup_bytes
                .saturating_add(name.len())
                .saturating_add(server.command.as_os_str().len())
                .saturating_add(server.args.iter().map(String::len).sum::<usize>());
            let mut environment = HashSet::new();
            for variable in &server.env {
                if variable.name.is_empty()
                    || variable.name.contains(['=', '\0'])
                    || !environment.insert(variable.name.as_str())
                {
                    return Err(acp::Error::invalid_params().data(format!(
                        "MCP server {name} has an invalid or duplicate environment variable"
                    )));
                }
                setup_bytes = setup_bytes
                    .saturating_add(variable.name.len())
                    .saturating_add(variable.value.len());
            }
            if setup_bytes > MAX_MCP_SETUP_BYTES {
                return Err(acp::Error::invalid_params()
                    .data(format!("MCP setup exceeds {MAX_MCP_SETUP_BYTES} bytes")));
            }
        }

        Ok(Self { cwd, mcp_servers })
    }

    #[must_use]
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    #[must_use]
    pub fn mcp_servers(&self) -> &[acp::McpServer] {
        &self.mcp_servers
    }
}

impl fmt::Debug for SessionSetup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSetup")
            .field("cwd", &self.cwd)
            .field("mcp_server_count", &self.mcp_servers.len())
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_relative_paths() {
        assert!(
            SessionSetup::new(PathBuf::from("relative"), Vec::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_missing_paths() {
        assert!(
            SessionSetup::new(PathBuf::from("/path/that/does/not/exist"), Vec::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_relative_mcp_commands() {
        let server = acp::McpServerStdio::new("test", "relative-command");
        assert!(
            SessionSetup::new(std::env::temp_dir(), vec![acp::McpServer::Stdio(server)])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_duplicate_mcp_server_names() {
        let first = acp::McpServerStdio::new("test", "/bin/first");
        let second = acp::McpServerStdio::new("test", "/bin/second");
        assert!(
            SessionSetup::new(std::env::temp_dir(), vec![
                acp::McpServer::Stdio(first),
                acp::McpServer::Stdio(second)
            ],)
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_duplicate_mcp_environment_names() {
        let server = acp::McpServerStdio::new("test", "/bin/test").env(vec![
            acp::EnvVariable::new("TOKEN", "first"),
            acp::EnvVariable::new("TOKEN", "second"),
        ]);
        assert!(
            SessionSetup::new(std::env::temp_dir(), vec![acp::McpServer::Stdio(server)])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn debug_does_not_expose_mcp_environment() {
        let server = acp::McpServerStdio::new("test", "/bin/test")
            .env(vec![acp::EnvVariable::new("TOKEN", "secret")]);
        let setup = SessionSetup::new(std::env::temp_dir(), vec![acp::McpServer::Stdio(server)])
            .await
            .expect("valid setup");
        let debug = format!("{setup:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("TOKEN"));
    }

    #[tokio::test]
    async fn rejects_too_many_mcp_servers() {
        let servers = (0..=MAX_MCP_SERVERS)
            .map(|index| {
                acp::McpServer::Stdio(acp::McpServerStdio::new(
                    format!("server-{index}"),
                    "/bin/test",
                ))
            })
            .collect();
        assert!(
            SessionSetup::new(std::env::temp_dir(), servers)
                .await
                .is_err()
        );
    }
}
