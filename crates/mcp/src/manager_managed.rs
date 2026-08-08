use crate::{
    error::Result,
    managed_repositories::{
        ManagedApprovalRequest, ManagedInstallSelection, ManagedReconciliationResult,
        ManagedRepository, ManagedRepositoryId, ManagedRepositoryPreview,
    },
    manager::McpManager,
};

impl McpManager {
    /// Atomically install discovered managed configs without starting them.
    pub async fn install_managed_repository(
        &self,
        repository: ManagedRepository,
        preview: &ManagedRepositoryPreview,
        selection: ManagedInstallSelection,
    ) -> Result<ManagedReconciliationResult> {
        self.inner
            .write()
            .await
            .registry
            .install_managed_repository(repository, preview, selection)
    }

    /// Atomically reconcile a new immutable revision without starting servers.
    pub async fn update_managed_repository(
        &self,
        repository_id: &ManagedRepositoryId,
        preview: &ManagedRepositoryPreview,
    ) -> Result<ManagedReconciliationResult> {
        let (result, clients) = {
            let mut inner = self.inner.write().await;
            let result = inner
                .registry
                .update_managed_repository(repository_id, preview)?;
            let clients = result
                .updated
                .iter()
                .chain(&result.removed)
                .filter_map(|name| Self::invalidate_server(&mut inner, name, true))
                .collect::<Vec<_>>();
            (result, clients)
        };
        for client in clients {
            client.write().await.shutdown().await;
        }
        Ok(result)
    }

    /// Atomically reconcile the previous immutable revision without starting servers.
    pub async fn rollback_managed_repository(
        &self,
        repository_id: &ManagedRepositoryId,
        preview: &ManagedRepositoryPreview,
    ) -> Result<ManagedReconciliationResult> {
        let (result, clients) = {
            let mut inner = self.inner.write().await;
            let result = inner
                .registry
                .rollback_managed_repository(repository_id, preview)?;
            let clients = result
                .updated
                .iter()
                .chain(&result.removed)
                .filter_map(|name| Self::invalidate_server(&mut inner, name, true))
                .collect::<Vec<_>>();
            (result, clients)
        };
        for client in clients {
            client.write().await.shutdown().await;
        }
        Ok(result)
    }

    /// Remove repository-owned configs and stop their existing processes.
    pub async fn remove_managed_repository(
        &self,
        repository_id: &ManagedRepositoryId,
    ) -> Result<bool> {
        let (removed, clients) = {
            let mut inner = self.inner.write().await;
            let names = inner
                .registry
                .servers
                .iter()
                .filter(|(_, config)| {
                    config
                        .managed_origin
                        .as_ref()
                        .is_some_and(|origin| &origin.repository_id == repository_id)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            let removed = inner.registry.remove_managed_repository(repository_id)?;
            let clients = if removed {
                names
                    .iter()
                    .filter_map(|name| Self::invalidate_server(&mut inner, name, true))
                    .collect()
            } else {
                Vec::new()
            };
            (removed, clients)
        };
        for client in clients {
            client.write().await.shutdown().await;
        }
        Ok(removed)
    }

    /// Persist selected approvals; enabling changes config only and never starts processes.
    pub async fn approve_managed_selected(
        &self,
        repository_id: &ManagedRepositoryId,
        expected_commit: &str,
        requests: &[ManagedApprovalRequest],
        enable: bool,
    ) -> Result<()> {
        self.inner.write().await.registry.approve_managed_selected(
            repository_id,
            expected_commit,
            requests,
            enable,
        )
    }

    /// Persist approval for exactly all current repository servers without starting them.
    pub async fn approve_managed_all(
        &self,
        repository_id: &ManagedRepositoryId,
        expected_commit: &str,
        requests: &[ManagedApprovalRequest],
        enable: bool,
    ) -> Result<()> {
        self.inner.write().await.registry.approve_managed_all(
            repository_id,
            expected_commit,
            requests,
            enable,
        )
    }
}
