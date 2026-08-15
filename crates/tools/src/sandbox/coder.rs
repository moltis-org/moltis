//! Coder Sandbox backend — remote workspaces via the Coder API.
//!
//! Each Moltis session gets an ephemeral Coder workspace. Lifecycle operations
//! use the Coder REST API; commands run through the workspace-agent
//! reconnecting-PTY WebSocket. Workspaces created by Moltis are deleted during
//! cleanup.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    async_trait::async_trait,
    base64::Engine,
    futures::{SinkExt, StreamExt},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tokio::sync::{RwLock, Semaphore},
    tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest},
    tracing::{debug, info},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::{
        file_system::SandboxReadResult,
        types::{Sandbox, SandboxConfig, SandboxId, sanitize_path_component},
    },
};

const DEFAULT_WORKSPACE_DIR: &str = "/home/coder";
const GENERIC_WORKSPACE: &str = "/home/sandbox";
const GENERIC_WORKSPACE_PREFIX: &str = "/home/sandbox/";
const DEFAULT_CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

struct CoderSession {
    workspace_id: String,
    workspace_name: String,
    agent_id: String,
    workspace_dir: String,
}

/// Coder sandbox backend configuration.
#[derive(Debug, Clone)]
pub struct CoderSandboxConfig {
    pub url: String,
    pub token: Secret<String>,
    pub organization: Option<String>,
    pub user: String,
    pub template_id: Option<String>,
    pub template_name: Option<String>,
    pub workspace_prefix: String,
    pub ttl_ms: Option<i64>,
    pub size: Option<String>,
    pub template_presets: HashMap<String, String>,
    pub parameter_values: HashMap<String, String>,
}

/// Coder sandbox backend.
pub struct CoderSandbox {
    #[allow(dead_code)]
    config: SandboxConfig,
    coder: CoderSandboxConfig,
    client: reqwest::Client,
    active: RwLock<HashMap<String, CoderSession>>,
    creation_permits: RwLock<HashMap<String, Arc<Semaphore>>>,
}

#[derive(Debug, Deserialize)]
struct CoderTemplate {
    id: String,
    name: Option<String>,
    active_version_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoderPreset {
    id: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct CreateWorkspaceRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_version_preset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl_ms: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rich_parameter_values: Vec<CoderParameterValue>,
}

#[derive(Debug, Serialize)]
struct CoderParameterValue {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct CoderWorkspace {
    id: String,
    name: String,
    latest_build: Option<CoderBuild>,
}

#[derive(Debug, Deserialize)]
struct CoderBuild {
    status: Option<String>,
    job: Option<CoderJob>,
    resources: Vec<CoderResource>,
}

#[derive(Debug, Deserialize)]
struct CoderJob {
    status: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoderResource {
    agents: Vec<CoderAgent>,
}

#[derive(Debug, Deserialize)]
struct CoderAgent {
    id: String,
    status: Option<String>,
    lifecycle_state: Option<String>,
    directory: Option<String>,
    expanded_directory: Option<String>,
}

struct ParsedPtyOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

impl CoderSandbox {
    pub fn new(config: SandboxConfig, coder: CoderSandboxConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();
        Self {
            config,
            coder,
            client,
            active: RwLock::new(HashMap::new()),
            creation_permits: RwLock::new(HashMap::new()),
        }
    }

    async fn creation_permit(&self, id: &SandboxId) -> Arc<Semaphore> {
        if let Some(permit) = self.creation_permits.read().await.get(&id.key).cloned() {
            return permit;
        }
        let mut permits = self.creation_permits.write().await;
        permits
            .entry(id.key.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    async fn existing_creation_permit(&self, id: &SandboxId) -> Option<Arc<Semaphore>> {
        self.creation_permits.read().await.get(&id.key).cloned()
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.coder.url);
        self.client
            .request(method, url)
            .header("Coder-Session-Token", self.coder.token.expose_secret())
    }

    fn translate_working_dir(working_dir: Option<&str>, workspace_dir: &str) -> String {
        match working_dir {
            Some(path) if path == GENERIC_WORKSPACE => workspace_dir.to_string(),
            Some(path) if path.starts_with(GENERIC_WORKSPACE_PREFIX) => {
                format!("{workspace_dir}{}", &path[GENERIC_WORKSPACE.len()..])
            },
            Some(path) => path.to_string(),
            None => workspace_dir.to_string(),
        }
    }

    fn workspace_name(&self, id: &SandboxId) -> String {
        workspace_name(&self.coder.workspace_prefix, &id.key)
    }

    async fn resolve_template(&self) -> Result<CoderTemplate> {
        if let Some(template_id) = self.coder.template_id.as_deref() {
            return self
                .get_json(&format!("/api/v2/templates/{template_id}"))
                .await;
        }

        let Some(template_name) = self.coder.template_name.as_deref() else {
            return Err(Error::message(
                "coder: configure coder_template_id or coder_template_name",
            ));
        };

        if let Some(org) = self.coder.organization.as_deref() {
            return self
                .get_json(&format!(
                    "/api/v2/organizations/{org}/templates/{template_name}"
                ))
                .await;
        }

        let templates: Vec<CoderTemplate> = self.get_json("/api/v2/templates").await?;
        templates
            .into_iter()
            .find(|template| template.name.as_deref() == Some(template_name))
            .ok_or_else(|| Error::message(format!("coder: template {template_name:?} not found")))
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let resp = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .map_err(|e| Error::message(format!("coder: request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: request failed (HTTP {status}): {text}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| Error::message(format!("coder: invalid response: {e}")))
    }

    async fn resolve_preset_id(&self, template: &CoderTemplate) -> Result<Option<String>> {
        let Some(size) = self
            .coder
            .size
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        let preset = self
            .coder
            .template_presets
            .get(size)
            .map(String::as_str)
            .unwrap_or(size)
            .trim();
        if looks_like_uuid(preset) {
            return Ok(Some(preset.to_string()));
        }
        let Some(active_version_id) = template.active_version_id.as_deref() else {
            return Err(Error::message(
                "coder: template response missing active_version_id for preset resolution",
            ));
        };
        let presets: Vec<CoderPreset> = self
            .get_json(&format!(
                "/api/v2/templateversions/{active_version_id}/presets"
            ))
            .await?;
        presets
            .into_iter()
            .find(|candidate| candidate.name == preset)
            .map(|candidate| Some(candidate.id))
            .ok_or_else(|| Error::message(format!("coder: template preset {preset:?} not found")))
    }

    async fn create_workspace(&self, id: &SandboxId) -> Result<CoderSession> {
        let template = self.resolve_template().await?;
        let preset_id = self.resolve_preset_id(&template).await?;
        let workspace_name = self.workspace_name(id);
        let body = CreateWorkspaceRequest {
            name: workspace_name.clone(),
            template_id: Some(template.id),
            template_version_preset_id: preset_id,
            ttl_ms: self.coder.ttl_ms,
            rich_parameter_values: self
                .coder
                .parameter_values
                .iter()
                .map(|(name, value)| CoderParameterValue {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        };
        let path = if let Some(org) = self.coder.organization.as_deref() {
            format!(
                "/api/v2/organizations/{org}/members/{}/workspaces",
                self.coder.user
            )
        } else {
            format!("/api/v2/users/{}/workspaces", self.coder.user)
        };
        let resp = self
            .request(reqwest::Method::POST, &path)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("coder: failed to create workspace: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: create workspace failed (HTTP {status}): {text}"
            )));
        }
        let workspace: CoderWorkspace = resp
            .json()
            .await
            .map_err(|e| Error::message(format!("coder: invalid create response: {e}")))?;
        self.wait_for_ready_workspace(workspace.id, workspace.name)
            .await
    }

    async fn wait_for_ready_workspace(
        &self,
        workspace_id: String,
        workspace_name: String,
    ) -> Result<CoderSession> {
        let start = tokio::time::Instant::now();
        loop {
            if start.elapsed() > DEFAULT_CREATE_TIMEOUT {
                return Err(Error::message(format!(
                    "coder: workspace {workspace_name} did not become ready within {}s",
                    DEFAULT_CREATE_TIMEOUT.as_secs()
                )));
            }

            let workspace: CoderWorkspace = self
                .get_json(&format!("/api/v2/workspaces/{workspace_id}"))
                .await?;
            if let Some(job) = workspace
                .latest_build
                .as_ref()
                .and_then(|build| build.job.as_ref())
                && matches!(job.status.as_deref(), Some("failed") | Some("canceled"))
            {
                return Err(Error::message(format!(
                    "coder: workspace build failed: {}",
                    job.error.as_deref().unwrap_or("no error detail")
                )));
            }
            if let Some(build) = workspace.latest_build.as_ref()
                && matches!(build.status.as_deref(), Some("failed") | Some("canceled"))
            {
                return Err(Error::message(format!(
                    "coder: workspace build ended with status {}",
                    build.status.as_deref().unwrap_or("unknown")
                )));
            }
            if let Some(agent) = ready_agent(&workspace) {
                return Ok(CoderSession {
                    workspace_id,
                    workspace_name,
                    agent_id: agent.id.clone(),
                    workspace_dir: agent
                        .expanded_directory
                        .clone()
                        .or_else(|| agent.directory.clone())
                        .filter(|dir| !dir.trim().is_empty())
                        .unwrap_or_else(|| DEFAULT_WORKSPACE_DIR.to_string()),
                });
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn run_pty_command(
        &self,
        session: &CoderSession,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let marker = uuid::Uuid::new_v4().simple().to_string();
        let cwd = Self::translate_working_dir(
            opts.working_dir.as_ref().and_then(|path| path.to_str()),
            &session.workspace_dir,
        );
        let wrapped = wrapped_command(command, &cwd, &opts.env, &marker);
        let mut ws_url = self
            .coder
            .url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        ws_url.push_str(&format!("/api/v2/workspaceagents/{}/pty", session.agent_id));
        let mut url = url::Url::parse(&ws_url)
            .map_err(|e| Error::message(format!("coder: invalid pty URL: {e}")))?;
        url.query_pairs_mut()
            .append_pair("reconnect", &uuid::Uuid::new_v4().to_string())
            .append_pair("width", "160")
            .append_pair("height", "48")
            .append_pair("backend_type", "buffered")
            .append_pair("command", &wrapped);
        let mut req = url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::message(format!("coder: invalid pty request: {e}")))?;
        req.headers_mut().insert(
            "Coder-Session-Token",
            self.coder
                .token
                .expose_secret()
                .parse()
                .map_err(|e| Error::message(format!("coder: invalid token header: {e}")))?,
        );
        let (mut ws, _) = connect_async(req)
            .await
            .map_err(|e| Error::message(format!("coder: pty websocket failed: {e}")))?;

        // No stdin is needed. This nudges the PTY size and verifies the input
        // side is writable without sending user data.
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"height": 48, "width": 160})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|e| Error::message(format!("coder: pty resize failed: {e}")))?;

        let mut combined = String::new();
        let read = async {
            while let Some(message) = ws.next().await {
                let message =
                    message.map_err(|e| Error::message(format!("coder: pty read failed: {e}")))?;
                match message {
                    tokio_tungstenite::tungstenite::Message::Text(text) => combined.push_str(&text),
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        combined.push_str(&String::from_utf8_lossy(&bytes));
                    },
                    tokio_tungstenite::tungstenite::Message::Close(_) => break,
                    _ => {},
                }
                if has_complete_markers(&combined, &marker) {
                    break;
                }
            }
            Ok::<String, Error>(combined)
        };

        let combined = tokio::time::timeout(opts.timeout, read)
            .await
            .map_err(|_| {
                Error::message(format!(
                    "coder: pty command timed out after {}s",
                    opts.timeout.as_secs()
                ))
            })??;
        let mut parsed = parse_pty_output(&combined, &marker)?;
        crate::sandbox::types::truncate_output_for_display(
            &mut parsed.stdout,
            opts.max_output_bytes,
        );
        crate::sandbox::types::truncate_output_for_display(
            &mut parsed.stderr,
            opts.max_output_bytes,
        );
        Ok(ExecResult {
            stdout: parsed.stdout,
            stderr: parsed.stderr,
            exit_code: parsed.exit_code,
        })
    }
}

#[async_trait]
impl Sandbox for CoderSandbox {
    fn backend_name(&self) -> &'static str {
        "coder"
    }

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        if self.active.read().await.contains_key(&id.key) {
            return Ok(());
        }
        let permit = self.creation_permit(id).await;
        let _guard = permit
            .acquire()
            .await
            .map_err(|_| Error::message("coder: creation semaphore closed"))?;
        if self.active.read().await.contains_key(&id.key) {
            return Ok(());
        }
        info!(session = %id.key, "creating coder workspace");
        let session = self.create_workspace(id).await?;
        info!(
            session = %id.key,
            workspace = session.workspace_name,
            agent = session.agent_id,
            "coder workspace ready"
        );
        self.active.write().await.insert(id.key.clone(), session);
        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        self.ensure_ready(id, None).await?;
        let active = self.active.read().await;
        let session = active
            .get(&id.key)
            .ok_or_else(|| Error::message("coder: missing active workspace after ensure_ready"))?;
        self.run_pty_command(session, command, opts).await
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if let Some(permit) = self.existing_creation_permit(id).await {
            let _guard = permit
                .acquire()
                .await
                .map_err(|_| Error::message("coder: creation semaphore closed"))?;
            self.cleanup_active(id).await
        } else {
            self.cleanup_active(id).await
        }
    }

    async fn read_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        crate::sandbox::file_system::command_read_file(self, id, file_path, max_bytes).await
    }

    fn workspace_dir(&self) -> &str {
        DEFAULT_WORKSPACE_DIR
    }

    async fn workspace_dir_for(&self, id: &SandboxId) -> String {
        self.active
            .read()
            .await
            .get(&id.key)
            .map(|session| session.workspace_dir.clone())
            .unwrap_or_else(|| DEFAULT_WORKSPACE_DIR.to_string())
    }

    fn is_isolated(&self) -> bool {
        true
    }

    fn provides_fs_isolation(&self) -> bool {
        true
    }
}

impl CoderSandbox {
    async fn cleanup_active(&self, id: &SandboxId) -> Result<()> {
        let Some(session) = self.active.write().await.remove(&id.key) else {
            return Ok(());
        };
        debug!(
            workspace = session.workspace_name,
            "deleting coder workspace"
        );
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v2/workspaces/{}/builds", session.workspace_id),
            )
            .json(&serde_json::json!({"transition": "delete"}))
            .send()
            .await
            .map_err(|e| Error::message(format!("coder: failed to delete workspace: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: delete workspace failed (HTTP {status}): {text}"
            )));
        }
        Ok(())
    }
}

fn ready_agent(workspace: &CoderWorkspace) -> Option<&CoderAgent> {
    workspace
        .latest_build
        .as_ref()?
        .resources
        .iter()
        .flat_map(|resource| resource.agents.iter())
        .find(|agent| {
            agent.status.as_deref() == Some("connected")
                && matches!(
                    agent.lifecycle_state.as_deref(),
                    Some("ready") | Some("created") | Some("starting") | None
                )
        })
}

fn looks_like_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

pub(crate) fn workspace_name(prefix: &str, key: &str) -> String {
    let prefix = sanitize_path_component(&prefix.to_ascii_lowercase());
    let key = sanitize_path_component(&key.to_ascii_lowercase());
    let mut name = format!("{prefix}-{key}");
    if name.len() > 32 {
        name.truncate(32);
        name = name.trim_end_matches(['-', '.', '_']).to_string();
    }
    if name.is_empty() {
        "moltis".to_string()
    } else {
        name
    }
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn wrapped_command(command: &str, cwd: &str, env: &[(String, String)], marker: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(command.as_bytes());
    let encoded = shell_words::quote(&encoded);
    let cwd = shell_words::quote(cwd);
    let env_prefix = env
        .iter()
        .filter(|(key, _)| valid_env_key(key))
        .map(|(key, value)| format!("{}={}", key, shell_words::quote(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let env_prefix = if env_prefix.is_empty() {
        String::new()
    } else {
        format!("env {env_prefix} ")
    };
    let exit_marker = format!("__MOLTIS_EXIT_{marker}__");
    let stderr_begin = format!("__MOLTIS_STDERR_{marker}_BEGIN__");
    let stderr_end = format!("__MOLTIS_STDERR_{marker}_END__");

    format!(
        "tmp=$(mktemp /tmp/moltis-cmd.XXXXXX) || exit 125; \
         err=$(mktemp /tmp/moltis-stderr.XXXXXX) || {{ rm -f \"$tmp\"; exit 125; }}; \
         printf %s {encoded} | base64 -d > \"$tmp\"; decode_status=$?; \
         if [ \"$decode_status\" -ne 0 ]; then rm -f \"$tmp\" \"$err\"; exit \"$decode_status\"; fi; \
         (cd {cwd} && {env_prefix}sh \"$tmp\") 2>\"$err\"; status=$?; \
         printf '\n{exit_marker}%s\n' \"$status\"; \
         printf '{stderr_begin}\n'; cat \"$err\" 2>/dev/null; printf '\n{stderr_end}\n'; \
         rm -f \"$tmp\" \"$err\"; exit \"$status\""
    )
}

fn has_complete_markers(output: &str, marker: &str) -> bool {
    output.contains(&format!("__MOLTIS_EXIT_{marker}__"))
        && output.contains(&format!("__MOLTIS_STDERR_{marker}_END__"))
}

fn strip_ansi(input: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap_or_else(|_| unreachable!());
    re.replace_all(input, "").into_owned()
}

fn parse_pty_output(output: &str, marker: &str) -> Result<ParsedPtyOutput> {
    let normalized = strip_ansi(output).replace('\r', "");
    let exit_marker = format!("__MOLTIS_EXIT_{marker}__");
    let stderr_begin = format!("__MOLTIS_STDERR_{marker}_BEGIN__");
    let stderr_end = format!("__MOLTIS_STDERR_{marker}_END__");

    let exit_pos = normalized
        .find(&exit_marker)
        .ok_or_else(|| Error::message("coder: pty output missing exit marker"))?;
    let after_exit = &normalized[exit_pos + exit_marker.len()..];
    let exit_line = after_exit.lines().next().unwrap_or_default().trim();
    let exit_code = exit_line
        .parse::<i32>()
        .map_err(|e| Error::message(format!("coder: invalid pty exit code {exit_line:?}: {e}")))?;
    let stderr_start = normalized
        .find(&stderr_begin)
        .ok_or_else(|| Error::message("coder: pty output missing stderr begin marker"))?
        + stderr_begin.len();
    let stderr_end_pos = normalized
        .find(&stderr_end)
        .ok_or_else(|| Error::message("coder: pty output missing stderr end marker"))?;
    let stdout = normalized[..exit_pos].trim_end_matches('\n').to_string();
    let stderr = normalized[stderr_start..stderr_end_pos]
        .trim_matches('\n')
        .to_string();
    Ok(ParsedPtyOutput {
        stdout,
        stderr,
        exit_code,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn workspace_name_is_sanitized_and_limited() {
        let name = workspace_name("Moltis!", "Session/With/Long/Unsafe/Characters/AndMore");
        assert!(name.starts_with("moltis-"));
        assert!(name.len() <= 32);
        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        );
    }

    #[test]
    fn parses_pty_markers() {
        let marker = "abc";
        let output = "hello\r\n__MOLTIS_EXIT_abc__7\r\n__MOLTIS_STDERR_abc_BEGIN__\r\noops\r\n__MOLTIS_STDERR_abc_END__\r\n";
        let parsed = parse_pty_output(output, marker).unwrap();
        assert_eq!(parsed.stdout, "hello");
        assert_eq!(parsed.stderr, "oops");
        assert_eq!(parsed.exit_code, 7);
    }

    #[test]
    fn wrapped_command_includes_cwd_env_and_markers() {
        let cmd = wrapped_command(
            "printf hi",
            "/home/coder/project",
            &[
                ("FOO".into(), "bar baz".into()),
                ("BAD-KEY".into(), "x".into()),
            ],
            "abc",
        );
        assert!(cmd.contains("cd /home/coder/project"));
        assert!(cmd.contains("FOO='bar baz'") || cmd.contains("FOO=bar\\ baz"));
        assert!(!cmd.contains("BAD-KEY"));
        assert!(cmd.contains("__MOLTIS_EXIT_abc__"));
    }
}
