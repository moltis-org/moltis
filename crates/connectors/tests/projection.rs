use {
    moltis_connectors::{
        AccountCreate, ConnectorItemInput, ConnectorKind, Dataset, DatasetCreate, DatasetUpdate,
        ItemQuery, ProjectionConfig, SqliteConnectorStore, cleanup_projection_artifacts,
        run_migrations, write_projection,
    },
    serde_json::{Value, json},
    sqlx::sqlite::SqlitePoolOptions,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn fixture() -> TestResult<(SqliteConnectorStore, Dataset)> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    run_migrations(&pool).await?;
    let store = SqliteConnectorStore::new(pool);
    let account = store
        .create_account(AccountCreate {
            kind: ConnectorKind::Caldav,
            source_key: None,
            name: "Calendar".to_owned(),
            config: json!({}),
            enabled: true,
        })
        .await?;
    let dataset = store
        .create_dataset(DatasetCreate {
            account_id: account.id,
            name: "Projection".to_owned(),
            instruction: None,
            config: json!({}),
            schedule_minutes: None,
            projections: ProjectionConfig {
                jsonl: true,
                markdown: true,
            },
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    Ok((store, dataset))
}

#[tokio::test]
async fn writes_sanitized_jsonl_markdown_and_manifest_then_replaces_export() -> TestResult {
    let (store, dataset) = fixture().await?;
    let input = ConnectorItemInput {
        remote_id: "../../unsafe/item".to_owned(),
        kind: "event".to_owned(),
        remote_version: Some("1".to_owned()),
        occurred_at: Some("2026-08-06T09:00:00Z".to_owned()),
        updated_at: None,
        body_json: json!({"summary": "literal </script> and ``` fence", "count": 2}),
        search_text: "literal script fence".to_owned(),
        content_hash: "hash".to_owned(),
    };
    store
        .apply_snapshot(
            &dataset.id,
            std::slice::from_ref(&input),
            std::slice::from_ref(&input.remote_id),
            true,
        )
        .await?;
    let items = store.query_items(&dataset.id, ItemQuery::default()).await?;
    let temporary = tempfile::tempdir()?;
    let manifest = write_projection(temporary.path(), "../../my dataset", &dataset, &items).await?;
    assert_eq!(manifest.item_count, 1);
    assert_eq!(manifest.jsonl_path.as_deref(), Some("items.jsonl"));
    assert_eq!(manifest.source_plan_revision, dataset.synced_plan_revision);
    let markdown_relative = manifest.items[0]
        .markdown_path
        .as_deref()
        .ok_or("markdown path missing")?;
    assert!(!markdown_relative.contains(".."));

    let mut root_entries = tokio::fs::read_dir(temporary.path()).await?;
    let export = root_entries
        .next_entry()
        .await?
        .ok_or("export missing")?
        .path();
    assert!(root_entries.next_entry().await?.is_none());
    assert!(export.starts_with(temporary.path()));
    let manifest_value: Value =
        serde_json::from_slice(&tokio::fs::read(export.join("manifest.json")).await?)?;
    assert_eq!(manifest_value["dataset_id"], dataset.id);

    let jsonl = tokio::fs::read_to_string(export.join("items.jsonl")).await?;
    let projected: Value = serde_json::from_str(jsonl.trim_end())?;
    assert_eq!(projected["remote_id"], "../../unsafe/item");
    let markdown = tokio::fs::read_to_string(export.join(markdown_relative)).await?;
    assert!(markdown.starts_with("# Connector item\n\n````json\n"));
    assert!(markdown.contains("\"summary\": \"literal </script> and ``` fence\""));
    assert!(!markdown.contains("<pre>"));

    let disabled = store
        .update_dataset(&dataset.id, DatasetUpdate {
            name: dataset.name.clone(),
            instruction: dataset.instruction.clone(),
            config: dataset.config.clone(),
            schedule_minutes: None,
            projections: ProjectionConfig::default(),
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    let disabled_manifest =
        write_projection(temporary.path(), "../../my dataset", &disabled, &items).await?;
    assert!(disabled_manifest.jsonl_path.is_none());
    assert!(disabled_manifest.items[0].markdown_path.is_none());
    assert!(
        tokio::fs::metadata(export.join("manifest.json"))
            .await
            .is_ok()
    );
    assert!(
        tokio::fs::metadata(export.join("items.jsonl"))
            .await
            .is_err()
    );
    assert!(tokio::fs::metadata(export.join("items")).await.is_err());
    Ok(())
}

#[tokio::test]
async fn rejects_cross_dataset_items_without_replacing_existing_export() -> TestResult {
    let (store, dataset) = fixture().await?;
    let temporary = tempfile::tempdir()?;
    write_projection(temporary.path(), "events", &dataset, &[]).await?;
    let mut entries = tokio::fs::read_dir(temporary.path()).await?;
    let export = entries.next_entry().await?.ok_or("export missing")?.path();
    let before = tokio::fs::read(export.join("manifest.json")).await?;

    let other = store
        .create_dataset(DatasetCreate {
            account_id: dataset.account_id.clone(),
            name: "Other".to_owned(),
            instruction: None,
            config: json!({}),
            schedule_minutes: None,
            projections: dataset.projections,
            enabled: true,
            next_sync_at: None,
        })
        .await?;
    let input = ConnectorItemInput {
        remote_id: "other".to_owned(),
        kind: "event".to_owned(),
        remote_version: None,
        occurred_at: None,
        updated_at: None,
        body_json: json!({}),
        search_text: String::new(),
        content_hash: "hash".to_owned(),
    };
    store
        .apply_snapshot(
            &other.id,
            std::slice::from_ref(&input),
            std::slice::from_ref(&input.remote_id),
            true,
        )
        .await?;
    let other_items = store.query_items(&other.id, ItemQuery::default()).await?;
    assert!(
        write_projection(temporary.path(), "events", &dataset, &other_items)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(export.join("manifest.json")).await?, before);
    Ok(())
}

#[tokio::test]
async fn cleanup_removes_only_abandoned_projection_artifacts() -> TestResult {
    let temp = tempfile::tempdir()?;
    let export_root = temp.path().join("exports");
    tokio::fs::create_dir_all(export_root.join(".connector-export-stale")).await?;
    tokio::fs::create_dir_all(export_root.join(".connector-backup-stale")).await?;
    let current = export_root.join("current-dataset");
    tokio::fs::create_dir_all(&current).await?;

    assert_eq!(cleanup_projection_artifacts(&export_root).await?, 2);
    assert!(current.exists());
    assert!(export_root.join("stale").exists());
    assert_eq!(cleanup_projection_artifacts(&export_root).await?, 0);
    Ok(())
}
