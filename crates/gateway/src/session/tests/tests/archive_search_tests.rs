use super::*;

#[tokio::test]
async fn patch_archived_allows_unarchive_for_current_channel_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let pool = sqlite_pool().await;
    let metadata = Arc::new(SqliteSessionMetadata::new(pool));
    let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();
    metadata
        .upsert("telegram:bot1:123", Some("Telegram current".to_string()))
        .await
        .unwrap();
    metadata
        .set_channel_binding("telegram:bot1:123", Some(binding))
        .await;
    metadata.set_archived("telegram:bot1:123", true).await;

    let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

    let result = svc
        .patch(serde_json::json!({ "key": "telegram:bot1:123", "archived": false }))
        .await
        .unwrap();
    assert_eq!(
        result.get("archived").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert!(!metadata.get("telegram:bot1:123").await.unwrap().archived);
}

#[tokio::test]
async fn patch_archived_rejection_does_not_partially_mutate_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let pool = sqlite_pool().await;
    let metadata = Arc::new(SqliteSessionMetadata::new(pool));
    let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();
    metadata
        .upsert("telegram:bot1:123", Some("Telegram current".to_string()))
        .await
        .unwrap();
    metadata
        .set_channel_binding("telegram:bot1:123", Some(binding))
        .await;
    metadata
        .set_model("telegram:bot1:123", Some("claude-sonnet".to_string()))
        .await;

    let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

    let error = svc
        .patch(serde_json::json!({
            "key": "telegram:bot1:123",
            "label": "Mutated?",
            "model": "gpt-5",
            "archived": true
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("cannot be archived"));

    let entry = metadata.get("telegram:bot1:123").await.unwrap();
    assert_eq!(entry.label.as_deref(), Some("Telegram current"));
    assert_eq!(entry.model.as_deref(), Some("claude-sonnet"));
    assert!(!entry.archived);
}

#[tokio::test]
async fn search_excludes_archived_sessions_unless_requested() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let pool = sqlite_pool().await;
    let metadata = Arc::new(SqliteSessionMetadata::new(pool));
    metadata
        .upsert("session:visible", Some("Visible".to_string()))
        .await
        .unwrap();
    metadata
        .upsert("session:hidden", Some("Hidden".to_string()))
        .await
        .unwrap();
    metadata.set_archived("session:hidden", true).await;
    store
        .append(
            "session:visible",
            &serde_json::json!({"role": "user", "content": "archive needle visible"}),
        )
        .await
        .unwrap();
    store
        .append(
            "session:hidden",
            &serde_json::json!({"role": "user", "content": "archive needle hidden"}),
        )
        .await
        .unwrap();

    let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

    let default_results = svc
        .search(serde_json::json!({ "query": "needle", "limit": 10 }))
        .await
        .unwrap()
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(default_results.len(), 1);
    assert_eq!(default_results[0]["sessionKey"], "session:visible");
    assert_eq!(default_results[0]["archived"], false);

    let include_archived_results = svc
        .search(serde_json::json!({
            "query": "needle",
            "limit": 10,
            "includeArchived": true
        }))
        .await
        .unwrap()
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(include_archived_results.len(), 2);
    assert!(
        include_archived_results
            .iter()
            .any(|entry| entry["sessionKey"] == "session:hidden" && entry["archived"] == true)
    );
}

#[tokio::test]
async fn search_includes_results_without_metadata_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let pool = sqlite_pool().await;
    let metadata = Arc::new(SqliteSessionMetadata::new(pool));
    store
        .append(
            "session:orphaned",
            &serde_json::json!({"role": "user", "content": "needle without metadata"}),
        )
        .await
        .unwrap();

    let svc = LiveSessionService::new(Arc::clone(&store), Arc::clone(&metadata));

    let results = svc
        .search(serde_json::json!({ "query": "needle", "limit": 10 }))
        .await
        .unwrap()
        .as_array()
        .cloned()
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["sessionKey"], "session:orphaned");
    assert!(results[0]["label"].is_null());
    assert_eq!(results[0]["archived"], false);
}
