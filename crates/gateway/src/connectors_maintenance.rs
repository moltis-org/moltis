use super::*;

#[derive(Clone, Copy)]
pub(super) enum SyncTrigger {
    Manual,
    Scheduled { due_at: OffsetDateTime },
}

pub(super) struct ActiveSyncGuard<'a> {
    active_datasets: &'a DashSet<String>,
    active_accounts: &'a DashSet<String>,
    dataset_id: String,
    account_id: String,
}

impl<'a> ActiveSyncGuard<'a> {
    pub(super) fn new(
        active_datasets: &'a DashSet<String>,
        active_accounts: &'a DashSet<String>,
        dataset_id: &str,
        account_id: &str,
    ) -> Self {
        Self {
            active_datasets,
            active_accounts,
            dataset_id: dataset_id.to_owned(),
            account_id: account_id.to_owned(),
        }
    }
}

impl Drop for ActiveSyncGuard<'_> {
    fn drop(&mut self) {
        self.active_datasets.remove(&self.dataset_id);
        self.active_accounts.remove(&self.account_id);
    }
}

impl ConnectorManager {
    pub async fn start_projection_maintenance(self: &Arc<Self>) {
        let mut maintenance = self.maintenance.lock().await;
        if maintenance
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
        {
            return;
        }
        if let Some(handle) = maintenance.take() {
            let _ = handle.await;
        }

        let manager = Arc::downgrade(self);
        let cancellation = self.cancellation.clone();
        *maintenance = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(PROJECTION_MAINTENANCE_INTERVAL);
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let Some(manager) = manager.upgrade() else {
                            break;
                        };
                        if let Err(error) = manager.reconcile_projection_directories().await {
                            tracing::warn!(?error, "connector projection maintenance failed");
                        }
                    },
                }
            }
        }));
    }

    pub(super) async fn write_dataset_projection(&self, dataset: &Dataset) -> Result<()> {
        let mut maintenance = self.projection_maintenance.lock().await;
        let result = self.write_dataset_projection_locked(dataset).await;
        if result.is_ok() {
            maintenance.revision = maintenance.revision.wrapping_add(1);
        }
        result
    }

    async fn write_dataset_projection_locked(&self, dataset: &Dataset) -> Result<()> {
        let items = self.all_active_items(&dataset.id).await?;
        moltis_connectors::write_projection(&self.export_root, &dataset.name, dataset, &items)
            .await
            .map_err(ConnectorManagerError::from)?;
        let current = self.dataset_required(&dataset.id).await?;
        if current.last_sync_at != dataset.last_sync_at
            || current.synced_plan_revision != dataset.synced_plan_revision
            || current.item_count != dataset.item_count
            || current.name != dataset.name
            || current.projections != dataset.projections
        {
            return Err(ConnectorManagerError::Conflict(dataset.id.clone()));
        }
        Ok(())
    }

    async fn all_active_items(&self, dataset_id: &str) -> Result<Vec<ConnectorItem>> {
        let mut items = Vec::new();
        let mut offset = 0;
        loop {
            let page = self
                .store
                .query_items(dataset_id, ItemQuery {
                    limit: MAX_QUERY_LIMIT,
                    offset,
                    include_deleted: false,
                    text: None,
                })
                .await
                .map_err(ConnectorManagerError::from)?;
            let page_len = u64::try_from(page.len())
                .map_err(|error| internal(error, "convert connector projection page length"))?;
            items.extend(page);
            if page_len < MAX_QUERY_LIMIT {
                break;
            }
            offset = offset.checked_add(page_len).ok_or_else(|| {
                ConnectorManagerError::InvalidInput("connector item offset overflow".to_owned())
            })?;
        }
        Ok(items)
    }

    pub(super) async fn fail_and_reschedule(
        &self,
        dataset: &Dataset,
        run_id: &str,
        error: &str,
    ) -> Result<()> {
        let next_sync_at = next_sync_at(dataset.enabled, dataset.schedule_minutes)?;
        self.store
            .fail_sync_run_and_reschedule(run_id, error, next_sync_at)
            .await
            .map_err(ConnectorManagerError::from)?;
        Ok(())
    }

    pub(super) async fn run_due_datasets(self: &Arc<Self>) -> Result<()> {
        let now = OffsetDateTime::now_utc();
        let accounts = self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?;
        let mut due = Vec::new();
        for account in accounts.into_iter().filter(|account| account.enabled) {
            let datasets = self
                .store
                .list_datasets(&account.id)
                .await
                .map_err(ConnectorManagerError::from)?;
            for dataset in datasets.into_iter().filter(|dataset| {
                dataset.enabled && dataset.next_sync_at.is_some_and(|next| next <= now)
            }) {
                due.push(dataset.id);
            }
        }
        futures::stream::iter(due)
            .for_each_concurrent(SCHEDULER_CONCURRENCY, |dataset_id| async move {
                if self.cancellation.is_cancelled() {
                    return;
                }
                if let Err(error) = self
                    .spawn_sync(&dataset_id, SyncTrigger::Scheduled { due_at: now })
                    .await
                    && !matches!(error, ConnectorManagerError::Conflict(_))
                {
                    tracing::warn!(
                        dataset_id,
                        error = ?error,
                        "scheduled connector sync failed"
                    );
                }
            })
            .await;
        Ok(())
    }

    pub(super) fn projection_path(&self, dataset: &Dataset) -> Result<Option<PathBuf>> {
        if !(dataset.projections.jsonl || dataset.projections.markdown) {
            return Ok(None);
        }
        moltis_connectors::projection_directory(&self.export_root, &dataset.name, &dataset.id)
            .map(Some)
            .map_err(ConnectorManagerError::from)
    }

    pub(super) async fn remove_projection_best_effort(&self, dataset: &Dataset) {
        match self.projection_path(dataset) {
            Ok(Some(path)) => self.remove_projection_path_best_effort(&path).await,
            Ok(None) => {},
            Err(error) => tracing::warn!(
                dataset_id = dataset.id,
                ?error,
                "failed to resolve obsolete connector projection path"
            ),
        }
    }

    pub(super) async fn remove_projection_path_best_effort(&self, path: &Path) {
        let mut maintenance = self.projection_maintenance.lock().await;
        remove_directory_best_effort(path).await;
        maintenance.revision = maintenance.revision.wrapping_add(1);
    }

    pub(super) async fn reconcile_projection_directories(&self) -> Result<()> {
        let mut maintenance = self.projection_maintenance.lock().await;
        moltis_connectors::cleanup_projection_artifacts(&self.export_root)
            .await
            .map_err(ConnectorManagerError::from)?;
        let accounts = self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?;
        let mut expected = HashSet::new();
        let mut projected = Vec::new();
        for account in accounts {
            for dataset in self
                .store
                .list_datasets(&account.id)
                .await
                .map_err(ConnectorManagerError::from)?
            {
                if let Some(path) = self.projection_path(&dataset)? {
                    expected.insert(path.clone());
                    projected.push((dataset, path));
                }
            }
        }

        let mut entries = tokio::fs::read_dir(&self.export_root)
            .await
            .map_err(|error| internal(error, "read connector projection directory"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| internal(error, "read connector projection entry"))?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| internal(error, "inspect connector projection entry"))?;
            if file_type.is_dir() && !expected.contains(&path) {
                remove_directory_best_effort(&path).await;
                maintenance.revision = maintenance.revision.wrapping_add(1);
            }
        }
        for (dataset, path) in projected {
            if self.active_dataset_ids.contains(&dataset.id)
                || !projection_needs_rebuild(&dataset, &path).await
            {
                continue;
            }
            match self.write_dataset_projection_locked(&dataset).await {
                Ok(()) => {
                    maintenance.revision = maintenance.revision.wrapping_add(1);
                    if dataset.last_error.as_deref() == Some(PROJECTION_ERROR)
                        && let Err(error) =
                            self.store.set_dataset_last_error(&dataset.id, None).await
                    {
                        tracing::warn!(
                            dataset_id = dataset.id,
                            ?error,
                            "failed to clear recovered connector projection error"
                        );
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        dataset_id = dataset.id,
                        ?error,
                        "failed to rebuild stale connector projection"
                    );
                    if let Err(metadata_error) = self
                        .store
                        .set_dataset_last_error(&dataset.id, Some(PROJECTION_ERROR))
                        .await
                    {
                        tracing::warn!(
                            dataset_id = dataset.id,
                            error = ?metadata_error,
                            "failed to record stale connector projection error"
                        );
                    }
                },
            }
        }
        Ok(())
    }
}
