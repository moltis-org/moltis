//! Coder Sandbox backend — remote workspaces via the Coder API.
//!
//! Each Moltis session gets an ephemeral Coder workspace. Lifecycle operations
//! use the Coder REST API; commands run through the workspace-agent
//! reconnecting-PTY WebSocket. Workspaces created by Moltis are deleted during
//! cleanup.
//!
//! # Command transport
//!
//! Coder exposes no REST endpoint for a workspace filesystem, so both commands
//! and file payloads travel over the agent PTY. The payload is **not** placed
//! in the PTY URL's `command` query parameter: URLs are bounded at a few
//! kilobytes by proxies, which would cap a `Write` far below
//! [`MAX_SANDBOX_WRITE_BYTES`](crate::sandbox::file_system::MAX_SANDBOX_WRITE_BYTES)
//! and break workspace sync outright.
//!
//! Instead the URL carries a fixed ~200 byte bootstrap that puts the terminal
//! into raw mode, announces a ready marker, and then decodes a base64 stream
//! from stdin into a temporary script which it runs. The real script is written
//! to the PTY stdin channel as `{"data": …}` frames, so payload size is bounded
//! by memory rather than by URL length. Raw mode also disables echo and output
//! post-processing, which keeps the marker framing parseable.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
    time::Duration,
};

use {
    async_trait::async_trait,
    futures::{SinkExt, StreamExt},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tokio::sync::Semaphore,
    tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest},
    tracing::{debug, info},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::{
        file_system::{SandboxReadResult, command_write_file_with_limit},
        types::{Sandbox, SandboxConfig, SandboxId},
    },
};

mod transport;

use transport::{
    PTY_COLS, PTY_ROWS, ParsedPtyOutput, PtyMarkers, PtyOutputParser, STDIN_CHUNK_BYTES,
    bootstrap_command, ctrl_c_message, encoded_script, eof_stdin, framed_stdin_chunk,
    resize_message, stdin_message, workspace_digest, wrapped_command,
};

#[cfg(test)]
mod tests;

const DEFAULT_WORKSPACE_DIR: &str = "/home/coder";
const GENERIC_WORKSPACE: &str = "/home/sandbox";
const GENERIC_WORKSPACE_PREFIX: &str = "/home/sandbox/";
const DEFAULT_CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_DELETE_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(1);

/// Coder rejects workspace names longer than this.
const MAX_WORKSPACE_NAME_LEN: usize = 32;

/// How long to wait for the bootstrap's ready marker before streaming anyway.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for a single `write_file` through the PTY stdin stream.
///
/// Far above [`MAX_SANDBOX_WRITE_BYTES`](crate::sandbox::file_system::MAX_SANDBOX_WRITE_BYTES)
/// because the transport is not bounded by `ARG_MAX`, but still bounded: the
/// payload is base64-encoded and buffered in memory on both sides.
const CODER_MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;

/// Upper bound for a workspace-sync transfer.
///
/// Sync-out reads the tarball back as base64 on stdout, which inflates it by
/// 4/3 and must still fit the file service's output budget — so this ceiling
/// is set by the *read* path, not the write path, and sits well below the
/// shared default.
const CODER_MAX_TRANSFER_BYTES: u64 = 16 * 1024 * 1024;

// The whole point of the streaming transport is that this backend is not bound
// by the command-line write limit.
const _: () = assert!(CODER_MAX_WRITE_BYTES > crate::sandbox::file_system::MAX_SANDBOX_WRITE_BYTES);
const _: () = assert!(CODER_MAX_TRANSFER_BYTES < crate::sandbox::types::DEFAULT_MAX_TRANSFER_BYTES);
// A transfer must survive base64 expansion on the way back out, or sync-out
// truncates the tarball and the decode fails instead of reporting "too large".
const _: () = assert!(
    CODER_MAX_TRANSFER_BYTES.div_ceil(3) * 4
        < crate::sandbox::file_system::DEFAULT_SANDBOX_OUTPUT_BYTES as u64
);

/// The agent PTY WebSocket, split into a reader and writer by [`CoderSandbox::run_pty_script`].
type PtyStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone)]
struct CoderSession {
    workspace_id: String,
    workspace_name: String,
    agent_id: String,
    workspace_dir: String,
}

#[derive(Clone)]
enum TrackedWorkspace {
    Pending {
        workspace_name: String,
    },
    Provisional {
        workspace_id: String,
        workspace_name: String,
    },
    Ready(CoderSession),
}

impl TrackedWorkspace {
    fn workspace_id(&self) -> Option<&str> {
        match self {
            Self::Pending { .. } => None,
            Self::Provisional { workspace_id, .. } => Some(workspace_id),
            Self::Ready(session) => Some(&session.workspace_id),
        }
    }

    fn workspace_name(&self) -> &str {
        match self {
            Self::Pending { workspace_name } => workspace_name,
            Self::Provisional { workspace_name, .. } => workspace_name,
            Self::Ready(session) => &session.workspace_name,
        }
    }
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
    client: Option<reqwest::Client>,
    client_error: Option<String>,
    base_url: Option<url::Url>,
    url_error: Option<String>,
    active: RwLock<HashMap<String, TrackedWorkspace>>,
    creation_permits: tokio::sync::RwLock<HashMap<String, Arc<Semaphore>>>,
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
    status: Option<BuildStatus>,
    job: Option<CoderJob>,
    #[serde(default)]
    resources: Vec<CoderResource>,
}

#[derive(Debug, Deserialize)]
struct CoderJob {
    status: Option<JobStatus>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoderResource {
    agents: Vec<CoderAgent>,
}

#[derive(Debug, Deserialize)]
struct CoderAgent {
    id: String,
    status: Option<AgentStatus>,
    lifecycle_state: Option<AgentLifecycle>,
    directory: Option<String>,
    expanded_directory: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BuildStatus {
    Running,
    Stopped,
    Starting,
    Stopping,
    Deleting,
    Canceling,
    Pending,
    Failed,
    Canceled,
    Deleted,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Canceling,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentStatus {
    Connected,
    Connecting,
    Disconnected,
    Timeout,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentLifecycle {
    Created,
    Starting,
    Ready,
    StartTimeout,
    StartError,
    ShuttingDown,
    ShutdownTimeout,
    ShutdownError,
    Off,
    #[serde(other)]
    Unknown,
}

enum WorkspaceLookup {
    Found(CoderWorkspace),
    NotFound,
}

impl CoderSandbox {
    pub fn new(config: SandboxConfig, coder: CoderSandboxConfig) -> Self {
        let (client, client_error) = match reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(client) => (Some(client), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let (base_url, url_error) = match validated_coder_url(&coder.url) {
            Ok(url) => (Some(url), None),
            Err(error) => (None, Some(error.to_string())),
        };
        Self {
            config,
            coder,
            client,
            client_error,
            base_url,
            url_error,
            active: RwLock::new(HashMap::new()),
            creation_permits: tokio::sync::RwLock::new(HashMap::new()),
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

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let client = self.client.as_ref().ok_or_else(|| {
            Error::message(format!(
                "coder: failed to initialize HTTP client: {}",
                self.client_error.as_deref().unwrap_or("unknown error")
            ))
        })?;
        let url = self.api_url(path)?;
        Ok(client
            .request(method, url)
            .header("Coder-Session-Token", self.coder.token.expose_secret()))
    }

    fn api_url(&self, path: &str) -> Result<url::Url> {
        let mut url = self.base_url.clone().ok_or_else(|| {
            Error::message(format!(
                "coder: invalid URL: {}",
                self.url_error.as_deref().unwrap_or("unknown error")
            ))
        })?;
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| Error::message("coder: base URL cannot contain API paths"))?;
        segments.pop_if_empty();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(segment);
        }
        drop(segments);
        Ok(url)
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
            .request(reqwest::Method::GET, path)?
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

    async fn get_workspace_by_name(&self, workspace_name: &str) -> Result<WorkspaceLookup> {
        self.get_workspace_at(&format!(
            "/api/v2/users/{}/workspace/{workspace_name}",
            self.coder.user
        ))
        .await
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
        let workspace_name = self.workspace_name(id);
        self.track_pending(&id.key, &workspace_name);
        if let WorkspaceLookup::Found(workspace) =
            self.get_workspace_by_name(&workspace_name).await?
        {
            return self
                .finish_workspace_startup(id, workspace, &workspace_name)
                .await;
        }

        let template = self.resolve_template().await?;
        let preset_id = self.resolve_preset_id(&template).await?;
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
            .request(reqwest::Method::POST, &path)?
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
        self.finish_workspace_startup(id, workspace, &workspace_name)
            .await
    }

    async fn finish_workspace_startup(
        &self,
        id: &SandboxId,
        workspace: CoderWorkspace,
        deterministic_name: &str,
    ) -> Result<CoderSession> {
        let workspace_id = workspace.id;
        let workspace_name = workspace.name;
        self.track_provisional_if_pending(
            &id.key,
            &workspace_id,
            deterministic_name,
            &workspace_name,
        );

        match self
            .make_workspace_ready(&workspace_id, &workspace_name)
            .await
        {
            Ok(Some(session)) => {
                self.track_ready_if_current(&id.key, session.clone());
                Ok(session)
            },
            Ok(None) => {
                self.remove_if_current(&id.key, &workspace_id);
                Err(Error::message(format!(
                    "coder: workspace {workspace_name} disappeared during startup"
                )))
            },
            Err(startup_error) => {
                match self.delete_workspace(&workspace_id).await {
                    Ok(()) => self.remove_if_current(&id.key, &workspace_id),
                    Err(cleanup_error) => {
                        return Err(Error::message(format!(
                            "{startup_error}; cleanup also failed: {cleanup_error}"
                        )));
                    },
                }
                Err(startup_error)
            },
        }
    }

    async fn make_workspace_ready(
        &self,
        workspace_id: &str,
        workspace_name: &str,
    ) -> Result<Option<CoderSession>> {
        let workspace = match self.get_workspace(workspace_id).await? {
            WorkspaceLookup::Found(workspace) => workspace,
            WorkspaceLookup::NotFound => return Ok(None),
        };
        if workspace_is_stopped(&workspace) && !self.start_workspace(workspace_id).await? {
            return Ok(None);
        }
        if let Some(session) = workspace_session(workspace_id, workspace_name, &workspace)? {
            return Ok(Some(session));
        }
        self.wait_for_ready_workspace(workspace_id, workspace_name)
            .await
    }

    async fn wait_for_ready_workspace(
        &self,
        workspace_id: &str,
        workspace_name: &str,
    ) -> Result<Option<CoderSession>> {
        let start = tokio::time::Instant::now();
        loop {
            if start.elapsed() > DEFAULT_CREATE_TIMEOUT {
                return Err(Error::message(format!(
                    "coder: workspace {workspace_name} did not become ready within {}s",
                    DEFAULT_CREATE_TIMEOUT.as_secs()
                )));
            }

            let workspace = match self.get_workspace(workspace_id).await? {
                WorkspaceLookup::Found(workspace) => workspace,
                WorkspaceLookup::NotFound => return Ok(None),
            };
            if let Some(session) = workspace_session(workspace_id, workspace_name, &workspace)? {
                return Ok(Some(session));
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn get_workspace(&self, workspace_id: &str) -> Result<WorkspaceLookup> {
        self.get_workspace_at(&format!("/api/v2/workspaces/{workspace_id}"))
            .await
    }

    async fn get_workspace_at(&self, path: &str) -> Result<WorkspaceLookup> {
        let resp = self
            .request(reqwest::Method::GET, path)?
            .send()
            .await
            .map_err(|error| Error::message(format!("coder: request failed: {error}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(WorkspaceLookup::NotFound);
        }
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: request failed (HTTP {status}): {text}"
            )));
        }
        resp.json()
            .await
            .map(WorkspaceLookup::Found)
            .map_err(|error| Error::message(format!("coder: invalid response: {error}")))
    }

    async fn start_workspace(&self, workspace_id: &str) -> Result<bool> {
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v2/workspaces/{workspace_id}/builds"),
            )?
            .json(&serde_json::json!({"transition": "start"}))
            .send()
            .await
            .map_err(|error| {
                Error::message(format!("coder: failed to start workspace: {error}"))
            })?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: start workspace failed (HTTP {status}): {text}"
            )));
        }
        Ok(true)
    }

    /// Open the agent PTY with the fixed bootstrap script.
    ///
    /// Only the bootstrap travels in the URL; it is a constant ~200 bytes, so
    /// no command or file payload can ever push the request over a proxy's URL
    /// limit.
    async fn open_pty(&self, session: &CoderSession, markers: &PtyMarkers) -> Result<PtyStream> {
        let mut url = self.api_url(&format!("/api/v2/workspaceagents/{}/pty", session.agent_id))?;
        let websocket_scheme = match url.scheme() {
            "https" => "wss",
            "http" => "ws",
            scheme => {
                return Err(Error::message(format!(
                    "coder: unsupported WebSocket base scheme {scheme:?}"
                )));
            },
        };
        url.set_scheme(websocket_scheme)
            .map_err(|()| Error::message("coder: failed to derive pty WebSocket URL"))?;
        url.query_pairs_mut()
            .append_pair("reconnect", &uuid::Uuid::new_v4().to_string())
            .append_pair("width", &PTY_COLS.to_string())
            .append_pair("height", &PTY_ROWS.to_string())
            .append_pair("backend_type", "buffered")
            .append_pair("command", &bootstrap_command(markers));

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
        let (ws, _) = connect_async(req)
            .await
            .map_err(|e| Error::message(format!("coder: pty websocket failed: {e}")))?;
        Ok(ws)
    }

    /// Stream `script` into the workspace over the PTY stdin channel and
    /// collect everything the workspace writes back.
    ///
    /// Reading and writing are driven concurrently: the workspace can block on
    /// writing output while we are still streaming input, so a write-then-read
    /// sequence would deadlock on any script that produces output as it runs.
    async fn run_pty_script(
        &self,
        session: &CoderSession,
        script: &str,
        markers: &PtyMarkers,
        remote_timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<ParsedPtyOutput> {
        let deadline = tokio::time::Instant::now() + remote_timeout;
        let Some(ws) = complete_before(deadline, self.open_pty(session, markers)).await else {
            return Err(command_timeout_error(remote_timeout));
        };
        let ws = ws?;
        let (mut sink, mut stream) = ws.split();
        let payload = encoded_script(script);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let reader_markers = markers.clone();
        let reader = async move {
            let mut ready_tx = Some(ready_tx);
            let mut parser = PtyOutputParser::new(reader_markers, max_output_bytes);
            while let Some(message) = stream.next().await {
                let message =
                    message.map_err(|e| Error::message(format!("coder: pty read failed: {e}")))?;
                if matches!(message, tokio_tungstenite::tungstenite::Message::Close(_)) {
                    break;
                }
                let became_ready = parser.feed_message(&message)?;
                if became_ready && let Some(tx) = ready_tx.take() {
                    let _ = tx.send(());
                }
                if parser.is_done() {
                    break;
                }
            }
            parser.finish()
        };

        let operation_result = {
            let writer = async {
                sink.send(resize_message()?)
                    .await
                    .map_err(|e| Error::message(format!("coder: pty resize failed: {e}")))?;

                let ready_wait = tokio::time::timeout(READY_TIMEOUT, ready_rx).await;
                if !matches!(ready_wait, Ok(Ok(()))) {
                    debug!("coder: pty ready marker not seen, streaming payload anyway");
                }

                for chunk in payload.as_bytes().chunks(STDIN_CHUNK_BYTES) {
                    let chunk = std::str::from_utf8(chunk).map_err(|error| {
                        Error::message(format!("coder: invalid encoded script chunk: {error}"))
                    })?;
                    let framed = framed_stdin_chunk(chunk);
                    sink.send(stdin_message(&framed)?)
                        .await
                        .map_err(|e| Error::message(format!("coder: pty write failed: {e}")))?;
                }
                let eof = eof_stdin(markers);
                sink.send(stdin_message(&eof)?)
                    .await
                    .map_err(|e| Error::message(format!("coder: pty write failed: {e}")))?;
                sink.flush()
                    .await
                    .map_err(|e| Error::message(format!("coder: pty flush failed: {e}")))
            };
            let operation = async {
                let (output, ()) = futures::future::try_join(reader, writer).await?;
                Ok::<ParsedPtyOutput, Error>(output)
            };
            complete_before(deadline, operation).await
        };

        if let Some(result) = operation_result {
            return result;
        }

        let _ = tokio::time::timeout(CANCEL_TIMEOUT, async {
            if let Ok(message) = ctrl_c_message() {
                let _ = sink.send(message).await;
                let _ = sink.flush().await;
            }
            let _ = sink.close().await;
        })
        .await;
        Err(command_timeout_error(remote_timeout))
    }

    async fn run_pty_command(
        &self,
        session: &CoderSession,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let markers = PtyMarkers::new();
        let cwd = Self::translate_working_dir(
            opts.working_dir.as_ref().and_then(|path| path.to_str()),
            &session.workspace_dir,
        );
        let script = wrapped_command(command, &cwd, &opts.env, &markers, opts.timeout);
        let parsed = self
            .run_pty_script(
                session,
                &script,
                &markers,
                opts.timeout,
                opts.max_output_bytes,
            )
            .await?;
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
        let permit = self.creation_permit(id).await;
        let _guard = permit
            .acquire()
            .await
            .map_err(|_| Error::message("coder: creation semaphore closed"))?;
        let tracked = self
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .cloned();
        if let Some(tracked) = tracked
            && let Some(workspace_id) = tracked.workspace_id().map(str::to_string)
        {
            let workspace_name = tracked.workspace_name().to_string();
            match self
                .make_workspace_ready(&workspace_id, &workspace_name)
                .await?
            {
                Some(session) => {
                    self.track_ready_if_current(&id.key, session);
                    return Ok(());
                },
                None => self.remove_if_current(&id.key, &workspace_id),
            }
        }
        info!(session = %id.key, "creating coder workspace");
        let session = self.create_workspace(id).await?;
        info!(
            session = %id.key,
            workspace = session.workspace_name,
            agent = session.agent_id,
            "coder workspace ready"
        );
        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        self.ensure_ready(id, None).await?;
        let session = self
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(|tracked| match tracked {
                TrackedWorkspace::Ready(session) => Some(session.clone()),
                TrackedWorkspace::Pending { .. } | TrackedWorkspace::Provisional { .. } => None,
            })
            .ok_or_else(|| Error::message("coder: missing ready workspace after ensure_ready"))?;
        self.run_pty_command(&session, command, opts).await
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if let Some(permit) = self.existing_creation_permit(id).await {
            let guard = permit
                .acquire()
                .await
                .map_err(|_| Error::message("coder: creation semaphore closed"))?;
            let result = self.cleanup_active(id).await;
            if result.is_ok() {
                self.retire_creation_permit(id, &permit, guard).await;
            }
            result
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

    /// Write through the PTY stdin stream rather than a command line.
    ///
    /// The shared helper caps writes at `MAX_SANDBOX_WRITE_BYTES` because most
    /// backends embed the base64 payload in an `exec` argument list. This
    /// backend streams the script instead, so it raises the cap to
    /// [`CODER_MAX_WRITE_BYTES`] — without that, workspace sync-in fails for
    /// any workspace larger than 512 KB and aborts the command that triggered
    /// it.
    ///
    /// Content is base64-encoded twice on the way in: once to embed it in the
    /// script, and once to frame the script for the stdin stream. That is a
    /// ~1.78x wire amplification, kept for now because collapsing it would
    /// mean duplicating this helper's symlink and parent-directory handling
    /// here.
    async fn write_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        command_write_file_with_limit(self, id, file_path, content, CODER_MAX_WRITE_BYTES).await
    }

    fn max_transfer_bytes(&self) -> u64 {
        CODER_MAX_TRANSFER_BYTES
    }

    fn workspace_dir(&self) -> &str {
        DEFAULT_WORKSPACE_DIR
    }

    async fn workspace_dir_for(&self, id: &SandboxId) -> String {
        self.active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .and_then(|tracked| match tracked {
                TrackedWorkspace::Ready(session) => Some(session.workspace_dir.clone()),
                TrackedWorkspace::Pending { .. } | TrackedWorkspace::Provisional { .. } => None,
            })
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
        let tracked = self
            .active
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id.key)
            .cloned();
        let Some(tracked) = tracked else {
            return Ok(());
        };
        debug!(
            workspace = tracked.workspace_name(),
            "deleting coder workspace"
        );
        let workspace_id = if let Some(workspace_id) = tracked.workspace_id() {
            workspace_id.to_string()
        } else {
            let workspace_name = tracked.workspace_name().to_string();
            let workspace = match self.get_workspace_by_name(&workspace_name).await? {
                WorkspaceLookup::Found(workspace) => workspace,
                WorkspaceLookup::NotFound => {
                    self.remove_if_pending(&id.key, &workspace_name);
                    return Ok(());
                },
            };
            self.track_provisional_if_pending(
                &id.key,
                &workspace.id,
                &workspace_name,
                &workspace.name,
            );
            workspace.id
        };
        self.delete_workspace(&workspace_id).await?;
        self.remove_if_current(&id.key, &workspace_id);
        Ok(())
    }

    async fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v2/workspaces/{workspace_id}/builds"),
            )?
            .json(&serde_json::json!({"transition": "delete"}))
            .send()
            .await
            .map_err(|e| Error::message(format!("coder: failed to delete workspace: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "coder: delete workspace failed (HTTP {status}): {text}"
            )));
        }
        let build: CoderBuild = resp
            .json()
            .await
            .map_err(|error| Error::message(format!("coder: invalid delete response: {error}")))?;
        ensure_delete_build_succeeded(&build)?;
        if build.status == Some(BuildStatus::Deleted) {
            return Ok(());
        }
        self.wait_for_deleted_workspace(workspace_id, DEFAULT_DELETE_TIMEOUT, POLL_INTERVAL)
            .await
    }

    async fn wait_for_deleted_workspace(
        &self,
        workspace_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let Some(workspace) = complete_before(deadline, self.get_workspace(workspace_id)).await
            else {
                return Err(delete_timeout_error(workspace_id, timeout));
            };
            match workspace? {
                WorkspaceLookup::NotFound => return Ok(()),
                WorkspaceLookup::Found(workspace) => {
                    if let Some(build) = workspace.latest_build.as_ref() {
                        ensure_delete_build_succeeded(build)?;
                        if build.status == Some(BuildStatus::Deleted) {
                            return Ok(());
                        }
                    }
                },
            }
            if complete_before(deadline, tokio::time::sleep(poll_interval))
                .await
                .is_none()
            {
                return Err(delete_timeout_error(workspace_id, timeout));
            }
        }
    }

    async fn retire_creation_permit<'a>(
        &self,
        id: &SandboxId,
        permit: &'a Arc<Semaphore>,
        guard: tokio::sync::SemaphorePermit<'a>,
    ) {
        let mut permits = self.creation_permits.write().await;
        let can_remove = permits
            .get(&id.key)
            .is_some_and(|current| Arc::ptr_eq(current, permit))
            && Arc::strong_count(permit) == 2;
        if can_remove {
            permits.remove(&id.key);
        }
        // Releasing under the map lock prevents a replacement semaphore from
        // being observed while this cleanup still owns the old one.
        drop(guard);
    }

    fn track_pending(&self, key: &str, workspace_name: &str) {
        self.active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.to_string(), TrackedWorkspace::Pending {
                workspace_name: workspace_name.to_string(),
            });
    }

    fn track_provisional_if_pending(
        &self,
        key: &str,
        workspace_id: &str,
        pending_name: &str,
        workspace_name: &str,
    ) {
        let mut active = self
            .active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            active.get(key),
            Some(TrackedWorkspace::Pending { workspace_name }) if workspace_name == pending_name
        ) {
            active.insert(key.to_string(), TrackedWorkspace::Provisional {
                workspace_id: workspace_id.to_string(),
                workspace_name: workspace_name.to_string(),
            });
        }
    }

    fn track_ready_if_current(&self, key: &str, session: CoderSession) {
        let mut active = self
            .active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.get(key).and_then(TrackedWorkspace::workspace_id)
            == Some(session.workspace_id.as_str())
        {
            active.insert(key.to_string(), TrackedWorkspace::Ready(session));
        }
    }

    fn remove_if_current(&self, key: &str, workspace_id: &str) {
        let mut active = self
            .active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.get(key).and_then(TrackedWorkspace::workspace_id) == Some(workspace_id) {
            active.remove(key);
        }
    }

    fn remove_if_pending(&self, key: &str, workspace_name: &str) {
        let mut active = self
            .active
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            active.get(key),
            Some(TrackedWorkspace::Pending { workspace_name: current }) if current == workspace_name
        ) {
            active.remove(key);
        }
    }
}

async fn complete_before<T>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = T>,
) -> Option<T> {
    tokio::time::timeout_at(deadline, future).await.ok()
}

fn command_timeout_error(timeout: Duration) -> Error {
    Error::message(format!(
        "coder: pty command timed out after configured timeout {timeout:?}"
    ))
}

fn delete_timeout_error(workspace_id: &str, timeout: Duration) -> Error {
    Error::message(format!(
        "coder: workspace {workspace_id} was not deleted within {timeout:?}"
    ))
}

fn ensure_delete_build_succeeded(build: &CoderBuild) -> Result<()> {
    if let Some(job) = build.job.as_ref()
        && matches!(job.status, Some(JobStatus::Failed | JobStatus::Canceled))
    {
        return Err(Error::message(format!(
            "coder: delete workspace build failed: {}",
            job.error.as_deref().unwrap_or("no error detail")
        )));
    }
    if matches!(
        build.status,
        Some(BuildStatus::Failed | BuildStatus::Canceled)
    ) {
        return Err(Error::message(format!(
            "coder: delete workspace build ended with status {:?}",
            build.status
        )));
    }
    Ok(())
}

fn ready_agent(workspace: &CoderWorkspace) -> Option<&CoderAgent> {
    workspace_agents(workspace).find(|agent| {
        agent.status == Some(AgentStatus::Connected)
            && agent.lifecycle_state == Some(AgentLifecycle::Ready)
    })
}

fn workspace_agents(workspace: &CoderWorkspace) -> impl Iterator<Item = &CoderAgent> {
    workspace
        .latest_build
        .iter()
        .flat_map(|build| build.resources.iter())
        .flat_map(|resource| resource.agents.iter())
}

fn failed_agent_state(workspace: &CoderWorkspace) -> Option<AgentLifecycle> {
    workspace_agents(workspace)
        .filter_map(|agent| agent.lifecycle_state)
        .find(|state| {
            matches!(
                state,
                AgentLifecycle::StartTimeout
                    | AgentLifecycle::StartError
                    | AgentLifecycle::ShuttingDown
                    | AgentLifecycle::ShutdownTimeout
                    | AgentLifecycle::ShutdownError
                    | AgentLifecycle::Off
            )
        })
}

fn workspace_is_stopped(workspace: &CoderWorkspace) -> bool {
    workspace
        .latest_build
        .as_ref()
        .and_then(|build| build.status)
        == Some(BuildStatus::Stopped)
}

fn workspace_session(
    workspace_id: &str,
    workspace_name: &str,
    workspace: &CoderWorkspace,
) -> Result<Option<CoderSession>> {
    if let Some(build) = workspace.latest_build.as_ref() {
        if let Some(job) = build.job.as_ref()
            && matches!(job.status, Some(JobStatus::Failed | JobStatus::Canceled))
        {
            return Err(Error::message(format!(
                "coder: workspace build failed: {}",
                job.error.as_deref().unwrap_or("no error detail")
            )));
        }
        if matches!(
            build.status,
            Some(BuildStatus::Failed | BuildStatus::Canceled | BuildStatus::Deleted)
        ) {
            return Err(Error::message(format!(
                "coder: workspace build ended with status {:?}",
                build.status
            )));
        }
        if build.status != Some(BuildStatus::Running) {
            return Ok(None);
        }
    } else {
        return Ok(None);
    }
    if let Some(state) = failed_agent_state(workspace) {
        return Err(Error::message(format!(
            "coder: workspace {workspace_name} agent entered terminal state {state:?}"
        )));
    }
    let Some(agent) = ready_agent(workspace) else {
        return Ok(None);
    };
    Ok(Some(CoderSession {
        workspace_id: workspace_id.to_string(),
        workspace_name: workspace_name.to_string(),
        agent_id: agent.id.clone(),
        workspace_dir: agent
            .expanded_directory
            .clone()
            .or_else(|| agent.directory.clone())
            .filter(|directory| !directory.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_WORKSPACE_DIR.to_string()),
    }))
}

fn looks_like_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

/// Reduce arbitrary text to a Coder-legal name fragment.
///
/// Coder validates workspace names against `^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$`,
/// so `.` and `_` — both permitted by the generic path sanitizer — are not
/// usable here. Runs of illegal characters collapse to a single separator and
/// leading/trailing separators are dropped.
fn coder_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_separator = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('-');
            }
            pending_separator = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    out
}

/// Build a Coder-legal, collision-resistant workspace name for a sandbox key.
pub(crate) fn workspace_name(prefix: &str, key: &str) -> String {
    let digest = workspace_digest(prefix, key);
    let mut prefix = coder_slug(prefix);
    if prefix.is_empty() {
        prefix.push_str("moltis");
    }
    let key_slug = coder_slug(key);
    let full = if key_slug.is_empty() {
        prefix
    } else {
        format!("{prefix}-{key_slug}")
    };
    let budget = MAX_WORKSPACE_NAME_LEN - digest.len() - 1;
    let mut head = full[..full.len().min(budget)]
        .trim_end_matches('-')
        .to_string();
    if head.is_empty() {
        head.push_str("moltis");
    }
    format!("{head}-{digest}")
}

fn validated_coder_url(value: &str) -> Result<url::Url> {
    if raw_url_authority(value).is_some_and(|authority| authority.contains('@')) {
        return Err(Error::message("coder URL must not contain userinfo"));
    }
    let url = url::Url::parse(value)
        .map_err(|error| Error::message(format!("coder URL could not be parsed: {error}")))?;
    if url.cannot_be_a_base() || url.host().is_none() {
        return Err(Error::message(
            "coder URL must be an absolute URL with a host",
        ));
    }
    let authority = &url[url::Position::BeforeUsername..url::Position::AfterPort];
    if authority.contains('@') || !url.username().is_empty() || url.password().is_some() {
        return Err(Error::message("coder URL must not contain userinfo"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::message(
            "coder URL must not contain a query string or fragment",
        ));
    }
    match url.scheme() {
        "https" => {},
        "http" if is_loopback_url(&url) => {},
        "http" => {
            return Err(Error::message(
                "coder URL must use HTTPS unless it is loopback",
            ));
        },
        scheme => {
            return Err(Error::message(format!(
                "coder URL scheme {scheme:?} is unsupported"
            )));
        },
    }
    Ok(url)
}

fn raw_url_authority(value: &str) -> Option<&str> {
    value
        .split_once("://")
        .map(|(_, remainder)| remainder.split(['/', '?', '#']).next().unwrap_or_default())
}

fn is_loopback_url(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}
