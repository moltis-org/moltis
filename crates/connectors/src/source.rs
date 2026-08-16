use std::collections::{BTreeMap, HashMap, HashSet};

use {
    sqlx::{Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow},
    time::OffsetDateTime,
    uuid::Uuid,
};

use crate::{
    ConnectorError, ConnectorItemInput, Result, SnapshotResult, SourceDisposition,
    SourceObservation, SourceState, SourceStateMap,
};

pub(crate) async fn read_states(pool: &SqlitePool, dataset_id: &str) -> Result<SourceStateMap> {
    let rows = sqlx::query(
        "SELECT * FROM connector_source_states WHERE dataset_id = ? ORDER BY remote_id",
    )
    .bind(dataset_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(source_state_from_row)
        .map(|result| result.map(|state| (state.remote_id.clone(), state)))
        .collect::<Result<BTreeMap<_, _>>>()
}

pub(crate) async fn commit_observations(
    transaction: &mut Transaction<'_, Sqlite>,
    run_id: &str,
    dataset_id: &str,
    items: &[ConnectorItemInput],
    observations: &[SourceObservation],
    now: i64,
) -> Result<SnapshotResult> {
    validate_observations(items, observations)?;
    for observation in observations {
        sqlx::query(
            "INSERT INTO connector_source_states \
             (dataset_id, remote_id, remote_version, disposition, filter_reason, \
              evaluated_plan_revision, last_seen_run_id, observed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(dataset_id, remote_id) DO UPDATE SET \
               remote_version = excluded.remote_version, disposition = excluded.disposition, \
               filter_reason = excluded.filter_reason, \
               evaluated_plan_revision = excluded.evaluated_plan_revision, \
               last_seen_run_id = excluded.last_seen_run_id, observed_at = excluded.observed_at",
        )
        .bind(dataset_id)
        .bind(&observation.remote_id)
        .bind(&observation.remote_version)
        .bind(observation.disposition.as_str())
        .bind(&observation.filter_reason)
        .bind(u64_to_i64(observation.evaluated_plan_revision)?)
        .bind(run_id)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    }

    upsert_items(transaction, dataset_id, items, now).await?;
    let deleted = sqlx::query(
        "UPDATE connector_items AS item SET deleted_at = ? \
         WHERE item.dataset_id = ? AND item.deleted_at IS NULL \
           AND NOT EXISTS ( \
             SELECT 1 FROM connector_source_states AS source \
             WHERE source.dataset_id = item.dataset_id AND source.remote_id = item.remote_id \
               AND source.last_seen_run_id = ? AND source.disposition = 'included' \
           )",
    )
    .bind(now)
    .bind(dataset_id)
    .bind(run_id)
    .execute(&mut **transaction)
    .await?
    .rows_affected();

    sqlx::query(
        "DELETE FROM connector_source_states WHERE dataset_id = ? AND last_seen_run_id <> ?",
    )
    .bind(dataset_id)
    .bind(run_id)
    .execute(&mut **transaction)
    .await?;

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM connector_items WHERE dataset_id = ? AND deleted_at IS NULL",
    )
    .bind(dataset_id)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(SnapshotResult {
        upserted: usize_to_u64(items.len())?,
        deleted,
        active: i64_to_u64(active, "active item count")?,
    })
}

async fn upsert_items(
    transaction: &mut Transaction<'_, Sqlite>,
    dataset_id: &str,
    items: &[ConnectorItemInput],
    now: i64,
) -> Result<()> {
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
    Ok(())
}

fn validate_observations(
    items: &[ConnectorItemInput],
    observations: &[SourceObservation],
) -> Result<()> {
    let mut supplied = HashSet::with_capacity(observations.len());
    let by_remote_id = observations
        .iter()
        .map(|observation| (observation.remote_id.as_str(), observation.disposition))
        .collect::<HashMap<_, _>>();
    for observation in observations {
        if observation.remote_id.trim().is_empty() || observation.evaluated_plan_revision == 0 {
            return Err(ConnectorError::InvalidInput(
                "source observation remote_id must not be empty and plan revision must be positive"
                    .to_owned(),
            ));
        }
        if !supplied.insert(observation.remote_id.as_str()) {
            return Err(ConnectorError::InvalidInput(format!(
                "duplicate source observation remote_id: {}",
                observation.remote_id
            )));
        }
    }
    for item in items {
        if by_remote_id.get(item.remote_id.as_str()) != Some(&SourceDisposition::Included) {
            return Err(ConnectorError::InvalidInput(format!(
                "snapshot item has no included source observation: {}",
                item.remote_id
            )));
        }
    }
    Ok(())
}

fn source_state_from_row(row: &SqliteRow) -> Result<SourceState> {
    let disposition: String = row.try_get("disposition")?;
    Ok(SourceState {
        dataset_id: row.try_get("dataset_id")?,
        remote_id: row.try_get("remote_id")?,
        remote_version: row.try_get("remote_version")?,
        disposition: SourceDisposition::try_from(disposition.as_str())?,
        filter_reason: row.try_get("filter_reason")?,
        evaluated_plan_revision: i64_to_u64(
            row.try_get("evaluated_plan_revision")?,
            "evaluated_plan_revision",
        )?,
        last_seen_run_id: row.try_get("last_seen_run_id")?,
        observed_at: millis_to_datetime(row.try_get("observed_at")?)?,
    })
}

fn millis_to_datetime(value: i64) -> Result<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000_000)
        .map_err(|_| ConnectorError::InvalidTimestamp(value))
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
