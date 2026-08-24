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
    sync::{Arc, LazyLock},
    time::Duration,
};

use {
    async_trait::async_trait,
    base64::Engine,
    futures::{SinkExt, StreamExt},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tokio::sync::{RwLock, Semaphore},
    tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest},
    tracing::{debug, info, warn},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::{
        file_system::{SandboxReadResult, command_write_file_with_limit},
        types::{Sandbox, SandboxConfig, SandboxId},
    },
};

const DEFAULT_WORKSPACE_DIR: &str = "/home/coder";
const GENERIC_WORKSPACE: &str = "/home/sandbox";
const GENERIC_WORKSPACE_PREFIX: &str = "/home/sandbox/";
const DEFAULT_CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Coder rejects workspace names longer than this.
const MAX_WORKSPACE_NAME_LEN: usize = 32;

/// PTY window reported to the workspace. Programs that self-format to
/// `COLUMNS` (`ls`, `ps`, `git log --graph`) wrap their own output, so the
/// window is made wide enough that ordinary command output survives intact.
/// The kernel line discipline never wraps, and raw mode disables `ONLCR`.
const PTY_COLS: u16 = 1000;
const PTY_ROWS: u16 = 200;

/// Base64 line length in the stdin stream. `sed` reads the stream a line at a
/// time, so lines are kept short enough for any implementation to buffer.
const STREAM_LINE_LEN: usize = 1024;
/// Approximate payload size of a single stdin WebSocket frame.
const STREAM_FRAME_BYTES: usize = 32 * 1024;
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

/// The per-invocation sentinels that frame one PTY exchange.
///
/// Every marker embeds a random nonce so a command that happens to print a
/// marker literal cannot terminate or mis-frame its own output.
struct PtyMarkers {
    ready: String,
    eof: String,
    exit: String,
    stderr_begin: String,
    stderr_end: String,
}

impl PtyMarkers {
    fn new() -> Self {
        Self::with_nonce(&uuid::Uuid::new_v4().simple().to_string())
    }

    fn with_nonce(nonce: &str) -> Self {
        Self {
            ready: format!("__MOLTIS_READY_{nonce}__"),
            eof: format!("__MOLTIS_EOF_{nonce}__"),
            exit: format!("__MOLTIS_EXIT_{nonce}__"),
            stderr_begin: format!("__MOLTIS_STDERR_{nonce}_BEGIN__"),
            stderr_end: format!("__MOLTIS_STDERR_{nonce}_END__"),
        }
    }
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
            if let Some(state) = failed_agent_state(&workspace) {
                return Err(Error::message(format!(
                    "coder: workspace {workspace_name} agent entered terminal state {state:?}"
                )));
            }
            if let Some(agent) = ready_agent(&workspace) {
                if agent_lifecycle(agent) == AgentLifecycle::Degraded {
                    warn!(
                        workspace = workspace_name,
                        "coder: workspace startup script timed out; tooling may be incomplete"
                    );
                }
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

    /// Open the agent PTY with the fixed bootstrap script.
    ///
    /// Only the bootstrap travels in the URL; it is a constant ~200 bytes, so
    /// no command or file payload can ever push the request over a proxy's URL
    /// limit.
    async fn open_pty(&self, session: &CoderSession, markers: &PtyMarkers) -> Result<PtyStream> {
        let ws_base = self
            .coder
            .url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        let mut url = url::Url::parse(&format!(
            "{ws_base}/api/v2/workspaceagents/{}/pty",
            session.agent_id
        ))
        .map_err(|e| Error::message(format!("coder: invalid pty URL: {e}")))?;
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
        timeout: Duration,
    ) -> Result<String> {
        let ws = self.open_pty(session, markers).await?;
        let (mut sink, mut stream) = ws.split();
        let frames = stream_frames(
            &base64::engine::general_purpose::STANDARD.encode(script.as_bytes()),
            &markers.eof,
        );

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        let reader = async {
            let mut ready_tx = Some(ready_tx);
            let mut combined = String::new();
            while let Some(message) = stream.next().await {
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
                if ready_tx.is_some() && combined.contains(&markers.ready) {
                    // Raw mode is active from here on, so streamed stdin is
                    // neither echoed back nor line-wrapped into the output.
                    if let Some(tx) = ready_tx.take() {
                        let _ = tx.send(());
                    }
                }
                if has_complete_markers(&combined, markers) {
                    break;
                }
            }
            Ok::<String, Error>(combined)
        };

        let writer = async {
            sink.send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({"height": PTY_ROWS, "width": PTY_COLS})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|e| Error::message(format!("coder: pty resize failed: {e}")))?;

            if tokio::time::timeout(READY_TIMEOUT, ready_rx).await.is_err() {
                debug!("coder: pty ready marker not seen, streaming payload anyway");
            }

            for frame in frames {
                sink.send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({ "data": frame }).to_string().into(),
                ))
                .await
                .map_err(|e| Error::message(format!("coder: pty write failed: {e}")))?;
            }
            sink.flush()
                .await
                .map_err(|e| Error::message(format!("coder: pty flush failed: {e}")))
        };

        tokio::time::timeout(timeout, futures::future::try_join(reader, writer))
            .await
            .map_err(|_| {
                Error::message(format!(
                    "coder: pty command timed out after {}s",
                    timeout.as_secs()
                ))
            })?
            .map(|(combined, ())| combined)
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
        let script = wrapped_command(command, &cwd, &opts.env, &markers);
        let combined = self
            .run_pty_script(session, &script, &markers, opts.timeout)
            .await?;

        let mut parsed = parse_pty_output(&combined, &markers)?;
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

    /// Write through the PTY stdin stream rather than a command line.
    ///
    /// The shared helper caps writes at `MAX_SANDBOX_WRITE_BYTES` because most
    /// backends embed the base64 payload in an `exec` argument list. This
    /// backend streams the script instead, so it raises the cap to
    /// [`CODER_MAX_WRITE_BYTES`] — without that, workspace sync-in fails for
    /// any workspace larger than 512 KB and aborts the command that triggered
    /// it.
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

/// The workspace agent Moltis should run commands on, if one is usable yet.
///
/// A connected agent is not a ready agent: Coder reports `created` and
/// `starting` while the template's startup script is still installing tooling,
/// so accepting those states races the very setup the template exists to
/// perform. Only `ready` (or a deployment old enough not to report the field
/// at all) counts.
fn ready_agent(workspace: &CoderWorkspace) -> Option<&CoderAgent> {
    workspace_agents(workspace).find(|agent| {
        agent.status.as_deref() == Some("connected") && agent_lifecycle(agent).is_usable()
    })
}

fn workspace_agents(workspace: &CoderWorkspace) -> impl Iterator<Item = &CoderAgent> {
    workspace
        .latest_build
        .iter()
        .flat_map(|build| build.resources.iter())
        .flat_map(|resource| resource.agents.iter())
}

/// How a reported agent lifecycle state maps onto "can we use this workspace".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLifecycle {
    /// Startup finished (or the deployment does not report lifecycle state).
    Usable,
    /// Startup still running — keep polling.
    Pending,
    /// Startup overran its timeout. The agent is up but tooling may be
    /// incomplete, so proceed with a warning rather than block forever.
    Degraded,
    /// Terminal state; polling will never succeed.
    Failed,
}

impl AgentLifecycle {
    fn is_usable(self) -> bool {
        matches!(self, Self::Usable | Self::Degraded)
    }
}

fn agent_lifecycle(agent: &CoderAgent) -> AgentLifecycle {
    match agent.lifecycle_state.as_deref() {
        // Deployments predating the lifecycle field report nothing.
        None | Some("ready") => AgentLifecycle::Usable,
        Some("created" | "starting") => AgentLifecycle::Pending,
        Some("start_timeout") => AgentLifecycle::Degraded,
        Some("start_error" | "shutting_down" | "shutdown_timeout" | "shutdown_error" | "off") => {
            AgentLifecycle::Failed
        },
        // Unknown future states: keep polling rather than guess.
        Some(_) => AgentLifecycle::Pending,
    }
}

/// The terminal agent state blocking this workspace, if any.
fn failed_agent_state(workspace: &CoderWorkspace) -> Option<&str> {
    workspace_agents(workspace)
        .find(|agent| agent_lifecycle(agent) == AgentLifecycle::Failed)
        .and_then(|agent| agent.lifecycle_state.as_deref())
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

/// FNV-1a. Used only to disambiguate truncated workspace names, but must stay
/// deterministic across builds so a given session maps to a stable name.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Build a Coder-legal, collision-resistant workspace name for a sandbox key.
///
/// Names over [`MAX_WORKSPACE_NAME_LEN`] get a digest of the full key appended
/// so that two sessions sharing a long prefix — the common shape for
/// `project-<name>-<uuid>` keys — do not truncate onto the same workspace.
pub(crate) fn workspace_name(prefix: &str, key: &str) -> String {
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
    if full.len() <= MAX_WORKSPACE_NAME_LEN {
        return full;
    }

    let digest = format!("{:06x}", fnv1a(key.as_bytes()) & 0x00ff_ffff);
    let budget = MAX_WORKSPACE_NAME_LEN - digest.len() - 1;
    let mut head = full[..budget].trim_end_matches('-').to_string();
    if head.is_empty() {
        head.push_str("moltis");
    }
    format!("{head}-{digest}")
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// The fixed script handed to Coder in the PTY URL's `command` parameter.
///
/// It must stay short — this is the one part of the exchange that is bounded
/// by URL length. Everything it needs arrives afterwards on stdin.
///
/// `stty raw -echo` matters for correctness, not just tidiness: without it the
/// streamed payload is echoed back into the output we are parsing, and `ONLCR`
/// rewrites every `\n` on the way out.
fn bootstrap_command(markers: &PtyMarkers) -> String {
    let PtyMarkers { ready, eof, .. } = markers;
    format!(
        "stty raw -echo 2>/dev/null; TERM=dumb; export TERM; \
         printf '\n{ready}\n'; \
         t=$(mktemp /tmp/moltis-rx.XXXXXX) || exit 125; \
         sed -n '/^{eof}$/q;p' | base64 -d > \"$t\"; \
         sh \"$t\" </dev/null; rm -f \"$t\""
    )
}

/// Split a base64 payload into newline-delimited stdin frames.
///
/// Lines are bounded so `sed` never has to buffer an unbounded line, and
/// frames are batched so a large payload does not become one WebSocket message
/// per kilobyte. The trailing frame carries the EOF sentinel that stops `sed`.
fn stream_frames(payload: &str, eof_marker: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut frame = String::with_capacity(STREAM_FRAME_BYTES + STREAM_LINE_LEN);
    let mut offset = 0;
    while offset < payload.len() {
        let end = (offset + STREAM_LINE_LEN).min(payload.len());
        frame.push_str(&payload[offset..end]);
        frame.push('\n');
        offset = end;
        if frame.len() >= STREAM_FRAME_BYTES {
            frames.push(std::mem::take(&mut frame));
        }
    }
    frame.push_str(eof_marker);
    frame.push('\n');
    frames.push(frame);
    frames
}

fn wrapped_command(
    command: &str,
    cwd: &str,
    env: &[(String, String)],
    markers: &PtyMarkers,
) -> String {
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
    let PtyMarkers {
        exit: exit_marker,
        stderr_begin,
        stderr_end,
        ..
    } = markers;

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

fn has_complete_markers(output: &str, markers: &PtyMarkers) -> bool {
    output.contains(&markers.exit) && output.contains(&markers.stderr_end)
}

static ANSI_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    #[allow(clippy::unwrap_used)]
    regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap()
});

fn strip_ansi(input: &str) -> String {
    ANSI_RE.replace_all(input, "").into_owned()
}

fn parse_pty_output(output: &str, markers: &PtyMarkers) -> Result<ParsedPtyOutput> {
    let normalized = strip_ansi(output).replace('\r', "");
    // Everything before the ready marker is bootstrap noise, not command output.
    let body = match normalized.find(&markers.ready) {
        Some(pos) => normalized[pos + markers.ready.len()..].trim_start_matches('\n'),
        None => normalized.as_str(),
    };

    let exit_pos = body
        .find(&markers.exit)
        .ok_or_else(|| Error::message("coder: pty output missing exit marker"))?;
    let after_exit = &body[exit_pos + markers.exit.len()..];
    let exit_line = after_exit.lines().next().unwrap_or_default().trim();
    let exit_code = exit_line
        .parse::<i32>()
        .map_err(|e| Error::message(format!("coder: invalid pty exit code {exit_line:?}: {e}")))?;
    let stderr_start = body
        .find(&markers.stderr_begin)
        .ok_or_else(|| Error::message("coder: pty output missing stderr begin marker"))?
        + markers.stderr_begin.len();
    let stderr_end_pos = body
        .find(&markers.stderr_end)
        .ok_or_else(|| Error::message("coder: pty output missing stderr end marker"))?;
    let stdout = body[..exit_pos].trim_end_matches('\n').to_string();
    let stderr = body[stderr_start..stderr_end_pos]
        .trim_matches('\n')
        .to_string();
    Ok(ParsedPtyOutput {
        stdout,
        stderr,
        exit_code,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn agent(lifecycle: Option<&str>, status: Option<&str>) -> CoderAgent {
        CoderAgent {
            id: "agent-1".into(),
            status: status.map(ToOwned::to_owned),
            lifecycle_state: lifecycle.map(ToOwned::to_owned),
            directory: None,
            expanded_directory: None,
        }
    }

    fn workspace_with(agents: Vec<CoderAgent>) -> CoderWorkspace {
        CoderWorkspace {
            id: "ws-1".into(),
            name: "ws".into(),
            latest_build: Some(CoderBuild {
                status: Some("running".into()),
                job: None,
                resources: vec![CoderResource { agents }],
            }),
        }
    }

    // ── Workspace naming ───────────────────────────────────────────────────

    /// Coder validates names against `^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$`.
    fn is_valid_coder_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= MAX_WORKSPACE_NAME_LEN
            && name.split('-').all(|segment| {
                !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_alphanumeric())
            })
    }

    #[test]
    fn workspace_name_is_sanitized_and_limited() {
        let name = workspace_name("Moltis!", "Session/With/Long/Unsafe/Characters/AndMore");
        assert!(name.starts_with("moltis-"), "{name}");
        assert!(is_valid_coder_name(&name), "{name}");
    }

    #[test]
    fn workspace_name_rejects_underscores_and_dots() {
        // sanitize_path_component would keep these; Coder does not accept them.
        let name = workspace_name("moltis", "session_key.v2");
        assert_eq!(name, "moltis-session-key-v2");
        assert!(is_valid_coder_name(&name), "{name}");
    }

    #[test]
    fn workspace_name_collapses_separator_runs_and_trims_edges() {
        let name = workspace_name("__moltis__", "///abc///");
        assert_eq!(name, "moltis-abc");
        assert!(is_valid_coder_name(&name), "{name}");
    }

    #[test]
    fn workspace_name_disambiguates_truncated_keys() {
        let a = workspace_name("moltis", "project-with-a-very-long-name-aaaaaaaa");
        let b = workspace_name("moltis", "project-with-a-very-long-name-bbbbbbbb");
        assert!(is_valid_coder_name(&a), "{a}");
        assert!(is_valid_coder_name(&b), "{b}");
        assert_ne!(a, b, "truncated names must stay distinct");
    }

    #[test]
    fn workspace_name_is_deterministic() {
        let key = "project-with-a-very-long-name-aaaaaaaa";
        assert_eq!(workspace_name("moltis", key), workspace_name("moltis", key));
    }

    #[test]
    fn workspace_name_falls_back_when_everything_is_stripped() {
        let name = workspace_name("!!!", "///");
        assert_eq!(name, "moltis");
        assert!(is_valid_coder_name(&name), "{name}");
    }

    // ── Agent readiness ────────────────────────────────────────────────────

    #[test]
    fn connected_but_starting_agent_is_not_ready() {
        for state in ["created", "starting"] {
            let ws = workspace_with(vec![agent(Some(state), Some("connected"))]);
            assert!(
                ready_agent(&ws).is_none(),
                "{state} must not be treated as ready"
            );
            assert!(failed_agent_state(&ws).is_none());
        }
    }

    #[test]
    fn ready_agent_requires_connected_status() {
        let ws = workspace_with(vec![agent(Some("ready"), Some("connecting"))]);
        assert!(ready_agent(&ws).is_none());
    }

    #[test]
    fn ready_agent_accepts_ready_and_legacy_deployments() {
        let ws = workspace_with(vec![agent(Some("ready"), Some("connected"))]);
        assert!(ready_agent(&ws).is_some());
        let legacy = workspace_with(vec![agent(None, Some("connected"))]);
        assert!(ready_agent(&legacy).is_some());
    }

    #[test]
    fn start_timeout_is_usable_but_degraded() {
        let ws = workspace_with(vec![agent(Some("start_timeout"), Some("connected"))]);
        let found = ready_agent(&ws).expect("degraded agent is still usable");
        assert_eq!(agent_lifecycle(found), AgentLifecycle::Degraded);
    }

    #[test]
    fn terminal_agent_states_are_reported() {
        for state in ["start_error", "off", "shutdown_error"] {
            let ws = workspace_with(vec![agent(Some(state), Some("connected"))]);
            assert_eq!(failed_agent_state(&ws), Some(state));
            assert!(ready_agent(&ws).is_none());
        }
    }

    #[test]
    fn unknown_lifecycle_states_keep_polling() {
        let ws = workspace_with(vec![agent(Some("some_future_state"), Some("connected"))]);
        assert!(ready_agent(&ws).is_none());
        assert!(failed_agent_state(&ws).is_none());
    }

    // ── PTY framing ────────────────────────────────────────────────────────

    #[test]
    fn bootstrap_command_stays_url_sized() {
        let markers = PtyMarkers::new();
        let bootstrap = bootstrap_command(&markers);
        assert!(
            bootstrap.len() < 1024,
            "bootstrap must stay far below URL limits, got {} bytes",
            bootstrap.len()
        );
        assert!(bootstrap.contains("stty raw -echo"));
        assert!(bootstrap.contains(&markers.ready));
        assert!(bootstrap.contains(&markers.eof));
    }

    #[test]
    fn stream_frames_terminate_with_the_eof_sentinel() {
        let frames = stream_frames("abcd", "__EOF__");
        assert_eq!(frames, vec!["abcd\n__EOF__\n".to_string()]);
    }

    #[test]
    fn stream_frames_bound_line_and_frame_size() {
        let payload = "x".repeat(STREAM_FRAME_BYTES * 2 + 7);
        let frames = stream_frames(&payload, "__EOF__");
        assert!(
            frames.len() > 1,
            "large payloads must be split across frames"
        );

        let last = frames.last().unwrap();
        assert!(last.ends_with("__EOF__\n"));

        for frame in &frames {
            for line in frame.lines().filter(|line| *line != "__EOF__") {
                assert!(
                    line.len() <= STREAM_LINE_LEN,
                    "line of {} bytes exceeds the sed line budget",
                    line.len()
                );
            }
        }

        // The payload must round-trip exactly: no bytes dropped or duplicated.
        let rebuilt: String = frames
            .concat()
            .lines()
            .filter(|line| *line != "__EOF__")
            .collect();
        assert_eq!(rebuilt, payload);
    }

    /// A 512 KB write used to be impossible: the script went into the PTY URL
    /// query string, which proxies cap at a few kilobytes.
    #[test]
    fn large_writes_stream_instead_of_entering_the_url() {
        let markers = PtyMarkers::new();
        let bootstrap = bootstrap_command(&markers);
        let script = wrapped_command(&"a".repeat(512 * 1024), "/home/coder", &[], &markers);
        assert!(script.len() > 512 * 1024);

        // Only the bootstrap is URL-borne, and it does not grow with the script.
        assert!(!bootstrap.contains(&script));
        assert!(bootstrap.len() < 1024);

        let payload = base64::engine::general_purpose::STANDARD.encode(script.as_bytes());
        let frames = stream_frames(&payload, &markers.eof);
        assert!(frames.len() > 1);
    }

    // ── Output parsing ─────────────────────────────────────────────────────

    #[test]
    fn parses_pty_markers() {
        let markers = PtyMarkers::with_nonce("abc");
        let output = "hello\r\n__MOLTIS_EXIT_abc__7\r\n__MOLTIS_STDERR_abc_BEGIN__\r\noops\r\n__MOLTIS_STDERR_abc_END__\r\n";
        let parsed = parse_pty_output(output, &markers).unwrap();
        assert_eq!(parsed.stdout, "hello");
        assert_eq!(parsed.stderr, "oops");
        assert_eq!(parsed.exit_code, 7);
    }

    #[test]
    fn ready_marker_and_bootstrap_noise_are_stripped_from_stdout() {
        let markers = PtyMarkers::with_nonce("abc");
        let output = format!(
            "some login banner\n{}\nhello\n{}0\n{}\n\n{}\n",
            markers.ready, markers.exit, markers.stderr_begin, markers.stderr_end
        );
        let parsed = parse_pty_output(&output, &markers).unwrap();
        assert_eq!(parsed.stdout, "hello");
        assert_eq!(parsed.stderr, "");
        assert_eq!(parsed.exit_code, 0);
    }

    #[test]
    fn ansi_escapes_are_stripped() {
        let markers = PtyMarkers::with_nonce("abc");
        let output = format!(
            "\x1b[32mgreen\x1b[0m\n{}0\n{}\n\n{}\n",
            markers.exit, markers.stderr_begin, markers.stderr_end
        );
        let parsed = parse_pty_output(&output, &markers).unwrap();
        assert_eq!(parsed.stdout, "green");
    }

    #[test]
    fn missing_markers_error_rather_than_returning_partial_output() {
        let markers = PtyMarkers::with_nonce("abc");
        assert!(parse_pty_output("no markers here", &markers).is_err());
    }

    #[test]
    fn markers_are_unique_per_invocation() {
        assert_ne!(PtyMarkers::new().exit, PtyMarkers::new().exit);
    }

    #[test]
    fn wrapped_command_includes_cwd_env_and_markers() {
        let markers = PtyMarkers::with_nonce("abc");
        let cmd = wrapped_command(
            "printf hi",
            "/home/coder/project",
            &[
                ("FOO".into(), "bar baz".into()),
                ("BAD-KEY".into(), "x".into()),
            ],
            &markers,
        );
        assert!(cmd.contains("cd /home/coder/project"));
        assert!(cmd.contains("FOO='bar baz'") || cmd.contains("FOO=bar\\ baz"));
        assert!(!cmd.contains("BAD-KEY"));
        assert!(cmd.contains("__MOLTIS_EXIT_abc__"));
    }

    // ── Transfer limits ────────────────────────────────────────────────────

    #[test]
    fn test_coder_sandbox_backend_name() {
        let sandbox = CoderSandbox::new(SandboxConfig::default(), CoderSandboxConfig {
            url: "https://coder.example.com".into(),
            token: Secret::new("token".into()),
            organization: None,
            user: "me".into(),
            template_id: None,
            template_name: Some("devbox".into()),
            workspace_prefix: "moltis".into(),
            ttl_ms: None,
            size: None,
            template_presets: HashMap::new(),
            parameter_values: HashMap::new(),
        });
        assert_eq!(sandbox.backend_name(), "coder");
        assert!(sandbox.is_isolated());
        assert!(sandbox.provides_fs_isolation());
        assert_eq!(sandbox.max_transfer_bytes(), CODER_MAX_TRANSFER_BYTES);
    }

    #[test]
    fn translate_working_dir_maps_the_generic_sandbox_home() {
        assert_eq!(
            CoderSandbox::translate_working_dir(Some("/home/sandbox"), "/home/coder"),
            "/home/coder"
        );
        assert_eq!(
            CoderSandbox::translate_working_dir(Some("/home/sandbox/proj"), "/home/coder"),
            "/home/coder/proj"
        );
        assert_eq!(
            CoderSandbox::translate_working_dir(None, "/home/coder"),
            "/home/coder"
        );
    }
}
