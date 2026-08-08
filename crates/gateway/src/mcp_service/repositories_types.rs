#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryRequest {
    #[serde(default)]
    id: Option<String>,
    alias: String,
    source: SourceInput,
    #[serde(default = "default_requested_ref", rename = "ref")]
    requested_ref: String,
    #[serde(default)]
    discovery: DiscoveryInput,
    #[serde(default)]
    https_credential_id: Option<i64>,
    #[serde(default)]
    ssh_target_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum SourceInput {
    Https { url: String, private: bool },
    Ssh { remote: String },
    Local { path: PathBuf },
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiscoveryInput {
    #[default]
    Explicit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallRequest {
    #[serde(flatten)]
    repository: RepositoryRequest,
    expected_commit: String,
    selection: CandidateSelection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
enum CandidateSelection {
    All { candidates: Vec<ExpectedCandidate> },
    Selected { candidates: Vec<ExpectedCandidate> },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedCandidate {
    identity: String,
    digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryIdRequest {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateApplyRequest {
    id: String,
    expected_commit: String,
    candidates: Vec<ExpectedCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RollbackRequest {
    id: String,
    expected_commit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalRequest {
    id: String,
    expected_commit: String,
    selection: CandidateSelection,
    #[serde(default)]
    enable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialCreateRequest {
    host: String,
    username: String,
    token: Secret<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialUpdateRequest {
    id: i64,
    host: String,
    username: String,
    token: Secret<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialRemoveRequest {
    id: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewResponse {
    repository: RepositoryProjection,
    commit: String,
    candidates: Vec<CandidateProjection>,
    warnings: Vec<WarningProjection>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryProjection {
    id: String,
    alias: String,
    source: SourceProjection,
    #[serde(rename = "ref")]
    requested_ref: String,
    discovery: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum SourceProjection {
    Https {
        url: String,
        private: bool,
        #[serde(rename = "httpsCredentialId", skip_serializing_if = "Option::is_none")]
        credential_id: Option<i64>,
    },
    Ssh {
        remote: String,
        #[serde(rename = "sshTargetId")]
        target_id: i64,
    },
    Local {
        path: PathBuf,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateProjection {
    runtime_name: String,
    identity: String,
    digest: String,
    transport: TransportType,
    command: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    env_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    header_names: Vec<String>,
    approved: bool,
    approval_blocked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_block_reason: Option<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WarningProjection {
    kind: String,
    source_manifest_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_name: Option<String>,
}

#[derive(Debug)]
struct ResolvedRepositoryRequest {
    id: ManagedRepositoryId,
    alias: ManagedRepositoryAlias,
    source: RepositorySource,
    managed_source: ManagedRepositorySource,
    requested_ref: RequestedRevision,
    requested_ref_value: String,
    https_credential_id: Option<i64>,
    ssh_target_id: Option<i64>,
    https_credentials: Option<HttpsCredentials>,
    ssh_credentials: Option<SshCredentials>,
}

struct RepositoryOperationGuard {
    id: ManagedRepositoryId,
    operations: Arc<Mutex<HashSet<ManagedRepositoryId>>>,
}

impl Drop for RepositoryOperationGuard {
    fn drop(&mut self) {
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        operations.remove(&self.id);
    }
}

fn default_requested_ref() -> String {
    "HEAD".to_string()
}

fn parse_repository_id(value: String) -> Result<ManagedRepositoryId, ServiceError> {
    ManagedRepositoryId::parse(value).map_err(service_message)
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, ServiceError> {
    serde_json::from_value(params).map_err(|error| ServiceError::message(error.to_string()))
}

fn parse_empty_params(params: Value) -> Result<(), ServiceError> {
    if params.is_null() || params.as_object().is_some_and(serde_json::Map::is_empty) {
        Ok(())
    } else {
        Err(ServiceError::message("this method accepts no parameters"))
    }
}

fn service_message(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::message(error)
}

fn record_operation(operation: &'static str) {
    #[cfg(feature = "metrics")]
    moltis_metrics::counter!("mcp_repository_operations_total", "operation" => operation)
        .increment(1);
    #[cfg(not(feature = "metrics"))]
    let _ = operation;
}

async fn ensure_owned_revision_path(
    data_dir: &Path,
    id: &ManagedRepositoryId,
    revision: &Path,
) -> Result<(), ServiceError> {
    let revisions_root = data_dir
        .join("mcp-repositories")
        .join(id.as_str())
        .join("revisions");
    let revision = revision.to_path_buf();
    task::spawn_blocking(move || {
        let canonical_root = std::fs::canonicalize(revisions_root)
            .map_err(|_| ServiceError::message("managed repository revisions are unavailable"))?;
        let revision_name = revision
            .file_name()
            .map(ToOwned::to_owned)
            .ok_or_else(|| ServiceError::message("managed repository revision is invalid"))?;
        let canonical_revision = std::fs::canonicalize(&revision)
            .map_err(|_| ServiceError::message("managed repository revision is unavailable"))?;
        if canonical_revision == canonical_root.join(revision_name) {
            Ok(())
        } else {
            Err(ServiceError::message(
                "managed repository revision escaped its owned directory",
            ))
        }
    })
    .await
    .map_err(|_| ServiceError::message("repository path validation task failed"))?
}

fn warning_kind(kind: &moltis_mcp::ManagedWarningKind) -> String {
    kind.as_str().to_string()
}

fn ssh_target_authority(target: &str, port: Option<u16>) -> Result<String, ServiceError> {
    let host = target
        .trim()
        .rsplit_once('@')
        .map_or(target.trim(), |(_, host)| host)
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".:-".contains(&byte))
    {
        return Err(ServiceError::message(
            "managed SSH target has an invalid host",
        ));
    }
    Ok(match port {
        Some(port) => format!("[{host}]:{port}"),
        None if host.contains(':') => format!("[{host}]"),
        None => host,
    })
}
