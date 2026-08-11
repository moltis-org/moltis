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
}
