//! Vercel Sandbox backend — Firecracker microVM via the Vercel API.
//!
//! Each session gets an ephemeral Vercel sandbox. Commands run via the
//! REST API, files transfer via gzipped tar uploads and raw reads. The
//! sandbox is stopped on cleanup.
//!
//! Requires `VERCEL_TOKEN` (or `VERCEL_OIDC_TOKEN`) and a Vercel project.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    async_trait::async_trait,
    flate2::{Compression, write::GzEncoder},
    secrecy::{ExposeSecret, Secret},
    tokio::sync::{RwLock, Semaphore},
    tracing::{debug, info, warn},
};

use crate::{
    error::{Error, Result},
    exec::{ExecOpts, ExecResult},
    sandbox::{
        file_system::SandboxReadResult,
        types::{Sandbox, SandboxConfig, SandboxId},
    },
};

/// Base URL for Vercel API.
const VERCEL_API_BASE: &str = "https://vercel.com/api";

/// Default sandbox workspace directory inside Vercel sandboxes.
const VERCEL_WORKSPACE: &str = "/vercel/sandbox";

/// Generic workspace path used by the shared sandbox tool contract.
const GENERIC_WORKSPACE: &str = "/home/sandbox";
const GENERIC_WORKSPACE_PREFIX: &str = "/home/sandbox/";

/// Default timeout for sandbox creation (5 minutes).
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

/// State of a live Vercel sandbox session.
struct VercelSession {
    sandbox_id: String,
}

#[derive(Debug, Default)]
struct VercelCommandEvents {
    command_id: Option<String>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Vercel Sandbox backend configuration.
#[derive(Debug, Clone)]
pub struct VercelSandboxConfig {
    pub token: Secret<String>,
    pub project_id: Option<String>,
    pub team_id: Option<String>,
    pub runtime: String,
    pub timeout_ms: u64,
    pub vcpus: u32,
    pub snapshot_id: Option<String>,
}

impl Default for VercelSandboxConfig {
    fn default() -> Self {
        Self {
            token: Secret::new(String::new()),
            project_id: None,
            team_id: None,
            runtime: "node24".into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            vcpus: 2,
            snapshot_id: None,
        }
    }
}

/// Vercel Sandbox backend.
pub struct VercelSandbox {
    #[allow(dead_code)]
    config: SandboxConfig,
    vercel: VercelSandboxConfig,
    client: reqwest::Client,
    active: RwLock<HashMap<String, VercelSession>>,
    creation_permits: RwLock<HashMap<String, Arc<Semaphore>>>,
}

impl VercelSandbox {
    pub fn new(config: SandboxConfig, vercel: VercelSandboxConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();
        Self {
            config,
            vercel,
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

    fn translate_working_dir(working_dir: Option<&str>) -> String {
        match working_dir {
            Some(path) if path == GENERIC_WORKSPACE => VERCEL_WORKSPACE.to_string(),
            Some(path) if path.starts_with(GENERIC_WORKSPACE_PREFIX) => {
                format!("{VERCEL_WORKSPACE}{}", &path[GENERIC_WORKSPACE.len()..])
            },
            Some(path) => path.to_string(),
            None => VERCEL_WORKSPACE.to_string(),
        }
    }

    fn env_object(env: &[(String, String)]) -> serde_json::Map<String, serde_json::Value> {
        env.iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect()
    }

    fn non_empty_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned)
    }

    fn sandbox_id_from_create_response(data: &serde_json::Value) -> Option<String> {
        Self::non_empty_id(data["sandbox"]["id"].as_str())
            .or_else(|| Self::non_empty_id(data["sandbox"]["sandboxId"].as_str()))
            .or_else(|| Self::non_empty_id(data["sandbox"]["sandbox_id"].as_str()))
            .or_else(|| Self::non_empty_id(data["id"].as_str()))
            .or_else(|| Self::non_empty_id(data["sandboxId"].as_str()))
            .or_else(|| Self::non_empty_id(data["sandbox_id"].as_str()))
    }

    fn sandbox_id_from_location(location: Option<&reqwest::header::HeaderValue>) -> Option<String> {
        let location = location.and_then(|value| value.to_str().ok())?;
        let path = location.split('?').next().unwrap_or(location);
        Self::non_empty_id(path.rsplit('/').next())
    }

    fn snapshot_id_from_response(data: &serde_json::Value) -> Option<String> {
        Self::non_empty_id(data["snapshot"]["id"].as_str())
            .or_else(|| Self::non_empty_id(data["snapshot"]["snapshotId"].as_str()))
            .or_else(|| Self::non_empty_id(data["snapshot"]["snapshot_id"].as_str()))
            .or_else(|| Self::non_empty_id(data["id"].as_str()))
            .or_else(|| Self::non_empty_id(data["snapshotId"].as_str()))
            .or_else(|| Self::non_empty_id(data["snapshot_id"].as_str()))
    }

    fn parse_command_events(text: &str, max_output_bytes: usize) -> VercelCommandEvents {
        let mut events = VercelCommandEvents::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };

            if let Some(code) = value["command"]["exitCode"]
                .as_i64()
                .or_else(|| value["command"]["exit_code"].as_i64())
                .or_else(|| value["exitCode"].as_i64())
                .or_else(|| value["exit_code"].as_i64())
            {
                events.exit_code = Some(code as i32);
            }

            if events.command_id.is_none() {
                events.command_id = value["command"]["id"]
                    .as_str()
                    .or_else(|| value["command"]["commandId"].as_str())
                    .or_else(|| value["command"]["command_id"].as_str())
                    .or_else(|| value["id"].as_str())
                    .or_else(|| value["commandId"].as_str())
                    .or_else(|| value["command_id"].as_str())
                    .and_then(|id| Self::non_empty_id(Some(id)));
            }

            Self::append_inline_command_output(&value, &mut events.stdout, &mut events.stderr);
        }

        events
            .stdout
            .truncate(events.stdout.floor_char_boundary(max_output_bytes));
        events
            .stderr
            .truncate(events.stderr.floor_char_boundary(max_output_bytes));
        events
    }

    fn append_inline_command_output(
        value: &serde_json::Value,
        stdout: &mut String,
        stderr: &mut String,
    ) {
        for candidate in [
            &value["stdout"],
            &value["command"]["stdout"],
            &value["result"]["stdout"],
            &value["command"]["result"]["stdout"],
        ] {
            if let Some(text) = candidate.as_str() {
                stdout.push_str(text);
            }
        }

        for candidate in [
            &value["stderr"],
            &value["command"]["stderr"],
            &value["result"]["stderr"],
            &value["command"]["result"]["stderr"],
        ] {
            if let Some(text) = candidate.as_str() {
                stderr.push_str(text);
            }
        }

        let stream = value["stream"]
            .as_str()
            .or_else(|| value["command"]["stream"].as_str())
            .unwrap_or("stdout");
        for candidate in [
            &value["data"],
            &value["output"],
            &value["result"],
            &value["command"]["data"],
            &value["command"]["output"],
            &value["command"]["result"],
        ] {
            if let Some(text) = candidate.as_str() {
                match stream {
                    "stderr" => stderr.push_str(text),
                    _ => stdout.push_str(text),
                }
            }
        }
    }

    fn command_output_from_logs(
        events: VercelCommandEvents,
        logs: Result<(String, String)>,
        sandbox_id: &str,
    ) -> (String, String) {
        match logs {
            Ok(logs) if !logs.0.is_empty() || !logs.1.is_empty() => logs,
            Ok(_) => (events.stdout, events.stderr),
            Err(e) => {
                warn!(
                    vercel_id = sandbox_id,
                    error = %e,
                    "vercel: failed to fetch command logs, using inline output"
                );
                (events.stdout, events.stderr)
            },
        }
    }

    /// Build an authenticated request with team scoping.
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut url = format!("{VERCEL_API_BASE}{path}");
        if let Some(ref team_id) = self.vercel.team_id {
            url.push_str(&format!(
                "{}teamId={team_id}",
                if url.contains('?') {
                    "&"
                } else {
                    "?"
                }
            ));
        }
        self.client
            .request(method, &url)
            .bearer_auth(self.vercel.token.expose_secret())
    }

    /// Create a Vercel sandbox, returning the sandbox ID.
    async fn create_sandbox(&self) -> Result<String> {
        let project_id = self.vercel.project_id.as_deref().ok_or_else(|| {
            Error::message(
                "vercel: project_id is required (set VERCEL_PROJECT_ID or configure in settings)",
            )
        })?;

        let mut body = serde_json::json!({
            "projectId": project_id,
            "runtime": self.vercel.runtime,
            "timeout": self.vercel.timeout_ms,
            "resources": { "vcpus": self.vercel.vcpus },
        });

        if let Some(ref snapshot_id) = self.vercel.snapshot_id {
            body["source"] = serde_json::json!({
                "type": "snapshot",
                "snapshotId": snapshot_id,
            });
        }

        let resp = self
            .request(reqwest::Method::POST, "/v1/sandboxes")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: failed to create sandbox: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "vercel: create sandbox failed (HTTP {status}): {text}"
            )));
        }

        let location = resp.headers().get(reqwest::header::LOCATION).cloned();
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::message(format!("vercel: invalid create response: {e}")))?;

        match Self::sandbox_id_from_create_response(&data)
            .or_else(|| Self::sandbox_id_from_location(location.as_ref()))
        {
            Some(id) => Ok(id),
            None => {
                warn!(
                    "vercel: sandbox create response contained no parseable sandbox ID; \
                    the VM will run until its timeout"
                );
                Err(Error::message(
                    "vercel: missing sandbox.id in create response",
                ))
            },
        }
    }

    /// Wait for a sandbox to reach "running" status.
    async fn wait_for_running(&self, sandbox_id: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            let resp = self
                .request(reqwest::Method::GET, &format!("/v1/sandboxes/{sandbox_id}"))
                .send()
                .await
                .map_err(|e| Error::message(format!("vercel: failed to get sandbox: {e}")))?;

            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(Error::message(format!(
                    "vercel: get sandbox failed: {text}"
                )));
            }

            let data: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| Error::message(format!("vercel: invalid get response: {e}")))?;

            let status = data["sandbox"]["status"].as_str().unwrap_or("unknown");
            match status {
                "running" => return Ok(()),
                "failed" | "aborted" | "stopped" => {
                    return Err(Error::message(format!(
                        "vercel: sandbox entered terminal state: {status}"
                    )));
                },
                _ => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::message(format!(
                            "vercel: sandbox did not reach running state within 120s (current: {status})"
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                },
            }
        }
    }

    async fn prepare_created_sandbox(&self, id: &SandboxId, sandbox_id: &str) -> Result<()> {
        debug!(%id, vercel_id = sandbox_id, "vercel: sandbox created, waiting for running state");

        if let Err(e) = self.wait_for_running(sandbox_id).await {
            self.stop_after_setup_failure(sandbox_id, "wait_for_running")
                .await;
            return Err(e);
        }
        if let Err(e) = self.mkdir(sandbox_id, VERCEL_WORKSPACE).await {
            self.stop_after_setup_failure(sandbox_id, "mkdir").await;
            return Err(e);
        }

        Ok(())
    }

    /// Run a command and wait for completion via NDJSON streaming.
    async fn run_command(
        &self,
        sandbox_id: &str,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let cwd = Self::translate_working_dir(opts.working_dir.as_ref().and_then(|p| p.to_str()));
        let mut body = serde_json::json!({
            "command": "sh",
            "args": ["-c", command],
            "cwd": cwd,
            "wait": true,
        });
        if !opts.env.is_empty() {
            body["env"] = serde_json::Value::Object(Self::env_object(&opts.env));
        }

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/cmd"),
            )
            .timeout(opts.timeout + Duration::from_secs(5))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: command request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "vercel: command failed (HTTP {status}): {text}"
            )));
        }

        // Response is NDJSON: first line = started, last line = finished.
        let text = resp
            .text()
            .await
            .map_err(|e| Error::message(format!("vercel: failed to read command response: {e}")))?;

        let events = Self::parse_command_events(&text, opts.max_output_bytes);
        let exit_code = events.exit_code.unwrap_or(-1);

        let (stdout, stderr) = if let Some(cmd_id) = events.command_id.clone() {
            let logs = self.fetch_command_logs(sandbox_id, &cmd_id, opts).await;
            Self::command_output_from_logs(events, logs, sandbox_id)
        } else {
            (events.stdout, events.stderr)
        };

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Fetch stdout/stderr logs for a completed command.
    async fn fetch_command_logs(
        &self,
        sandbox_id: &str,
        cmd_id: &str,
        opts: &ExecOpts,
    ) -> Result<(String, String)> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/sandboxes/{sandbox_id}/cmd/{cmd_id}/logs"),
            )
            .timeout(opts.timeout + Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: failed to fetch logs: {e}")))?;

        if !resp.status().is_success() {
            return Ok((String::new(), String::new()));
        }

        let text = resp.text().await.unwrap_or_default();
        let mut stdout = String::new();
        let mut stderr = String::new();

        for line in text.lines().filter(|l| !l.is_empty()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let stream = v["stream"].as_str().unwrap_or("");
                let data = v["data"].as_str().unwrap_or("");
                match stream {
                    "stdout" => stdout.push_str(data),
                    "stderr" => stderr.push_str(data),
                    _ => {},
                }
            }
        }

        stdout.truncate(stdout.floor_char_boundary(opts.max_output_bytes));
        stderr.truncate(stderr.floor_char_boundary(opts.max_output_bytes));

        Ok((stdout, stderr))
    }

    /// Write files to the sandbox using gzipped tar.
    async fn write_files_tar(&self, sandbox_id: &str, files: &[(&str, &[u8])]) -> Result<()> {
        let gz_bytes = {
            let buf = Vec::new();
            let enc = GzEncoder::new(buf, Compression::fast());
            let mut ar = tar::Builder::new(enc);

            for &(path, content) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                ar.append_data(&mut header, path.trim_start_matches('/'), content)
                    .map_err(|e| Error::message(format!("vercel: tar append failed: {e}")))?;
            }

            ar.into_inner()
                .and_then(|enc| enc.finish())
                .map_err(|e| Error::message(format!("vercel: tar finalize failed: {e}")))?
        };

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/fs/write"),
            )
            .header("Content-Type", "application/gzip")
            .header("X-Cwd", "/")
            .body(gz_bytes)
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: file write request failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!("vercel: file write failed: {text}")));
        }

        Ok(())
    }

    /// Read a file from the sandbox.
    async fn read_file_raw(&self, sandbox_id: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let body = serde_json::json!({
            "path": path,
            "cwd": "/",
        });

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/fs/read"),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: file read request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!("vercel: file read failed: {text}")));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::message(format!("vercel: failed to read file bytes: {e}")))?;

        Ok(Some(bytes.to_vec()))
    }

    /// Create a directory in the sandbox.
    async fn mkdir(&self, sandbox_id: &str, path: &str) -> Result<()> {
        let body = serde_json::json!({ "path": path });

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/fs/mkdir"),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: mkdir request failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!("vercel: mkdir failed: {text}")));
        }

        Ok(())
    }

    /// Stop a sandbox.
    async fn stop_sandbox(&self, sandbox_id: &str) -> Result<()> {
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/stop"),
            )
            .send()
            .await
            .map_err(|e| Error::message(format!("vercel: stop request failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            if !text.contains("already stopped") && !text.contains("not running") {
                return Err(Error::message(format!(
                    "vercel: stop sandbox failed: {text}"
                )));
            }
        }

        Ok(())
    }

    async fn stop_after_setup_failure(&self, sandbox_id: &str, step: &str) {
        if let Err(e) = self.stop_sandbox(sandbox_id).await {
            warn!(
                vercel_id = sandbox_id,
                step,
                error = %e,
                "vercel: failed to stop sandbox after setup failure"
            );
        }
    }

    /// Get the sandbox ID for a session, or None.
    async fn session_sandbox_id(&self, id: &SandboxId) -> Option<String> {
        self.active
            .read()
            .await
            .get(&id.key)
            .map(|s| s.sandbox_id.clone())
    }
}

#[async_trait]
impl Sandbox for VercelSandbox {
    fn backend_name(&self) -> &'static str {
        "vercel"
    }

    fn is_real(&self) -> bool {
        true
    }

    fn provides_fs_isolation(&self) -> bool {
        true
    }

    fn is_isolated(&self) -> bool {
        true
    }

    fn workspace_dir(&self) -> &str {
        "/vercel/sandbox"
    }

    /// Vercel sandboxes run Amazon Linux 2023 which uses `dnf`, not `apt-get`.
    async fn provision_packages(&self, id: &SandboxId, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        // Map common Debian package names to Amazon Linux equivalents.
        let mapped: Vec<&str> = packages
            .iter()
            .filter_map(|p| debian_to_amzn_package(p))
            .collect();
        if mapped.is_empty() {
            return Ok(());
        }
        let pkg_list = mapped.join(" ");
        let cmd = format!("sudo dnf install -y -q {pkg_list}");
        let opts = ExecOpts {
            timeout: Duration::from_secs(600),
            ..Default::default()
        };
        let result = self.exec(id, &cmd, &opts).await?;
        if result.exit_code != 0 {
            tracing::warn!(
                %id,
                exit_code = result.exit_code,
                stderr = result.stderr.trim(),
                "vercel: package provisioning failed (non-fatal)"
            );
        }
        Ok(())
    }

    /// Build a Vercel snapshot with packages pre-installed.
    ///
    /// Creates a temporary sandbox, installs packages via dnf, takes a
    /// snapshot, and stops the sandbox. The snapshot ID is returned as
    /// the "tag" — future `ensure_ready()` calls create sandboxes from
    /// this snapshot, skipping package installation entirely.
    async fn build_image(
        &self,
        _base: &str,
        packages: &[String],
    ) -> Result<Option<super::types::BuildImageResult>> {
        if packages.is_empty() {
            return Ok(None);
        }

        // If a snapshot is already configured, skip building.
        if self.vercel.snapshot_id.is_some() {
            return Ok(None);
        }

        info!("vercel: building snapshot with packages pre-installed");

        let sandbox_id = self.create_sandbox().await?;
        if let Err(e) = self.wait_for_running(&sandbox_id).await {
            self.stop_after_setup_failure(&sandbox_id, "wait_for_running")
                .await;
            return Err(e);
        }

        // Install packages using the Vercel-specific provisioning.
        let mapped: Vec<&str> = packages
            .iter()
            .filter_map(|p| debian_to_amzn_package(p))
            .collect();
        if !mapped.is_empty() {
            let pkg_list = mapped.join(" ");
            let cmd = format!("sudo dnf install -y -q {pkg_list}");
            let opts = ExecOpts {
                timeout: Duration::from_secs(600),
                ..Default::default()
            };
            let result = self.run_command(&sandbox_id, &cmd, &opts).await;
            if let Ok(r) = result
                && r.exit_code != 0
            {
                warn!(
                    exit_code = r.exit_code,
                    "vercel: package install for snapshot failed (continuing)"
                );
            }
        }

        // Take a snapshot.
        let resp = match self
            .request(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{sandbox_id}/snapshot"),
            )
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let _ = self.stop_sandbox(&sandbox_id).await;
                return Err(Error::message(format!(
                    "vercel: snapshot request failed: {e}"
                )));
            },
        };

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!("vercel: snapshot failed: {text}");
            let _ = self.stop_sandbox(&sandbox_id).await;
            return Ok(None);
        }

        let data: serde_json::Value = match resp.json().await {
            Ok(data) => data,
            Err(e) => {
                let _ = self.stop_sandbox(&sandbox_id).await;
                return Err(Error::message(format!(
                    "vercel: invalid snapshot response: {e}"
                )));
            },
        };

        let Some(snapshot_id) = Self::snapshot_id_from_response(&data) else {
            let _ = self.stop_sandbox(&sandbox_id).await;
            warn!("vercel: snapshot response missing snapshot id");
            return Ok(None);
        };

        info!(snapshot_id, "vercel: snapshot created with packages");

        if snapshot_id.is_empty() {
            let _ = self.stop_sandbox(&sandbox_id).await;
            return Ok(None);
        }

        if let Err(e) = self.stop_sandbox(&sandbox_id).await {
            warn!(error = %e, "vercel: sandbox stop failed after snapshot creation");
        }

        Ok(Some(super::types::BuildImageResult {
            tag: snapshot_id,
            built: true,
        }))
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        if self.session_sandbox_id(id).await.is_some() {
            return Ok(());
        }
        let permit = self.creation_permit(id).await;
        let _permit = permit
            .acquire_owned()
            .await
            .map_err(|e| Error::message(format!("vercel: sandbox creation permit closed: {e}")))?;
        if self.session_sandbox_id(id).await.is_some() {
            return Ok(());
        }

        info!(%id, runtime = self.vercel.runtime, "vercel: creating sandbox");

        // Use snapshot if available (from build_image() or config).
        let effective_snapshot = image_override
            .filter(|s| !s.is_empty())
            .or(self.vercel.snapshot_id.as_deref());

        let sandbox_id = if let Some(snap_id) = effective_snapshot {
            // Create from snapshot — packages already installed, much faster.
            debug!(%id, snapshot = snap_id, "vercel: creating from snapshot");
            let mut body = serde_json::json!({
                "source": { "type": "snapshot", "snapshotId": snap_id },
                "runtime": self.vercel.runtime,
                "timeout": self.vercel.timeout_ms,
                "resources": { "vcpus": self.vercel.vcpus },
            });
            if let Some(ref project_id) = self.vercel.project_id {
                body["projectId"] = serde_json::Value::String(project_id.clone());
            }
            let resp = self
                .request(reqwest::Method::POST, "/v1/sandboxes")
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::message(format!("vercel: failed to create sandbox: {e}")))?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(Error::message(format!(
                    "vercel: create from snapshot failed (HTTP {status}): {text}"
                )));
            }
            let location = resp.headers().get(reqwest::header::LOCATION).cloned();
            let text = resp.text().await.map_err(|e| {
                Error::message(format!("vercel: failed to read create response: {e}"))
            })?;
            let data: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| Error::message(format!("vercel: invalid create response: {e}")))?;
            match Self::sandbox_id_from_create_response(&data)
                .or_else(|| Self::sandbox_id_from_location(location.as_ref()))
            {
                Some(sandbox_id) => sandbox_id,
                None => {
                    warn!(
                        %id,
                        "vercel: sandbox created from snapshot but response contained no \
                        parseable sandbox ID; the VM will run until its timeout"
                    );
                    return Err(Error::message(
                        "vercel: missing sandbox.id in snapshot-create response",
                    ));
                },
            }
        } else {
            self.create_sandbox().await?
        };

        self.prepare_created_sandbox(id, &sandbox_id).await?;

        info!(%id, vercel_id = sandbox_id, "vercel: sandbox ready");

        self.active
            .write()
            .await
            .insert(id.key.clone(), VercelSession { sandbox_id });

        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let sandbox_id = self
            .session_sandbox_id(id)
            .await
            .ok_or_else(|| Error::message(format!("vercel: no active sandbox for {id}")))?;

        self.run_command(&sandbox_id, command, opts).await
    }

    async fn read_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        let sandbox_id = self
            .session_sandbox_id(id)
            .await
            .ok_or_else(|| Error::message(format!("vercel: no active sandbox for {id}")))?;

        match self.read_file_raw(&sandbox_id, file_path).await? {
            None => Ok(SandboxReadResult::NotFound),
            Some(bytes) => {
                if bytes.len() as u64 > max_bytes {
                    Ok(SandboxReadResult::TooLarge(bytes.len() as u64))
                } else {
                    Ok(SandboxReadResult::Ok(bytes))
                }
            },
        }
    }

    async fn write_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        let sandbox_id = self
            .session_sandbox_id(id)
            .await
            .ok_or_else(|| Error::message(format!("vercel: no active sandbox for {id}")))?;

        self.write_files_tar(&sandbox_id, &[(file_path, content)])
            .await?;

        Ok(None)
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let permit = self.existing_creation_permit(id).await;
        let _permit = match permit {
            Some(permit) => Some(permit.acquire_owned().await.map_err(|e| {
                Error::message(format!("vercel: sandbox creation permit closed: {e}"))
            })?),
            None => None,
        };
        let session = self.active.write().await.remove(&id.key);
        self.creation_permits.write().await.remove(&id.key);
        if let Some(session) = session {
            debug!(%id, vercel_id = session.sandbox_id, "vercel: stopping sandbox");
            if let Err(e) = self.stop_sandbox(&session.sandbox_id).await {
                warn!(%id, error = %e, "vercel: sandbox stop failed during cleanup");
            }
        }
        Ok(())
    }
}

/// Map a Debian/Ubuntu package name to its Amazon Linux 2023 equivalent.
/// Returns `None` for packages that have no equivalent or are unavailable.
fn debian_to_amzn_package(debian_name: &str) -> Option<&str> {
    // Direct matches (same name on both distros).
    const DIRECT: &[&str] = &[
        "curl",
        "wget",
        "git",
        "jq",
        "rsync",
        "tar",
        "zip",
        "unzip",
        "bzip2",
        "xz",
        "zstd",
        "lz4",
        "cmake",
        "autoconf",
        "automake",
        "libtool",
        "make",
        "gcc",
        "gcc-c++",
        "clang",
        "tmux",
        "sqlite",
        "vim",
        "ImageMagick",
        "ffmpeg",
        "pandoc",
        "gnupg2",
    ];

    // Debian → Amazon Linux name mappings.
    match debian_name {
        // Direct matches
        p if DIRECT.contains(&p) => Some(p),
        // Common renames
        "build-essential" => Some("gcc gcc-c++ make"),
        "ca-certificates" => Some("ca-certificates"),
        "python3" | "python3-dev" => Some("python3"),
        "python3-pip" => Some("python3-pip"),
        "python3-venv" => Some("python3"),
        "python-is-python3" => None, // not needed on AL2023
        "nodejs" => None,            // already available on Vercel's node runtime
        "ruby" | "ruby-dev" => Some("ruby"),
        "golang-go" => Some("golang"),
        "default-jdk" => Some("java-17-amazon-corretto-devel"),
        "openssh-client" => Some("openssh-clients"),
        "iproute2" => Some("iproute"),
        "net-tools" => Some("net-tools"),
        "imagemagick" => Some("ImageMagick"),
        "graphicsmagick" => Some("GraphicsMagick"),
        "sqlite3" => Some("sqlite"),
        "postgresql-client" => Some("postgresql15"),
        "shellcheck" => Some("ShellCheck"),
        "p7zip" | "p7zip-full" => Some("p7zip"),
        // Skip packages that don't exist on Amazon Linux
        "dnsutils" | "netcat-openbsd" | "csvtool" | "datamash" | "miller" | "antiword" | "khal"
        | "vdirsyncer" | "isync" | "notmuch" | "aerc" | "mutt" | "neomutt" | "php-cli"
        | "php-mbstring" | "php-xml" | "php-curl" | "perl" | "maven" | "ninja-build" => None,
        // For anything else, try the name directly (dnf will skip unknown ones)
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_vercel_sandbox_backend_name() {
        let sandbox = VercelSandbox::new(SandboxConfig::default(), VercelSandboxConfig::default());
        assert_eq!(sandbox.backend_name(), "vercel");
        assert!(sandbox.is_real());
        assert!(sandbox.provides_fs_isolation());
        assert!(sandbox.is_isolated());
    }

    #[test]
    fn test_vercel_config_defaults() {
        let config = VercelSandboxConfig::default();
        assert_eq!(config.runtime, "node24");
        assert_eq!(config.vcpus, 2);
        assert_eq!(config.timeout_ms, 300_000);
        assert!(config.project_id.is_none());
        assert!(config.team_id.is_none());
        assert!(config.snapshot_id.is_none());
    }

    #[test]
    fn test_translate_working_dir_preserves_workspace_subdirectory() {
        assert_eq!(
            VercelSandbox::translate_working_dir(Some("/home/sandbox/myproject/src")),
            "/vercel/sandbox/myproject/src"
        );
        assert_eq!(
            VercelSandbox::translate_working_dir(Some("/home/sandbox")),
            "/vercel/sandbox"
        );
        assert_eq!(
            VercelSandbox::translate_working_dir(Some("/tmp/build")),
            "/tmp/build"
        );
        assert_eq!(
            VercelSandbox::translate_working_dir(None),
            "/vercel/sandbox"
        );
    }

    #[test]
    fn test_env_object_includes_exec_env() {
        let env = VercelSandbox::env_object(&[
            ("API_TOKEN".to_string(), "secret-value".to_string()),
            ("SESSION_ID".to_string(), "abc123".to_string()),
        ]);

        assert_eq!(
            env.get("API_TOKEN").and_then(serde_json::Value::as_str),
            Some("secret-value")
        );
        assert_eq!(
            env.get("SESSION_ID").and_then(serde_json::Value::as_str),
            Some("abc123")
        );
    }

    #[test]
    fn test_sandbox_id_from_create_response_accepts_known_shapes() {
        assert_eq!(
            VercelSandbox::sandbox_id_from_create_response(&serde_json::json!({
                "sandbox": { "id": "sb_nested" },
            }))
            .as_deref(),
            Some("sb_nested")
        );
        assert_eq!(
            VercelSandbox::sandbox_id_from_create_response(&serde_json::json!({
                "sandboxId": "sb_top",
            }))
            .as_deref(),
            Some("sb_top")
        );
        assert_eq!(
            VercelSandbox::sandbox_id_from_create_response(&serde_json::json!({
                "sandbox": { "sandbox_id": "sb_snake" },
            }))
            .as_deref(),
            Some("sb_snake")
        );
        assert_eq!(
            VercelSandbox::sandbox_id_from_create_response(&serde_json::json!({
                "sandbox": { "id": "" },
            })),
            None
        );
    }

    #[test]
    fn test_sandbox_id_from_location_header() {
        let location =
            reqwest::header::HeaderValue::from_static("/api/v1/sandboxes/sb_from_location?x=1");
        assert_eq!(
            VercelSandbox::sandbox_id_from_location(Some(&location)).as_deref(),
            Some("sb_from_location")
        );

        let uuid_location = reqwest::header::HeaderValue::from_static(
            "/api/v1/sandboxes/6f9619ff-8b86-d011-b42d-00cf4fc964ff",
        );
        assert_eq!(
            VercelSandbox::sandbox_id_from_location(Some(&uuid_location)).as_deref(),
            Some("6f9619ff-8b86-d011-b42d-00cf4fc964ff")
        );

        let invalid = reqwest::header::HeaderValue::from_static("/api/v1/sandboxes/");
        assert_eq!(
            VercelSandbox::sandbox_id_from_location(Some(&invalid)),
            None
        );
    }

    #[test]
    fn test_snapshot_id_from_response_accepts_known_shapes() {
        assert_eq!(
            VercelSandbox::snapshot_id_from_response(&serde_json::json!({
                "snapshot": { "id": "snap_nested" },
            }))
            .as_deref(),
            Some("snap_nested")
        );
        assert_eq!(
            VercelSandbox::snapshot_id_from_response(&serde_json::json!({
                "snapshot": { "snapshotId": "snap_camel" },
            }))
            .as_deref(),
            Some("snap_camel")
        );
        assert_eq!(
            VercelSandbox::snapshot_id_from_response(&serde_json::json!({
                "snapshot_id": "snap_snake",
            }))
            .as_deref(),
            Some("snap_snake")
        );
        assert_eq!(
            VercelSandbox::snapshot_id_from_response(&serde_json::json!({
                "snapshot": { "snapshotId": "" },
            })),
            None
        );
    }

    #[test]
    fn test_parse_command_events_uses_inline_output_without_command_id() {
        let events =
            VercelSandbox::parse_command_events(r#"{"stream":"stdout","data":"hello\n"}"#, 1024);

        assert_eq!(events.command_id, None);
        assert_eq!(events.exit_code, None);
        assert_eq!(events.stdout, "hello\n");
        assert_eq!(events.stderr, "");
    }

    #[test]
    fn test_parse_command_events_accepts_result_output_and_exit_code() {
        let events = VercelSandbox::parse_command_events(
            r#"{"exitCode":7,"result":{"stdout":"ok","stderr":"bad"}}"#,
            1024,
        );

        assert_eq!(events.command_id, None);
        assert_eq!(events.exit_code, Some(7));
        assert_eq!(events.stdout, "ok");
        assert_eq!(events.stderr, "bad");
    }

    #[test]
    fn test_parse_command_events_accepts_command_id_and_truncates() {
        let events = VercelSandbox::parse_command_events(
            r#"{"command":{"commandId":"cmd_123","exit_code":0},"output":"abcdef"}"#,
            3,
        );

        assert_eq!(events.command_id.as_deref(), Some("cmd_123"));
        assert_eq!(events.exit_code, Some(0));
        assert_eq!(events.stdout, "abc");
    }

    #[test]
    fn test_command_output_from_logs_prefers_non_empty_logs() {
        let events = VercelCommandEvents {
            stdout: "inline".into(),
            stderr: "inline-err".into(),
            ..Default::default()
        };

        let output = VercelSandbox::command_output_from_logs(
            events,
            Ok(("logs".into(), String::new())),
            "sb_test",
        );

        assert_eq!(output, ("logs".into(), String::new()));
    }

    #[test]
    fn test_command_output_from_logs_falls_back_on_empty_logs() {
        let events = VercelCommandEvents {
            stdout: "inline".into(),
            stderr: "inline-err".into(),
            ..Default::default()
        };

        let output = VercelSandbox::command_output_from_logs(
            events,
            Ok((String::new(), String::new())),
            "sb_test",
        );

        assert_eq!(output, ("inline".into(), "inline-err".into()));
    }

    #[test]
    fn test_command_output_from_logs_falls_back_on_fetch_error() {
        let events = VercelCommandEvents {
            stdout: "inline".into(),
            stderr: "inline-err".into(),
            ..Default::default()
        };

        let output = VercelSandbox::command_output_from_logs(
            events,
            Err(Error::message("logs unavailable")),
            "sb_test",
        );

        assert_eq!(output, ("inline".into(), "inline-err".into()));
    }

    #[test]
    fn test_gzip_tar_roundtrip() {
        let files: Vec<(&str, &[u8])> = vec![
            ("/tmp/test.txt", b"hello world"),
            ("/tmp/dir/nested.txt", b"nested content"),
        ];

        let gz_bytes = {
            let buf = Vec::new();
            let enc = GzEncoder::new(buf, Compression::fast());
            let mut ar = tar::Builder::new(enc);

            for &(path, content) in &files {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                ar.append_data(&mut header, path.trim_start_matches('/'), content)
                    .unwrap();
            }

            ar.into_inner().and_then(|enc| enc.finish()).unwrap()
        };

        // Verify it's valid gzip by decompressing.
        use {flate2::read::GzDecoder, std::io::Read};
        let mut decoder = GzDecoder::new(&gz_bytes[..]);
        let mut tar_bytes = Vec::new();
        decoder.read_to_end(&mut tar_bytes).unwrap();

        let mut archive = tar::Archive::new(&tar_bytes[..]);
        let entries: Vec<_> = archive.entries().unwrap().collect();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_no_active_sandbox_returns_error() {
        let sandbox = VercelSandbox::new(SandboxConfig::default(), VercelSandboxConfig::default());
        let id = SandboxId {
            scope: crate::sandbox::types::SandboxScope::Session,
            key: "test".into(),
        };
        let opts = ExecOpts::default();
        let result = sandbox.exec(&id, "echo hello", &opts).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no active sandbox")
        );
    }
}
