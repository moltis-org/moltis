use std::sync::atomic::Ordering;

use {
    axum::{
        Json,
        extract::{Path, State},
        http::StatusCode,
        response::{IntoResponse, Response},
    },
    secrecy::{ExposeSecret, SecretString},
    serde::Serialize,
    tokio::process::Command,
};

use moltis_gateway::{
    auth::{SshAuthMode, SshKeyEntry, SshResolvedTarget, SshTargetEntry},
    node_exec::exec_resolved_ssh_target,
};

const SSH_STORE_UNAVAILABLE: &str = "SSH_STORE_UNAVAILABLE";
const SSH_KEY_NAME_REQUIRED: &str = "SSH_KEY_NAME_REQUIRED";
const SSH_PRIVATE_KEY_REQUIRED: &str = "SSH_PRIVATE_KEY_REQUIRED";
const SSH_TARGET_LABEL_REQUIRED: &str = "SSH_TARGET_LABEL_REQUIRED";
const SSH_TARGET_REQUIRED: &str = "SSH_TARGET_REQUIRED";
const SSH_LIST_FAILED: &str = "SSH_LIST_FAILED";
const SSH_KEY_GENERATE_FAILED: &str = "SSH_KEY_GENERATE_FAILED";
const SSH_KEY_IMPORT_FAILED: &str = "SSH_KEY_IMPORT_FAILED";
const SSH_KEY_DELETE_FAILED: &str = "SSH_KEY_DELETE_FAILED";
const SSH_TARGET_CREATE_FAILED: &str = "SSH_TARGET_CREATE_FAILED";
const SSH_TARGET_DELETE_FAILED: &str = "SSH_TARGET_DELETE_FAILED";
const SSH_TARGET_DEFAULT_FAILED: &str = "SSH_TARGET_DEFAULT_FAILED";
const SSH_TARGET_TEST_FAILED: &str = "SSH_TARGET_TEST_FAILED";
const SSH_HOST_SCAN_FAILED: &str = "SSH_HOST_SCAN_FAILED";
const SSH_HOST_PIN_FAILED: &str = "SSH_HOST_PIN_FAILED";
const SSH_HOST_PIN_CLEAR_FAILED: &str = "SSH_HOST_PIN_CLEAR_FAILED";

fn validate_ssh_target_value(target: &str) -> Result<&str, ApiError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(ApiError::bad_request(
            SSH_TARGET_REQUIRED,
            "target is required",
        ));
    }
    if target.starts_with('-') {
        return Err(ApiError::bad_request(
            SSH_TARGET_REQUIRED,
            "target must be a user@host or hostname, not an ssh option",
        ));
    }
    Ok(target)
}

#[derive(Serialize)]
pub struct SshStatusResponse {
    keys: Vec<SshKeyEntry>,
    targets: Vec<SshTargetEntry>,
}

impl IntoResponse for SshStatusResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Serialize)]
pub struct SshMutationResponse {
    ok: bool,
    id: Option<i64>,
}

impl SshMutationResponse {
    fn success(id: Option<i64>) -> Self {
        Self { ok: true, id }
    }
}

impl IntoResponse for SshMutationResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Serialize)]
pub struct SshHostScanResponse {
    ok: bool,
    host: String,
    port: Option<u16>,
    known_host: String,
}

impl IntoResponse for SshHostScanResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Serialize)]
pub struct SshTestResponse {
    ok: bool,
    reachable: bool,
    stdout: String,
    stderr: String,
    exit_code: i32,
    route_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_hint: Option<String>,
}

impl IntoResponse for SshTestResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Clone, Serialize)]
pub struct SshDoctorCheck {
    id: &'static str,
    level: &'static str,
    title: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct SshDoctorRoute {
    #[serde(skip_serializing_if = "Option::is_none")]
    target_id: Option<i64>,
    label: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    host_pinned: bool,
    auth_mode: &'static str,
    source: &'static str,
}

#[derive(Serialize)]
pub struct SshDoctorResponse {
    ok: bool,
    exec_host: String,
    ssh_binary_available: bool,
    ssh_binary_version: Option<String>,
    paired_node_count: usize,
    managed_key_count: usize,
    encrypted_key_count: usize,
    managed_target_count: usize,
    pinned_target_count: usize,
    configured_node: Option<String>,
    legacy_target: Option<String>,
    active_route: Option<SshDoctorRoute>,
    checks: Vec<SshDoctorCheck>,
}

impl IntoResponse for SshDoctorResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn service_unavailable(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: message.into(),
        }
    }

    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    fn internal(code: &'static str, err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            code: &'static str,
            error: String,
        }

        (
            self.status,
            Json(Body {
                code: self.code,
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(serde::Deserialize)]
pub struct GenerateKeyRequest {
    name: String,
}

#[derive(serde::Deserialize)]
pub struct ImportKeyRequest {
    name: String,
    private_key: SecretString,
    passphrase: Option<SecretString>,
}

#[derive(serde::Deserialize)]
pub struct CreateTargetRequest {
    label: String,
    target: String,
    port: Option<u16>,
    known_host: Option<String>,
    auth_mode: SshAuthMode,
    key_id: Option<i64>,
    #[serde(default)]
    is_default: bool,
}

#[derive(serde::Deserialize)]
pub struct ScanHostRequest {
    target: String,
    port: Option<u16>,
}

#[derive(serde::Deserialize)]
pub struct PinHostRequest {
    known_host: String,
}

pub async fn ssh_status(
    State(state): State<crate::server::AppState>,
) -> Result<SshStatusResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let keys = store
        .list_ssh_keys()
        .await
        .map_err(|err| ApiError::internal(SSH_LIST_FAILED, err))?;
    let targets = store
        .list_ssh_targets()
        .await
        .map_err(|err| ApiError::internal(SSH_LIST_FAILED, err))?;
    Ok(SshStatusResponse { keys, targets })
}

pub async fn ssh_generate_key(
    State(state): State<crate::server::AppState>,
    Json(body): Json<GenerateKeyRequest>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request(
            SSH_KEY_NAME_REQUIRED,
            "ssh key name is required",
        ));
    }

    let (private_key, public_key, fingerprint) = generate_ssh_key_material(name)
        .await
        .map_err(|err| ApiError::internal(SSH_KEY_GENERATE_FAILED, err))?;
    let id = store
        .create_ssh_key(name, private_key.expose_secret(), &public_key, &fingerprint)
        .await
        .map_err(|err| ApiError::internal(SSH_KEY_GENERATE_FAILED, err))?;

    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_import_key(
    State(state): State<crate::server::AppState>,
    Json(body): Json<ImportKeyRequest>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request(
            SSH_KEY_NAME_REQUIRED,
            "ssh key name is required",
        ));
    }
    if body.private_key.expose_secret().trim().is_empty() {
        return Err(ApiError::bad_request(
            SSH_PRIVATE_KEY_REQUIRED,
            "private key is required",
        ));
    }

    let import_passphrase = body
        .passphrase
        .as_ref()
        .filter(|value| !value.expose_secret().trim().is_empty());
    let (private_key, public_key, fingerprint) =
        inspect_imported_private_key(&body.private_key, import_passphrase)
            .await
            .map_err(|err| ApiError::bad_request(SSH_KEY_IMPORT_FAILED, err.to_string()))?;
    let id = store
        .create_ssh_key(name, private_key.expose_secret(), &public_key, &fingerprint)
        .await
        .map_err(|err| ApiError::internal(SSH_KEY_IMPORT_FAILED, err))?;

    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_delete_key(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    store
        .delete_ssh_key(id)
        .await
        .map_err(|err| ApiError::bad_request(SSH_KEY_DELETE_FAILED, err.to_string()))?;
    Ok(SshMutationResponse::success(None))
}

pub async fn ssh_create_target(
    State(state): State<crate::server::AppState>,
    Json(body): Json<CreateTargetRequest>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    if body.label.trim().is_empty() {
        return Err(ApiError::bad_request(
            SSH_TARGET_LABEL_REQUIRED,
            "target label is required",
        ));
    }
    let target = validate_ssh_target_value(&body.target)?;
    let known_host = body
        .known_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(known_host) = known_host {
        validate_known_host_entry(known_host)
            .await
            .map_err(|err| ApiError::bad_request(SSH_TARGET_CREATE_FAILED, err.to_string()))?;
    }

    let id = store
        .create_ssh_target(
            &body.label,
            target,
            body.port,
            known_host,
            body.auth_mode,
            body.key_id,
            body.is_default,
        )
        .await
        .map_err(|err| ApiError::bad_request(SSH_TARGET_CREATE_FAILED, err.to_string()))?;
    refresh_ssh_target_count(&state).await;

    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_delete_target(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    store
        .delete_ssh_target(id)
        .await
        .map_err(|err| ApiError::internal(SSH_TARGET_DELETE_FAILED, err))?;
    refresh_ssh_target_count(&state).await;

    Ok(SshMutationResponse::success(None))
}

pub async fn ssh_set_default_target(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    store
        .set_default_ssh_target(id)
        .await
        .map_err(|err| ApiError::bad_request(SSH_TARGET_DEFAULT_FAILED, err.to_string()))?;
    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_test_target(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<SshTestResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let target = store
        .resolve_ssh_target_by_id(id)
        .await
        .map_err(|err| ApiError::internal(SSH_TARGET_TEST_FAILED, err))?
        .ok_or_else(|| ApiError::bad_request(SSH_TARGET_TEST_FAILED, "ssh target not found"))?;

    let probe = "__moltis_ssh_probe__";
    let result = exec_resolved_ssh_target(
        store,
        &target,
        &format!("printf '%s' {probe}"),
        10,
        None,
        None,
        8 * 1024,
    )
    .await;

    Ok(build_ssh_test_response(Some(target.label), probe, result))
}

pub async fn ssh_scan_host_key(
    Json(body): Json<ScanHostRequest>,
) -> Result<SshHostScanResponse, ApiError> {
    let target = validate_ssh_target_value(&body.target)?;
    let scan = scan_target_known_host(target, body.port)
        .await
        .map_err(|err| ApiError::bad_request(SSH_HOST_SCAN_FAILED, err.to_string()))?;
    Ok(SshHostScanResponse {
        ok: true,
        host: scan.host,
        port: scan.port,
        known_host: scan.known_host,
    })
}

pub async fn ssh_pin_target_host_key(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
    Json(body): Json<PinHostRequest>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;
    let known_host = body.known_host.trim();
    if known_host.is_empty() {
        return Err(ApiError::bad_request(
            SSH_HOST_PIN_FAILED,
            "known host entry is required",
        ));
    }
    validate_known_host_entry(known_host)
        .await
        .map_err(|err| ApiError::bad_request(SSH_HOST_PIN_FAILED, err.to_string()))?;
    store
        .update_ssh_target_known_host(id, Some(known_host))
        .await
        .map_err(|err| ApiError::bad_request(SSH_HOST_PIN_FAILED, err.to_string()))?;
    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_clear_target_host_key(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<SshMutationResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;
    store
        .update_ssh_target_known_host(id, None)
        .await
        .map_err(|err| ApiError::bad_request(SSH_HOST_PIN_CLEAR_FAILED, err.to_string()))?;
    Ok(SshMutationResponse::success(Some(id)))
}

pub async fn ssh_doctor(
    State(state): State<crate::server::AppState>,
) -> Result<SshDoctorResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let keys = store
        .list_ssh_keys()
        .await
        .map_err(|err| ApiError::internal(SSH_LIST_FAILED, err))?;
    let targets = store
        .list_ssh_targets()
        .await
        .map_err(|err| ApiError::internal(SSH_LIST_FAILED, err))?;

    let config = moltis_config::discover_and_load();
    let exec_host = config.tools.exec.host.trim().to_string();
    let configured_node = config
        .tools
        .exec
        .node
        .clone()
        .filter(|value: &String| !value.trim().is_empty());
    let legacy_target = config
        .tools
        .exec
        .ssh_target
        .clone()
        .filter(|value: &String| !value.trim().is_empty());
    let default_target = targets.iter().find(|target| target.is_default).cloned();
    let (ssh_binary_available, ssh_binary_version) = detect_ssh_binary().await;
    let paired_node_count = {
        let inner = state.gateway.inner.read().await;
        inner.nodes.list().len()
    };
    let encrypted_key_count = keys.iter().filter(|entry| entry.encrypted).count();
    let pinned_target_count = targets
        .iter()
        .filter(|target| target.known_host.is_some())
        .count();
    #[cfg(feature = "vault")]
    let vault_is_unsealed = match state.gateway.vault.as_ref() {
        Some(vault) => vault.is_unsealed().await,
        None => false,
    };
    #[cfg(not(feature = "vault"))]
    let vault_is_unsealed = false;

    let active_route = if exec_host == "ssh" {
        default_target
            .as_ref()
            .map(|target| SshDoctorRoute {
                target_id: Some(target.id),
                label: format!("SSH: {}", target.label),
                target: target.target.clone(),
                port: target.port,
                host_pinned: target.known_host.is_some(),
                auth_mode: match target.auth_mode {
                    SshAuthMode::Managed => "managed",
                    SshAuthMode::System => "system",
                },
                source: "managed",
            })
            .or_else(|| {
                legacy_target
                    .as_ref()
                    .map(|target: &String| SshDoctorRoute {
                        target_id: None,
                        label: format!("SSH: {target}"),
                        target: target.clone(),
                        port: None,
                        host_pinned: false,
                        auth_mode: "system",
                        source: "legacy_config",
                    })
            })
    } else {
        None
    };

    let checks = build_doctor_checks(DoctorInputs {
        exec_host: &exec_host,
        ssh_binary_available,
        paired_node_count,
        managed_target_count: targets.len(),
        pinned_target_count,
        managed_key_count: keys.len(),
        encrypted_key_count,
        configured_node: configured_node.as_deref(),
        legacy_target: legacy_target.as_deref(),
        default_target: default_target.as_ref(),
        vault_is_unsealed,
    });

    Ok(SshDoctorResponse {
        ok: true,
        exec_host,
        ssh_binary_available,
        ssh_binary_version,
        paired_node_count,
        managed_key_count: keys.len(),
        encrypted_key_count,
        managed_target_count: targets.len(),
        pinned_target_count,
        configured_node,
        legacy_target,
        active_route,
        checks,
    })
}

pub async fn ssh_doctor_test_active(
    State(state): State<crate::server::AppState>,
) -> Result<SshTestResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(SSH_STORE_UNAVAILABLE, "no credential store")
    })?;

    let config = moltis_config::discover_and_load();
    if config.tools.exec.host.trim() != "ssh" {
        return Err(ApiError::bad_request(
            SSH_TARGET_TEST_FAILED,
            "remote exec is not configured to use ssh",
        ));
    }

    let route = if let Some(target) = store
        .get_default_ssh_target()
        .await
        .map_err(|err| ApiError::internal(SSH_TARGET_TEST_FAILED, err))?
    {
        target
    } else if let Some(target) = config
        .tools
        .exec
        .ssh_target
        .clone()
        .filter(|value: &String| !value.trim().is_empty())
    {
        SshResolvedTarget {
            id: 0,
            node_id: format!("ssh:{target}"),
            label: target.clone(),
            target,
            port: None,
            known_host: None,
            auth_mode: SshAuthMode::System,
            key_id: None,
            key_name: None,
        }
    } else {
        return Err(ApiError::bad_request(
            SSH_TARGET_TEST_FAILED,
            "no active ssh route is configured",
        ));
    };

    let probe = "__moltis_ssh_probe__";
    let result = exec_resolved_ssh_target(
        store,
        &route,
        &format!("printf '%s' {probe}"),
        10,
        None,
        None,
        8 * 1024,
    )
    .await;

    Ok(build_ssh_test_response(Some(route.label), probe, result))
}

async fn refresh_ssh_target_count(state: &crate::server::AppState) {
    let Some(store) = state.gateway.credential_store.as_ref() else {
        return;
    };
    match store.ssh_target_count().await {
        Ok(count) => state
            .gateway
            .ssh_target_count
            .store(count, Ordering::Relaxed),
        Err(error) => tracing::warn!(%error, "failed to refresh ssh target count"),
    }
}

fn classify_ssh_failure(stderr: &str) -> Option<(&'static str, String)> {
    let normalized = stderr.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_lowercase();

    if lower.contains("remote host identification has changed") || lower.contains("offending ") {
        return Some((
            "host_key_changed",
            "The remote host key changed. Refresh the stored host pin if you expected this change, or investigate the server before reconnecting.".to_string(),
        ));
    }
    if lower.contains("host key verification failed") {
        return Some((
            "host_key_verification_failed",
            "SSH host verification failed. Refresh or clear the host pin if the server was rebuilt, otherwise inspect the host before trusting it.".to_string(),
        ));
    }
    if lower.contains("permission denied") {
        return Some((
            "auth_failed",
            "SSH authentication failed. Check the selected user, the managed key or ssh-agent state, and the remote authorized_keys file.".to_string(),
        ));
    }
    if lower.contains("timed out") || lower.contains("operation timed out") {
        return Some((
            "timeout",
            "SSH timed out. Check hostname resolution, port selection, firewall rules, and whether the remote host is reachable.".to_string(),
        ));
    }
    if lower.contains("vault is locked") {
        return Some((
            "vault_locked",
            "The vault is locked, so Moltis cannot decrypt the managed SSH key. Unlock the vault in Settings → Encryption and retry.".to_string(),
        ));
    }

    None
}

fn build_ssh_test_response(
    route_label: Option<String>,
    probe: &str,
    result: anyhow::Result<moltis_gateway::node_exec::NodeExecResult>,
) -> SshTestResponse {
    match result {
        Ok(result) => {
            let reachable = result.exit_code == 0 && result.stdout.contains(probe);
            let classified_failure = (!reachable)
                .then(|| classify_ssh_failure(&result.stderr))
                .flatten();
            SshTestResponse {
                ok: true,
                reachable,
                stdout: result.stdout,
                stderr: result.stderr,
                exit_code: result.exit_code,
                route_label,
                failure_code: classified_failure.as_ref().map(|(code, _)| *code),
                failure_hint: classified_failure.map(|(_, hint)| hint),
            }
        },
        Err(error) => {
            let stderr = error.to_string();
            let classified_failure = classify_ssh_failure(&stderr);
            SshTestResponse {
                ok: false,
                reachable: false,
                stdout: String::new(),
                stderr,
                exit_code: -1,
                route_label,
                failure_code: classified_failure.as_ref().map(|(code, _)| *code),
                failure_hint: classified_failure.map(|(_, hint)| hint),
            }
        },
    }
}

async fn generate_ssh_key_material(name: &str) -> anyhow::Result<(SecretString, String, String)> {
    let dir = tempfile::tempdir()?;
    let key_path = dir.path().join("moltis_deploy_key");
    let output = Command::new("ssh-keygen")
        .arg("-t")
        .arg("ed25519")
        .arg("-N")
        .arg("")
        .arg("-C")
        .arg(format!("moltis:{name}"))
        .arg("-f")
        .arg(&key_path)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    let private_key = SecretString::new(tokio::fs::read_to_string(&key_path).await?);
    let public_key: String = tokio::fs::read_to_string(key_path.with_extension("pub")).await?;
    let fingerprint = ssh_keygen_fingerprint(&key_path).await?;
    Ok((private_key, public_key.trim().to_string(), fingerprint))
}

async fn inspect_imported_private_key(
    private_key: &SecretString,
    passphrase: Option<&SecretString>,
) -> anyhow::Result<(SecretString, String, String)> {
    let dir = tempfile::tempdir()?;
    let key_path = dir.path().join("imported_key");
    tokio::fs::write(&key_path, private_key.expose_secret()).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let mut public_command = Command::new("ssh-keygen");
    public_command.arg("-y");
    let _public_askpass = if let Some(passphrase) = passphrase {
        Some(configure_ssh_askpass(&mut public_command, passphrase)?)
    } else {
        None
    };
    let public_output = public_command
        .arg("-f")
        .arg(&key_path)
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    if !public_output.status.success() {
        let stderr = String::from_utf8_lossy(&public_output.stderr)
            .trim()
            .to_string();
        if passphrase.is_none() && looks_like_passphrase_error(&stderr) {
            anyhow::bail!(
                "this private key is passphrase-protected, provide the passphrase to import it"
            );
        }
        anyhow::bail!(stderr);
    }

    if let Some(passphrase) = passphrase {
        let mut decrypt_command = Command::new("ssh-keygen");
        decrypt_command
            .arg("-p")
            .arg("-N")
            .arg("")
            .arg("-f")
            .arg(&key_path)
            .stdin(std::process::Stdio::null());
        let _decrypt_askpass = configure_ssh_askpass(&mut decrypt_command, passphrase)?;
        let decrypt_output = decrypt_command.output().await?;
        if !decrypt_output.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&decrypt_output.stderr).trim());
        }
    }

    let fingerprint = ssh_keygen_fingerprint(&key_path).await?;
    let decrypted_private_key = SecretString::new(tokio::fs::read_to_string(&key_path).await?);
    let public_key = String::from_utf8(public_output.stdout)?.trim().to_string();
    Ok((decrypted_private_key, public_key, fingerprint))
}

fn looks_like_passphrase_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "passphrase",
        "bad decrypt",
        "wrong pass phrase",
        "wrong passphrase",
        "incorrect passphrase",
        "incorrect pass phrase",
        "error in libcrypto",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn configure_ssh_askpass(
    command: &mut Command,
    passphrase: &SecretString,
) -> anyhow::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    let askpass_path = dir.path().join("askpass.sh");
    let passphrase_path = dir.path().join("askpass.sh.pass");
    std::fs::write(&passphrase_path, passphrase.expose_secret())?;
    std::fs::write(&askpass_path, "#!/bin/sh\nexec cat \"$0.pass\"\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&passphrase_path, std::fs::Permissions::from_mode(0o600))?;
        std::fs::set_permissions(&askpass_path, std::fs::Permissions::from_mode(0o700))?;
    }
    command
        .env("SSH_ASKPASS", &askpass_path)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "moltis-askpass");
    Ok(dir)
}

async fn ssh_keygen_fingerprint(path: &std::path::Path) -> anyhow::Result<String> {
    let output = Command::new("ssh-keygen")
        .arg("-lf")
        .arg(path)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn validate_known_host_entry(known_host: &str) -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let known_hosts_path = dir.path().join("known_hosts");
    tokio::fs::write(&known_hosts_path, format!("{known_host}\n")).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&known_hosts_path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = ssh_keygen_fingerprint(&known_hosts_path)
        .await
        .map_err(|_| anyhow::anyhow!("known host entry is not a valid known_hosts line"))?;
    Ok(())
}

struct ResolvedScanTarget {
    host: String,
    port: Option<u16>,
}

struct ScannedKnownHost {
    host: String,
    port: Option<u16>,
    known_host: String,
}

fn parse_ssh_g_output(config: &str) -> ResolvedScanTarget {
    let mut host = None;
    let mut port = None;
    for line in config.lines() {
        if let Some(value) = line.strip_prefix("hostname ") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                host = Some(trimmed.to_string());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("port ") {
            port = value.trim().parse::<u16>().ok();
        }
    }

    ResolvedScanTarget {
        host: host.unwrap_or_default(),
        port,
    }
}

fn fallback_scan_host(target: &str) -> String {
    target
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(target)
        .trim()
        .to_string()
}

async fn resolve_scan_target(
    target: &str,
    port: Option<u16>,
) -> anyhow::Result<ResolvedScanTarget> {
    let output = Command::new("ssh")
        .arg("-G")
        .arg("--")
        .arg(target)
        .output()
        .await;
    if let Ok(output) = output
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut resolved = parse_ssh_g_output(&text);
        if resolved.host.is_empty() {
            resolved.host = fallback_scan_host(target);
        }
        if port.is_some() {
            resolved.port = port;
        }
        return Ok(resolved);
    }

    Ok(ResolvedScanTarget {
        host: fallback_scan_host(target),
        port,
    })
}

async fn scan_target_known_host(
    target: &str,
    port: Option<u16>,
) -> anyhow::Result<ScannedKnownHost> {
    let resolved = resolve_scan_target(target, port).await?;
    if resolved.host.is_empty() {
        anyhow::bail!("could not resolve a hostname for ssh target '{target}'");
    }

    let mut command = Command::new("ssh-keyscan");
    command.arg("-H");
    if let Some(port) = resolved.port {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(&resolved.host);
    let output = command.output().await?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let known_host = String::from_utf8(output.stdout)?.trim().to_string();
    if known_host.is_empty() {
        anyhow::bail!(
            "ssh-keyscan did not return any host keys for {}{}",
            resolved.host,
            resolved
                .port
                .map(|value| format!(":{value}"))
                .unwrap_or_default()
        );
    }
    validate_known_host_entry(&known_host).await?;

    Ok(ScannedKnownHost {
        host: resolved.host,
        port: resolved.port,
        known_host,
    })
}

struct DoctorInputs<'a> {
    exec_host: &'a str,
    ssh_binary_available: bool,
    paired_node_count: usize,
    managed_target_count: usize,
    pinned_target_count: usize,
    managed_key_count: usize,
    encrypted_key_count: usize,
    configured_node: Option<&'a str>,
    legacy_target: Option<&'a str>,
    default_target: Option<&'a SshTargetEntry>,
    vault_is_unsealed: bool,
}

fn build_doctor_checks(input: DoctorInputs<'_>) -> Vec<SshDoctorCheck> {
    let mut checks = Vec::new();

    checks.push(SshDoctorCheck {
        id: "exec-host",
        level: "ok",
        title: "Execution backend",
        message: match input.exec_host {
            "ssh" => "Remote exec is currently routed through SSH.".to_string(),
            "node" => "Remote exec is currently routed through paired nodes.".to_string(),
            _ => "Remote exec is currently running locally.".to_string(),
        },
        hint: Some("Change this in tools.exec.host or from the chat node picker.".to_string()),
    });

    if input.ssh_binary_available {
        checks.push(SshDoctorCheck {
            id: "ssh-binary",
            level: "ok",
            title: "SSH client",
            message: "System ssh client is available.".to_string(),
            hint: None,
        });
    } else {
        checks.push(SshDoctorCheck {
            id: "ssh-binary",
            level: "error",
            title: "SSH client",
            message: "System ssh client is not available in PATH.".to_string(),
            hint: Some(
                "Install OpenSSH or fix PATH before using SSH execution targets.".to_string(),
            ),
        });
    }

    match input.exec_host {
        "ssh" => {
            if let Some(target) = input.default_target {
                checks.push(SshDoctorCheck {
                    id: "ssh-route",
                    level: "ok",
                    title: "Active SSH route",
                    message: format!(
                        "Using managed target '{}' ({})",
                        target.label, target.target
                    ),
                    hint: None,
                });
                if target.known_host.is_none() {
                    checks.push(SshDoctorCheck {
                        id: "ssh-host-pinning",
                        level: "warn",
                        title: "Host verification",
                        message: "The active SSH target does not have a pinned host key.".to_string(),
                        hint: Some("Paste a known_hosts line into Settings → SSH to force strict host-key verification for this target.".to_string()),
                    });
                }
                if target.auth_mode == SshAuthMode::Managed
                    && input.encrypted_key_count > 0
                    && !input.vault_is_unsealed
                {
                    checks.push(SshDoctorCheck {
                        id: "managed-key-vault",
                        level: "error",
                        title: "Managed key access",
                        message: "The active SSH route uses a managed key, but the vault is locked.".to_string(),
                        hint: Some("Unlock the vault in Settings → Encryption before testing or using this target.".to_string()),
                    });
                }
            } else if let Some(target) = input.legacy_target {
                checks.push(SshDoctorCheck {
                    id: "ssh-route",
                    level: "warn",
                    title: "Active SSH route",
                    message: format!("Using legacy config target '{target}'."),
                    hint: Some("Move this into Settings → SSH if you want named targets, testing, and managed deploy keys.".to_string()),
                });
            } else {
                checks.push(SshDoctorCheck {
                    id: "ssh-route",
                    level: "error",
                    title: "Active SSH route",
                    message: "SSH execution is enabled, but no target is configured.".to_string(),
                    hint: Some(
                        "Add a target in Settings → SSH or set tools.exec.ssh_target.".to_string(),
                    ),
                });
            }
        },
        "node" => {
            if input.paired_node_count == 0 {
                checks.push(SshDoctorCheck {
                    id: "paired-node-route",
                    level: "error",
                    title: "Paired node route",
                    message: "Remote exec is set to use paired nodes, but none are connected.".to_string(),
                    hint: Some("Generate a connection token from the Nodes page or switch tools.exec.host back to local.".to_string()),
                });
            } else if let Some(node) = input.configured_node {
                checks.push(SshDoctorCheck {
                    id: "paired-node-route",
                    level: "ok",
                    title: "Paired node route",
                    message: format!("Default node preference is '{node}'."),
                    hint: None,
                });
            } else {
                checks.push(SshDoctorCheck {
                    id: "paired-node-route",
                    level: "warn",
                    title: "Paired node route",
                    message: "Paired nodes are available, but no default node is configured.".to_string(),
                    hint: Some("Select a node from chat or set tools.exec.node if you want a fixed default.".to_string()),
                });
            }
        },
        _ => {
            checks.push(SshDoctorCheck {
                id: "local-route",
                level: "warn",
                title: "Remote exec route",
                message: "The current backend is local, so SSH and node targets are only available when selected explicitly.".to_string(),
                hint: Some("Switch tools.exec.host if you want remote execution by default.".to_string()),
            });
        },
    }

    if input.managed_key_count == 0
        && input.managed_target_count == 0
        && input.legacy_target.is_none()
    {
        checks.push(SshDoctorCheck {
            id: "ssh-onboarding",
            level: "warn",
            title: "SSH onboarding",
            message: "No SSH targets are configured yet.".to_string(),
            hint: Some("Generate a deploy key in Settings → SSH, copy the public key to the remote host, then add a named target.".to_string()),
        });
    } else if input.managed_target_count > 0 {
        checks.push(SshDoctorCheck {
            id: "ssh-inventory",
            level: "ok",
            title: "Managed SSH inventory",
            message: format!(
                "{} key(s), {} target(s), {} pinned target(s), {} encrypted key(s).",
                input.managed_key_count,
                input.managed_target_count,
                input.pinned_target_count,
                input.encrypted_key_count
            ),
            hint: None,
        });
    }

    checks
}

async fn detect_ssh_binary() -> (bool, Option<String>) {
    match Command::new("ssh").arg("-V").output().await {
        Ok(output) => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            };
            (output.status.success(), (!text.is_empty()).then_some(text))
        },
        Err(_) => (false, None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn generated_key_material_round_trips() {
        let (private_key, public_key, fingerprint) =
            generate_ssh_key_material("test-key").await.unwrap();
        assert!(
            private_key
                .expose_secret()
                .contains("BEGIN OPENSSH PRIVATE KEY")
        );
        assert!(public_key.starts_with("ssh-ed25519 "));
        assert!(fingerprint.contains("SHA256:"));
    }

    #[tokio::test]
    async fn imported_key_is_validated() {
        let (private_key, ..) = generate_ssh_key_material("importable").await.unwrap();
        let (_, public_key, fingerprint) = inspect_imported_private_key(&private_key, None)
            .await
            .unwrap();
        assert!(public_key.starts_with("ssh-ed25519 "));
        assert!(fingerprint.contains("SHA256:"));
    }

    #[tokio::test]
    async fn imported_encrypted_key_accepts_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("encrypted");
        let output = Command::new("ssh-keygen")
            .arg("-q")
            .arg("-t")
            .arg("ed25519")
            .arg("-N")
            .arg("correct horse battery staple")
            .arg("-C")
            .arg("moltis-encrypted")
            .arg("-f")
            .arg(&key_path)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());

        let private_key = tokio::fs::read_to_string(&key_path).await.unwrap();
        let private_key = SecretString::new(private_key);
        let passphrase = SecretString::new("correct horse battery staple".to_string());
        let (decrypted_private_key, public_key, fingerprint) =
            inspect_imported_private_key(&private_key, Some(&passphrase))
                .await
                .unwrap();
        assert!(
            decrypted_private_key
                .expose_secret()
                .contains("BEGIN OPENSSH PRIVATE KEY")
        );
        assert!(public_key.starts_with("ssh-ed25519 "));
        assert!(fingerprint.contains("SHA256:"));
    }

    #[test]
    fn parse_ssh_g_output_extracts_host_and_port() {
        let resolved = parse_ssh_g_output(
            "host prod\nhostname app.internal.example\nport 2222\nuser deploy\n",
        );
        assert_eq!(resolved.host, "app.internal.example");
        assert_eq!(resolved.port, Some(2222));
    }

    #[test]
    fn fallback_scan_host_strips_user_prefix() {
        assert_eq!(fallback_scan_host("deploy@example.com"), "example.com");
        assert_eq!(fallback_scan_host("prod-box"), "prod-box");
    }

    #[test]
    fn classify_ssh_failure_recognizes_host_key_verification() {
        assert_eq!(
            classify_ssh_failure("Host key verification failed.\r\n")
                .map(|classified| classified.0),
            Some("host_key_verification_failed")
        );
    }

    #[test]
    fn classify_ssh_failure_recognizes_permission_denied() {
        assert_eq!(
            classify_ssh_failure("Permission denied (publickey).").map(|classified| classified.0),
            Some("auth_failed")
        );
    }

    #[test]
    fn validate_ssh_target_value_rejects_option_like_targets() {
        let error = validate_ssh_target_value("  -oProxyCommand=sh ").unwrap_err();
        assert_eq!(error.code, SSH_TARGET_REQUIRED);
        assert_eq!(
            error.message,
            "target must be a user@host or hostname, not an ssh option"
        );
    }

    #[test]
    fn looks_like_passphrase_error_matches_common_ssh_keygen_messages() {
        assert!(looks_like_passphrase_error(
            "load key \"/tmp/key\": incorrect passphrase supplied to decrypt private key",
        ));
        assert!(looks_like_passphrase_error(
            "Load key \"/tmp/key\": error in libcrypto",
        ));
        assert!(!looks_like_passphrase_error("invalid format"));
    }

    #[test]
    fn doctor_checks_flag_missing_ssh_target() {
        let checks = build_doctor_checks(DoctorInputs {
            exec_host: "ssh",
            ssh_binary_available: true,
            paired_node_count: 0,
            managed_target_count: 0,
            pinned_target_count: 0,
            managed_key_count: 0,
            encrypted_key_count: 0,
            configured_node: None,
            legacy_target: None,
            default_target: None,
            vault_is_unsealed: false,
        });

        assert!(
            checks
                .iter()
                .any(|check| check.id == "ssh-route" && check.level == "error")
        );
    }

    #[test]
    fn doctor_checks_flag_locked_vault_for_managed_route() {
        let default_target = SshTargetEntry {
            id: 1,
            label: "prod".to_string(),
            target: "deploy@example.com".to_string(),
            port: None,
            known_host: None,
            auth_mode: SshAuthMode::Managed,
            key_id: Some(1),
            key_name: Some("prod-key".to_string()),
            is_default: true,
            created_at: "2026-03-28T00:00:00Z".to_string(),
            updated_at: "2026-03-28T00:00:00Z".to_string(),
        };
        let checks = build_doctor_checks(DoctorInputs {
            exec_host: "ssh",
            ssh_binary_available: true,
            paired_node_count: 0,
            managed_target_count: 1,
            pinned_target_count: 0,
            managed_key_count: 1,
            encrypted_key_count: 1,
            configured_node: None,
            legacy_target: None,
            default_target: Some(&default_target),
            vault_is_unsealed: false,
        });

        assert!(
            checks
                .iter()
                .any(|check| check.id == "managed-key-vault" && check.level == "error")
        );
    }

    #[test]
    fn doctor_checks_warn_when_active_target_is_not_pinned() {
        let default_target = SshTargetEntry {
            id: 1,
            label: "prod".to_string(),
            target: "deploy@example.com".to_string(),
            port: None,
            known_host: None,
            auth_mode: SshAuthMode::System,
            key_id: None,
            key_name: None,
            is_default: true,
            created_at: "2026-03-28T00:00:00Z".to_string(),
            updated_at: "2026-03-28T00:00:00Z".to_string(),
        };
        let checks = build_doctor_checks(DoctorInputs {
            exec_host: "ssh",
            ssh_binary_available: true,
            paired_node_count: 0,
            managed_target_count: 1,
            pinned_target_count: 0,
            managed_key_count: 0,
            encrypted_key_count: 0,
            configured_node: None,
            legacy_target: None,
            default_target: Some(&default_target),
            vault_is_unsealed: false,
        });

        assert!(
            checks
                .iter()
                .any(|check| check.id == "ssh-host-pinning" && check.level == "warn")
        );
    }
}
