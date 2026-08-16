use {
    moltis_connectors::{
        AccountCreate, AccountUpdate, ConnectorError, ConnectorItemInput, ConnectorKind,
        ConnectorReader, Dataset, DatasetCreate, DatasetUpdate, ItemQuery, MAX_QUERY_LIMIT,
        ProjectionConfig, SnapshotResult, SourceDisposition, SourceObservation,
        SqliteConnectorStore, SyncRunStatus, run_migrations,
    },
    serde_json::json,
    sqlx::sqlite::SqlitePoolOptions,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn store() -> TestResult<SqliteConnectorStore> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    run_migrations(&pool).await?;
    Ok(SqliteConnectorStore::new(pool))
}

#[tokio::test]
async fn provider_reader_never_crosses_connector_kinds() -> TestResult {
    let store = store().await?;
    let gmail_account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Gmail,
            source_key: None,
            name: "Gmail".to_owned(),
            config: json!({"schemaVersion": 1}),
            enabled: true,
        })
        .await?;
    let gmail_dataset = store
        .create_dataset(DatasetCreate {
            account_id: gmail_account.id,
            name: "Inbox".to_owned(),
            instruction: None,
            config: json!({"schemaVersion": 1}),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    store
        .apply_snapshot(
            &gmail_dataset.id,
            &[item("gmail-message", "1", "private email")],
            &["gmail-message".to_owned()],
            true,
        )
        .await?;

    assert_eq!(
        store
            .list_datasets_for_kind(ConnectorKind::Gmail)
            .await?
            .len(),
        1
    );
    assert!(
        store
            .list_datasets_for_kind(ConnectorKind::Himalaya)
            .await?
            .is_empty()
    );
    assert!(
        store
            .query_items_for_kind(
                ConnectorKind::Himalaya,
                &gmail_dataset.id,
                ItemQuery::default(),
            )
            .await?
            .is_empty()
    );
    let gmail_item = store
        .query_items_for_kind(
            ConnectorKind::Gmail,
            &gmail_dataset.id,
            ItemQuery::default(),
        )
        .await?
        .into_iter()
        .next()
        .ok_or("Gmail item missing")?;
    assert!(
        store
            .get_item_for_kind(ConnectorKind::Himalaya, &gmail_dataset.id, &gmail_item.id,)
            .await?
            .is_none()
    );
    Ok(())
}

async fn create_dataset(store: &SqliteConnectorStore) -> TestResult<Dataset> {
    let account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: None,
            name: "Calendar".to_owned(),
            config: json!({"encrypted_password": "ciphertext"}),
            enabled: true,
        })
        .await?;
    Ok(store
        .create_dataset(DatasetCreate {
            account_id: account.id,
            name: "Events".to_owned(),
            instruction: Some("Include useful events".to_owned()),
            config: json!({"calendar": "/personal"}),
            schedule_minutes: Some(15),
            projections: ProjectionConfig {
                jsonl: true,
                markdown: true,
            },
            enabled: true,
            next_sync_at: Some(time::OffsetDateTime::now_utc()),
        })
        .await?)
}

fn item(remote_id: &str, version: &str, summary: &str) -> ConnectorItemInput {
    ConnectorItemInput {
        remote_id: remote_id.to_owned(),
        kind: "event".to_owned(),
        remote_version: Some(version.to_owned()),
        occurred_at: Some("2026-08-06T09:00:00Z".to_owned()),
        updated_at: Some("2026-08-06T10:00:00Z".to_owned()),
        body_json: json!({"summary": summary}),
        search_text: summary.to_owned(),
        content_hash: format!("hash-{version}"),
    }
}

fn observation(
    remote_id: &str,
    version: &str,
    disposition: SourceDisposition,
) -> SourceObservation {
    SourceObservation {
        remote_id: remote_id.to_owned(),
        remote_version: Some(version.to_owned()),
        disposition,
        filter_reason: (disposition == SourceDisposition::Filtered)
            .then(|| "outside plan".to_owned()),
        evaluated_plan_revision: 1,
    }
}

#[tokio::test]
async fn account_config_change_bumps_every_owned_dataset_plan() -> TestResult {
    let store = store().await?;
    let first = create_dataset(&store).await?;
    let second = store
        .create_dataset(DatasetCreate {
            account_id: first.account_id.clone(),
            name: "Work events".to_owned(),
            instruction: None,
            config: json!({"calendar": "/work"}),
            schedule_minutes: Some(30),
            projections: ProjectionConfig::default(),
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    let unrelated = create_dataset(&store).await?;
    for dataset in [&first, &second] {
        let run = store.start_sync_run(&dataset.id).await?;
        store
            .commit_sync_snapshot(&run.id, &dataset.id, &[], &[], None)
            .await?;
    }

    let account = store
        .get_account(&first.account_id)
        .await?
        .ok_or("account missing")?;
    let updated = store
        .update_account(&account.id, AccountUpdate {
            name: account.name,
            config: json!({"encrypted_password": "new-ciphertext"}),
            enabled: account.enabled,
        })
        .await?;
    assert_eq!(updated.config["encrypted_password"], "new-ciphertext");

    let datasets = store.list_datasets(&updated.id).await?;
    assert_eq!(datasets.len(), 2);
    assert!(datasets.iter().all(|dataset| dataset.plan_revision == 2));
    assert!(
        datasets
            .iter()
            .all(|dataset| dataset.synced_plan_revision == Some(1))
    );
    assert_eq!(
        store
            .get_dataset(&unrelated.id)
            .await?
            .ok_or("unrelated dataset missing")?
            .plan_revision,
        1
    );
    Ok(())
}

#[tokio::test]
async fn account_name_and_enabled_changes_do_not_bump_dataset_plan() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let account = store
        .get_account(&dataset.account_id)
        .await?
        .ok_or("account missing")?;

    let updated = store
        .update_account(&account.id, AccountUpdate {
            name: "Renamed calendar".to_owned(),
            config: account.config,
            enabled: false,
        })
        .await?;
    assert_eq!(updated.name, "Renamed calendar");
    assert!(!updated.enabled);
    assert_eq!(
        store
            .get_dataset(&dataset.id)
            .await?
            .ok_or("dataset missing")?
            .plan_revision,
        dataset.plan_revision
    );
    Ok(())
}

#[tokio::test]
async fn account_source_key_round_trips_and_is_looked_up_by_kind() -> TestResult {
    let store = store().await?;
    let account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: Some("vault:calendar:primary".to_owned()),
            name: "Imported calendar".to_owned(),
            config: json!({}),
            enabled: true,
        })
        .await?;

    assert_eq!(
        account.source_key.as_deref(),
        Some("vault:calendar:primary")
    );
    assert_eq!(
        store
            .get_account_by_source_key(ConnectorKind::Caldav, "vault:calendar:primary")
            .await?,
        Some(account)
    );
    assert!(
        store
            .get_account_by_source_key(ConnectorKind::Caldav, "vault:calendar:missing")
            .await?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn account_source_key_is_unique_within_kind() -> TestResult {
    let store = store().await?;
    let source_key = "vault:calendar:shared";
    store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: Some(source_key.to_owned()),
            name: "First".to_owned(),
            config: json!({}),
            enabled: true,
        })
        .await?;

    let duplicate = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: Some(source_key.to_owned()),
            name: "Duplicate".to_owned(),
            config: json!({}),
            enabled: true,
        })
        .await;
    assert!(matches!(
        duplicate,
        Err(ConnectorError::Database(sqlx::Error::Database(error)))
            if error.is_unique_violation()
    ));

    sqlx::query(
        "INSERT INTO connector_accounts \
         (id, kind, source_key, name, config, enabled, created_at, updated_at) \
         VALUES ('other-kind', 'other', ?, 'Other', '{}', 1, 0, 0)",
    )
    .bind(source_key)
    .execute(store.pool())
    .await?;
    Ok(())
}

#[tokio::test]
async fn account_source_key_can_be_adopted_and_survives_account_update() -> TestResult {
    let store = store().await?;
    let account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: None,
            name: "Existing calendar".to_owned(),
            config: json!({"url": "https://calendar.example"}),
            enabled: true,
        })
        .await?;

    let adopted = store
        .adopt_account(&account.id, "vault:calendar:adopted", AccountUpdate {
            name: "Adopted calendar".to_owned(),
            config: json!({"url": "https://new.example"}),
            enabled: false,
        })
        .await?;
    assert_eq!(
        adopted.source_key.as_deref(),
        Some("vault:calendar:adopted")
    );

    assert_eq!(adopted.name, "Adopted calendar");
    assert_eq!(adopted.config["url"], "https://new.example");
    assert!(!adopted.enabled);
    assert!(matches!(
        store
            .adopt_account(&account.id, "vault:calendar:other", AccountUpdate {
                name: "Other".to_owned(),
                config: json!({}),
                enabled: true,
            },)
            .await,
        Err(ConnectorError::InvalidInput(_))
    ));

    let updated = store
        .update_account(&account.id, AccountUpdate {
            name: "Updated adopted calendar".to_owned(),
            config: adopted.config,
            enabled: true,
        })
        .await?;
    assert_eq!(
        updated.source_key.as_deref(),
        Some("vault:calendar:adopted")
    );
    Ok(())
}

#[tokio::test]
async fn account_source_keys_are_validated() -> TestResult {
    let store = store().await?;
    for source_key in ["", "   ", &"x".repeat(1025)] {
        let result = store
            .create_account(AccountCreate {
                kind: ConnectorKind::Caldav,
                source_key: Some(source_key.to_owned()),
                name: "Invalid".to_owned(),
                config: json!({}),
                enabled: true,
            })
            .await;
        assert!(matches!(result, Err(ConnectorError::InvalidInput(_))));
    }
    assert!(matches!(
        store
            .get_account_by_source_key(ConnectorKind::Caldav, "")
            .await,
        Err(ConnectorError::InvalidInput(_))
    ));
    assert!(matches!(
        store
            .adopt_account("missing", " ", AccountUpdate {
                name: "Missing".to_owned(),
                config: json!({}),
                enabled: true,
            },)
            .await,
        Err(ConnectorError::InvalidInput(_))
    ));
    Ok(())
}

#[tokio::test]
async fn account_update_rolls_back_when_dataset_plan_bump_fails() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let account = store
        .get_account(&dataset.account_id)
        .await?
        .ok_or("account missing")?;
    sqlx::query(
        "CREATE TRIGGER reject_account_plan_bump BEFORE UPDATE OF plan_revision \
         ON connector_datasets BEGIN SELECT RAISE(ABORT, 'rejected'); END",
    )
    .execute(store.pool())
    .await?;

    let result = store
        .update_account(&account.id, AccountUpdate {
            name: "Should roll back".to_owned(),
            config: json!({"encrypted_password": "new-ciphertext"}),
            enabled: false,
        })
        .await;
    assert!(matches!(result, Err(ConnectorError::Database(_))));
    assert_eq!(store.get_account(&account.id).await?, Some(account));
    assert_eq!(
        store
            .get_dataset(&dataset.id)
            .await?
            .ok_or("dataset missing")?
            .plan_revision,
        dataset.plan_revision
    );
    Ok(())
}

#[test]
fn connector_kind_conversions_and_serde() -> TestResult {
    assert_eq!(ConnectorKind::Caldav.as_str(), "caldav");
    assert_eq!(ConnectorKind::Caldav.to_string(), "caldav");
    assert_eq!(ConnectorKind::try_from("caldav")?, ConnectorKind::Caldav);
    assert_eq!(serde_json::to_string(&ConnectorKind::Caldav)?, "\"caldav\"");
    assert_eq!(ConnectorKind::ChannelHistory.as_str(), "channel_history");
    assert_eq!(
        ConnectorKind::try_from("channel_history")?,
        ConnectorKind::ChannelHistory
    );
    assert_eq!(ConnectorKind::try_from("gmail")?, ConnectorKind::Gmail);
    assert_eq!(
        ConnectorKind::try_from("himalaya")?,
        ConnectorKind::Himalaya
    );
    assert!(matches!(
        ConnectorKind::try_from("other"),
        Err(ConnectorError::UnknownConnectorKind(_))
    ));
    Ok(())
}

#[tokio::test]
async fn account_and_dataset_crud_and_cascade() -> TestResult {
    let store = store().await?;
    let account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: None,
            name: "Primary".to_owned(),
            config: json!({"secret": {"ciphertext": "abc"}}),
            enabled: true,
        })
        .await?;
    assert_eq!(account.config["secret"]["ciphertext"], "abc");
    assert_eq!(store.list_accounts().await?, vec![account.clone()]);

    let account = store
        .update_account(&account.id, AccountUpdate {
            name: "Renamed".to_owned(),
            config: json!({"secret": {"ciphertext": "def"}}),
            enabled: false,
        })
        .await?;
    assert_eq!(account.name, "Renamed");
    assert!(!account.enabled);

    let dataset = store
        .create_dataset(DatasetCreate {
            account_id: account.id.clone(),
            name: "Work".to_owned(),
            instruction: None,
            config: json!({"path": "/work"}),
            schedule_minutes: Some(30),
            projections: ProjectionConfig::default(),
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    assert_eq!(store.list_datasets(&account.id).await?, vec![
        dataset.clone()
    ]);
    let dataset = store
        .update_dataset(&dataset.id, DatasetUpdate {
            name: "Work events".to_owned(),
            instruction: Some("Only customer meetings".to_owned()),
            config: json!({"path": "/work/updated"}),
            schedule_minutes: None,
            projections: ProjectionConfig {
                jsonl: true,
                markdown: false,
            },
            enabled: false,
            next_sync_at: None,
        })
        .await?;
    assert_eq!(dataset.schedule_minutes, None);
    assert!(dataset.projections.jsonl);
    assert!(!dataset.enabled);
    assert_eq!(dataset.plan_revision, 2);
    assert_eq!(dataset.synced_plan_revision, None);

    let instruction_only = store
        .update_dataset(&dataset.id, DatasetUpdate {
            name: dataset.name.clone(),
            instruction: Some("Only internal meetings".to_owned()),
            config: dataset.config.clone(),
            schedule_minutes: dataset.schedule_minutes,
            projections: dataset.projections,
            enabled: dataset.enabled,
            next_sync_at: dataset.next_sync_at,
        })
        .await?;
    assert_eq!(instruction_only.plan_revision, dataset.plan_revision);

    assert!(store.remove_account(&account.id).await?);
    assert!(!store.remove_account(&account.id).await?);
    assert!(store.get_dataset(&dataset.id).await?.is_none());
    assert!(matches!(
        store
            .update_dataset("missing", DatasetUpdate {
                name: "x".to_owned(),
                instruction: None,
                config: json!({}),
                schedule_minutes: Some(1),
                projections: ProjectionConfig::default(),
                enabled: true,
                next_sync_at: None,
            })
            .await,
        Err(ConnectorError::NotFound { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn incomplete_snapshot_does_not_delete_and_complete_snapshot_does() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let first = vec![item("a", "1", "A"), item("b", "1", "B")];
    let observed = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        store
            .apply_snapshot(&dataset.id, &first, &observed, true)
            .await?,
        SnapshotResult {
            upserted: 2,
            deleted: 0,
            active: 2,
        }
    );

    let only_a = vec![item("a", "2", "A2")];
    let observed_a = vec!["a".to_owned()];
    let incomplete = store
        .apply_snapshot(&dataset.id, &only_a, &[], false)
        .await?;
    assert_eq!(incomplete.deleted, 0);
    assert_eq!(incomplete.active, 2);

    let complete = store
        .apply_snapshot(&dataset.id, &only_a, &observed_a, true)
        .await?;
    assert_eq!(complete.deleted, 1);
    assert_eq!(complete.active, 1);
    let all = store
        .query_items(&dataset.id, ItemQuery {
            include_deleted: true,
            ..ItemQuery::default()
        })
        .await?;
    let deleted_b = all.iter().find(|stored| stored.remote_id == "b");
    assert!(deleted_b.is_some_and(|stored| stored.deleted_at.is_some()));
    assert_eq!(
        store.get_dataset(&dataset.id).await?.map(|d| d.item_count),
        Some(1)
    );
    Ok(())
}

#[tokio::test]
async fn source_states_track_included_and_filtered_transitions() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let run = store.start_sync_run(&dataset.id).await?;
    store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[item("a", "1", "A"), item("b", "1", "B")],
            &[
                observation("a", "1", SourceDisposition::Included),
                observation("b", "1", SourceDisposition::Included),
                observation("c", "1", SourceDisposition::Filtered),
            ],
            None,
        )
        .await?;
    let states = store.source_states(&dataset.id).await?;
    assert_eq!(states.len(), 3);
    assert_eq!(states["c"].disposition, SourceDisposition::Filtered);

    let run = store.start_sync_run(&dataset.id).await?;
    let (stats, _) = store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[],
            &[
                observation("a", "1", SourceDisposition::Included),
                observation("b", "1", SourceDisposition::Filtered),
            ],
            None,
        )
        .await?;
    assert_eq!(stats.deleted, 1);
    assert_eq!(stats.active, 1);
    let states = store.source_states(&dataset.id).await?;
    assert_eq!(states.len(), 2);
    assert_eq!(states["b"].disposition, SourceDisposition::Filtered);
    assert!(!states.contains_key("c"));

    let run = store.start_sync_run(&dataset.id).await?;
    let (stats, _) = store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[item("b", "2", "B2")],
            &[
                observation("a", "1", SourceDisposition::Included),
                observation("b", "2", SourceDisposition::Included),
            ],
            None,
        )
        .await?;
    assert_eq!(stats.active, 2);
    assert_eq!(
        store.source_states(&dataset.id).await?["b"].disposition,
        SourceDisposition::Included
    );
    Ok(())
}

#[tokio::test]
async fn snapshot_failure_rolls_back_prior_upserts() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    sqlx::query(
        "CREATE TRIGGER reject_bad_item BEFORE INSERT ON connector_items \
         WHEN NEW.remote_id = 'bad' BEGIN SELECT RAISE(ABORT, 'rejected'); END",
    )
    .execute(store.pool())
    .await?;

    let result = store
        .apply_snapshot(
            &dataset.id,
            &[item("good", "1", "Good"), item("bad", "1", "Bad")],
            &["good".to_owned(), "bad".to_owned()],
            true,
        )
        .await;
    assert!(matches!(result, Err(ConnectorError::Database(_))));
    assert!(
        store
            .query_items(&dataset.id, ItemQuery::default())
            .await?
            .is_empty()
    );
    assert!(store.source_states(&dataset.id).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn sync_run_history_tracks_success_and_failure() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let successful = store.start_sync_run(&dataset.id).await?;
    let successful = store
        .finish_sync_run(&successful.id, SnapshotResult {
            upserted: 4,
            deleted: 1,
            active: 3,
        })
        .await?;
    assert_eq!(successful.status, SyncRunStatus::Succeeded);
    assert!(successful.finished_at.is_some());
    assert_eq!(successful.active, 3);
    let last_success_at = store
        .get_dataset(&dataset.id)
        .await?
        .ok_or("dataset missing")?
        .last_sync_at;

    let failed = store.start_sync_run(&dataset.id).await?;
    let failed = store.fail_sync_run(&failed.id, "network error").await?;
    assert_eq!(failed.status, SyncRunStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("network error"));
    let history = store.list_recent_sync_runs(&dataset.id, 2).await?;
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .any(|run| run.status == SyncRunStatus::Succeeded)
    );
    assert!(
        history
            .iter()
            .any(|run| run.status == SyncRunStatus::Failed)
    );
    let dataset = store
        .get_dataset(&dataset.id)
        .await?
        .ok_or("dataset missing")?;
    assert_eq!(dataset.last_error.as_deref(), Some("network error"));
    assert_eq!(dataset.last_sync_at, last_success_at);
    Ok(())
}

#[tokio::test]
async fn interrupted_sync_runs_are_failed_and_exposed_on_the_dataset() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let running = store.start_sync_run(&dataset.id).await?;
    assert!(matches!(
        store.start_sync_run(&dataset.id).await,
        Err(ConnectorError::Database(_))
    ));

    assert_eq!(
        store.fail_running_sync_runs("process interrupted").await?,
        1
    );
    assert_eq!(
        store.fail_running_sync_runs("process interrupted").await?,
        0
    );

    let recovered = store.list_recent_sync_runs(&dataset.id, 1).await?;
    assert_eq!(recovered[0].id, running.id);
    assert_eq!(recovered[0].status, SyncRunStatus::Failed);
    assert_eq!(recovered[0].error.as_deref(), Some("process interrupted"));
    assert!(recovered[0].finished_at.is_some());
    let dataset = store
        .get_dataset(&dataset.id)
        .await?
        .ok_or("dataset missing")?;
    assert_eq!(dataset.last_error.as_deref(), Some("process interrupted"));
    assert!(dataset.last_sync_at.is_none());
    Ok(())
}

#[tokio::test]
async fn sync_snapshot_and_run_completion_commit_atomically() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let run = store.start_sync_run(&dataset.id).await?;
    sqlx::query(
        "CREATE TRIGGER reject_sync_finish BEFORE UPDATE ON connector_sync_runs \
         WHEN NEW.status = 'succeeded' BEGIN SELECT RAISE(ABORT, 'rejected'); END",
    )
    .execute(store.pool())
    .await?;

    let result = store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[item("a", "1", "A")],
            &[observation("a", "1", SourceDisposition::Included)],
            None,
        )
        .await;
    assert!(matches!(result, Err(ConnectorError::Database(_))));
    assert!(
        store
            .query_items(&dataset.id, ItemQuery::default())
            .await?
            .is_empty()
    );
    assert!(store.source_states(&dataset.id).await?.is_empty());
    assert_eq!(
        store.list_recent_sync_runs(&dataset.id, 1).await?[0].status,
        SyncRunStatus::Running
    );
    assert!(
        store
            .get_dataset(&dataset.id)
            .await?
            .ok_or("dataset missing")?
            .last_sync_at
            .is_none()
    );

    sqlx::query("DROP TRIGGER reject_sync_finish")
        .execute(store.pool())
        .await?;
    let (stats, completed) = store
        .commit_sync_snapshot(
            &run.id,
            &dataset.id,
            &[item("a", "1", "A")],
            &[observation("a", "1", SourceDisposition::Included)],
            None,
        )
        .await?;
    assert_eq!(stats.active, 1);
    assert_eq!(completed.status, SyncRunStatus::Succeeded);
    assert!(
        store
            .get_dataset(&dataset.id)
            .await?
            .ok_or("dataset missing")?
            .last_sync_at
            .is_some()
    );
    assert_eq!(
        store
            .get_dataset(&dataset.id)
            .await?
            .ok_or("dataset missing")?
            .synced_plan_revision,
        Some(dataset.plan_revision)
    );
    Ok(())
}

#[tokio::test]
async fn failed_run_and_reschedule_commit_atomically() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let run = store.start_sync_run(&dataset.id).await?;
    sqlx::query(
        "CREATE TRIGGER reject_sync_error BEFORE UPDATE ON connector_datasets \
         WHEN NEW.last_error = 'network error' BEGIN SELECT RAISE(ABORT, 'rejected'); END",
    )
    .execute(store.pool())
    .await?;

    let next_sync_at = time::OffsetDateTime::now_utc() + time::Duration::minutes(5);
    let result = store
        .fail_sync_run_and_reschedule(&run.id, "network error", Some(next_sync_at))
        .await;
    assert!(matches!(result, Err(ConnectorError::Database(_))));
    assert_eq!(
        store.list_recent_sync_runs(&dataset.id, 1).await?[0].status,
        SyncRunStatus::Running
    );
    let unchanged = store
        .get_dataset(&dataset.id)
        .await?
        .ok_or("dataset missing")?;
    assert!(unchanged.last_error.is_none());

    sqlx::query("DROP TRIGGER reject_sync_error")
        .execute(store.pool())
        .await?;
    let failed = store
        .fail_sync_run_and_reschedule(&run.id, "network error", Some(next_sync_at))
        .await?;
    assert_eq!(failed.status, SyncRunStatus::Failed);
    let dataset = store
        .get_dataset(&dataset.id)
        .await?
        .ok_or("dataset missing")?;
    assert_eq!(dataset.last_error.as_deref(), Some("network error"));
    assert_eq!(
        dataset.next_sync_at.map(|value| value.unix_timestamp()),
        Some(next_sync_at.unix_timestamp())
    );
    Ok(())
}

#[tokio::test]
async fn item_queries_are_bounded_and_paginated() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let items = vec![item("a", "1", "A"), item("b", "1", "B")];
    store
        .apply_snapshot(&dataset.id, &items, &["a".to_owned(), "b".to_owned()], true)
        .await?;
    let first = store
        .query_items(&dataset.id, ItemQuery {
            limit: 1,
            offset: 0,
            include_deleted: false,
            text: None,
        })
        .await?;
    let second = store
        .query_items(&dataset.id, ItemQuery {
            limit: 1,
            offset: 1,
            include_deleted: false,
            text: None,
        })
        .await?;
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].remote_id, second[0].remote_id);
    assert!(matches!(
        store
            .query_items(&dataset.id, ItemQuery {
                limit: MAX_QUERY_LIMIT + 1,
                ..ItemQuery::default()
            })
            .await,
        Err(ConnectorError::QueryLimit { .. })
    ));
    assert!(matches!(
        store.list_recent_sync_runs(&dataset.id, 0).await,
        Err(ConnectorError::QueryLimit { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn search_is_safe_dataset_scoped_and_tracks_updates() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let other = create_dataset(&store).await?;
    let mut searchable = item("shared", "1", "Alice's (Q3) planning + roadmap");
    searchable.search_text = "Alice's (Q3) planning + roadmap".to_owned();
    store
        .apply_snapshot(
            &dataset.id,
            std::slice::from_ref(&searchable),
            &["shared".to_owned()],
            true,
        )
        .await?;
    store
        .apply_snapshot(
            &other.id,
            std::slice::from_ref(&searchable),
            &["shared".to_owned()],
            true,
        )
        .await?;

    let matches = store
        .query_items(&dataset.id, ItemQuery {
            text: Some("Alice (Q3) ***".to_owned()),
            ..ItemQuery::default()
        })
        .await?;
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].dataset_id, dataset.id);
    assert!(
        store
            .query_items(&dataset.id, ItemQuery {
                text: Some("***".to_owned()),
                ..ItemQuery::default()
            })
            .await?
            .is_empty()
    );

    searchable.remote_version = Some("2".to_owned());
    searchable.content_hash = "hash-2".to_owned();
    searchable.search_text = "Bob's annual review".to_owned();
    store
        .apply_snapshot(
            &dataset.id,
            std::slice::from_ref(&searchable),
            &["shared".to_owned()],
            true,
        )
        .await?;
    assert!(
        store
            .query_items(&dataset.id, ItemQuery {
                text: Some("Alice".to_owned()),
                ..ItemQuery::default()
            })
            .await?
            .is_empty()
    );
    assert_eq!(
        store
            .query_items(&dataset.id, ItemQuery {
                text: Some("Bob review".to_owned()),
                ..ItemQuery::default()
            })
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn get_item_is_dataset_scoped_and_honors_deleted_visibility() -> TestResult {
    let store = store().await?;
    let dataset = create_dataset(&store).await?;
    let other = create_dataset(&store).await?;
    store
        .apply_snapshot(&dataset.id, &[item("a", "1", "A")], &["a".to_owned()], true)
        .await?;
    let stored = store.query_items(&dataset.id, ItemQuery::default()).await?[0].clone();
    assert_eq!(
        store.get_item(&dataset.id, &stored.id, false).await?,
        Some(stored.clone())
    );
    assert!(store.get_item(&other.id, &stored.id, true).await?.is_none());

    store.apply_snapshot(&dataset.id, &[], &[], true).await?;
    assert!(
        store
            .get_item(&dataset.id, &stored.id, false)
            .await?
            .is_none()
    );
    assert!(
        store
            .get_item(&dataset.id, &stored.id, true)
            .await?
            .is_some_and(|item| item.deleted_at.is_some())
    );
    Ok(())
}
