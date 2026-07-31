use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use {
    moltis_git_repositories::{
        Access, HttpsCredentials, HttpsSource, RepositorySource, RequestedRevision, SshCredentials,
        SshSource,
    },
    moltis_mcp::{
        ManagedApprovalRequest, ManagedDiscoveryMode, ManagedInstallSelection,
        ManagedReconciliationResult, ManagedRepository, ManagedRepositoryAlias,
        ManagedRepositoryId, ManagedRepositoryPreview, ManagedRepositorySource,
        ManagedServerCandidate, ManagedServerIdentity, TransportType,
    },
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::task,
    tracing::instrument,
};

use crate::{
    auth::{CredentialStore, SshAuthMode},
    services::{ServiceError, ServiceResult},
};

use super::LiveMcpService;

include!("repositories_types.rs");

#[path = "repositories_credentials.rs"]
mod credentials;
#[path = "repositories_sanitize.rs"]
mod sanitize;
#[path = "repositories_storage.rs"]
mod storage;

#[cfg(test)]
#[path = "repositories_credentials_tests.rs"]
mod credential_tests;

use {
    sanitize::sanitize_args,
    storage::{
        discover_materialized_revision, reconciliation_preview, remove_owned_repository_directory,
    },
};

impl LiveMcpService {
    #[instrument(skip(self, params))]
    pub(super) async fn repositories_list_impl(&self, params: Value) -> ServiceResult {
        parse_empty_params(params)?;
        let registry = self.manager.registry_snapshot().await;
        let statuses = self.manager.status_all().await;
        let repositories = registry
            .repositories
            .values()
            .map(|repository| {
                let servers: Vec<_> = statuses
                    .iter()
                    .filter(|status| {
                        status
                            .managed
                            .as_ref()
                            .is_some_and(|managed| managed.repository_id == repository.id)
                    })
                    .collect();
                serde_json::json!({
                    "repository": repository_projection(repository),
                    "activeCommit": repository.active.as_ref().map(|revision| &revision.commit),
                    "previousCommit": repository.previous.as_ref().map(|revision| &revision.commit),
                    "servers": servers,
                })
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({ "repositories": repositories }))
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_preview_impl(&self, params: Value) -> ServiceResult {
        record_operation("preview");
        let request: RepositoryRequest = parse_params(params)?;
        let resolved = self.resolve_repository_request(request).await?;
        let _guard = self.begin_repository_operation(&resolved.id)?;
        let (preview, _temporary_revision) = self.materialize_ephemeral_preview(&resolved).await?;
        serialize_preview(&resolved, &preview)
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_install_impl(&self, params: Value) -> ServiceResult {
        record_operation("install");
        let request: InstallRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let resolved = self.resolve_repository_request(request.repository).await?;
        let _guard = self.begin_repository_operation(&resolved.id)?;
        let preview = match self.materialize_owned_preview(&resolved).await {
            Ok(preview) => preview,
            Err(error) => {
                self.prune_repository_revisions(&resolved.id).await?;
                return Err(error);
            },
        };
        let selection = match ensure_expected_commit(&preview, &request.expected_commit)
            .and_then(|()| validate_selection(&preview, request.selection))
        {
            Ok(selection) => selection,
            Err(error) => {
                self.prune_repository_revisions(&resolved.id).await?;
                return Err(error);
            },
        };
        let mut repository = ManagedRepository::new(
            resolved.id.clone(),
            resolved.alias.clone(),
            resolved.managed_source.clone(),
            resolved.requested_ref_value.clone(),
            ManagedDiscoveryMode::Explicit,
        );
        repository.https_credential_id = resolved.https_credential_id;
        repository.ssh_target_id = resolved.ssh_target_id;
        let result = match self
            .manager
            .install_managed_repository(repository, &preview, selection)
            .await
            .map_err(service_message)
        {
            Ok(result) => result,
            Err(error) => {
                self.prune_repository_revisions(&resolved.id).await?;
                return Err(error);
            },
        };
        self.prune_repository_revisions(&resolved.id).await?;
        self.sync_tools_if_ready().await;
        Ok(serde_json::json!({
            "repository": repository_projection_from_resolved(&resolved),
            "commit": preview.revision.commit,
            "reconciliation": result,
        }))
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_update_preview_impl(&self, params: Value) -> ServiceResult {
        record_operation("update_preview");
        let request: RepositoryIdRequest = parse_params(params)?;
        let id = parse_repository_id(request.id)?;
        let _guard = self.begin_repository_operation(&id)?;
        let repository = self.installed_repository(&id).await?;
        let resolved = self.resolve_installed_repository(&repository).await?;
        let (preview, _temporary_revision) = self.materialize_ephemeral_preview(&resolved).await?;
        let diff = reconciliation_preview(
            &repository,
            &preview,
            &self.manager.registry_snapshot().await,
        );
        let mut response = serialize_preview_value(&resolved, &preview)?;
        response["diff"] = serde_json::to_value(diff)?;
        Ok(response)
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_update_apply_impl(&self, params: Value) -> ServiceResult {
        record_operation("update_apply");
        let request: UpdateApplyRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let id = parse_repository_id(request.id)?;
        let _guard = self.begin_repository_operation(&id)?;
        let repository = self.installed_repository(&id).await?;
        let resolved = self.resolve_installed_repository(&repository).await?;
        let preview = match self.materialize_owned_preview(&resolved).await {
            Ok(preview) => preview,
            Err(error) => {
                self.prune_repository_revisions(&id).await?;
                return Err(error);
            },
        };
        if let Err(error) = ensure_expected_commit(&preview, &request.expected_commit)
            .and_then(|()| ensure_all_candidates(&preview, &request.candidates))
        {
            self.prune_repository_revisions(&id).await?;
            return Err(error);
        }
        let result = match self
            .manager
            .update_managed_repository(&id, &preview)
            .await
            .map_err(service_message)
        {
            Ok(result) => result,
            Err(error) => {
                self.prune_repository_revisions(&id).await?;
                return Err(error);
            },
        };
        self.prune_repository_revisions(&id).await?;
        self.sync_tools_if_ready().await;
        Ok(reconciliation_response(&preview, result))
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_rollback_impl(&self, params: Value) -> ServiceResult {
        record_operation("rollback");
        let request: RollbackRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let id = parse_repository_id(request.id)?;
        let _guard = self.begin_repository_operation(&id)?;
        let repository = self.installed_repository(&id).await?;
        let previous = repository
            .previous
            .clone()
            .ok_or_else(|| ServiceError::message("managed repository has no previous revision"))?;
        if previous.commit != request.expected_commit {
            return Err(ServiceError::message(
                "managed repository rollback commit is stale",
            ));
        }
        ensure_owned_revision_path(&self.data_dir, &id, &previous.path).await?;
        let preview = discover_materialized_revision(
            id.clone(),
            repository.alias.clone(),
            previous.path.clone(),
            previous.commit.clone(),
        )
        .await?;
        let result = self
            .manager
            .rollback_managed_repository(&id, &preview)
            .await
            .map_err(service_message)?;
        self.prune_repository_revisions(&id).await?;
        self.sync_tools_if_ready().await;
        Ok(reconciliation_response(&preview, result))
    }

    #[instrument(skip(self, params))]
    pub(super) async fn repositories_remove_impl(&self, params: Value) -> ServiceResult {
        record_operation("remove");
        let request: RepositoryIdRequest = parse_params(params)?;
        let _mutation = self.begin_managed_repository_mutation().await;
        let id = parse_repository_id(request.id)?;
        let _guard = self.begin_repository_operation(&id)?;
        let repository = self.installed_repository(&id).await?;
        let removed = self
            .manager
            .remove_managed_repository(&id)
            .await
            .map_err(service_message)?;
        if removed {
            remove_owned_repository_directory(&self.data_dir, &repository.id).await?;
            self.sync_tools_if_ready().await;
        }
        Ok(serde_json::json!({ "removed": removed }))
    }

    #[instrument(skip(self, params))]
    pub(super) async fn managed_approve_impl(&self, params: Value) -> ServiceResult {
        record_operation("approve");
        let request: ApprovalRequest = parse_params(params)?;
        let id = parse_repository_id(request.id)?;
        let _guard = self.begin_repository_operation(&id)?;
        let registry = self.manager.registry_snapshot().await;
        let repository = registry
            .repositories
            .get(&id)
            .ok_or_else(|| ServiceError::message(format!("managed repository '{id}' not found")))?;
        let active = repository
            .active
            .as_ref()
            .ok_or_else(|| ServiceError::message("managed repository has no active revision"))?;
        if active.commit != request.expected_commit {
            return Err(ServiceError::message(
                "managed repository approval commit is stale",
            ));
        }
        let owned = owned_expected_candidates(&registry, &id);
        let (all, expected) = selection_parts(request.selection);
        validate_expected_set(&owned, &expected, all)?;
        let approvals = expected
            .iter()
            .filter_map(|candidate| {
                registry
                    .servers
                    .iter()
                    .find(|(_, config)| {
                        config.managed_origin.as_ref().is_some_and(|origin| {
                            origin.repository_id == id
                                && origin.identity.as_str() == candidate.identity
                        })
                    })
                    .map(|(name, _)| ManagedApprovalRequest {
                        runtime_name: name.clone(),
                        config_digest: candidate.digest.clone(),
                    })
            })
            .collect::<Vec<_>>();
        if all {
            self.manager
                .approve_managed_all(&id, &request.expected_commit, &approvals, request.enable)
                .await
                .map_err(service_message)?;
        } else {
            self.manager
                .approve_managed_selected(&id, &request.expected_commit, &approvals, request.enable)
                .await
                .map_err(service_message)?;
        }

        let mut starts = Vec::new();
        if request.enable {
            self.refresh_manager_env_overrides().await;
            let current = self.manager.registry_snapshot().await;
            for approval in &approvals {
                let result = if let Some(config) = current.servers.get(&approval.runtime_name) {
                    self.manager
                        .start_server(&approval.runtime_name, config)
                        .await
                } else {
                    Err(moltis_mcp::Error::message(
                        "approved MCP server disappeared",
                    ))
                };
                starts.push(serde_json::json!({
                    "name": approval.runtime_name,
                    "started": result.is_ok(),
                    "error": result.err().map(|_| "failed to start approved MCP server"),
                }));
            }
        }
        self.sync_tools_if_ready().await;
        Ok(serde_json::json!({
            "approved": approvals.iter().map(|approval| &approval.runtime_name).collect::<Vec<_>>(),
            "enabled": request.enable,
            "starts": starts,
        }))
    }

    fn begin_repository_operation(
        &self,
        id: &ManagedRepositoryId,
    ) -> Result<RepositoryOperationGuard, ServiceError> {
        let mut operations = self
            .repository_operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if operations.len() >= super::MAX_CONCURRENT_MANAGED_REPOSITORY_OPERATIONS {
            return Err(ServiceError::message(
                "too many managed repository operations are already running",
            ));
        }
        if !operations.insert(id.clone()) {
            return Err(ServiceError::message(format!(
                "managed repository '{id}' already has an operation in progress"
            )));
        }
        Ok(RepositoryOperationGuard {
            id: id.clone(),
            operations: Arc::clone(&self.repository_operations),
        })
    }

    async fn credential_store(&self) -> Result<Arc<CredentialStore>, ServiceError> {
        self.credential_store
            .read()
            .await
            .clone()
            .ok_or_else(|| ServiceError::message("credential store is not available"))
    }

    async fn installed_repository(
        &self,
        id: &ManagedRepositoryId,
    ) -> Result<ManagedRepository, ServiceError> {
        self.manager
            .registry_snapshot()
            .await
            .repositories
            .get(id)
            .cloned()
            .ok_or_else(|| ServiceError::message(format!("managed repository '{id}' not found")))
    }

    async fn resolve_repository_request(
        &self,
        request: RepositoryRequest,
    ) -> Result<ResolvedRepositoryRequest, ServiceError> {
        let id = match request.id {
            Some(id) => parse_repository_id(id)?,
            None => ManagedRepositoryId::parse(uuid::Uuid::new_v4().to_string())
                .map_err(service_message)?,
        };
        let alias = ManagedRepositoryAlias::parse(request.alias).map_err(service_message)?;
        let requested_ref = RequestedRevision::parse(&request.requested_ref)
            .map_err(|error| ServiceError::message(error.to_string()))?;
        let requested_ref_value = requested_ref.as_str().to_string();
        let _ = request.discovery;
        let store = self.credential_store.read().await.clone();

        let mut resolved = match request.source {
            SourceInput::Https { url, private } => {
                if !private && request.https_credential_id.is_some() {
                    return Err(ServiceError::message(
                        "public HTTPS repository cannot specify httpsCredentialId",
                    ));
                }
                if request.ssh_target_id.is_some() {
                    return Err(ServiceError::message(
                        "HTTPS repository cannot specify sshTargetId",
                    ));
                }
                let access = if private {
                    Access::Private
                } else {
                    Access::Public
                };
                let source = HttpsSource::new(&url, access)
                    .map_err(|error| ServiceError::message(error.to_string()))?;
                let credential = match request.https_credential_id {
                    Some(credential_id) => {
                        let store = store.as_ref().ok_or_else(|| {
                            ServiceError::message("credential store is not available")
                        })?;
                        let credential = store
                            .get_git_https_credential(credential_id)
                            .await
                            .map_err(service_message)?
                            .ok_or_else(|| {
                                ServiceError::message("HTTPS Git credential not found")
                            })?;
                        Some(
                            HttpsCredentials::new(
                                credential.host,
                                credential.username,
                                credential.token,
                            )
                            .map_err(|error| ServiceError::message(error.to_string()))?,
                        )
                    },
                    None if private => {
                        return Err(ServiceError::message(
                            "private HTTPS repository requires httpsCredentialId",
                        ));
                    },
                    None => None,
                };
                ResolvedRepositoryRequest {
                    id,
                    alias,
                    managed_source: ManagedRepositorySource::Https {
                        url: source.url().as_str().to_string(),
                        access,
                    },
                    source: RepositorySource::Https(source),
                    requested_ref,
                    requested_ref_value,
                    https_credential_id: request.https_credential_id,
                    ssh_target_id: None,
                    https_credentials: credential,
                    ssh_credentials: None,
                }
            },
            SourceInput::Ssh { remote } => {
                if request.https_credential_id.is_some() {
                    return Err(ServiceError::message(
                        "SSH repository cannot specify httpsCredentialId",
                    ));
                }
                let target_id = request
                    .ssh_target_id
                    .ok_or_else(|| ServiceError::message("SSH repository requires sshTargetId"))?;
                let source = SshSource::new(&remote, Access::Private)
                    .map_err(|error| ServiceError::message(error.to_string()))?;
                let store = store
                    .as_ref()
                    .ok_or_else(|| ServiceError::message("credential store is not available"))?;
                let target = store
                    .resolve_ssh_target_by_id(target_id)
                    .await
                    .map_err(service_message)?
                    .ok_or_else(|| ServiceError::message("managed SSH target not found"))?;
                if target.auth_mode != SshAuthMode::Managed {
                    return Err(ServiceError::message(
                        "repository SSH target must use managed authentication",
                    ));
                }
                let key_id = target.key_id.ok_or_else(|| {
                    ServiceError::message("managed SSH target has no key configured")
                })?;
                let known_host = target
                    .known_host
                    .filter(|pin| !pin.trim().is_empty())
                    .ok_or_else(|| {
                        ServiceError::message("managed SSH target has no confirmed known_host")
                    })?;
                let target_host = ssh_target_authority(&target.target, target.port)?;
                if !target_host.eq_ignore_ascii_case(source.host()) {
                    return Err(ServiceError::message(
                        "managed SSH target host/port does not match repository source",
                    ));
                }
                let private_key = store
                    .get_ssh_private_key(key_id)
                    .await
                    .map_err(service_message)?
                    .ok_or_else(|| ServiceError::message("managed SSH key not found"))?;
                let credentials =
                    SshCredentials::new(source.host(), private_key, Secret::new(known_host))
                        .map_err(|error| ServiceError::message(error.to_string()))?;
                ResolvedRepositoryRequest {
                    id,
                    alias,
                    managed_source: ManagedRepositorySource::Ssh {
                        remote: source.remote().to_string(),
                        access: Access::Private,
                    },
                    source: RepositorySource::Ssh(source),
                    requested_ref,
                    requested_ref_value,
                    https_credential_id: None,
                    ssh_target_id: Some(target_id),
                    https_credentials: None,
                    ssh_credentials: Some(credentials),
                }
            },
            SourceInput::Local { path } => {
                if request.https_credential_id.is_some() || request.ssh_target_id.is_some() {
                    return Err(ServiceError::message(
                        "local repository cannot specify credentials",
                    ));
                }
                let source = task::spawn_blocking(move || RepositorySource::local(path))
                    .await
                    .map_err(|_| ServiceError::message("local repository validation task failed"))?
                    .map_err(|error| ServiceError::message(error.to_string()))?;
                let canonical_path = match &source {
                    RepositorySource::Local(local) => local.path().to_path_buf(),
                    _ => return Err(ServiceError::message("invalid local repository source")),
                };
                ResolvedRepositoryRequest {
                    id,
                    alias,
                    managed_source: ManagedRepositorySource::Local { canonical_path },
                    source,
                    requested_ref,
                    requested_ref_value,
                    https_credential_id: None,
                    ssh_target_id: None,
                    https_credentials: None,
                    ssh_credentials: None,
                }
            },
        };
        validate_credential_host(&mut resolved)?;
        Ok(resolved)
    }

    async fn resolve_installed_repository(
        &self,
        repository: &ManagedRepository,
    ) -> Result<ResolvedRepositoryRequest, ServiceError> {
        let source = match &repository.source {
            ManagedRepositorySource::Https { url, access } => SourceInput::Https {
                url: url.clone(),
                private: *access == Access::Private,
            },
            ManagedRepositorySource::Ssh { remote, .. } => SourceInput::Ssh {
                remote: remote.clone(),
            },
            ManagedRepositorySource::Local { canonical_path } => SourceInput::Local {
                path: canonical_path.clone(),
            },
        };
        self.resolve_repository_request(RepositoryRequest {
            id: Some(repository.id.to_string()),
            alias: repository.alias.as_str().to_string(),
            source,
            requested_ref: repository.requested_ref.clone(),
            discovery: DiscoveryInput::Explicit,
            https_credential_id: repository.https_credential_id,
            ssh_target_id: repository.ssh_target_id,
        })
        .await
    }
}

fn validate_selection(
    preview: &ManagedRepositoryPreview,
    selection: CandidateSelection,
) -> Result<ManagedInstallSelection, ServiceError> {
    let (all, expected) = selection_parts(selection);
    let available = preview_expected_candidates(preview);
    validate_expected_set(&available, &expected, all)?;
    if all {
        Ok(ManagedInstallSelection::All)
    } else {
        Ok(ManagedInstallSelection::Selected(
            expected
                .into_iter()
                .map(|candidate| ManagedServerIdentity::parse(candidate.identity))
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(service_message)?,
        ))
    }
}

fn ensure_all_candidates(
    preview: &ManagedRepositoryPreview,
    expected: &[ExpectedCandidate],
) -> Result<(), ServiceError> {
    validate_expected_set(&preview_expected_candidates(preview), expected, true)
}

fn selection_parts(selection: CandidateSelection) -> (bool, Vec<ExpectedCandidate>) {
    match selection {
        CandidateSelection::All { candidates } => (true, candidates),
        CandidateSelection::Selected { candidates } => (false, candidates),
    }
}

fn validate_expected_set(
    available: &[ExpectedCandidate],
    expected: &[ExpectedCandidate],
    require_all: bool,
) -> Result<(), ServiceError> {
    if expected.is_empty() {
        return Err(ServiceError::message("candidate selection cannot be empty"));
    }
    let available: BTreeSet<_> = available.iter().cloned().collect();
    let expected_set: BTreeSet<_> = expected.iter().cloned().collect();
    if expected_set.len() != expected.len() {
        return Err(ServiceError::message(
            "candidate selection contains duplicates",
        ));
    }
    if !expected_set.is_subset(&available) || require_all && expected_set != available {
        return Err(ServiceError::message(
            "candidate selection is stale or does not match the preview",
        ));
    }
    Ok(())
}

fn preview_expected_candidates(preview: &ManagedRepositoryPreview) -> Vec<ExpectedCandidate> {
    preview
        .candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .config
                .managed_origin
                .as_ref()
                .map(|origin| ExpectedCandidate {
                    identity: candidate.identity.as_str().to_string(),
                    digest: origin.config_digest.clone(),
                })
        })
        .collect()
}

fn owned_expected_candidates(
    registry: &moltis_mcp::McpRegistry,
    id: &ManagedRepositoryId,
) -> Vec<ExpectedCandidate> {
    registry
        .servers
        .values()
        .filter_map(|config| {
            config.managed_origin.as_ref().and_then(|origin| {
                (origin.repository_id == *id).then(|| ExpectedCandidate {
                    identity: origin.identity.as_str().to_string(),
                    digest: origin.config_digest.clone(),
                })
            })
        })
        .collect()
}

fn ensure_expected_commit(
    preview: &ManagedRepositoryPreview,
    expected: &str,
) -> Result<(), ServiceError> {
    if preview.revision.commit == expected.to_ascii_lowercase() {
        Ok(())
    } else {
        Err(ServiceError::message(
            "materialized repository commit does not match expectedCommit",
        ))
    }
}

fn serialize_preview(
    resolved: &ResolvedRepositoryRequest,
    preview: &ManagedRepositoryPreview,
) -> ServiceResult {
    Ok(serde_json::to_value(preview_response(resolved, preview))?)
}

fn serialize_preview_value(
    resolved: &ResolvedRepositoryRequest,
    preview: &ManagedRepositoryPreview,
) -> ServiceResult {
    serialize_preview(resolved, preview)
}

fn preview_response(
    resolved: &ResolvedRepositoryRequest,
    preview: &ManagedRepositoryPreview,
) -> PreviewResponse {
    PreviewResponse {
        repository: repository_projection_from_resolved(resolved),
        commit: preview.revision.commit.clone(),
        candidates: preview
            .candidates
            .iter()
            .map(|candidate| candidate_projection(candidate, &preview.revision.path))
            .collect(),
        warnings: preview
            .warnings
            .iter()
            .map(|warning| WarningProjection {
                kind: warning_kind(&warning.kind),
                source_manifest_path: warning.source_manifest_path.clone(),
                plugin_name: warning.plugin_name.clone(),
                source_name: warning.source_name.clone(),
            })
            .collect(),
    }
}

fn candidate_projection(candidate: &ManagedServerCandidate, root: &Path) -> CandidateProjection {
    let config = &candidate.config;
    let origin = config.managed_origin.as_ref();
    let mut env_names = config.env.keys().cloned().collect::<Vec<_>>();
    env_names.sort();
    let mut header_names = config.headers.keys().cloned().collect::<Vec<_>>();
    header_names.sort();
    let redact_literals = origin.is_some_and(|origin| {
        origin
            .warnings
            .iter()
            .any(|warning| warning.kind == moltis_mcp::ManagedWarningKind::LikelyLiteralSecret)
    });
    let approval_block_reason = moltis_mcp::managed_approval_block_reason(config).map(String::from);
    CandidateProjection {
        runtime_name: candidate.runtime_name.clone(),
        identity: candidate.identity.as_str().to_string(),
        digest: origin
            .map(|origin| origin.config_digest.clone())
            .unwrap_or_default(),
        transport: config.transport,
        command: if redact_literals {
            "[REDACTED]".to_string()
        } else {
            config.command.clone()
        },
        args: if redact_literals && !config.args.is_empty() {
            vec!["[REDACTED]".to_string()]
        } else {
            sanitize_args(&config.args)
        },
        cwd: config
            .cwd
            .as_deref()
            .and_then(|cwd| cwd.strip_prefix(root).ok())
            .map(Path::to_path_buf),
        env_names,
        url: config.url.as_ref().map(|url| {
            if redact_literals {
                "[REDACTED]".to_string()
            } else {
                moltis_mcp::remote::sanitize_url_for_display(secrecy::ExposeSecret::expose_secret(
                    url,
                ))
            }
        }),
        header_names,
        approved: origin.is_some_and(moltis_mcp::ManagedOrigin::is_currently_approved),
        approval_blocked: approval_block_reason.is_some(),
        approval_block_reason,
        warnings: origin
            .into_iter()
            .flat_map(|origin| &origin.warnings)
            .map(|warning| warning_kind(&warning.kind))
            .collect(),
    }
}

fn repository_projection(repository: &ManagedRepository) -> RepositoryProjection {
    RepositoryProjection {
        id: repository.id.to_string(),
        alias: repository.alias.as_str().to_string(),
        source: source_projection(
            &repository.source,
            repository.https_credential_id,
            repository.ssh_target_id,
        ),
        requested_ref: repository.requested_ref.clone(),
        discovery: "explicit",
    }
}

fn repository_projection_from_resolved(
    resolved: &ResolvedRepositoryRequest,
) -> RepositoryProjection {
    RepositoryProjection {
        id: resolved.id.to_string(),
        alias: resolved.alias.as_str().to_string(),
        source: source_projection(
            &resolved.managed_source,
            resolved.https_credential_id,
            resolved.ssh_target_id,
        ),
        requested_ref: resolved.requested_ref_value.clone(),
        discovery: "explicit",
    }
}

fn source_projection(
    source: &ManagedRepositorySource,
    https_credential_id: Option<i64>,
    ssh_target_id: Option<i64>,
) -> SourceProjection {
    match source {
        ManagedRepositorySource::Https { url, access } => SourceProjection::Https {
            url: moltis_mcp::remote::sanitize_url_for_display(url),
            private: *access == Access::Private,
            credential_id: https_credential_id,
        },
        ManagedRepositorySource::Ssh { remote, .. } => SourceProjection::Ssh {
            remote: remote.clone(),
            target_id: ssh_target_id.unwrap_or_default(),
        },
        ManagedRepositorySource::Local { canonical_path } => SourceProjection::Local {
            path: canonical_path.clone(),
        },
    }
}

fn reconciliation_response(
    preview: &ManagedRepositoryPreview,
    result: ManagedReconciliationResult,
) -> Value {
    serde_json::json!({
        "commit": preview.revision.commit,
        "reconciliation": result,
    })
}

fn validate_credential_host(request: &mut ResolvedRepositoryRequest) -> Result<(), ServiceError> {
    if let (RepositorySource::Https(source), Some(credentials)) =
        (&request.source, &request.https_credentials)
        && !source.authority().eq_ignore_ascii_case(credentials.host())
    {
        return Err(ServiceError::message(
            "HTTPS credential host does not match repository source",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::{fs, process::Command, sync::Arc};

    use {
        moltis_mcp::{
            ManagedDiscoveryMode, ManagedRepository, ManagedRepositoryAccess,
            ManagedRepositoryAlias, ManagedRepositoryId, ManagedRepositorySource, McpManager,
            McpRegistry,
        },
        serde_json::json,
        sqlx::SqlitePool,
    };

    use {super::*, sanitize::expected_candidates};

    struct Fixture {
        _data: tempfile::TempDir,
        source: tempfile::TempDir,
        service: LiveMcpService,
        store: Arc<CredentialStore>,
    }

    impl Fixture {
        async fn new(servers: &str) -> Self {
            let data = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            git(source.path(), &["init", "-q", "--initial-branch=main"]);
            write_manifest(source.path(), servers);
            git(source.path(), &["add", ".mcp.json"]);
            git(source.path(), &["commit", "-q", "-m", "initial"]);
            let registry = McpRegistry::load(&data.path().join("mcp-servers.json")).unwrap();
            let store = Arc::new(
                CredentialStore::new(SqlitePool::connect("sqlite::memory:").await.unwrap())
                    .await
                    .unwrap(),
            );
            let service = LiveMcpService::new(
                Arc::new(McpManager::new(registry)),
                Default::default(),
                Some(Arc::clone(&store)),
                data.path().to_path_buf(),
                moltis_mcp::ManagedRepositoryLock::try_acquire(data.path()).unwrap(),
            );
            Self {
                _data: data,
                source,
                service,
                store,
            }
        }

        fn repository_params(&self) -> Value {
            json!({
                "id": "repo-1",
                "alias": "tools",
                "source": { "kind": "local", "path": self.source.path() },
                "ref": "HEAD",
                "discovery": "explicit"
            })
        }

        async fn preview(&self) -> Value {
            self.service
                .repositories_preview_impl(self.repository_params())
                .await
                .unwrap()
        }

        async fn install(&self, preview: &Value) -> Value {
            let mut params = self.repository_params();
            params["expectedCommit"] = preview["commit"].clone();
            params["selection"] = json!({
                "mode": "all",
                "candidates": expected_candidates(preview),
            });
            self.service
                .repositories_install_impl(params)
                .await
                .unwrap()
        }

        fn commit(&self, servers: &str, message: &str) -> String {
            write_manifest(self.source.path(), servers);
            git(self.source.path(), &["add", ".mcp.json"]);
            git(self.source.path(), &["commit", "-q", "-m", message]);
            git(self.source.path(), &["rev-parse", "HEAD"])
        }
    }

    fn git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgSign=false"])
            .args(args)
            .current_dir(directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Moltis Test")
            .env("GIT_AUTHOR_EMAIL", "test@moltis.invalid")
            .env("GIT_COMMITTER_NAME", "Moltis Test")
            .env("GIT_COMMITTER_EMAIL", "test@moltis.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn write_manifest(root: &Path, servers: &str) {
        fs::write(
            root.join(".mcp.json"),
            format!(r#"{{"mcpServers":{{{servers}}}}}"#),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn preview_is_sanitized_and_does_not_mutate_registry() {
        let fixture = Fixture::new(
            r#""remote":{"command":"runner","args":["--token","literal-secret"],"url":"https://user:pass@example.com/mcp?token=secret","headers":{"Authorization":"header-secret"},"type":"http"},"stdio":{"command":"stdio","env":{"API_TOKEN":"env-secret"}}"#,
        )
        .await;

        let preview = fixture.preview().await;
        let serialized = preview.to_string();
        assert!(!serialized.contains("literal-secret"));
        assert!(!serialized.contains("env-secret"));
        assert!(!serialized.contains("header-secret"));
        assert!(!serialized.contains("user:pass"));
        let candidates = preview["candidates"].as_array().unwrap();
        let remote = candidates
            .iter()
            .find(|candidate| candidate["transport"] == "streamable-http")
            .unwrap();
        let stdio = candidates
            .iter()
            .find(|candidate| candidate["envNames"][0] == "API_TOKEN")
            .unwrap();
        assert_eq!(remote["args"][0], "[REDACTED]");
        assert_eq!(stdio["envNames"][0], "API_TOKEN");
        assert_eq!(remote["headerNames"][0], "Authorization");
        assert!(
            fixture
                .service
                .manager
                .registry_snapshot()
                .await
                .repositories
                .is_empty()
        );
        assert_eq!(storage::revision_count(fixture._data.path()), 0);
    }

    #[tokio::test]
    async fn repeated_previews_leave_no_persistent_revisions() {
        let fixture = Fixture::new(r#""one":{"command":"one"}"#).await;

        for _ in 0..3 {
            fixture.preview().await;
            assert_eq!(storage::revision_count(fixture._data.path()), 0);
        }
    }

    #[tokio::test]
    async fn install_is_disabled_and_rejects_stale_commit_or_digest() {
        let fixture = Fixture::new(r#""one":{"command":"one"},"two":{"command":"two"}"#).await;
        let preview = fixture.preview().await;
        let mut stale_commit = fixture.repository_params();
        stale_commit["expectedCommit"] = json!("1111111111111111111111111111111111111111");
        stale_commit["selection"] = json!({
            "mode": "all",
            "candidates": expected_candidates(&preview),
        });
        assert!(
            fixture
                .service
                .repositories_install_impl(stale_commit)
                .await
                .is_err()
        );

        let mut stale_digest = fixture.repository_params();
        stale_digest["expectedCommit"] = preview["commit"].clone();
        let mut candidates = expected_candidates(&preview);
        candidates[0]["digest"] = json!("stale");
        stale_digest["selection"] = json!({ "mode": "all", "candidates": candidates });
        assert!(
            fixture
                .service
                .repositories_install_impl(stale_digest)
                .await
                .is_err()
        );

        fixture.install(&preview).await;
        let registry = fixture.service.manager.registry_snapshot().await;
        assert_eq!(registry.repositories.len(), 1);
        assert_eq!(storage::revision_count(fixture._data.path()), 1);
        assert!(registry.servers.values().all(|config| !config.enabled));
        assert!(registry.servers.values().all(|config| {
            config
                .managed_origin
                .as_ref()
                .is_some_and(|origin| !origin.is_currently_approved())
        }));
        let statuses = fixture.service.manager.status_all().await;
        assert!(statuses.iter().all(|status| {
            status.managed.as_ref().is_some_and(|managed| {
                managed.repository_id.as_str() == "repo-1" && !managed.approved
            })
        }));
    }

    #[tokio::test]
    async fn blocked_candidate_projection_and_status_are_secret_free() {
        let fixture = Fixture::new(
            r#""remote":{"type":"http","url":"https://example.test/${OPENAI_API_KEY}","headers":{"Authorization":"Bearer ${OPENAI_API_KEY}"}}"#,
        )
        .await;
        let preview = fixture.preview().await;
        let candidate = &preview["candidates"][0];
        assert_eq!(candidate["approvalBlocked"], true);
        assert_eq!(
            candidate["approvalBlockReason"],
            "repository manifest contains an unbound environment placeholder"
        );
        assert!(!preview.to_string().contains("Bearer"));
        fixture.install(&preview).await;
        let status = fixture.service.manager.status_all().await;
        let managed = status[0].managed.as_ref().unwrap();
        assert!(managed.approval_blocked);
        assert_eq!(
            managed.approval_block_reason.as_deref(),
            Some("repository manifest contains an unbound environment placeholder")
        );
        assert!(
            fixture
                .service
                .managed_approve_impl(json!({
                    "id": "repo-1",
                    "expectedCommit": preview["commit"],
                    "selection": { "mode": "all", "candidates": expected_candidates(&preview) },
                    "enable": true,
                }))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn service_holds_repository_lock_for_its_lifetime() {
        let fixture = Fixture::new(r#""one":{"command":"one"}"#).await;
        assert!(moltis_mcp::ManagedRepositoryLock::try_acquire(fixture._data.path()).is_err());
        drop(fixture);
    }

    #[tokio::test]
    async fn approve_selected_and_all_persist_even_when_enable_start_fails() {
        let fixture = Fixture::new(
            r#""one":{"command":"/definitely/missing/mcp-one"},"two":{"command":"/definitely/missing/mcp-two"}"#,
        )
        .await;
        let preview = fixture.preview().await;
        fixture.install(&preview).await;
        let candidates = expected_candidates(&preview);
        let selected = fixture
            .service
            .managed_approve_impl(json!({
                "id": "repo-1",
                "expectedCommit": preview["commit"],
                "selection": { "mode": "selected", "candidates": [candidates[0].clone()] },
                "enable": false,
            }))
            .await
            .unwrap();
        assert_eq!(selected["approved"].as_array().unwrap().len(), 1);

        let enabled = fixture
            .service
            .managed_approve_impl(json!({
                "id": "repo-1",
                "expectedCommit": preview["commit"],
                "selection": { "mode": "all", "candidates": candidates },
                "enable": true,
            }))
            .await
            .unwrap();
        assert_eq!(enabled["starts"].as_array().unwrap().len(), 2);
        assert!(
            enabled["starts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|start| !start["started"].as_bool().unwrap())
        );
        let registry = fixture.service.manager.registry_snapshot().await;
        assert!(registry.servers.values().all(|config| config.enabled));
        assert!(registry.servers.values().all(|config| {
            config
                .managed_origin
                .as_ref()
                .unwrap()
                .is_currently_approved()
        }));
    }

    #[tokio::test]
    async fn update_preview_apply_rollback_and_remove_are_safe() {
        let fixture = Fixture::new(
            r#""same":{"command":"same"},"changed":{"command":"old"},"removed":{"command":"gone"}"#,
        )
        .await;
        let first = fixture.preview().await;
        let first_commit = first["commit"].as_str().unwrap().to_string();
        fixture.install(&first).await;
        let status_before = git(fixture.source.path(), &["status", "--porcelain=v1"]);
        let second_commit = fixture.commit(
            r#""same":{"command":"same"},"changed":{"command":"new"},"added":{"command":"fresh"}"#,
            "update",
        );

        let mut update = Value::Null;
        for _ in 0..3 {
            update = fixture
                .service
                .repositories_update_preview_impl(json!({ "id": "repo-1" }))
                .await
                .unwrap();
            assert_eq!(storage::revision_count(fixture._data.path()), 1);
        }
        assert_eq!(update["repository"]["id"], "repo-1");
        assert_eq!(update["commit"].as_str().unwrap().len(), 40);
        assert!(
            update["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| {
                    candidate["runtimeName"].is_string()
                        && candidate["identity"].is_string()
                        && candidate["digest"].is_string()
                })
        );
        assert_eq!(update["diff"]["added"].as_array().unwrap().len(), 1);
        assert_eq!(update["diff"]["removed"].as_array().unwrap().len(), 1);
        assert!(!update["diff"]["updated"].as_array().unwrap().is_empty());
        fixture
            .service
            .repositories_update_apply_impl(json!({
                "id": "repo-1",
                "expectedCommit": update["commit"],
                "candidates": expected_candidates(&update),
            }))
            .await
            .unwrap();
        assert_eq!(storage::revision_count(fixture._data.path()), 2);
        assert!(
            fixture
                .service
                .manager
                .registry_snapshot()
                .await
                .servers
                .values()
                .all(|config| !config.enabled)
        );

        fixture
            .service
            .repositories_rollback_impl(json!({
                "id": "repo-1",
                "expectedCommit": first_commit,
            }))
            .await
            .unwrap();
        assert_eq!(storage::revision_count(fixture._data.path()), 2);
        assert!(
            fixture
                .service
                .manager
                .registry_snapshot()
                .await
                .servers
                .values()
                .any(|config| config.command == "old")
        );

        fixture.commit(
            r#""same":{"command":"third"},"added":{"command":"fresh"}"#,
            "third",
        );
        let third = fixture
            .service
            .repositories_update_preview_impl(json!({ "id": "repo-1" }))
            .await
            .unwrap();
        fixture
            .service
            .repositories_update_apply_impl(json!({
                "id": "repo-1",
                "expectedCommit": third["commit"],
                "candidates": expected_candidates(&third),
            }))
            .await
            .unwrap();
        assert_eq!(storage::revision_count(fixture._data.path()), 2);
        assert!(!storage::has_revision(fixture._data.path(), &second_commit));

        let source_path = fixture.source.path().to_path_buf();
        fixture
            .service
            .repositories_remove_impl(json!({ "id": "repo-1" }))
            .await
            .unwrap();
        assert!(source_path.exists());
        assert_eq!(
            git(&source_path, &["status", "--porcelain=v1"]),
            status_before
        );
        assert!(
            fixture
                .service
                .manager
                .registry_snapshot()
                .await
                .repositories
                .is_empty()
        );
    }

    #[tokio::test]
    async fn credential_rpcs_redact_tokens_and_reject_in_use_removal() {
        let fixture = Fixture::new(r#""one":{"command":"one"}"#).await;
        let created = fixture
            .service
            .git_credentials_create_impl(json!({
                "host": "git.example",
                "username": "moltis",
                "token": "top-secret-token",
            }))
            .await
            .unwrap();
        let id = created["credential"]["id"].as_i64().unwrap();
        assert!(!created.to_string().contains("top-secret-token"));
        let listed = fixture
            .service
            .git_credentials_list_impl(json!({}))
            .await
            .unwrap();
        assert!(!listed.to_string().contains("top-secret-token"));
        let updated = fixture
            .service
            .git_credentials_update_impl(json!({
                "id": id,
                "host": "git.example",
                "username": "moltis-updated",
                "token": "replacement-secret-token",
            }))
            .await
            .unwrap();
        assert_eq!(updated["credential"]["username"], "moltis-updated");
        assert!(!updated.to_string().contains("replacement-secret-token"));

        let mut repository = ManagedRepository::new(
            ManagedRepositoryId::parse("private-repo").unwrap(),
            ManagedRepositoryAlias::parse("private").unwrap(),
            ManagedRepositorySource::Https {
                url: "https://git.example/owner/repo.git".into(),
                access: ManagedRepositoryAccess::Private,
            },
            "HEAD",
            ManagedDiscoveryMode::Explicit,
        );
        repository.https_credential_id = Some(id);
        fixture
            .service
            .manager
            .inner
            .write()
            .await
            .registry
            .repositories
            .insert(repository.id.clone(), repository);
        assert!(
            fixture
                .service
                .git_credentials_remove_impl(json!({ "id": id }))
                .await
                .is_err()
        );
        assert!(
            fixture
                .store
                .get_git_https_credential(id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn ssh_repository_requires_managed_pinned_matching_target() {
        let fixture = Fixture::new(r#""one":{"command":"one"}"#).await;
        let key_id = fixture
            .store
            .create_ssh_key("git", "PRIVATE KEY", "ssh-ed25519 AAAATEST", "SHA256:test")
            .await
            .unwrap();
        let missing_pin = fixture
            .store
            .create_ssh_target(
                "missing-pin",
                "git.example",
                None,
                None,
                SshAuthMode::Managed,
                Some(key_id),
                true,
            )
            .await
            .unwrap();
        let wrong_host = fixture
            .store
            .create_ssh_target(
                "wrong-host",
                "other.example",
                None,
                Some("other.example ssh-ed25519 AAAATEST"),
                SshAuthMode::Managed,
                Some(key_id),
                false,
            )
            .await
            .unwrap();
        for target_id in [missing_pin, wrong_host] {
            assert!(
                fixture
                    .service
                    .repositories_preview_impl(json!({
                        "id": "ssh-repo",
                        "alias": "ssh-tools",
                        "source": { "kind": "ssh", "remote": "git@git.example:owner/repo.git" },
                        "sshTargetId": target_id,
                    }))
                    .await
                    .is_err()
            );
        }
    }
}
