use std::collections::HashSet;

use {
    async_trait::async_trait,
    sqlx::{Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow},
    time::OffsetDateTime,
    uuid::Uuid,
};

use crate::{
    Account, AccountCreate, AccountUpdate, ConnectorError, ConnectorItem, ConnectorItemInput,
    ConnectorKind, ConnectorReader, Dataset, DatasetCreate, DatasetUpdate, ItemQuery,
    ProjectionConfig, Result, SnapshotResult, SourceObservation, SourceStateMap, SyncRun,
    SyncRunStatus, search::fts_query,
};

pub const MAX_QUERY_LIMIT: u64 = 500;
const MAX_SOURCE_KEY_LENGTH: usize = 1024;

pub struct SqliteConnectorStore {
    pool: SqlitePool,
}

impl SqliteConnectorStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn dataset_has_kind(&self, dataset_id: &str, kind: ConnectorKind) -> Result<bool> {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM connector_datasets AS dataset \
             JOIN connector_accounts AS account ON account.id = dataset.account_id \
             WHERE dataset.id = ? AND account.kind = ?)",
        )
        .bind(dataset_id)
        .bind(kind.as_str())
        .fetch_one(&self.pool)
        .await
        .map(|exists| exists != 0)
        .map_err(Into::into)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, input), fields(kind = %input.kind)))]
    pub async fn create_account(&self, input: AccountCreate) -> Result<Account> {
        let id = Uuid::new_v4().to_string();
        self.create_account_with_id(&id, input).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, input), fields(kind = %input.kind)))]
    pub async fn create_account_with_id(&self, id: &str, input: AccountCreate) -> Result<Account> {
        require_name(&input.name, "account")?;
        validate_optional_source_key(input.source_key.as_deref())?;
        if id.trim().is_empty() {
            return Err(ConnectorError::InvalidInput(
                "account id must not be empty".to_owned(),
            ));
        }
        let now = now_millis();
        let config = serde_json::to_string(&input.config)?;
        sqlx::query(
            "INSERT INTO connector_accounts \
             (id, kind, source_key, name, config, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(input.kind.as_str())
        .bind(&input.source_key)
        .bind(&input.name)
        .bind(config)
        .bind(input.enabled)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        record_write("account.create");
        self.get_account_required(id).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, input)))]
    pub async fn update_account(&self, id: &str, input: AccountUpdate) -> Result<Account> {
        require_name(&input.name, "account")?;
        let config = serde_json::to_string(&input.config)?;
        let mut transaction = self.pool.begin().await?;
        let previous_config =
            sqlx::query_scalar::<_, String>("SELECT config FROM connector_accounts WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| not_found("account", id))?;
        let config_changed = previous_config != config;
        let result = sqlx::query(
            "UPDATE connector_accounts SET name = ?, config = ?, enabled = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(input.name)
        .bind(config)
        .bind(input.enabled)
        .bind(now_millis())
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        require_affected(result.rows_affected(), "account", id)?;
        if config_changed {
            sqlx::query(
                "UPDATE connector_datasets SET plan_revision = plan_revision + 1 \
                 WHERE account_id = ?",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        record_write("account.update");
        self.get_account_required(id).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn remove_account(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM connector_accounts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        let removed = result.rows_affected() != 0;
        if removed {
            record_write("account.remove");
        }
        Ok(removed)
    }

    pub async fn get_account(&self, id: &str) -> Result<Option<Account>> {
        sqlx::query("SELECT * FROM connector_accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| account_from_row(&row))
            .transpose()
    }

    pub async fn get_account_by_source_key(
        &self,
        kind: ConnectorKind,
        source_key: &str,
    ) -> Result<Option<Account>> {
        validate_source_key(source_key)?;
        sqlx::query("SELECT * FROM connector_accounts WHERE kind = ? AND source_key = ?")
            .bind(kind.as_str())
            .bind(source_key)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| account_from_row(&row))
            .transpose()
    }

    pub async fn adopt_account(
        &self,
        id: &str,
        source_key: &str,
        input: AccountUpdate,
    ) -> Result<Account> {
        require_name(&input.name, "account")?;
        validate_source_key(source_key)?;
        let config = serde_json::to_string(&input.config)?;
        let mut transaction = self.pool.begin().await?;
        let previous_config = sqlx::query_scalar::<_, String>(
            "SELECT config FROM connector_accounts WHERE id = ? AND source_key IS NULL",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            ConnectorError::InvalidInput(format!("account {id} is not available for adoption"))
        })?;
        let config_changed = previous_config != config;
        let result = sqlx::query(
            "UPDATE connector_accounts SET source_key = ?, name = ?, config = ?, enabled = ?, \
             updated_at = ? WHERE id = ? AND source_key IS NULL",
        )
        .bind(source_key)
        .bind(input.name)
        .bind(config)
        .bind(input.enabled)
        .bind(now_millis())
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(ConnectorError::InvalidInput(format!(
                "account {id} is not available for adoption"
            )));
        }
        if config_changed {
            sqlx::query(
                "UPDATE connector_datasets SET plan_revision = plan_revision + 1 \
                 WHERE account_id = ?",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        record_write("account.adopt");
        self.get_account_required(id).await
    }

    pub async fn list_accounts(&self) -> Result<Vec<Account>> {
        let rows = sqlx::query("SELECT * FROM connector_accounts ORDER BY name, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(account_from_row).collect()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, input)))]
    pub async fn create_dataset(&self, input: DatasetCreate) -> Result<Dataset> {
        require_name(&input.name, "dataset")?;
        validate_schedule(input.schedule_minutes)?;
        let id = Uuid::new_v4().to_string();
        let now = now_millis();
        let config = serde_json::to_string(&input.config)?;
        sqlx::query(
            "INSERT INTO connector_datasets \
             (id, account_id, name, instruction, config, schedule_minutes, projection_jsonl, \
              projection_md, enabled, next_sync_at, created_at, updated_at) \
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.account_id)
        .bind(&input.name)
        .bind(&input.instruction)
        .bind(config)
        .bind(input.schedule_minutes.map(u64_to_i64).transpose()?)
        .bind(input.projections.jsonl)
        .bind(input.projections.markdown)
        .bind(input.enabled)
        .bind(input.next_sync_at.map(datetime_to_millis).transpose()?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        record_write("dataset.create");
        self.get_dataset_required(&id).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, input)))]
    pub async fn update_dataset(&self, id: &str, input: DatasetUpdate) -> Result<Dataset> {
        require_name(&input.name, "dataset")?;
        validate_schedule(input.schedule_minutes)?;
        let config = serde_json::to_string(&input.config)?;
        let result = sqlx::query(
            "UPDATE connector_datasets SET name = ?, instruction = ?, \
             plan_revision = plan_revision + CASE WHEN config <> ? THEN 1 ELSE 0 END, \
             config = ?, schedule_minutes = ?, projection_jsonl = ?, projection_md = ?, \
             enabled = ?, next_sync_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(input.name)
        .bind(input.instruction)
        .bind(&config)
        .bind(&config)
        .bind(input.schedule_minutes.map(u64_to_i64).transpose()?)
        .bind(input.projections.jsonl)
        .bind(input.projections.markdown)
        .bind(input.enabled)
        .bind(input.next_sync_at.map(datetime_to_millis).transpose()?)
        .bind(now_millis())
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_affected(result.rows_affected(), "dataset", id)?;
        record_write("dataset.update");
        self.get_dataset_required(id).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn remove_dataset(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM connector_datasets WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        let removed = result.rows_affected() != 0;
        if removed {
            record_write("dataset.remove");
        }
        Ok(removed)
    }

    pub async fn set_dataset_next_sync_at(
        &self,
        id: &str,
        next_sync_at: Option<OffsetDateTime>,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE connector_datasets SET next_sync_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(next_sync_at.map(datetime_to_millis).transpose()?)
        .bind(now_millis())
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_affected(result.rows_affected(), "dataset", id)
    }

    pub async fn set_dataset_last_error(&self, id: &str, error: Option<&str>) -> Result<()> {
        let result = sqlx::query(
            "UPDATE connector_datasets SET last_error = ?, updated_at = ? WHERE id = ?",
        )
        .bind(error)
        .bind(now_millis())
        .bind(id)
        .execute(&self.pool)
        .await?;
        require_affected(result.rows_affected(), "dataset", id)
    }

    pub async fn get_dataset(&self, id: &str) -> Result<Option<Dataset>> {
        sqlx::query("SELECT * FROM connector_datasets WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| dataset_from_row(&row))
            .transpose()
    }

    pub async fn list_datasets(&self, account_id: &str) -> Result<Vec<Dataset>> {
        let rows =
            sqlx::query("SELECT * FROM connector_datasets WHERE account_id = ? ORDER BY name, id")
                .bind(account_id)
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(dataset_from_row).collect()
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self, items, observed_remote_ids), fields(item_count = items.len(), complete))
    )]
    pub async fn apply_snapshot(
        &self,
        dataset_id: &str,
        items: &[ConnectorItemInput],
        observed_remote_ids: &[String],
        complete: bool,
    ) -> Result<SnapshotResult> {
        validate_snapshot(items)?;
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        let result = apply_snapshot_transaction(
            &mut transaction,
            dataset_id,
            items,
            observed_remote_ids,
            complete,
            now,
        )
        .await?;
        transaction.commit().await?;
        record_snapshot(result);
        Ok(result)
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self, items, observations), fields(item_count = items.len()))
    )]
    pub async fn commit_sync_snapshot(
        &self,
        run_id: &str,
        dataset_id: &str,
        items: &[ConnectorItemInput],
        observations: &[SourceObservation],
        next_sync_at: Option<OffsetDateTime>,
    ) -> Result<(SnapshotResult, SyncRun)> {
        validate_snapshot(items)?;
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        let stats = crate::source::commit_observations(
            &mut transaction,
            run_id,
            dataset_id,
            items,
            observations,
            now,
        )
        .await?;
        let run_update = sqlx::query(
            "UPDATE connector_sync_runs SET status = ?, finished_at = ?, upserted = ?, \
             deleted = ?, active = ?, error = NULL \
             WHERE id = ? AND dataset_id = ? AND status = 'running'",
        )
        .bind(SyncRunStatus::Succeeded.as_str())
        .bind(now)
        .bind(u64_to_i64(stats.upserted)?)
        .bind(u64_to_i64(stats.deleted)?)
        .bind(u64_to_i64(stats.active)?)
        .bind(run_id)
        .bind(dataset_id)
        .execute(&mut *transaction)
        .await?;
        require_affected(run_update.rows_affected(), "running sync run", run_id)?;
        let dataset_update = sqlx::query(
            "UPDATE connector_datasets SET last_sync_at = ?, last_error = NULL, item_count = ?, \
              next_sync_at = ?, synced_plan_revision = plan_revision, updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(u64_to_i64(stats.active)?)
        .bind(next_sync_at.map(datetime_to_millis).transpose()?)
        .bind(now)
        .bind(dataset_id)
        .execute(&mut *transaction)
        .await?;
        require_affected(dataset_update.rows_affected(), "dataset", dataset_id)?;
        let run = sync_run_in_transaction(&mut transaction, run_id).await?;
        transaction.commit().await?;
        record_snapshot(stats);
        record_write("sync.finish");
        Ok((stats, run))
    }

    pub async fn source_states(&self, dataset_id: &str) -> Result<SourceStateMap> {
        crate::source::read_states(&self.pool, dataset_id).await
    }

    pub async fn query_items(
        &self,
        dataset_id: &str,
        query: ItemQuery,
    ) -> Result<Vec<ConnectorItem>> {
        validate_limit(query.limit)?;
        let fts = query.text.as_deref().map(fts_query).transpose()?.flatten();
        if query.text.is_some() && fts.is_none() {
            return Ok(Vec::new());
        }
        let sql = match (query.include_deleted, fts.is_some()) {
            (true, false) => {
                "SELECT item.* FROM connector_items AS item WHERE item.dataset_id = ? \
                 ORDER BY item.occurred_at DESC, item.remote_id LIMIT ? OFFSET ?"
            },
            (false, false) => {
                "SELECT item.* FROM connector_items AS item WHERE item.dataset_id = ? \
                 AND item.deleted_at IS NULL \
                 ORDER BY item.occurred_at DESC, item.remote_id LIMIT ? OFFSET ?"
            },
            (true, true) => {
                "SELECT item.* FROM connector_items AS item \
                 JOIN connector_items_fts ON connector_items_fts.rowid = item.rowid \
                 WHERE item.dataset_id = ? AND connector_items_fts MATCH ? \
                 ORDER BY bm25(connector_items_fts), item.occurred_at DESC, item.remote_id \
                 LIMIT ? OFFSET ?"
            },
            (false, true) => {
                "SELECT item.* FROM connector_items AS item \
                 JOIN connector_items_fts ON connector_items_fts.rowid = item.rowid \
                 WHERE item.dataset_id = ? AND item.deleted_at IS NULL \
                 AND connector_items_fts MATCH ? \
                 ORDER BY bm25(connector_items_fts), item.occurred_at DESC, item.remote_id \
                 LIMIT ? OFFSET ?"
            },
        };
        let mut statement = sqlx::query(sql).bind(dataset_id);
        if let Some(fts) = fts {
            statement = statement.bind(fts);
        }
        let rows = statement
            .bind(u64_to_i64(query.limit)?)
            .bind(u64_to_i64(query.offset)?)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(item_from_row).collect()
    }

    pub async fn get_item(
        &self,
        dataset_id: &str,
        item_id: &str,
        include_deleted: bool,
    ) -> Result<Option<ConnectorItem>> {
        let sql = if include_deleted {
            "SELECT * FROM connector_items WHERE dataset_id = ? AND id = ?"
        } else {
            "SELECT * FROM connector_items WHERE dataset_id = ? AND id = ? AND deleted_at IS NULL"
        };
        sqlx::query(sql)
            .bind(dataset_id)
            .bind(item_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| item_from_row(&row))
            .transpose()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn start_sync_run(&self, dataset_id: &str) -> Result<SyncRun> {
        let id = Uuid::new_v4().to_string();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO connector_sync_runs (id, dataset_id, status, started_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(dataset_id)
        .bind(SyncRunStatus::Running.as_str())
        .bind(now_millis())
        .execute(&mut *transaction)
        .await?;
        let run = sync_run_in_transaction(&mut transaction, &id).await?;
        transaction.commit().await?;
        record_write("sync.start");
        Ok(run)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn finish_sync_run(&self, id: &str, stats: SnapshotResult) -> Result<SyncRun> {
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE connector_sync_runs SET status = ?, finished_at = ?, upserted = ?, \
             deleted = ?, active = ?, error = NULL WHERE id = ? AND status = 'running'",
        )
        .bind(SyncRunStatus::Succeeded.as_str())
        .bind(now)
        .bind(u64_to_i64(stats.upserted)?)
        .bind(u64_to_i64(stats.deleted)?)
        .bind(u64_to_i64(stats.active)?)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        require_affected(result.rows_affected(), "running sync run", id)?;
        sqlx::query(
            "UPDATE connector_datasets SET last_sync_at = ?, last_error = NULL, item_count = ?, \
             updated_at = ? WHERE id = (SELECT dataset_id FROM connector_sync_runs WHERE id = ?)",
        )
        .bind(now)
        .bind(u64_to_i64(stats.active)?)
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        let run = sync_run_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        record_write("sync.finish");
        Ok(run)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, error)))]
    pub async fn fail_sync_run(&self, id: &str, error: &str) -> Result<SyncRun> {
        if error.trim().is_empty() {
            return Err(ConnectorError::InvalidInput(
                "sync failure error must not be empty".to_owned(),
            ));
        }
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE connector_sync_runs SET status = ?, finished_at = ?, error = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(SyncRunStatus::Failed.as_str())
        .bind(now)
        .bind(error)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        require_affected(result.rows_affected(), "running sync run", id)?;
        sqlx::query(
            "UPDATE connector_datasets SET last_error = ?, updated_at = ? \
             WHERE id = (SELECT dataset_id FROM connector_sync_runs WHERE id = ?)",
        )
        .bind(error)
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        let run = sync_run_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        record_write("sync.fail");
        Ok(run)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, error)))]
    pub async fn fail_sync_run_and_reschedule(
        &self,
        id: &str,
        error: &str,
        next_sync_at: Option<OffsetDateTime>,
    ) -> Result<SyncRun> {
        if error.trim().is_empty() {
            return Err(ConnectorError::InvalidInput(
                "sync failure error must not be empty".to_owned(),
            ));
        }
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE connector_sync_runs SET status = ?, finished_at = ?, error = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(SyncRunStatus::Failed.as_str())
        .bind(now)
        .bind(error)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        require_affected(result.rows_affected(), "running sync run", id)?;
        let dataset_update = sqlx::query(
            "UPDATE connector_datasets SET last_error = ?, next_sync_at = ?, updated_at = ? \
             WHERE id = (SELECT dataset_id FROM connector_sync_runs WHERE id = ?)",
        )
        .bind(error)
        .bind(next_sync_at.map(datetime_to_millis).transpose()?)
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        require_affected(dataset_update.rows_affected(), "sync run dataset", id)?;
        let run = sync_run_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        record_write("sync.fail");
        Ok(run)
    }

    pub async fn fail_running_sync_runs(&self, error: &str) -> Result<u64> {
        if error.trim().is_empty() {
            return Err(ConnectorError::InvalidInput(
                "sync failure error must not be empty".to_owned(),
            ));
        }
        let now = now_millis();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE connector_datasets SET last_error = ?, updated_at = ? \
             WHERE id IN (SELECT dataset_id FROM connector_sync_runs WHERE status = 'running')",
        )
        .bind(error)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let failed = sqlx::query(
            "UPDATE connector_sync_runs SET status = 'failed', finished_at = ?, error = ? \
             WHERE status = 'running'",
        )
        .bind(now)
        .bind(error)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        transaction.commit().await?;
        if failed != 0 {
            record_write("sync.recover");
        }
        Ok(failed)
    }

    pub async fn list_recent_sync_runs(
        &self,
        dataset_id: &str,
        limit: u64,
    ) -> Result<Vec<SyncRun>> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT * FROM connector_sync_runs WHERE dataset_id = ? \
             ORDER BY started_at DESC, id DESC LIMIT ?",
        )
        .bind(dataset_id)
        .bind(u64_to_i64(limit)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(sync_run_from_row).collect()
    }

    async fn get_account_required(&self, id: &str) -> Result<Account> {
        self.get_account(id)
            .await?
            .ok_or_else(|| not_found("account", id))
    }

    async fn get_dataset_required(&self, id: &str) -> Result<Dataset> {
        self.get_dataset(id)
            .await?
            .ok_or_else(|| not_found("dataset", id))
    }
}

#[async_trait]
impl ConnectorReader for SqliteConnectorStore {
    async fn list_datasets_for_kind(&self, kind: ConnectorKind) -> Result<Vec<Dataset>> {
        let rows = sqlx::query(
            "SELECT dataset.* FROM connector_datasets AS dataset \
             JOIN connector_accounts AS account ON account.id = dataset.account_id \
             WHERE account.kind = ? ORDER BY dataset.name, dataset.id",
        )
        .bind(kind.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(dataset_from_row).collect()
    }

    async fn query_items_for_kind(
        &self,
        kind: ConnectorKind,
        dataset_id: &str,
        query: ItemQuery,
    ) -> Result<Vec<ConnectorItem>> {
        if !self.dataset_has_kind(dataset_id, kind).await? {
            return Ok(Vec::new());
        }
        self.query_items(dataset_id, query).await
    }

    async fn get_item_for_kind(
        &self,
        kind: ConnectorKind,
        dataset_id: &str,
        item_id: &str,
    ) -> Result<Option<ConnectorItem>> {
        if !self.dataset_has_kind(dataset_id, kind).await? {
            return Ok(None);
        }
        self.get_item(dataset_id, item_id, false).await
    }
}

async fn sync_run_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<SyncRun> {
    sqlx::query("SELECT * FROM connector_sync_runs WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(|row| sync_run_from_row(&row))
        .transpose()?
        .ok_or_else(|| not_found("sync run", id))
}

async fn apply_snapshot_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    dataset_id: &str,
    items: &[ConnectorItemInput],
    observed_remote_ids: &[String],
    complete: bool,
    now: i64,
) -> Result<SnapshotResult> {
    for item in items {
        let body = serde_json::to_string(&item.body_json)?;
        sqlx::query(
            "INSERT INTO connector_items \
             (id, dataset_id, remote_id, kind, remote_version, occurred_at, updated_at, \
               body_json, search_text, content_hash, created_at, stored_at, deleted_at) \
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL) \
              ON CONFLICT(dataset_id, remote_id) DO UPDATE SET \
                kind = excluded.kind, remote_version = excluded.remote_version, \
                occurred_at = excluded.occurred_at, updated_at = excluded.updated_at, \
                body_json = excluded.body_json, search_text = excluded.search_text, \
                content_hash = excluded.content_hash, stored_at = excluded.stored_at, \
                deleted_at = NULL",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(dataset_id)
        .bind(&item.remote_id)
        .bind(&item.kind)
        .bind(&item.remote_version)
        .bind(&item.occurred_at)
        .bind(&item.updated_at)
        .bind(body)
        .bind(&item.search_text)
        .bind(&item.content_hash)
        .bind(now)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    }

    let deleted = if complete {
        tombstone_unobserved(transaction, dataset_id, observed_remote_ids, now).await?
    } else {
        0
    };
    let active = active_count(transaction, dataset_id).await?;
    let result = SnapshotResult {
        upserted: usize_to_u64(items.len())?,
        deleted,
        active,
    };
    let dataset_update =
        sqlx::query("UPDATE connector_datasets SET item_count = ?, updated_at = ? WHERE id = ?")
            .bind(u64_to_i64(active)?)
            .bind(now)
            .bind(dataset_id)
            .execute(&mut **transaction)
            .await?;
    require_affected(dataset_update.rows_affected(), "dataset", dataset_id)?;
    Ok(result)
}

async fn tombstone_unobserved(
    transaction: &mut Transaction<'_, Sqlite>,
    dataset_id: &str,
    observed_remote_ids: &[String],
    now: i64,
) -> Result<u64> {
    let unique: HashSet<&str> = observed_remote_ids.iter().map(String::as_str).collect();
    let observed = serde_json::to_string(&unique)?;
    Ok(sqlx::query(
        "UPDATE connector_items AS item SET deleted_at = ? \
         WHERE item.dataset_id = ? AND item.deleted_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM json_each(?) WHERE value = item.remote_id)",
    )
    .bind(now)
    .bind(dataset_id)
    .bind(observed)
    .execute(&mut **transaction)
    .await?
    .rows_affected())
}

async fn active_count(transaction: &mut Transaction<'_, Sqlite>, dataset_id: &str) -> Result<u64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM connector_items WHERE dataset_id = ? AND deleted_at IS NULL",
    )
    .bind(dataset_id)
    .fetch_one(&mut **transaction)
    .await?;
    i64_to_u64(count, "active item count")
}

fn account_from_row(row: &SqliteRow) -> Result<Account> {
    let kind: String = row.try_get("kind")?;
    let config: String = row.try_get("config")?;
    Ok(Account {
        id: row.try_get("id")?,
        kind: ConnectorKind::try_from(kind)?,
        source_key: row.try_get("source_key")?,
        name: row.try_get("name")?,
        config: serde_json::from_str(&config)?,
        enabled: row.try_get("enabled")?,
        created_at: millis_to_datetime(row.try_get("created_at")?)?,
        updated_at: millis_to_datetime(row.try_get("updated_at")?)?,
    })
}

fn dataset_from_row(row: &SqliteRow) -> Result<Dataset> {
    let config: String = row.try_get("config")?;
    let schedule: Option<i64> = row.try_get("schedule_minutes")?;
    Ok(Dataset {
        id: row.try_get("id")?,
        account_id: row.try_get("account_id")?,
        name: row.try_get("name")?,
        instruction: row.try_get("instruction")?,
        config: serde_json::from_str(&config)?,
        plan_revision: i64_to_u64(row.try_get("plan_revision")?, "plan_revision")?,
        synced_plan_revision: row
            .try_get::<Option<i64>, _>("synced_plan_revision")?
            .map(|revision| i64_to_u64(revision, "synced_plan_revision"))
            .transpose()?,
        schedule_minutes: schedule
            .map(|value| i64_to_u64(value, "schedule_minutes"))
            .transpose()?,
        projections: ProjectionConfig {
            jsonl: row.try_get("projection_jsonl")?,
            markdown: row.try_get("projection_md")?,
        },
        enabled: row.try_get("enabled")?,
        last_sync_at: optional_datetime(row.try_get("last_sync_at")?)?,
        next_sync_at: optional_datetime(row.try_get("next_sync_at")?)?,
        last_error: row.try_get("last_error")?,
        item_count: i64_to_u64(row.try_get("item_count")?, "item_count")?,
        created_at: millis_to_datetime(row.try_get("created_at")?)?,
        updated_at: millis_to_datetime(row.try_get("updated_at")?)?,
    })
}

fn item_from_row(row: &SqliteRow) -> Result<ConnectorItem> {
    let body: String = row.try_get("body_json")?;
    Ok(ConnectorItem {
        id: row.try_get("id")?,
        dataset_id: row.try_get("dataset_id")?,
        remote_id: row.try_get("remote_id")?,
        kind: row.try_get("kind")?,
        remote_version: row.try_get("remote_version")?,
        occurred_at: row.try_get("occurred_at")?,
        updated_at: row.try_get("updated_at")?,
        body_json: serde_json::from_str(&body)?,
        search_text: row.try_get("search_text")?,
        content_hash: row.try_get("content_hash")?,
        created_at: millis_to_datetime(row.try_get("created_at")?)?,
        stored_at: millis_to_datetime(row.try_get("stored_at")?)?,
        deleted_at: optional_datetime(row.try_get("deleted_at")?)?,
    })
}

fn sync_run_from_row(row: &SqliteRow) -> Result<SyncRun> {
    let status: String = row.try_get("status")?;
    Ok(SyncRun {
        id: row.try_get("id")?,
        dataset_id: row.try_get("dataset_id")?,
        status: SyncRunStatus::try_from(status.as_str())?,
        started_at: millis_to_datetime(row.try_get("started_at")?)?,
        finished_at: optional_datetime(row.try_get("finished_at")?)?,
        upserted: i64_to_u64(row.try_get("upserted")?, "sync upserted")?,
        deleted: i64_to_u64(row.try_get("deleted")?, "sync deleted")?,
        active: i64_to_u64(row.try_get("active")?, "sync active")?,
        error: row.try_get("error")?,
    })
}

fn validate_snapshot(items: &[ConnectorItemInput]) -> Result<()> {
    let mut supplied = HashSet::with_capacity(items.len());
    for item in items {
        if item.remote_id.trim().is_empty()
            || item.kind.trim().is_empty()
            || item.content_hash.is_empty()
        {
            return Err(ConnectorError::InvalidInput(
                "snapshot remote_id, kind, and content_hash must not be empty".to_owned(),
            ));
        }
        if !supplied.insert(item.remote_id.as_str()) {
            return Err(ConnectorError::InvalidInput(format!(
                "duplicate snapshot remote_id: {}",
                item.remote_id
            )));
        }
    }
    Ok(())
}

fn require_name(name: &str, entity: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(ConnectorError::InvalidInput(format!(
            "{entity} name must not be empty"
        )));
    }
    Ok(())
}

fn validate_optional_source_key(source_key: Option<&str>) -> Result<()> {
    source_key.map(validate_source_key).transpose().map(drop)
}

fn validate_source_key(source_key: &str) -> Result<()> {
    if source_key.trim().is_empty() {
        return Err(ConnectorError::InvalidInput(
            "account source_key must not be empty".to_owned(),
        ));
    }
    if source_key.chars().count() > MAX_SOURCE_KEY_LENGTH {
        return Err(ConnectorError::InvalidInput(format!(
            "account source_key must not exceed {MAX_SOURCE_KEY_LENGTH} characters"
        )));
    }
    Ok(())
}

fn validate_schedule(schedule: Option<u64>) -> Result<()> {
    if schedule == Some(0) {
        return Err(ConnectorError::InvalidInput(
            "schedule_minutes must be greater than zero".to_owned(),
        ));
    }
    if let Some(value) = schedule {
        u64_to_i64(value)?;
    }
    Ok(())
}

fn validate_limit(limit: u64) -> Result<()> {
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        return Err(ConnectorError::QueryLimit {
            requested: limit,
            max: MAX_QUERY_LIMIT,
        });
    }
    Ok(())
}

fn require_affected(rows: u64, entity: &'static str, id: &str) -> Result<()> {
    if rows == 0 {
        return Err(not_found(entity, id));
    }
    Ok(())
}

fn not_found(entity: &'static str, id: &str) -> ConnectorError {
    ConnectorError::NotFound {
        entity,
        id: id.to_owned(),
    }
}

fn now_millis() -> i64 {
    let millis = OffsetDateTime::now_utc()
        .unix_timestamp_nanos()
        .div_euclid(1_000_000);
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn datetime_to_millis(value: OffsetDateTime) -> Result<i64> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000_000)).map_err(|_| {
        ConnectorError::InvalidInput("timestamp is outside SQLite integer range".to_owned())
    })
}

fn millis_to_datetime(value: i64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
        .map_err(|_| ConnectorError::InvalidTimestamp(value))
}

fn optional_datetime(value: Option<i64>) -> Result<Option<OffsetDateTime>> {
    value.map(millis_to_datetime).transpose()
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        ConnectorError::InvalidInput("numeric value exceeds SQLite integer range".to_owned())
    })
}

fn i64_to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| ConnectorError::InvalidInput(format!("stored {field} must not be negative")))
}

fn usize_to_u64(value: usize) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| ConnectorError::InvalidInput("item count exceeds u64".to_owned()))
}

#[cfg(feature = "metrics")]
fn record_write(operation: &'static str) {
    metrics::counter!("moltis_connector_store_writes_total", "operation" => operation).increment(1);
}

#[cfg(not(feature = "metrics"))]
fn record_write(_operation: &'static str) {}

#[cfg(feature = "metrics")]
fn record_snapshot(result: SnapshotResult) {
    metrics::counter!("moltis_connector_snapshot_items_total", "action" => "upserted")
        .increment(result.upserted);
    metrics::counter!("moltis_connector_snapshot_items_total", "action" => "deleted")
        .increment(result.deleted);
}

#[cfg(not(feature = "metrics"))]
fn record_snapshot(_result: SnapshotResult) {}
