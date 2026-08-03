use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use {
    moltis_mcp::{
        ManagedReconciliationResult, ManagedRepository, ManagedRepositoryAlias,
        ManagedRepositoryId, ManagedRepositoryPreview, McpRegistry, discover_repository,
        preview_managed_repository,
    },
    tokio::task,
};

use crate::services::ServiceError;

use super::{LiveMcpService, ResolvedRepositoryRequest, service_message};

impl LiveMcpService {
    pub(super) async fn materialize_ephemeral_preview(
        &self,
        request: &ResolvedRepositoryRequest,
    ) -> Result<(ManagedRepositoryPreview, tempfile::TempDir), ServiceError> {
        let temporary_revision = tempfile::Builder::new()
            .prefix("moltis-mcp-preview-")
            .tempdir()
            .map_err(|error| {
                ServiceError::message(format!("failed to create repository preview: {error}"))
            })?;
        let preview = self
            .materialize_preview_at(request, temporary_revision.path().join("revisions"))
            .await?;
        Ok((preview, temporary_revision))
    }

    pub(super) async fn materialize_owned_preview(
        &self,
        request: &ResolvedRepositoryRequest,
    ) -> Result<ManagedRepositoryPreview, ServiceError> {
        let revisions_root = self
            .data_dir
            .join("mcp-repositories")
            .join(request.id.as_str())
            .join("revisions");
        self.materialize_preview_at(request, revisions_root).await
    }

    async fn materialize_preview_at(
        &self,
        request: &ResolvedRepositoryRequest,
        revisions_root: PathBuf,
    ) -> Result<ManagedRepositoryPreview, ServiceError> {
        let materializer = self.materializer.clone();
        let source = request.source.clone();
        let revision = request.requested_ref.clone();
        let https_credentials = request.https_credentials.clone();
        let ssh_credentials = request.ssh_credentials.clone();
        let materialized = task::spawn_blocking(move || {
            materializer.materialize(
                &source,
                &revision,
                &revisions_root,
                https_credentials.as_ref(),
                ssh_credentials.as_ref(),
            )
        })
        .await
        .map_err(|_| ServiceError::message("repository materialization task failed"))?
        .map_err(service_message)?;
        discover_materialized_revision(
            request.id.clone(),
            request.alias.clone(),
            materialized.path,
            materialized.commit,
        )
        .await
    }

    pub(super) async fn prune_repository_revisions(
        &self,
        id: &ManagedRepositoryId,
    ) -> Result<(), ServiceError> {
        let registry = self.manager.registry_snapshot().await;
        let repository = registry.repositories.get(id);
        prune_owned_revisions(&self.data_dir, id, repository).await
    }
}

pub(super) async fn discover_materialized_revision(
    id: ManagedRepositoryId,
    alias: ManagedRepositoryAlias,
    path: PathBuf,
    commit: String,
) -> Result<ManagedRepositoryPreview, ServiceError> {
    task::spawn_blocking(move || {
        let discovery = discover_repository(&path)?;
        preview_managed_repository(&discovery, id, alias, &commit, &path)
    })
    .await
    .map_err(|_| ServiceError::message("repository discovery task failed"))?
    .map_err(service_message)
}

pub(super) async fn remove_owned_repository_directory(
    data_dir: &Path,
    id: &ManagedRepositoryId,
) -> Result<(), ServiceError> {
    let root = data_dir.join("mcp-repositories");
    let repository_dir = root.join(id.as_str());
    let repository_name = id.as_str().to_string();
    task::spawn_blocking(move || {
        if !repository_dir.exists() {
            return Ok(());
        }
        let canonical_root = std::fs::canonicalize(&root)?;
        let canonical_repository = std::fs::canonicalize(&repository_dir)?;
        if canonical_repository != canonical_root.join(repository_name) {
            return Err(std::io::Error::other(
                "managed repository directory escaped its root",
            ));
        }
        std::fs::remove_dir_all(canonical_repository)
    })
    .await
    .map_err(|_| ServiceError::message("repository cleanup task failed"))?
    .map_err(|error| ServiceError::message(format!("failed to remove repository data: {error}")))
}

pub(super) fn reconciliation_preview(
    repository: &ManagedRepository,
    preview: &ManagedRepositoryPreview,
    registry: &McpRegistry,
) -> ManagedReconciliationResult {
    let old: BTreeMap<_, _> = registry
        .servers
        .iter()
        .filter_map(|(name, config)| {
            config.managed_origin.as_ref().and_then(|origin| {
                (origin.repository_id == repository.id).then(|| {
                    (
                        origin.identity.clone(),
                        (name.clone(), origin.config_digest.clone()),
                    )
                })
            })
        })
        .collect();
    let new: BTreeMap<_, _> = preview
        .candidates
        .iter()
        .filter_map(|candidate| {
            candidate.config.managed_origin.as_ref().map(|origin| {
                (
                    candidate.identity.clone(),
                    (candidate.runtime_name.clone(), origin.config_digest.clone()),
                )
            })
        })
        .collect();
    let mut result = ManagedReconciliationResult::default();
    for (identity, (name, digest)) in &new {
        match old.get(identity) {
            None => result.added.push(name.clone()),
            Some((old_name, old_digest)) if old_digest == digest => {
                result.unchanged.push(old_name.clone());
            },
            Some((old_name, _)) => result.updated.push(old_name.clone()),
        }
    }
    for (identity, (name, _)) in old {
        if !new.contains_key(&identity) {
            result.removed.push(name);
        }
    }
    result
}

async fn prune_owned_revisions(
    data_dir: &Path,
    id: &ManagedRepositoryId,
    repository: Option<&ManagedRepository>,
) -> Result<(), ServiceError> {
    let revisions_root = data_dir
        .join("mcp-repositories")
        .join(id.as_str())
        .join("revisions");
    let repositories_root = data_dir.join("mcp-repositories");
    let repository_name = id.as_str().to_string();
    let retained: BTreeSet<_> = repository
        .into_iter()
        .flat_map(|repository| [repository.active.as_ref(), repository.previous.as_ref()])
        .flatten()
        .map(|revision| revision.commit.clone())
        .collect();
    task::spawn_blocking(move || -> std::io::Result<()> {
        if !revisions_root.exists() {
            return Ok(());
        }
        let canonical_repositories = std::fs::canonicalize(&repositories_root)?;
        let canonical_revisions = std::fs::canonicalize(&revisions_root)?;
        if canonical_revisions
            != canonical_repositories
                .join(repository_name)
                .join("revisions")
        {
            return Err(std::io::Error::other(
                "managed repository revisions escaped their owned directory",
            ));
        }
        for entry in std::fs::read_dir(&canonical_revisions)? {
            let entry = entry?;
            if retained.contains(&entry.file_name().to_string_lossy().into_owned()) {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() && !file_type.is_symlink() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| ServiceError::message("repository revision cleanup task failed"))?
    .map_err(|error| {
        ServiceError::message(format!("failed to prune repository revisions: {error}"))
    })
}

#[cfg(test)]
pub(super) fn revision_count(data_dir: &Path) -> usize {
    std::fs::read_dir(data_dir.join("mcp-repositories/repo-1/revisions"))
        .map_or(0, |entries| entries.count())
}

#[cfg(test)]
pub(super) fn has_revision(data_dir: &Path, commit: &str) -> bool {
    data_dir
        .join("mcp-repositories/repo-1/revisions")
        .join(commit)
        .is_dir()
}
