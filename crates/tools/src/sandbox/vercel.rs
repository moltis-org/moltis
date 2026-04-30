//! Vercel Sandbox backend — Firecracker microVM via the Vercel API.
//!
//! Each session gets an ephemeral Vercel sandbox. Commands run via the
//! REST API, files transfer via gzipped tar uploads and raw reads. The
//! sandbox is stopped on cleanup.
//!
//! Requires `VERCEL_TOKEN` (or `VERCEL_OIDC_TOKEN`) and a Vercel project.

use std::{collections::HashMap, time::Duration};

use {
    async_trait::async_trait,
    flate2::{Compression, write::GzEncoder},
    secrecy::{ExposeSecret, Secret},
    tokio::sync::RwLock,
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

/// Default timeout for sandbox creation (5 minutes).
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

/// State of a live Vercel sandbox session.
struct VercelSession {
    sandbox_id: String,
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
        let mut body = serde_json::json!({
            "runtime": self.vercel.runtime,
            "timeout": self.vercel.timeout_ms,
            "resources": { "vcpus": self.vercel.vcpus },
        });

        if let Some(ref project_id) = self.vercel.project_id {
            body["projectId"] = serde_json::Value::String(project_id.clone());
        }

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

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::message(format!("vercel: invalid create response: {e}")))?;

        data["sandbox"]["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Error::message("vercel: missing sandbox.id in create response"))
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

    /// Run a command and wait for completion via NDJSON streaming.
    async fn run_command(
        &self,
        sandbox_id: &str,
        command: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        let body = serde_json::json!({
            "command": "sh",
            "args": ["-c", command],
            "cwd": opts.working_dir.as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or(VERCEL_WORKSPACE),
            "wait": true,
        });

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

        let mut exit_code: i32 = -1;
        let mut cmd_id = String::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(code) = v["command"]["exitCode"].as_i64() {
                    exit_code = code as i32;
                }
                if let Some(id) = v["command"]["id"].as_str() {
                    cmd_id = id.to_string();
                }
            }
        }

        // Fetch stdout/stderr logs for the command.
        let (stdout, stderr) = self.fetch_command_logs(sandbox_id, &cmd_id, opts).await?;

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
        if cmd_id.is_empty() {
            return Ok((String::new(), String::new()));
        }

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

        stdout.truncate(opts.max_output_bytes);
        stderr.truncate(opts.max_output_bytes);

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

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        if self.session_sandbox_id(id).await.is_some() {
            return Ok(());
        }

        info!(%id, runtime = self.vercel.runtime, "vercel: creating sandbox");

        let sandbox_id = self.create_sandbox().await?;
        debug!(%id, vercel_id = sandbox_id, "vercel: sandbox created, waiting for running state");

        self.wait_for_running(&sandbox_id).await?;
        self.mkdir(&sandbox_id, VERCEL_WORKSPACE).await?;

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
        let session = self.active.write().await.remove(&id.key);
        if let Some(session) = session {
            debug!(%id, vercel_id = session.sandbox_id, "vercel: stopping sandbox");
            if let Err(e) = self.stop_sandbox(&session.sandbox_id).await {
                warn!(%id, error = %e, "vercel: sandbox stop failed during cleanup");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
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
