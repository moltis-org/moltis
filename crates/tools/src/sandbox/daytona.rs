//! Daytona Sandbox backend — cloud sandboxes via the Daytona API.
//!
//! Each session gets an ephemeral Daytona sandbox. Commands run via the
//! toolbox REST API, files transfer via multipart upload and raw download.
//! The sandbox is deleted on cleanup.
//!
//! Requires `DAYTONA_API_KEY` and optionally `DAYTONA_API_URL`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    async_trait::async_trait,
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

/// Default Daytona API URL.
const DEFAULT_API_URL: &str = "https://app.daytona.io/api";

/// Default workspace directory inside Daytona sandboxes.
const DAYTONA_WORKSPACE: &str = "/home/daytona";

/// Generic workspace path used by the shared sandbox tool contract.
const GENERIC_WORKSPACE: &str = "/home/sandbox";
const GENERIC_WORKSPACE_PREFIX: &str = "/home/sandbox/";

/// State of a live Daytona sandbox session.
struct DaytonaSession {
    sandbox_id: String,
    workspace_dir: String,
}

/// Daytona Sandbox backend configuration.
#[derive(Debug, Clone)]
pub struct DaytonaSandboxConfig {
    pub api_key: Secret<String>,
    pub api_url: String,
    pub target: Option<String>,
    pub image: Option<String>,
    pub language: Option<String>,
}

impl Default for DaytonaSandboxConfig {
    fn default() -> Self {
        Self {
            api_key: Secret::new(String::new()),
            api_url: DEFAULT_API_URL.into(),
            target: None,
            image: None,
            language: None,
        }
    }
}

/// Daytona Sandbox backend.
pub struct DaytonaSandbox {
    #[allow(dead_code)]
    config: SandboxConfig,
    daytona: DaytonaSandboxConfig,
    client: reqwest::Client,
    active: RwLock<HashMap<String, DaytonaSession>>,
    creation_permits: RwLock<HashMap<String, Arc<Semaphore>>>,
}

impl DaytonaSandbox {
    pub fn new(config: SandboxConfig, daytona: DaytonaSandboxConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();
        Self {
            config,
            daytona,
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

    fn env_object(env: &[(String, String)]) -> serde_json::Map<String, serde_json::Value> {
        env.iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect()
    }

    fn wrapped_command(command: &str, stderr_file: &str) -> String {
        use base64::Engine;

        let encoded = base64::engine::general_purpose::STANDARD.encode(command.as_bytes());
        let encoded = shell_words::quote(&encoded);
        let stderr_file = shell_words::quote(stderr_file);

        format!(
            "decoded=$(mktemp /tmp/moltis-cmd.XXXXXX) || exit 125; \
             printf %s {encoded} | base64 -d > \"$decoded\"; status=$?; \
             if [ \"$status\" -ne 0 ]; then rm -f \"$decoded\"; exit \"$status\"; fi; \
             sh \"$decoded\" 2>{stderr_file}; status=$?; \
             rm -f \"$decoded\"; exit \"$status\""
        )
    }

    /// Build an authenticated request.
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{path}", self.daytona.api_url);
        self.client
            .request(method, &url)
            .bearer_auth(self.daytona.api_key.expose_secret())
            .header("X-Daytona-Source", "moltis")
    }

    /// Create a Daytona sandbox, returning (sandbox_id, workspace_dir).
    async fn create_sandbox(&self) -> Result<(String, String)> {
        let mut body = serde_json::json!({});

        if let Some(ref image) = self.daytona.image {
            body["image"] = serde_json::Value::String(image.clone());
        }
        if let Some(ref target) = self.daytona.target {
            body["target"] = serde_json::Value::String(target.clone());
        }
        if let Some(ref lang) = self.daytona.language {
            body["labels"] = serde_json::json!({
                "code-toolbox-language": lang,
            });
        }

        let resp = self
            .request(reqwest::Method::POST, "/sandbox")
            .timeout(Duration::from_secs(120))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("daytona: failed to create sandbox: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "daytona: create sandbox failed (HTTP {status}): {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::message(format!("daytona: invalid create response: {e}")))?;

        let sandbox_id = data["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Error::message("daytona: missing id in create response"))?;

        // Try to get the workspace directory from the response.
        let workspace_dir = data["info"]["projectDir"]
            .as_str()
            .or_else(|| data["info"]["homeDir"].as_str())
            .map(String::from)
            .unwrap_or_else(|| DAYTONA_WORKSPACE.to_string());

        Ok((sandbox_id, workspace_dir))
    }

    /// Run a command via the toolbox API.
    async fn run_command(
        &self,
        sandbox_id: &str,
        command: &str,
        cwd: &str,
        opts: &ExecOpts,
    ) -> Result<ExecResult> {
        // The Daytona toolbox API combines stdout and stderr in the `result`
        // field. To separate them, wrap the command to redirect stderr to a
        // temp file, then read it back in a second call.
        let stderr_file = format!("/tmp/moltis-stderr-{}", uuid::Uuid::new_v4());
        // Base64-encode the command to avoid any shell metacharacter issues
        // (braces, parentheses, quotes, etc.) in the wrapper.
        let wrapped = Self::wrapped_command(command, &stderr_file);
        let timeout_secs = opts.timeout.as_secs().max(1);
        let mut body = serde_json::json!({
            "command": wrapped,
            "cwd": cwd,
            "timeout": timeout_secs,
        });
        if !opts.env.is_empty() {
            body["env"] = serde_json::Value::Object(Self::env_object(&opts.env));
        }

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/toolbox/{sandbox_id}/toolbox/process/execute"),
            )
            .timeout(opts.timeout + Duration::from_secs(10))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::message(format!("daytona: command request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "daytona: command failed (HTTP {status}): {text}"
            )));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::message(format!("daytona: invalid command response: {e}")))?;

        let exit_code = data["exitCode"].as_i64().unwrap_or(-1) as i32;
        let mut stdout = data["result"].as_str().unwrap_or("").to_string();

        // Retrieve stderr from the temp file.
        let stderr_body = serde_json::json!({
            "command": format!("cat {stderr_file} 2>/dev/null; rm -f {stderr_file}"),
            "cwd": "/",
            "timeout": 5,
        });
        let mut stderr = String::new();
        if let Ok(resp) = self
            .request(
                reqwest::Method::POST,
                &format!("/toolbox/{sandbox_id}/toolbox/process/execute"),
            )
            .timeout(Duration::from_secs(10))
            .json(&stderr_body)
            .send()
            .await
            && let Ok(data) = resp.json::<serde_json::Value>().await
        {
            stderr = data["result"].as_str().unwrap_or("").to_string();
        }

        stdout.truncate(stdout.floor_char_boundary(opts.max_output_bytes));
        stderr.truncate(stderr.floor_char_boundary(opts.max_output_bytes));

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Upload a file to the sandbox via the toolbox API.
    async fn upload_file(&self, sandbox_id: &str, path: &str, content: &[u8]) -> Result<()> {
        let file_name: String = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let part = reqwest::multipart::Part::bytes(content.to_vec())
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|e| Error::message(format!("daytona: mime error: {e}")))?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/toolbox/{sandbox_id}/toolbox/files/upload"),
            )
            .query(&[("path", path)])
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::message(format!("daytona: file upload failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "daytona: file upload failed: {text}"
            )));
        }

        Ok(())
    }

    /// Download a file from the sandbox via the toolbox API.
    async fn download_file(&self, sandbox_id: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let resp = self
            .request(
                reqwest::Method::GET,
                &format!("/toolbox/{sandbox_id}/toolbox/files/download"),
            )
            .query(&[("path", path)])
            .send()
            .await
            .map_err(|e| Error::message(format!("daytona: file download failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "daytona: file download failed: {text}"
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::message(format!("daytona: failed to read file bytes: {e}")))?;

        Ok(Some(bytes.to_vec()))
    }

    /// Delete a sandbox.
    async fn delete_sandbox(&self, sandbox_id: &str) -> Result<()> {
        let resp = self
            .request(reqwest::Method::DELETE, &format!("/sandbox/{sandbox_id}"))
            .send()
            .await
            .map_err(|e| Error::message(format!("daytona: delete request failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            if !text.contains("not found") {
                return Err(Error::message(format!(
                    "daytona: delete sandbox failed: {text}"
                )));
            }
        }

        Ok(())
    }

    /// Get the session state for a sandbox, or None.
    async fn session_state(&self, id: &SandboxId) -> Option<(String, String)> {
        self.active
            .read()
            .await
            .get(&id.key)
            .map(|s| (s.sandbox_id.clone(), s.workspace_dir.clone()))
    }
}

#[async_trait]
impl Sandbox for DaytonaSandbox {
    fn backend_name(&self) -> &'static str {
        "daytona"
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
        DAYTONA_WORKSPACE
    }

    async fn workspace_dir_for(&self, id: &SandboxId) -> String {
        self.session_state(id)
            .await
            .map(|(_, workspace_dir)| workspace_dir)
            .unwrap_or_else(|| DAYTONA_WORKSPACE.to_string())
    }

    async fn ensure_ready(&self, id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        if self.session_state(id).await.is_some() {
            return Ok(());
        }
        let permit = self.creation_permit(id).await;
        let _permit = permit
            .acquire_owned()
            .await
            .map_err(|e| Error::message(format!("daytona: sandbox creation permit closed: {e}")))?;
        if self.session_state(id).await.is_some() {
            return Ok(());
        }

        info!(%id, "daytona: creating sandbox");

        let (sandbox_id, workspace_dir) = self.create_sandbox().await?;

        info!(%id, daytona_id = sandbox_id, workspace = workspace_dir, "daytona: sandbox ready");

        self.active
            .write()
            .await
            .insert(id.key.clone(), DaytonaSession {
                sandbox_id,
                workspace_dir,
            });

        Ok(())
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let (sandbox_id, workspace_dir) = self
            .session_state(id)
            .await
            .ok_or_else(|| Error::message(format!("daytona: no active sandbox for {id}")))?;

        // Map the generic /home/sandbox to the actual Daytona workspace dir.
        let cwd = Self::translate_working_dir(
            opts.working_dir.as_ref().and_then(|p| p.to_str()),
            &workspace_dir,
        );

        self.run_command(&sandbox_id, command, &cwd, opts).await
    }

    async fn read_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        let (sandbox_id, _) = self
            .session_state(id)
            .await
            .ok_or_else(|| Error::message(format!("daytona: no active sandbox for {id}")))?;

        match self.download_file(&sandbox_id, file_path).await? {
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
        let (sandbox_id, _) = self
            .session_state(id)
            .await
            .ok_or_else(|| Error::message(format!("daytona: no active sandbox for {id}")))?;

        // Ensure parent directory exists.
        if let Some(parent) = std::path::Path::new(file_path).parent()
            && let Some(parent_str) = parent.to_str()
        {
            let mkdir_opts = ExecOpts {
                timeout: Duration::from_secs(10),
                ..Default::default()
            };
            let _ = self
                .run_command(
                    &sandbox_id,
                    &format!("mkdir -p '{}'", parent_str.replace('\'', "'\\''")),
                    "/",
                    &mkdir_opts,
                )
                .await;
        }

        self.upload_file(&sandbox_id, file_path, content).await?;

        Ok(None)
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let permit = self.existing_creation_permit(id).await;
        let _permit = match permit {
            Some(permit) => Some(permit.acquire_owned().await.map_err(|e| {
                Error::message(format!("daytona: sandbox creation permit closed: {e}"))
            })?),
            None => None,
        };
        let session = self.active.write().await.remove(&id.key);
        self.creation_permits.write().await.remove(&id.key);
        if let Some(session) = session {
            debug!(%id, daytona_id = session.sandbox_id, "daytona: deleting sandbox");
            if let Err(e) = self.delete_sandbox(&session.sandbox_id).await {
                warn!(%id, error = %e, "daytona: sandbox deletion failed during cleanup");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_daytona_sandbox_backend_name() {
        let sandbox =
            DaytonaSandbox::new(SandboxConfig::default(), DaytonaSandboxConfig::default());
        assert_eq!(sandbox.backend_name(), "daytona");
        assert!(sandbox.is_real());
        assert!(sandbox.provides_fs_isolation());
        assert!(sandbox.is_isolated());
    }

    #[test]
    fn test_daytona_config_defaults() {
        let config = DaytonaSandboxConfig::default();
        assert_eq!(config.api_url, "https://app.daytona.io/api");
        assert!(config.target.is_none());
        assert!(config.image.is_none());
        assert!(config.language.is_none());
    }

    #[test]
    fn test_translate_working_dir_preserves_workspace_subdirectory() {
        assert_eq!(
            DaytonaSandbox::translate_working_dir(
                Some("/home/sandbox/myproject/src"),
                "/workspace/custom",
            ),
            "/workspace/custom/myproject/src"
        );
        assert_eq!(
            DaytonaSandbox::translate_working_dir(Some("/home/sandbox"), "/workspace/custom"),
            "/workspace/custom"
        );
        assert_eq!(
            DaytonaSandbox::translate_working_dir(Some("/tmp/build"), "/workspace/custom"),
            "/tmp/build"
        );
        assert_eq!(
            DaytonaSandbox::translate_working_dir(None, "/workspace/custom"),
            "/workspace/custom"
        );
    }

    #[test]
    fn test_env_object_includes_exec_env() {
        let env = DaytonaSandbox::env_object(&[
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
    fn test_wrapped_command_guards_base64_decode_failure() {
        let wrapped = DaytonaSandbox::wrapped_command("echo '; } weird'", "/tmp/stderr.log");

        assert!(wrapped.contains("base64 -d > \"$decoded\"; status=$?;"));
        assert!(wrapped.contains("if [ \"$status\" -ne 0 ]; then"));
        assert!(wrapped.contains("sh \"$decoded\" 2>/tmp/stderr.log"));
        assert!(!wrapped.contains("eval"));
    }

    #[tokio::test]
    async fn test_no_active_sandbox_returns_error() {
        let sandbox =
            DaytonaSandbox::new(SandboxConfig::default(), DaytonaSandboxConfig::default());
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
