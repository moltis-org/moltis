//! Gateway-owned connector lifecycle, persistence, scheduling, and API views.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration as StdDuration,
};

use {
    dashmap::DashSet,
    futures::StreamExt,
    moltis_channels::ChannelRegistry,
    moltis_connector_caldav::{CalDavAccountConfig, CalDavConnector, CalDavDatasetConfig},
    moltis_connectors::{
        Account, AccountCreate, AccountUpdate, ConnectorError, ConnectorItem, ConnectorKind,
        Dataset, DatasetCreate, DatasetUpdate, ItemQuery, MAX_QUERY_LIMIT, ProjectionManifest,
        SqliteConnectorStore, SyncRun,
    },
    secrecy::ExposeSecret,
    serde_json::Value,
    sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    time::{Duration, OffsetDateTime},
    tokio::{sync::Mutex, task::JoinHandle},
    tokio_util::{sync::CancellationToken, task::TaskTracker},
    uuid::Uuid,
};

#[path = "connectors_helpers.rs"]
mod helpers;
use helpers::*;
#[path = "connectors/channel_history.rs"]
mod channel_history;
#[path = "connectors_maintenance.rs"]
mod maintenance;
use maintenance::{ActiveSyncGuard, SyncTrigger};
#[path = "connectors_models.rs"]
mod models;
pub use models::*;
use models::{account_view, dataset_view};
#[path = "connectors_legacy.rs"]
mod legacy;

use crate::connectors_planner::ConnectorPlanner;

#[cfg(feature = "vault")]
use {
    moltis_secret_store::{
        decrypt_secret_fields, encrypt_secret_fields, has_encrypted_secret_fields,
        has_plaintext_secret_fields,
    },
    moltis_vault::VaultStatus,
};

#[cfg(feature = "vault")]
const PASSWORD_FIELD: &[&str] = &["password"];
const REDACTED_PASSWORD: &str = "[REDACTED]";
const SCHEDULER_INTERVAL: StdDuration = StdDuration::from_secs(30);
const PROJECTION_MAINTENANCE_INTERVAL: StdDuration = StdDuration::from_secs(300);
const SCHEDULER_CONCURRENCY: usize = 4;
const PROJECTION_ERROR: &str = "Dataset synchronized, but projection generation failed";
const MAX_INSTRUCTION_BYTES: usize = 16 * 1024;

pub type Result<T> = std::result::Result<T, ConnectorManagerError>;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorManagerError {
    #[error("invalid connector input: {0}")]
    InvalidInput(String),
    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },
    #[error("connector dataset is already synchronizing: {0}")]
    Conflict(String),
    #[error("connector service unavailable: {0}")]
    Unavailable(String),
    #[error("connector provider operation failed")]
    Provider(#[source] moltis_connector_caldav::CalDavConnectorError),
    #[error("channel history operation failed")]
    Channel(#[source] moltis_channels::Error),
    #[error("internal connector operation failed")]
    Internal(#[source] anyhow::Error),
}

pub struct ConnectorManager {
    store: Arc<SqliteConnectorStore>,
    caldav: CalDavConnector,
    export_root: PathBuf,
    #[cfg(feature = "vault")]
    vault: Option<Arc<moltis_vault::Vault>>,
    active_dataset_ids: DashSet<String>,
    active_account_ids: DashSet<String>,
    account_mutations: Mutex<AccountMutationState>,
    projection_maintenance: Mutex<ProjectionMaintenanceState>,
    task_admission: Mutex<TaskAdmissionState>,
    cancellation: CancellationToken,
    scheduler: Mutex<Option<JoinHandle<()>>>,
    maintenance: Mutex<Option<JoinHandle<()>>>,
    sync_tasks: TaskTracker,
    planner: OnceLock<ConnectorPlanner>,
    channel_registry: OnceLock<Arc<ChannelRegistry>>,
}

#[derive(Default)]
struct AccountMutationState {
    revision: u64,
}

#[derive(Default)]
struct ProjectionMaintenanceState {
    revision: u64,
}

#[derive(Default)]
struct TaskAdmissionState {
    closing: bool,
}

impl ConnectorManager {
    pub async fn open(
        data_dir: &Path,
        max_connections: u32,
        #[cfg(feature = "vault")] vault: Option<Arc<moltis_vault::Vault>>,
    ) -> Result<Self> {
        if max_connections == 0 {
            return Err(ConnectorManagerError::InvalidInput(
                "database max_connections must be greater than zero".to_owned(),
            ));
        }

        let export_root = data_dir.join("connectors").join("exports");
        tokio::fs::create_dir_all(&export_root)
            .await
            .map_err(|error| internal(error, "create connector export directory"))?;
        restrict_directory_permissions(&export_root).await?;
        let db_path = data_dir.join("connectors.db");
        let is_new = !db_path.exists();
        let mut options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(StdDuration::from_secs(5));
        if is_new {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(|error| internal(error, "open connector database"))?;
        restrict_file_permissions(&db_path).await?;
        moltis_connectors::run_migrations(&pool)
            .await
            .map_err(ConnectorManagerError::from)?;

        let store = Arc::new(SqliteConnectorStore::new(pool));
        store
            .fail_running_sync_runs("Connector process stopped before this sync completed")
            .await
            .map_err(ConnectorManagerError::from)?;

        let manager = Self {
            store,
            caldav: CalDavConnector,
            export_root,
            #[cfg(feature = "vault")]
            vault,
            active_dataset_ids: DashSet::new(),
            active_account_ids: DashSet::new(),
            account_mutations: Mutex::new(AccountMutationState::default()),
            projection_maintenance: Mutex::new(ProjectionMaintenanceState::default()),
            task_admission: Mutex::new(TaskAdmissionState::default()),
            cancellation: CancellationToken::new(),
            scheduler: Mutex::new(None),
            maintenance: Mutex::new(None),
            sync_tasks: TaskTracker::new(),
            planner: OnceLock::new(),
            channel_registry: OnceLock::new(),
        };
        if let Err(error) = manager.reconcile_projection_directories().await {
            tracing::warn!(?error, "initial connector projection maintenance failed");
        }
        Ok(manager)
    }

    pub fn configure_planner(
        &self,
        registry: Arc<tokio::sync::RwLock<moltis_providers::ProviderRegistry>>,
    ) -> Result<()> {
        self.planner
            .set(ConnectorPlanner::new(registry))
            .map_err(|_| {
                ConnectorManagerError::Internal(anyhow::anyhow!(
                    "connector planner was initialized more than once"
                ))
            })
    }

    #[must_use]
    pub fn available(&self) -> Vec<ConnectorDescriptor> {
        vec![
            ConnectorDescriptor {
                kind: ConnectorKind::Caldav,
                display_name: "CalDAV",
            },
            ConnectorDescriptor {
                kind: ConnectorKind::ChannelHistory,
                display_name: "Channel messages",
            },
        ]
    }

    pub async fn list_accounts(&self) -> Result<Vec<AccountView>> {
        self.store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?
            .into_iter()
            .map(account_view)
            .collect()
    }

    pub async fn add_account(&self, request: AccountCreateRequest) -> Result<AccountView> {
        self.add_account_with_source(request, None).await
    }

    async fn add_account_with_source(
        &self,
        request: AccountCreateRequest,
        source_key: Option<String>,
    ) -> Result<AccountView> {
        let mut mutation = self.account_mutations.lock().await;
        let id = Uuid::new_v4().to_string();
        let kind = request.kind;
        let config = match kind {
            ConnectorKind::Caldav => {
                validate_password_replacement(request.password.expose_secret())?;
                validate_account_fields(
                    &request.server_url,
                    &request.username,
                    request.timeout_seconds,
                    request.allow_insecure_http,
                )?;
                let mut config = account_config_value(
                    request.server_url,
                    request.username,
                    Value::String(request.password.expose_secret().to_owned()),
                    request.timeout_seconds,
                    request.allow_insecure_http,
                    request.allow_private_network,
                );
                self.prepare_secret_for_storage(&id, &mut config).await?;
                config
            },
            ConnectorKind::ChannelHistory => {
                let channel_type = request.channel_type.ok_or_else(|| {
                    ConnectorManagerError::InvalidInput(
                        "channelType is required for channel history".to_owned(),
                    )
                })?;
                let channel_account_id = request.channel_account_id.ok_or_else(|| {
                    ConnectorManagerError::InvalidInput(
                        "channelAccountId is required for channel history".to_owned(),
                    )
                })?;
                self.validate_channel_source(channel_type, &channel_account_id)
                    .await?;
                serde_json::to_value(ChannelHistoryAccountConfig {
                    channel_type,
                    channel_account_id,
                })
                .map_err(|error| internal(error, "serialize channel connector account"))?
            },
        };
        let account = self
            .store
            .create_account_with_id(&id, AccountCreate {
                kind,
                name: request.name,
                config,
                enabled: request.enabled,
                source_key,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        mutation.revision = mutation.revision.wrapping_add(1);
        account_view(account)
    }

    pub async fn update_account(
        &self,
        id: &str,
        request: AccountUpdateRequest,
    ) -> Result<AccountView> {
        let mut mutation = self.account_mutations.lock().await;
        let existing = self.account_required(id).await?;
        self.ensure_account_inactive(id).await?;
        if existing.kind == ConnectorKind::ChannelHistory {
            let config: ChannelHistoryAccountConfig =
                serde_json::from_value(existing.config.clone())
                    .map_err(|error| internal(error, "deserialize channel connector account"))?;
            self.validate_channel_source(config.channel_type, &config.channel_account_id)
                .await?;
            let account = self
                .store
                .update_account(id, AccountUpdate {
                    name: request.name,
                    config: existing.config,
                    enabled: request.enabled,
                })
                .await
                .map_err(ConnectorManagerError::from)?;
            mutation.revision = mutation.revision.wrapping_add(1);
            return account_view(account);
        }
        ensure_caldav(existing.kind)?;

        let replacement = request
            .password
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .map(String::as_str)
            .filter(|password| *password != REDACTED_PASSWORD);
        let stored: StoredAccountConfigView = serde_json::from_value(existing.config.clone())
            .map_err(|error| internal(error, "deserialize connector account"))?;
        let authority_changed =
            stored.server_url != request.server_url || stored.username != request.username;
        if existing.source_key.is_some()
            && (existing.name != request.name
                || authority_changed
                || stored.timeout_seconds != request.timeout_seconds
                || existing.enabled != request.enabled
                || replacement.is_some())
        {
            return Err(ConnectorManagerError::InvalidInput(
                "config-managed account fields must be changed in moltis.toml".to_owned(),
            ));
        }
        if existing.source_key.is_none() && authority_changed && replacement.is_none() {
            return Err(ConnectorManagerError::InvalidInput(
                "password must be re-entered when changing the server URL or username".to_owned(),
            ));
        }
        if let Some(password) = replacement {
            validate_password_replacement(password)?;
        }
        validate_account_fields(
            &request.server_url,
            &request.username,
            request.timeout_seconds,
            request.allow_insecure_http,
        )?;

        let password = match replacement {
            Some(password) => Value::String(password.to_owned()),
            None => stored_password(&existing.config)?.clone(),
        };
        let mut config = account_config_value(
            request.server_url,
            request.username,
            password,
            request.timeout_seconds,
            request.allow_insecure_http,
            request.allow_private_network,
        );
        self.prepare_secret_for_storage(id, &mut config).await?;
        let account = self
            .store
            .update_account(id, AccountUpdate {
                name: request.name,
                config,
                enabled: request.enabled,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        mutation.revision = mutation.revision.wrapping_add(1);
        account_view(account)
    }

    pub async fn remove_account(&self, id: &str) -> Result<()> {
        let mut mutation = self.account_mutations.lock().await;
        let account = self.account_required(id).await?;
        if account.source_key.is_some() {
            return Err(ConnectorManagerError::InvalidInput(
                "config-managed connector accounts must be removed from moltis.toml".to_owned(),
            ));
        }
        let datasets = self
            .store
            .list_datasets(&account.id)
            .await
            .map_err(ConnectorManagerError::from)?;
        self.ensure_datasets_inactive(&datasets)?;
        if self
            .store
            .remove_account(id)
            .await
            .map_err(ConnectorManagerError::from)?
        {
            mutation.revision = mutation.revision.wrapping_add(1);
            for dataset in datasets {
                self.remove_projection_best_effort(&dataset).await;
            }
            Ok(())
        } else {
            Err(not_found("account", id))
        }
    }

    pub async fn test_account(&self, id: &str) -> Result<AccountTestView> {
        let account = self.account_required(id).await?;
        if account.kind == ConnectorKind::ChannelHistory {
            let config: ChannelHistoryAccountConfig = serde_json::from_value(account.config)
                .map_err(|error| internal(error, "deserialize channel connector account"))?;
            self.validate_channel_source(config.channel_type, &config.channel_account_id)
                .await?;
            return Ok(AccountTestView::ChannelReady {
                channel_ready: true,
            });
        }
        let config = self.runtime_account_config(id).await?;
        self.caldav
            .test_connection(&config)
            .await
            .map_err(ConnectorManagerError::Provider)
            .map(|calendars| AccountTestView::Calendars {
                calendars: calendars
                    .into_iter()
                    .map(|calendar| CalendarView {
                        href: calendar.href,
                        display_name: calendar.display_name,
                        color: calendar.color,
                        description: calendar.description,
                        collection_etag: calendar.collection_etag,
                        supports_sync: calendar.supports_sync,
                    })
                    .collect(),
            })
    }

    pub async fn list_datasets(&self) -> Result<Vec<DatasetView>> {
        let accounts = self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?;
        let mut datasets = Vec::new();
        for account in accounts {
            let account_datasets = self
                .store
                .list_datasets(&account.id)
                .await
                .map_err(ConnectorManagerError::from)?;
            datasets.extend(
                account_datasets
                    .into_iter()
                    .map(|dataset| dataset_view(dataset, account.kind, &self.export_root))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        datasets.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(datasets)
    }

    pub async fn add_dataset(&self, request: DatasetCreateRequest) -> Result<DatasetView> {
        let mut mutation = self.account_mutations.lock().await;
        let account = self.account_required(&request.account_id).await?;
        validate_instruction(&request.instruction)?;
        let config = self.validate_and_serialize_dataset_config(
            account.kind,
            request.config,
            request.schedule_minutes,
        )?;
        let next_sync_at = next_sync_at(request.enabled, request.schedule_minutes)?;
        let dataset = self
            .store
            .create_dataset(DatasetCreate {
                account_id: request.account_id,
                name: request.name,
                instruction: Some(request.instruction),
                config,
                schedule_minutes: request.schedule_minutes,
                projections: request.projections,
                enabled: request.enabled,
                next_sync_at,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        mutation.revision = mutation.revision.wrapping_add(1);
        dataset_view(dataset, account.kind, &self.export_root)
    }

    pub async fn update_dataset(
        &self,
        id: &str,
        request: DatasetUpdateRequest,
    ) -> Result<DatasetView> {
        let mut mutation = self.account_mutations.lock().await;
        if self.active_dataset_ids.contains(id) {
            return Err(ConnectorManagerError::Conflict(id.to_owned()));
        }
        let existing = self.dataset_required(id).await?;
        let account = self.account_required(&existing.account_id).await?;
        validate_instruction(&request.instruction)?;
        let config = self.validate_and_serialize_dataset_config(
            account.kind,
            request.config,
            request.schedule_minutes,
        )?;
        let next_sync_at = next_sync_at(request.enabled, request.schedule_minutes)?;
        let old_projection_path = self.projection_path(&existing)?;
        let projection_config_changed = existing.projections != request.projections;
        let dataset = self
            .store
            .update_dataset(id, DatasetUpdate {
                name: request.name,
                instruction: Some(request.instruction),
                config,
                schedule_minutes: request.schedule_minutes,
                projections: request.projections,
                enabled: request.enabled,
                next_sync_at,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        let new_projection_path = self.projection_path(&dataset)?;
        if old_projection_path != new_projection_path || projection_config_changed {
            if dataset.last_sync_at.is_some() && new_projection_path.is_some() {
                match self.write_dataset_projection(&dataset).await {
                    Ok(_) => {
                        if old_projection_path != new_projection_path
                            && let Some(path) = old_projection_path
                        {
                            self.remove_projection_path_best_effort(&path).await;
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            dataset_id = dataset.id,
                            ?error,
                            "failed to rebuild changed connector projection"
                        );
                    },
                }
            } else if new_projection_path.is_none()
                && let Some(path) = old_projection_path
            {
                self.remove_projection_path_best_effort(&path).await;
            }
        }
        mutation.revision = mutation.revision.wrapping_add(1);
        dataset_view(dataset, account.kind, &self.export_root)
    }

    pub async fn validate_dataset_draft(
        &self,
        account_id: &str,
        dataset_id: Option<&str>,
        instruction: &str,
        draft: &DatasetDraft,
    ) -> Result<()> {
        let account = self.account_required(account_id).await?;
        ensure_caldav(account.kind)?;
        if let Some(dataset_id) = dataset_id {
            let dataset = self.dataset_required(dataset_id).await?;
            if dataset.account_id != account_id {
                return Err(ConnectorManagerError::InvalidInput(
                    "dataset does not belong to account".to_owned(),
                ));
            }
        }
        if draft.name.trim().is_empty() {
            return Err(ConnectorManagerError::InvalidInput(
                "dataset name must not be empty".to_owned(),
            ));
        }
        validate_instruction(instruction)?;
        let config = CalDavDatasetConfig::from(draft.config.clone());
        validate_dataset_request(&config, draft.schedule_minutes)
    }

    pub async fn compile_dataset(
        &self,
        request: DatasetCompileRequest,
    ) -> Result<DatasetCompileResponse> {
        validate_instruction(&request.instruction)?;
        let account = self.account_required(&request.account_id).await?;
        ensure_caldav(account.kind)?;
        let existing = if let Some(dataset_id) = request.dataset_id.as_deref() {
            let dataset = self.dataset_required(dataset_id).await?;
            if dataset.account_id != request.account_id {
                return Err(ConnectorManagerError::InvalidInput(
                    "dataset does not belong to account".to_owned(),
                ));
            }
            Some(dataset_view(dataset, account.kind, &self.export_root)?)
        } else {
            None
        };
        let AccountTestView::Calendars { calendars } =
            self.test_account(&request.account_id).await?
        else {
            return Err(ConnectorManagerError::InvalidInput(
                "channel history datasets are configured directly".to_owned(),
            ));
        };
        let planner = self.planner.get().ok_or_else(|| {
            ConnectorManagerError::Unavailable("connector planner is not initialized".to_owned())
        })?;
        let response = planner
            .compile(&request, &calendars, existing.as_ref())
            .await?;
        self.validate_dataset_draft(
            &request.account_id,
            request.dataset_id.as_deref(),
            &request.instruction,
            &response.draft,
        )
        .await?;
        Ok(response)
    }

    pub async fn remove_dataset(&self, id: &str) -> Result<()> {
        let mut mutation = self.account_mutations.lock().await;
        if self.active_dataset_ids.contains(id) {
            return Err(ConnectorManagerError::Conflict(id.to_owned()));
        }
        let dataset = self.dataset_required(id).await?;
        if self
            .store
            .remove_dataset(id)
            .await
            .map_err(ConnectorManagerError::from)?
        {
            mutation.revision = mutation.revision.wrapping_add(1);
            self.remove_projection_best_effort(&dataset).await;
            Ok(())
        } else {
            Err(not_found("dataset", id))
        }
    }

    pub async fn sync_dataset(self: &Arc<Self>, id: &str) -> Result<SyncRunView> {
        self.spawn_sync(id, SyncTrigger::Manual)
            .await?
            .ok_or_else(|| internal(anyhow::anyhow!("manual sync was not started"), "start sync"))
    }

    async fn spawn_sync(
        self: &Arc<Self>,
        id: &str,
        trigger: SyncTrigger,
    ) -> Result<Option<SyncRunView>> {
        let admission = self.task_admission.lock().await;
        if admission.closing || self.cancellation.is_cancelled() {
            return Err(ConnectorManagerError::Unavailable(
                "connector service is shutting down".to_owned(),
            ));
        }
        let manager = Arc::clone(self);
        let id = id.to_owned();
        let task = self
            .sync_tasks
            .spawn(async move { manager.sync_dataset_task(&id, trigger).await });
        drop(admission);
        task.await
            .map_err(|error| internal(error, "join connector sync task"))?
    }

    async fn sync_dataset_task(
        &self,
        id: &str,
        trigger: SyncTrigger,
    ) -> Result<Option<SyncRunView>> {
        let (dataset, _active_guard) = {
            let _mutation = self.account_mutations.lock().await;
            if self.cancellation.is_cancelled() {
                return Err(ConnectorManagerError::Unavailable(
                    "connector service is shutting down".to_owned(),
                ));
            }
            let dataset = self.dataset_required(id).await?;
            let account = self.account_required(&dataset.account_id).await?;
            if !account.enabled || !dataset.enabled {
                if matches!(trigger, SyncTrigger::Scheduled { .. }) {
                    return Ok(None);
                }
                return Err(ConnectorManagerError::InvalidInput(
                    "connector account and dataset must be enabled before synchronization"
                        .to_owned(),
                ));
            }
            if let SyncTrigger::Scheduled { due_at } = trigger
                && !dataset.next_sync_at.is_some_and(|next| next <= due_at)
            {
                return Ok(None);
            }
            if self.active_dataset_ids.contains(id)
                || self.active_account_ids.contains(&dataset.account_id)
            {
                return Err(ConnectorManagerError::Conflict(id.to_owned()));
            }
            self.active_dataset_ids.insert(id.to_owned());
            self.active_account_ids.insert(dataset.account_id.clone());
            let guard = ActiveSyncGuard::new(
                &self.active_dataset_ids,
                &self.active_account_ids,
                id,
                &dataset.account_id,
            );
            (dataset, guard)
        };

        let run = self
            .store
            .start_sync_run(id)
            .await
            .map_err(ConnectorManagerError::from)?;
        let result = tokio::select! {
            _ = self.cancellation.cancelled() => Err(ConnectorManagerError::Unavailable(
                "connector service is shutting down".to_owned(),
            )),
            result = self.sync_dataset_inner(&dataset, &run.id) => result,
        };
        match result {
            Ok(run) => Ok(Some(SyncRunView::from(run))),
            Err(error) => {
                let safe_error = if self.cancellation.is_cancelled() {
                    "Connector sync cancelled during shutdown"
                } else {
                    safe_sync_error(&error)
                };
                if let Err(cleanup_error) = self
                    .fail_and_reschedule(&dataset, &run.id, safe_error)
                    .await
                {
                    tracing::warn!(
                        dataset_id = id,
                        error = ?cleanup_error,
                        "failed to finalize unsuccessful connector sync"
                    );
                }
                Err(error)
            },
        }
    }

    pub async fn list_recent_runs(&self, dataset_id: &str, limit: u64) -> Result<Vec<SyncRunView>> {
        self.dataset_required(dataset_id).await?;
        self.store
            .list_recent_sync_runs(dataset_id, limit)
            .await
            .map_err(ConnectorManagerError::from)
            .map(|runs| runs.into_iter().map(SyncRunView::from).collect())
    }

    pub async fn query_items(
        &self,
        dataset_id: &str,
        request: ItemQueryRequest,
    ) -> Result<Vec<ConnectorItemView>> {
        self.dataset_required(dataset_id).await?;
        self.store
            .query_items(dataset_id, ItemQuery {
                limit: request.limit,
                offset: request.offset,
                include_deleted: request.include_deleted,
                text: request.text,
            })
            .await
            .map_err(ConnectorManagerError::from)
            .map(|items| items.into_iter().map(ConnectorItemView::from).collect())
    }

    pub async fn get_item(
        &self,
        dataset_id: &str,
        item_id: &str,
        include_deleted: bool,
    ) -> Result<ConnectorItemView> {
        self.dataset_required(dataset_id).await?;
        self.store
            .get_item(dataset_id, item_id, include_deleted)
            .await
            .map_err(ConnectorManagerError::from)?
            .map(ConnectorItemView::from)
            .ok_or_else(|| not_found("connector item", item_id))
    }

    pub async fn start_scheduler(self: &Arc<Self>) {
        let mut scheduler = self.scheduler.lock().await;
        if scheduler
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
        {
            return;
        }
        if let Some(handle) = scheduler.take() {
            let _ = handle.await;
        }

        let manager = Arc::downgrade(self);
        let cancellation = self.cancellation.clone();
        *scheduler = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(SCHEDULER_INTERVAL);
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let Some(manager) = manager.upgrade() else {
                            break;
                        };
                        if let Err(error) = manager.run_due_datasets().await {
                            tracing::warn!(error = ?error, "connector scheduler pass failed");
                        }
                    },
                }
            }
        }));
    }

    pub async fn shutdown(&self) {
        let mut admission = self.task_admission.lock().await;
        admission.closing = true;
        self.cancellation.cancel();
        self.sync_tasks.close();
        drop(admission);
        if let Some(mut handle) = self.scheduler.lock().await.take()
            && tokio::time::timeout(StdDuration::from_secs(5), &mut handle)
                .await
                .is_err()
        {
            handle.abort();
            let _ = handle.await;
        }
        if let Some(mut handle) = self.maintenance.lock().await.take()
            && tokio::time::timeout(StdDuration::from_secs(5), &mut handle)
                .await
                .is_err()
        {
            handle.abort();
            let _ = handle.await;
        }
        if tokio::time::timeout(StdDuration::from_secs(5), self.sync_tasks.wait())
            .await
            .is_err()
        {
            tracing::warn!("connector sync tasks exceeded graceful shutdown timeout");
        }
    }

    #[cfg(feature = "vault")]
    pub async fn migrate_plaintext_credentials(&self) -> Result<usize> {
        if !crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            return Ok(0);
        }
        let Some(vault) = self.vault.as_ref() else {
            return Ok(0);
        };
        if !vault.is_unsealed().await {
            return Ok(0);
        }

        let mut mutation = self.account_mutations.lock().await;
        let mut changed = 0;
        for account in self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?
        {
            let mut config = account.config.clone();
            if !has_plaintext_secret_fields(&config, PASSWORD_FIELD)
                .map_err(|error| internal(error, "inspect connector password"))?
            {
                continue;
            }
            encrypt_secret_fields(
                &mut config,
                PASSWORD_FIELD,
                &secret_aad_scope(&account.id),
                vault.as_ref(),
            )
            .await
            .map_err(|error| internal(error, "encrypt connector password"))?;
            self.store
                .update_account(&account.id, AccountUpdate {
                    name: account.name,
                    config,
                    enabled: account.enabled,
                })
                .await
                .map_err(ConnectorManagerError::from)?;
            changed += 1;
        }
        mutation.revision = mutation
            .revision
            .wrapping_add(u64::try_from(changed).unwrap_or(u64::MAX));
        Ok(changed)
    }

    #[cfg(feature = "vault")]
    pub async fn decrypt_credentials_and_disable_vault(
        &self,
        vault: &moltis_vault::Vault,
    ) -> Result<usize> {
        let mut mutation = self.account_mutations.lock().await;
        let changed = decrypt_credentials(&self.store, vault).await?;
        moltis_config::update_config(|config| {
            config.auth.vault_enabled = false;
        })
        .map_err(|error| internal(error, "persist disabled connector vault encryption"))?;
        crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
        mutation.revision = mutation
            .revision
            .wrapping_add(u64::try_from(changed).unwrap_or(u64::MAX));
        Ok(changed)
    }

    #[cfg(feature = "vault")]
    pub async fn decrypt_all_credentials_at(
        data_dir: &Path,
        vault: &moltis_vault::Vault,
    ) -> Result<usize> {
        let db_path = data_dir.join("connectors.db");
        if !db_path.exists() {
            return Ok(0);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_path)
                    .foreign_keys(true)
                    .busy_timeout(StdDuration::from_secs(5)),
            )
            .await
            .map_err(|error| internal(error, "open connector database for vault disable"))?;
        let store = SqliteConnectorStore::new(pool);
        decrypt_credentials(&store, vault).await
    }

    async fn account_required(&self, id: &str) -> Result<Account> {
        self.store
            .get_account(id)
            .await
            .map_err(ConnectorManagerError::from)?
            .ok_or_else(|| not_found("account", id))
    }

    async fn dataset_required(&self, id: &str) -> Result<Dataset> {
        self.store
            .get_dataset(id)
            .await
            .map_err(ConnectorManagerError::from)?
            .ok_or_else(|| not_found("dataset", id))
    }

    async fn ensure_account_inactive(&self, account_id: &str) -> Result<()> {
        let datasets = self
            .store
            .list_datasets(account_id)
            .await
            .map_err(ConnectorManagerError::from)?;
        self.ensure_datasets_inactive(&datasets)
    }

    fn validate_and_serialize_dataset_config(
        &self,
        kind: ConnectorKind,
        config: ConnectorDatasetConfigView,
        schedule_minutes: Option<u64>,
    ) -> Result<Value> {
        match (kind, config) {
            (ConnectorKind::Caldav, ConnectorDatasetConfigView::CalDav(config)) => {
                let config = CalDavDatasetConfig::from(config);
                validate_dataset_request(&config, schedule_minutes)?;
                serde_json::to_value(config)
                    .map_err(|error| internal(error, "serialize CalDAV dataset config"))
            },
            (ConnectorKind::ChannelHistory, ConnectorDatasetConfigView::ChannelHistory(config)) => {
                validate_channel_dataset_config(&config, schedule_minutes)?;
                serde_json::to_value(config)
                    .map_err(|error| internal(error, "serialize channel dataset config"))
            },
            _ => Err(ConnectorManagerError::InvalidInput(
                "dataset config does not match connector kind".to_owned(),
            )),
        }
    }

    fn ensure_datasets_inactive(&self, datasets: &[Dataset]) -> Result<()> {
        if let Some(dataset) = datasets
            .iter()
            .find(|dataset| self.active_dataset_ids.contains(&dataset.id))
        {
            return Err(ConnectorManagerError::Conflict(dataset.id.clone()));
        }
        Ok(())
    }

    async fn runtime_account_config(&self, id: &str) -> Result<CalDavAccountConfig> {
        let account = self.account_required(id).await?;
        ensure_caldav(account.kind)?;
        let mut config = account.config;
        self.decrypt_secret_for_runtime(id, &mut config).await?;
        let config: CalDavAccountConfig = serde_json::from_value(config)
            .map_err(|error| internal(error, "deserialize stored CalDAV account config"))?;
        config.validate().map_err(invalid_caldav)?;
        Ok(config)
    }

    async fn sync_dataset_inner(&self, dataset: &Dataset, run_id: &str) -> Result<SyncRun> {
        let account = self.account_required(&dataset.account_id).await?;
        if account.kind == ConnectorKind::ChannelHistory {
            return self.sync_channel_dataset(dataset, run_id, account).await;
        }
        ensure_caldav(account.kind)?;
        let account_config = self.runtime_account_config(&account.id).await?;
        let dataset_config: CalDavDatasetConfig = serde_json::from_value(dataset.config.clone())
            .map_err(|error| internal(error, "deserialize stored CalDAV dataset config"))?;
        dataset_config.validate().map_err(invalid_caldav)?;
        let existing = self
            .store
            .source_states(&dataset.id)
            .await
            .map_err(ConnectorManagerError::from)?;
        let snapshot = self
            .caldav
            .sync_dataset(
                &account_config,
                &dataset_config,
                existing,
                dataset.plan_revision,
            )
            .await
            .map_err(ConnectorManagerError::Provider)?;
        let next_sync_at = next_sync_at(dataset.enabled, dataset.schedule_minutes)?;
        let (_stats, run) = self
            .store
            .commit_sync_snapshot(
                run_id,
                &dataset.id,
                &snapshot.items,
                &snapshot.source_observations,
                next_sync_at,
            )
            .await
            .map_err(ConnectorManagerError::from)?;

        let projection_error = if dataset.projections.jsonl || dataset.projections.markdown {
            let projected_dataset = self.dataset_required(&dataset.id).await?;
            match self.write_dataset_projection(&projected_dataset).await {
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(
                        dataset_id = dataset.id,
                        ?error,
                        "connector projection update failed"
                    );
                    Some(PROJECTION_ERROR)
                },
            }
        } else {
            None
        };

        if let Some(error) = projection_error
            && let Err(metadata_error) = self
                .store
                .set_dataset_last_error(&dataset.id, Some(error))
                .await
        {
            tracing::warn!(
                dataset_id = dataset.id,
                error = ?metadata_error,
                "failed to record connector projection error"
            );
        }
        Ok(run)
    }

    async fn write_dataset_projection(&self, dataset: &Dataset) -> Result<()> {
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

    async fn fail_and_reschedule(
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

    async fn run_due_datasets(self: &Arc<Self>) -> Result<()> {
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

    fn projection_path(&self, dataset: &Dataset) -> Result<Option<PathBuf>> {
        if !(dataset.projections.jsonl || dataset.projections.markdown) {
            return Ok(None);
        }
        moltis_connectors::projection_directory(&self.export_root, &dataset.name, &dataset.id)
            .map(Some)
            .map_err(ConnectorManagerError::from)
    }

    async fn remove_projection_best_effort(&self, dataset: &Dataset) {
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

    async fn remove_projection_path_best_effort(&self, path: &Path) {
        let mut maintenance = self.projection_maintenance.lock().await;
        remove_directory_best_effort(path).await;
        maintenance.revision = maintenance.revision.wrapping_add(1);
    }

    async fn reconcile_projection_directories(&self) -> Result<()> {
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

    #[cfg(feature = "vault")]
    async fn prepare_secret_for_storage(&self, id: &str, config: &mut Value) -> Result<()> {
        if !crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            return Ok(());
        }
        let Some(vault) = self.vault.as_ref() else {
            return Err(ConnectorManagerError::Unavailable(
                "vault encryption is enabled, but the vault is unavailable".to_owned(),
            ));
        };
        if vault.is_unsealed().await {
            encrypt_secret_fields(
                config,
                PASSWORD_FIELD,
                &secret_aad_scope(id),
                vault.as_ref(),
            )
            .await
            .map_err(|error| internal(error, "encrypt connector password"))?;
            return Ok(());
        }
        let status = vault
            .status()
            .await
            .map_err(|error| internal(error, "check connector vault status"))?;
        if matches!(status, VaultStatus::Uninitialized) {
            return Ok(());
        }
        if has_plaintext_secret_fields(config, PASSWORD_FIELD)
            .map_err(|error| internal(error, "inspect connector password"))?
        {
            return Err(ConnectorManagerError::Unavailable(
                "vault is sealed; connector passwords cannot be persisted".to_owned(),
            ));
        }
        Ok(())
    }

    #[cfg(not(feature = "vault"))]
    async fn prepare_secret_for_storage(&self, _id: &str, _config: &mut Value) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "vault")]
    async fn decrypt_secret_for_runtime(&self, id: &str, config: &mut Value) -> Result<()> {
        if crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            let Some(vault) = self.vault.as_ref() else {
                return Err(ConnectorManagerError::Unavailable(
                    "vault encryption is enabled, but the vault is unavailable".to_owned(),
                ));
            };
            let status = vault
                .status()
                .await
                .map_err(|error| internal(error, "check connector vault status"))?;
            if matches!(status, VaultStatus::Sealed) {
                return Err(ConnectorManagerError::Unavailable(
                    "vault is sealed; connector passwords are unavailable".to_owned(),
                ));
            }
        }
        if !has_encrypted_secret_fields(config, PASSWORD_FIELD)
            .map_err(|error| internal(error, "inspect encrypted connector password"))?
        {
            return Ok(());
        }
        let Some(vault) = self.vault.as_ref() else {
            return Err(ConnectorManagerError::Unavailable(
                "encrypted connector passwords require the vault".to_owned(),
            ));
        };
        if !vault.is_unsealed().await {
            return Err(ConnectorManagerError::Unavailable(
                "vault is sealed; connector passwords are unavailable".to_owned(),
            ));
        }
        decrypt_secret_fields(
            config,
            PASSWORD_FIELD,
            &secret_aad_scope(id),
            vault.as_ref(),
        )
        .await
        .map_err(|error| internal(error, "decrypt connector password"))?;
        Ok(())
    }

    #[cfg(not(feature = "vault"))]
    async fn decrypt_secret_for_runtime(&self, _id: &str, _config: &mut Value) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "vault")]
async fn decrypt_credentials(
    store: &SqliteConnectorStore,
    vault: &moltis_vault::Vault,
) -> Result<usize> {
    let mut changed = 0;
    for account in store
        .list_accounts()
        .await
        .map_err(ConnectorManagerError::from)?
    {
        let mut config = account.config.clone();
        if !has_encrypted_secret_fields(&config, PASSWORD_FIELD)
            .map_err(|error| internal(error, "inspect connector secret"))?
        {
            continue;
        }
        decrypt_secret_fields(
            &mut config,
            PASSWORD_FIELD,
            &secret_aad_scope(&account.id),
            vault,
        )
        .await
        .map_err(|error| internal(error, "decrypt connector secret"))?;
        store
            .update_account(&account.id, AccountUpdate {
                name: account.name,
                config,
                enabled: account.enabled,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        changed += 1;
    }
    Ok(changed)
}

fn safe_sync_error(error: &ConnectorManagerError) -> &'static str {
    match error {
        ConnectorManagerError::Channel(_) => "Channel message history synchronization failed",
        ConnectorManagerError::Provider(
            moltis_connector_caldav::CalDavConnectorError::NetworkSafety(_),
        ) => "CalDAV server address was blocked by network safety policy",
        ConnectorManagerError::Provider(moltis_connector_caldav::CalDavConnectorError::Client(
            moltis_caldav::Error::Timeout(_),
        )) => "CalDAV server timed out",
        ConnectorManagerError::Provider(moltis_connector_caldav::CalDavConnectorError::Client(
            moltis_caldav::Error::Protocol(message),
        )) if message.contains("401") || message.contains("Unauthorized") => {
            "CalDAV authentication failed"
        },
        ConnectorManagerError::Provider(moltis_connector_caldav::CalDavConnectorError::Client(
            moltis_caldav::Error::Protocol(message),
        )) if message.contains("403") || message.contains("Forbidden") => {
            "CalDAV account does not have access to the requested calendars"
        },
        ConnectorManagerError::Provider(_) => "CalDAV server synchronization failed",
        ConnectorManagerError::Unavailable(_) => {
            "Connector credentials are unavailable while the vault is sealed"
        },
        _ => "Connector synchronization failed",
    }
}

impl From<ConnectorError> for ConnectorManagerError {
    fn from(error: ConnectorError) -> Self {
        match error {
            ConnectorError::InvalidInput(message) => Self::InvalidInput(message),
            ConnectorError::QueryLimit { requested, max } => Self::InvalidInput(format!(
                "query limit must be between 1 and {max}, got {requested}"
            )),
            ConnectorError::NotFound { entity, id } => Self::NotFound { entity, id },
            other => Self::Internal(anyhow::Error::new(other)),
        }
    }
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
