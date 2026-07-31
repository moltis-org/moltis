//! Managed MCP repository state, preview generation, and registry reconciliation.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use {
    fs2::FileExt,
    moltis_git_repositories::{Access, HttpsSource, RepositorySource, SshSource},
    secrecy::ExposeSecret,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
    time::OffsetDateTime,
};

use crate::{
    error::{Error, Result},
    registry::{McpRegistry, McpServerConfig, TransportType},
    remote::environment_placeholder_names,
    repository_discovery::{
        DiscoveryWarning, DiscoveryWarningKind, PluginIdentity, RepositoryDiscovery,
    },
};

const MAX_ALIAS_LEN: usize = 48;
const MAX_RUNTIME_NAME_LEN: usize = 80;

/// Cross-process guard for managed repository materialization and registry writes.
pub struct ManagedRepositoryLock {
    file: File,
}

impl ManagedRepositoryLock {
    pub fn try_acquire(data_dir: &Path) -> Result<Self> {
        let root = data_dir.join("mcp-repositories");
        std::fs::create_dir_all(&root).map_err(|error| {
            Error::external(
                format!(
                    "failed to create managed repository root {}",
                    root.display()
                ),
                error,
            )
        })?;
        let path = root.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                Error::external(
                    format!("failed to open managed repository lock {}", path.display()),
                    error,
                )
            })?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                Error::message(
                    "managed MCP repository registry is busy; stop the gateway or wait for the active repository operation",
                )
            } else {
                Error::external("failed to lock managed MCP repository registry", error)
            }
        })?;
        Ok(Self { file })
    }
}

impl Drop for ManagedRepositoryLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManagedRepositoryId(String);

impl ManagedRepositoryId {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_identifier("repository id", &value, 128)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ManagedRepositoryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManagedRepositoryAlias(String);

impl ManagedRepositoryAlias {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_identifier("repository alias", &value, MAX_ALIAS_LEN)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManagedServerIdentity(String);

impl ManagedServerIdentity {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_identifier("managed server identity", &value, 128)?;
        if !value.starts_with("mcp-") {
            return Err(Error::message("invalid managed server identity"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub type ManagedRepositoryAccess = Access;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManagedRepositorySource {
    Https {
        url: String,
        access: ManagedRepositoryAccess,
    },
    Ssh {
        remote: String,
        access: ManagedRepositoryAccess,
    },
    Local {
        canonical_path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedDiscoveryMode {
    Explicit,
    Recursive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedRevision {
    pub commit: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedRepository {
    pub id: ManagedRepositoryId,
    pub alias: ManagedRepositoryAlias,
    pub source: ManagedRepositorySource,
    pub requested_ref: String,
    pub discovery_mode: ManagedDiscoveryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub https_credential_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_target_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<ManagedRevision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ManagedRevision>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runtime_servers: BTreeMap<ManagedServerIdentity, String>,
}

impl ManagedRepository {
    #[must_use]
    pub fn new(
        id: ManagedRepositoryId,
        alias: ManagedRepositoryAlias,
        source: ManagedRepositorySource,
        requested_ref: impl Into<String>,
        discovery_mode: ManagedDiscoveryMode,
    ) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            id,
            alias,
            source,
            requested_ref: requested_ref.into(),
            discovery_mode,
            https_credential_id: None,
            ssh_target_id: None,
            active: None,
            previous: None,
            created_at: now,
            updated_at: now,
            runtime_servers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedApproval {
    pub commit: String,
    pub config_digest: String,
    #[serde(with = "time::serde::rfc3339")]
    pub approved_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedWarningKind {
    UnsupportedField,
    UnsupportedPluginSource,
    UnsafePath,
    MutableLatest,
    TlsVerificationDisabled,
    UnboundEnvironmentPlaceholder,
    LikelyLiteralSecret,
}

impl ManagedWarningKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedField => "unsupported_field",
            Self::UnsupportedPluginSource => "unsupported_plugin_source",
            Self::UnsafePath => "unsafe_path",
            Self::MutableLatest => "mutable_latest",
            Self::TlsVerificationDisabled => "tls_verification_disabled",
            Self::UnboundEnvironmentPlaceholder => "unbound_environment_placeholder",
            Self::LikelyLiteralSecret => "likely_literal_secret",
        }
    }

    #[must_use]
    pub const fn blocks_approval(self) -> bool {
        matches!(
            self,
            Self::UnboundEnvironmentPlaceholder | Self::LikelyLiteralSecret
        )
    }
}

impl From<DiscoveryWarningKind> for ManagedWarningKind {
    fn from(value: DiscoveryWarningKind) -> Self {
        match value {
            DiscoveryWarningKind::UnsupportedField => Self::UnsupportedField,
            DiscoveryWarningKind::UnsupportedPluginSource => Self::UnsupportedPluginSource,
            DiscoveryWarningKind::UnsafePath => Self::UnsafePath,
            DiscoveryWarningKind::MutableLatest => Self::MutableLatest,
            DiscoveryWarningKind::TlsVerificationDisabled => Self::TlsVerificationDisabled,
            DiscoveryWarningKind::UnboundEnvironmentPlaceholder => {
                Self::UnboundEnvironmentPlaceholder
            },
            DiscoveryWarningKind::LikelyLiteralSecret => Self::LikelyLiteralSecret,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedWarning {
    pub kind: ManagedWarningKind,
    pub source_manifest_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedOrigin {
    pub repository_id: ManagedRepositoryId,
    pub repository_alias: ManagedRepositoryAlias,
    pub identity: ManagedServerIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginIdentity>,
    pub source_name: String,
    pub source_manifest_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_root: Option<PathBuf>,
    pub discovered_commit: String,
    pub config_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ManagedApproval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<ManagedWarning>,
    /// Explicit placeholder-to-Moltis-env bindings. Empty until a binding API is available.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_bindings: BTreeMap<String, String>,
}

impl ManagedOrigin {
    #[must_use]
    pub fn is_currently_approved(&self) -> bool {
        self.approval.as_ref().is_some_and(|approval| {
            approval.commit == self.discovered_commit
                && approval.config_digest == self.config_digest
        })
    }

    #[must_use]
    pub fn approval_block_reason(&self) -> Option<&'static str> {
        if self
            .warnings
            .iter()
            .any(|warning| warning.kind == ManagedWarningKind::LikelyLiteralSecret)
        {
            return Some("repository manifest contains a literal secret-looking value");
        }
        if self
            .warnings
            .iter()
            .any(|warning| warning.kind == ManagedWarningKind::UnboundEnvironmentPlaceholder)
            && self.secret_bindings.is_empty()
        {
            return Some("repository manifest contains an unbound environment placeholder");
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct ManagedServerCandidate {
    pub runtime_name: String,
    pub identity: ManagedServerIdentity,
    pub config: McpServerConfig,
}

#[derive(Debug, Clone)]
pub struct ManagedRepositoryPreview {
    pub repository_id: ManagedRepositoryId,
    pub repository_alias: ManagedRepositoryAlias,
    pub revision: ManagedRevision,
    pub candidates: Vec<ManagedServerCandidate>,
    pub warnings: Vec<ManagedWarning>,
}

#[derive(Debug, Clone)]
pub enum ManagedInstallSelection {
    All,
    Selected(BTreeSet<ManagedServerIdentity>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedApprovalRequest {
    pub runtime_name: String,
    pub config_digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedReconciliationResult {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub removed: Vec<String>,
}

/// Convert discovery output into disabled managed candidates without mutating registry state.
pub fn preview_managed_repository(
    discovery: &RepositoryDiscovery,
    repository_id: ManagedRepositoryId,
    repository_alias: ManagedRepositoryAlias,
    commit: &str,
    revision_path: &Path,
) -> Result<ManagedRepositoryPreview> {
    validate_commit(commit)?;
    let revision_path = revision_path.canonicalize().map_err(|error| {
        Error::external(
            format!(
                "failed to resolve materialized revision {}",
                revision_path.display()
            ),
            error,
        )
    })?;
    if discovery.repository_root != revision_path {
        return Err(Error::message(
            "repository discovery root does not match the materialized revision",
        ));
    }

    let warnings = discovery
        .warnings
        .iter()
        .map(|warning| managed_warning(warning, &revision_path))
        .collect::<Result<Vec<_>>>()?;
    let mut candidates = Vec::with_capacity(discovery.servers.len());
    let mut identities = BTreeSet::new();
    let mut runtime_names = BTreeSet::new();
    for server in &discovery.servers {
        let manifest_path = relative_path(&revision_path, &server.source_manifest_path)?;
        let plugin_root = server
            .plugin_root
            .as_deref()
            .map(|path| relative_path(&revision_path, path))
            .transpose()?;
        let identity = server_identity(
            server.plugin.as_ref(),
            &server.name,
            &manifest_path,
            plugin_root.as_deref(),
        );
        if !identities.insert(identity.clone()) {
            return Err(Error::message(format!(
                "duplicate managed MCP server identity for '{}'",
                server.name
            )));
        }
        let runtime_name = runtime_server_name(
            &repository_alias,
            server.plugin.as_ref(),
            &server.name,
            &identity,
        );
        validate_runtime_name(&runtime_name)?;
        if !runtime_names.insert(runtime_name.clone()) {
            return Err(Error::message("managed MCP runtime name collision"));
        }

        let mut config = server.config.clone();
        config.enabled = false;
        if matches!(config.transport, TransportType::Stdio) {
            let cwd = config.cwd.as_deref().unwrap_or(&revision_path);
            ensure_path_within(&revision_path, cwd, "server working directory")?;
            config.cwd = Some(cwd.to_path_buf());
        } else {
            config.cwd = None;
        }
        let digest = config_digest(commit, &revision_path, &config);
        let origin_warnings = warnings
            .iter()
            .filter(|warning| warning_matches_server(warning, server.plugin.as_ref(), &server.name))
            .cloned()
            .collect();
        config.managed_origin = Some(ManagedOrigin {
            repository_id: repository_id.clone(),
            repository_alias: repository_alias.clone(),
            identity: identity.clone(),
            plugin: server.plugin.clone(),
            source_name: server.name.clone(),
            source_manifest_path: manifest_path,
            plugin_root,
            discovered_commit: commit.to_ascii_lowercase(),
            config_digest: digest,
            approval: None,
            warnings: origin_warnings,
            secret_bindings: BTreeMap::new(),
        });
        candidates.push(ManagedServerCandidate {
            runtime_name,
            identity,
            config,
        });
    }
    candidates.sort_by(|left, right| left.runtime_name.cmp(&right.runtime_name));

    Ok(ManagedRepositoryPreview {
        repository_id,
        repository_alias,
        revision: ManagedRevision {
            commit: commit.to_ascii_lowercase(),
            path: revision_path,
        },
        candidates,
        warnings,
    })
}

pub(crate) fn managed_config_is_current(
    config: &McpServerConfig,
    revision: &ManagedRevision,
) -> bool {
    config.managed_origin.as_ref().is_some_and(|origin| {
        origin.discovered_commit == revision.commit
            && origin.config_digest == config_digest(&revision.commit, &revision.path, config)
    })
}

impl McpRegistry {
    pub fn install_managed_repository(
        &mut self,
        mut repository: ManagedRepository,
        preview: &ManagedRepositoryPreview,
        selection: ManagedInstallSelection,
    ) -> Result<ManagedReconciliationResult> {
        validate_repository_preview(&repository, preview)?;
        if self.repositories.contains_key(&repository.id) {
            return Err(Error::message(format!(
                "managed repository '{}' is already installed",
                repository.id
            )));
        }
        if self
            .repositories
            .values()
            .any(|existing| existing.alias == repository.alias)
        {
            return Err(Error::message(format!(
                "managed repository alias '{}' is already in use",
                repository.alias.as_str()
            )));
        }
        let candidates = selected_candidates(preview, selection)?;
        ensure_candidate_names_available(&self.servers, &candidates, None)?;

        let mut staged = self.clone();
        let mut result = ManagedReconciliationResult::default();
        repository.active = Some(preview.revision.clone());
        repository.previous = None;
        repository.updated_at = OffsetDateTime::now_utc();
        for candidate in candidates {
            repository
                .runtime_servers
                .insert(candidate.identity.clone(), candidate.runtime_name.clone());
            staged
                .servers
                .insert(candidate.runtime_name.clone(), candidate.config.clone());
            result.added.push(candidate.runtime_name.clone());
        }
        staged
            .repositories
            .insert(repository.id.clone(), repository);
        self.commit_staged(staged)?;
        Ok(result)
    }

    pub fn update_managed_repository(
        &mut self,
        repository_id: &ManagedRepositoryId,
        preview: &ManagedRepositoryPreview,
    ) -> Result<ManagedReconciliationResult> {
        self.reconcile_managed_repository(repository_id, preview)
    }

    pub fn rollback_managed_repository(
        &mut self,
        repository_id: &ManagedRepositoryId,
        preview: &ManagedRepositoryPreview,
    ) -> Result<ManagedReconciliationResult> {
        let repository = self.repositories.get(repository_id).ok_or_else(|| {
            Error::message(format!("managed repository '{repository_id}' not found"))
        })?;
        let previous = repository.previous.as_ref().ok_or_else(|| {
            Error::message(format!(
                "managed repository '{repository_id}' has no previous revision"
            ))
        })?;
        if previous != &preview.revision {
            return Err(Error::message(
                "rollback preview does not match the repository's previous revision",
            ));
        }
        self.reconcile_managed_repository(repository_id, preview)
    }

    pub fn remove_managed_repository(
        &mut self,
        repository_id: &ManagedRepositoryId,
    ) -> Result<bool> {
        if !self.repositories.contains_key(repository_id) {
            return Ok(false);
        }
        let mut staged = self.clone();
        staged.repositories.remove(repository_id);
        staged.servers.retain(|_, config| {
            config
                .managed_origin
                .as_ref()
                .is_none_or(|origin| &origin.repository_id != repository_id)
        });
        self.commit_staged(staged)?;
        Ok(true)
    }

    pub fn approve_managed_selected(
        &mut self,
        repository_id: &ManagedRepositoryId,
        expected_commit: &str,
        requests: &[ManagedApprovalRequest],
        enable: bool,
    ) -> Result<()> {
        if requests.is_empty() {
            return Err(Error::message(
                "at least one managed server must be selected",
            ));
        }
        self.approve_managed(repository_id, expected_commit, requests, enable, false)
    }

    pub fn approve_managed_all(
        &mut self,
        repository_id: &ManagedRepositoryId,
        expected_commit: &str,
        requests: &[ManagedApprovalRequest],
        enable: bool,
    ) -> Result<()> {
        self.approve_managed(repository_id, expected_commit, requests, enable, true)
    }

    fn reconcile_managed_repository(
        &mut self,
        repository_id: &ManagedRepositoryId,
        preview: &ManagedRepositoryPreview,
    ) -> Result<ManagedReconciliationResult> {
        let repository = self.repositories.get(repository_id).ok_or_else(|| {
            Error::message(format!("managed repository '{repository_id}' not found"))
        })?;
        validate_repository_preview(repository, preview)?;
        let old_active = repository.active.clone().ok_or_else(|| {
            Error::message(format!(
                "managed repository '{repository_id}' has no active revision"
            ))
        })?;
        let candidates: Vec<&ManagedServerCandidate> = preview.candidates.iter().collect();
        ensure_candidate_names_available(&self.servers, &candidates, Some(repository_id))?;

        let old_names: BTreeSet<String> = self
            .servers
            .iter()
            .filter(|(_, config)| owned_by(config, repository_id))
            .map(|(name, _)| name.clone())
            .collect();
        let mut staged = self.clone();
        let mut result = ManagedReconciliationResult::default();
        let mut runtime_servers = BTreeMap::new();
        let mut retained_names = BTreeSet::new();

        for candidate in candidates {
            let existing_name = repository.runtime_servers.get(&candidate.identity);
            let runtime_name = existing_name.unwrap_or(&candidate.runtime_name).clone();
            if let Some(existing) = staged.servers.get(&runtime_name)
                && !owned_by(existing, repository_id)
            {
                return Err(Error::message(format!(
                    "MCP server name '{runtime_name}' is owned by a manual server"
                )));
            }
            let existing = staged.servers.get(&runtime_name).cloned();
            let mut config = candidate.config.clone();
            if let Some(existing) = existing.as_ref() {
                config.display_name = existing.display_name.clone();
                config.request_timeout_secs = existing.request_timeout_secs;
                let unchanged = same_current_config(existing, &config);
                if unchanged {
                    config.enabled = existing.enabled;
                    if let (Some(old_origin), Some(new_origin)) = (
                        existing.managed_origin.as_ref(),
                        config.managed_origin.as_mut(),
                    ) {
                        new_origin.approval = old_origin.approval.clone();
                    }
                    result.unchanged.push(runtime_name.clone());
                } else {
                    config.enabled = false;
                    result.updated.push(runtime_name.clone());
                }
            } else {
                config.enabled = false;
                result.added.push(runtime_name.clone());
            }
            staged.servers.insert(runtime_name.clone(), config);
            runtime_servers.insert(candidate.identity.clone(), runtime_name.clone());
            retained_names.insert(runtime_name);
        }

        for removed in old_names.difference(&retained_names) {
            if staged
                .servers
                .get(removed)
                .is_some_and(|config| owned_by(config, repository_id))
            {
                staged.servers.remove(removed);
                result.removed.push(removed.clone());
            }
        }
        let updated = staged.repositories.get_mut(repository_id).ok_or_else(|| {
            Error::message(format!("managed repository '{repository_id}' not found"))
        })?;
        updated.runtime_servers = runtime_servers;
        updated.active = Some(preview.revision.clone());
        updated.previous = if old_active != preview.revision {
            Some(old_active)
        } else {
            updated.previous.clone()
        };
        updated.updated_at = OffsetDateTime::now_utc();
        self.commit_staged(staged)?;
        Ok(result)
    }

    fn approve_managed(
        &mut self,
        repository_id: &ManagedRepositoryId,
        expected_commit: &str,
        requests: &[ManagedApprovalRequest],
        enable: bool,
        require_all: bool,
    ) -> Result<()> {
        let repository = self.repositories.get(repository_id).ok_or_else(|| {
            Error::message(format!("managed repository '{repository_id}' not found"))
        })?;
        let active = repository.active.as_ref().ok_or_else(|| {
            Error::message(format!(
                "managed repository '{repository_id}' has no active revision"
            ))
        })?;
        if active.commit != expected_commit {
            return Err(Error::message(
                "managed repository approval commit is stale",
            ));
        }
        let requested: BTreeMap<&str, &str> = requests
            .iter()
            .map(|request| {
                (
                    request.runtime_name.as_str(),
                    request.config_digest.as_str(),
                )
            })
            .collect();
        if requested.len() != requests.len() {
            return Err(Error::message("duplicate managed server approval request"));
        }
        let owned: BTreeSet<&str> = self
            .servers
            .iter()
            .filter(|(_, config)| owned_by(config, repository_id))
            .map(|(name, _)| name.as_str())
            .collect();
        if require_all && requested.keys().copied().collect::<BTreeSet<_>>() != owned {
            return Err(Error::message(
                "approval request does not match all current managed servers",
            ));
        }
        for (name, digest) in &requested {
            let config = self
                .servers
                .get(*name)
                .ok_or_else(|| Error::message(format!("managed MCP server '{name}' not found")))?;
            let origin = config
                .managed_origin
                .as_ref()
                .filter(|origin| &origin.repository_id == repository_id)
                .ok_or_else(|| {
                    Error::message(format!("MCP server '{name}' is not owned by repository"))
                })?;
            if origin.discovered_commit != expected_commit || origin.config_digest != *digest {
                return Err(Error::message(format!(
                    "managed MCP server '{name}' approval is stale"
                )));
            }
            if !managed_config_is_current(config, active) {
                return Err(Error::message(format!(
                    "managed MCP server '{name}' config no longer matches its active revision"
                )));
            }
            if let Some(reason) = managed_approval_block_reason(config) {
                return Err(Error::message(format!(
                    "managed MCP server '{name}' cannot be approved: {reason}"
                )));
            }
        }

        let mut staged = self.clone();
        let approved_at = OffsetDateTime::now_utc();
        for (name, digest) in requested {
            let config = staged
                .servers
                .get_mut(name)
                .ok_or_else(|| Error::message(format!("managed MCP server '{name}' not found")))?;
            let origin = config
                .managed_origin
                .as_mut()
                .ok_or_else(|| Error::message(format!("MCP server '{name}' is not managed")))?;
            origin.approval = Some(ManagedApproval {
                commit: expected_commit.to_string(),
                config_digest: digest.to_string(),
                approved_at,
            });
            if enable {
                config.enabled = true;
            }
        }
        self.commit_staged(staged)
    }
}

/// Return a secret-free reason when a managed server cannot be approved or started.
#[must_use]
pub fn managed_approval_block_reason(config: &McpServerConfig) -> Option<&'static str> {
    let origin = config.managed_origin.as_ref()?;
    if let Some(reason) = origin.approval_block_reason() {
        return Some(reason);
    }
    if config_placeholder_names(config)
        .iter()
        .any(|name| !origin.secret_bindings.contains_key(name))
    {
        return Some("repository manifest contains an unbound environment placeholder");
    }
    None
}

/// Restrict managed placeholder resolution to explicitly declared per-server bindings.
#[must_use]
pub fn managed_runtime_env_overrides(
    config: &McpServerConfig,
    global: &HashMap<String, String>,
) -> HashMap<String, String> {
    let Some(origin) = &config.managed_origin else {
        return global.clone();
    };
    origin
        .secret_bindings
        .iter()
        .filter_map(|(placeholder, source)| {
            global
                .get(source)
                .map(|value| (placeholder.clone(), value.clone()))
        })
        .collect()
}

fn config_placeholder_names(config: &McpServerConfig) -> BTreeSet<String> {
    config
        .args
        .iter()
        .chain(std::iter::once(&config.command))
        .flat_map(|value| environment_placeholder_names(value))
        .chain(
            config
                .env
                .values()
                .chain(config.headers.values())
                .flat_map(|value| environment_placeholder_names(value.expose_secret())),
        )
        .chain(
            config
                .url
                .iter()
                .flat_map(|value| environment_placeholder_names(value.expose_secret())),
        )
        .collect()
}

fn validate_identifier(label: &str, value: &str, max_len: usize) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(Error::message(format!("invalid {label}")))
    }
}

fn validate_commit(commit: &str) -> Result<()> {
    if commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::message(
            "managed repository commit must be a full SHA-1",
        ))
    }
}

fn validate_runtime_name(name: &str) -> Result<()> {
    validate_identifier("managed MCP runtime name", name, MAX_RUNTIME_NAME_LEN)
}

fn validate_repository_preview(
    repository: &ManagedRepository,
    preview: &ManagedRepositoryPreview,
) -> Result<()> {
    if repository.discovery_mode != ManagedDiscoveryMode::Explicit {
        return Err(Error::message(
            "recursive managed repository discovery is not supported",
        ));
    }
    if repository.id != preview.repository_id || repository.alias != preview.repository_alias {
        return Err(Error::message(
            "managed repository preview identity does not match repository state",
        ));
    }
    if repository.requested_ref.trim().is_empty() {
        return Err(Error::message("managed repository requested ref is empty"));
    }
    validate_repository_source(&repository.source)?;
    validate_commit(&preview.revision.commit)
}

fn validate_repository_source(source: &ManagedRepositorySource) -> Result<()> {
    match source {
        ManagedRepositorySource::Https { url, access } => {
            HttpsSource::new(url, *access).map_err(|error| {
                Error::external("invalid managed HTTPS repository source", error)
            })?;
        },
        ManagedRepositorySource::Ssh { remote, access } => {
            SshSource::new(remote, *access)
                .map_err(|error| Error::external("invalid managed SSH repository source", error))?;
        },
        ManagedRepositorySource::Local { canonical_path } => {
            if !canonical_path.is_absolute()
                || canonical_path.canonicalize().ok().as_deref() != Some(canonical_path)
            {
                return Err(Error::message(
                    "managed local repository source must be canonical",
                ));
            }
            RepositorySource::local(canonical_path).map_err(|error| {
                Error::external("invalid managed local repository source", error)
            })?;
        },
    }
    Ok(())
}

fn selected_candidates(
    preview: &ManagedRepositoryPreview,
    selection: ManagedInstallSelection,
) -> Result<Vec<&ManagedServerCandidate>> {
    match selection {
        ManagedInstallSelection::All => Ok(preview.candidates.iter().collect()),
        ManagedInstallSelection::Selected(selected) => {
            let available: BTreeSet<&ManagedServerIdentity> = preview
                .candidates
                .iter()
                .map(|candidate| &candidate.identity)
                .collect();
            if selected
                .iter()
                .any(|identity| !available.contains(identity))
            {
                return Err(Error::message(
                    "selected managed server is not present in the preview",
                ));
            }
            Ok(preview
                .candidates
                .iter()
                .filter(|candidate| selected.contains(&candidate.identity))
                .collect())
        },
    }
}

fn ensure_candidate_names_available(
    servers: &HashMap<String, McpServerConfig>,
    candidates: &[&ManagedServerCandidate],
    repository_id: Option<&ManagedRepositoryId>,
) -> Result<()> {
    for candidate in candidates {
        if let Some(existing) = servers.get(&candidate.runtime_name) {
            let same_identity = existing.managed_origin.as_ref().is_some_and(|origin| {
                repository_id.is_some_and(|id| &origin.repository_id == id)
                    && origin.identity == candidate.identity
            });
            if !same_identity {
                return Err(Error::message(format!(
                    "MCP server name '{}' is already in use",
                    candidate.runtime_name
                )));
            }
        }
    }
    Ok(())
}

fn owned_by(config: &McpServerConfig, repository_id: &ManagedRepositoryId) -> bool {
    config
        .managed_origin
        .as_ref()
        .is_some_and(|origin| &origin.repository_id == repository_id)
}

fn same_current_config(existing: &McpServerConfig, candidate: &McpServerConfig) -> bool {
    match (
        existing.managed_origin.as_ref(),
        candidate.managed_origin.as_ref(),
    ) {
        (Some(existing), Some(candidate)) => {
            existing.identity == candidate.identity
                && existing.discovered_commit == candidate.discovered_commit
                && existing.config_digest == candidate.config_digest
        },
        _ => false,
    }
}

fn managed_warning(warning: &DiscoveryWarning, root: &Path) -> Result<ManagedWarning> {
    Ok(ManagedWarning {
        kind: warning.kind.into(),
        source_manifest_path: relative_path(root, &warning.source_manifest_path)?,
        plugin_name: warning.plugin_name.clone(),
        source_name: warning.server_name.clone(),
    })
}

fn warning_matches_server(
    warning: &ManagedWarning,
    plugin: Option<&PluginIdentity>,
    source_name: &str,
) -> bool {
    warning
        .source_name
        .as_deref()
        .is_none_or(|name| name == source_name)
        && warning
            .plugin_name
            .as_deref()
            .is_none_or(|name| plugin.is_some_and(|identity| identity.plugin_name == name))
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path.strip_prefix(root).map_err(|_| {
        Error::message(format!(
            "managed repository path escapes materialized revision: {}",
            path.display()
        ))
    })?;
    if relative.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err(Error::message("unsafe managed repository relative path"));
    }
    Ok(relative.to_path_buf())
}

fn ensure_path_within(root: &Path, path: &Path, label: &str) -> Result<()> {
    if path.is_absolute() && path.starts_with(root) {
        return Ok(());
    }
    Err(Error::message(format!(
        "managed MCP {label} must be inside the materialized revision"
    )))
}

fn server_identity(
    plugin: Option<&PluginIdentity>,
    source_name: &str,
    manifest_path: &Path,
    plugin_root: Option<&Path>,
) -> ManagedServerIdentity {
    let mut digest = Sha256::new();
    hash_field(&mut digest, "source", source_name.as_bytes());
    hash_field(
        &mut digest,
        "manifest",
        manifest_path.to_string_lossy().as_bytes(),
    );
    if let Some(root) = plugin_root {
        hash_field(
            &mut digest,
            "plugin_root",
            root.to_string_lossy().as_bytes(),
        );
    }
    if let Some(plugin) = plugin {
        hash_optional(
            &mut digest,
            "marketplace",
            plugin.marketplace_name.as_deref(),
        );
        hash_field(&mut digest, "plugin", plugin.plugin_name.as_bytes());
        hash_optional(
            &mut digest,
            "plugin_manifest",
            plugin.manifest_name.as_deref(),
        );
    }
    ManagedServerIdentity(format!("mcp-{}", hex_digest(digest.finalize())))
}

fn runtime_server_name(
    alias: &ManagedRepositoryAlias,
    plugin: Option<&PluginIdentity>,
    source_name: &str,
    identity: &ManagedServerIdentity,
) -> String {
    let plugin_name = plugin.map(|value| value.plugin_name.as_str()).unwrap_or("");
    let base = slug(&format!("{}-{plugin_name}-{source_name}", alias.as_str()));
    let base_limit = MAX_RUNTIME_NAME_LEN.saturating_sub(11);
    let base = base.get(..base.len().min(base_limit)).unwrap_or(&base);
    let hash = identity
        .as_str()
        .strip_prefix("mcp-")
        .unwrap_or(identity.as_str());
    format!("{base}-{}", &hash[..10.min(hash.len())])
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('-');
            }
            output.push(char::from(byte.to_ascii_lowercase()));
            separator = false;
        } else {
            separator = true;
        }
    }
    if output.is_empty() {
        "mcp".to_string()
    } else {
        output
    }
}

fn config_digest(commit: &str, revision_root: &Path, config: &McpServerConfig) -> String {
    let mut digest = Sha256::new();
    hash_field(&mut digest, "version", b"managed-mcp-config-v1");
    hash_field(
        &mut digest,
        "commit",
        commit.to_ascii_lowercase().as_bytes(),
    );
    hash_field(
        &mut digest,
        "transport",
        config.transport.to_string().as_bytes(),
    );
    hash_field(
        &mut digest,
        "command",
        normalized_path_value(&config.command, revision_root).as_bytes(),
    );
    for arg in &config.args {
        hash_field(
            &mut digest,
            "arg",
            normalized_path_value(arg, revision_root).as_bytes(),
        );
    }
    if let Some(cwd) = &config.cwd {
        let cwd = cwd.strip_prefix(revision_root).unwrap_or(cwd);
        hash_field(&mut digest, "cwd", cwd.to_string_lossy().as_bytes());
    }
    hash_secret_map(&mut digest, "env", &config.env);
    hash_secret_map(&mut digest, "header", &config.headers);
    if let Some(url) = &config.url {
        hash_field(&mut digest, "url", url.expose_secret().as_bytes());
    }
    if let Some(oauth) = &config.oauth {
        hash_field(&mut digest, "oauth_client_id", oauth.client_id.as_bytes());
        hash_field(&mut digest, "oauth_auth_url", oauth.auth_url.as_bytes());
        hash_field(&mut digest, "oauth_token_url", oauth.token_url.as_bytes());
        for scope in &oauth.scopes {
            hash_field(&mut digest, "oauth_scope", scope.as_bytes());
        }
        if let Some(secret) = &oauth.client_secret {
            hash_field(
                &mut digest,
                "oauth_client_secret",
                secret.expose_secret().as_bytes(),
            );
        }
    }
    if let Some(origin) = &config.managed_origin {
        for (placeholder, source) in &origin.secret_bindings {
            hash_field(&mut digest, "binding_placeholder", placeholder.as_bytes());
            hash_field(&mut digest, "binding_source", source.as_bytes());
        }
    }
    hex_digest(digest.finalize())
}

fn normalized_path_value(value: &str, revision_root: &Path) -> String {
    value.replace(&revision_root.to_string_lossy().to_string(), "$REVISION")
}

fn hash_secret_map(
    digest: &mut Sha256,
    label: &str,
    values: &HashMap<String, secrecy::Secret<String>>,
) {
    let mut values: Vec<_> = values.iter().collect();
    values.sort_by(|left, right| left.0.cmp(right.0));
    for (name, value) in values {
        hash_field(digest, &format!("{label}_name"), name.as_bytes());
        hash_field(
            digest,
            &format!("{label}_value"),
            value.expose_secret().as_bytes(),
        );
    }
}

fn hash_optional(digest: &mut Sha256, label: &str, value: Option<&str>) {
    hash_field(digest, label, value.unwrap_or_default().as_bytes());
}

fn hash_field(digest: &mut Sha256, label: &str, value: &[u8]) {
    digest.update(label.len().to_be_bytes());
    digest.update(label.as_bytes());
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
