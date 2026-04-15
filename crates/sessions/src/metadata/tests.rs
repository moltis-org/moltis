#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn test_upsert_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    meta.upsert("session:abc", Some("My Chat".to_string()));

    let list = meta.list();
    assert_eq!(list.len(), 2);
    let keys: Vec<&str> = list.iter().map(|e| e.key.as_str()).collect();
    assert!(keys.contains(&"main"));
    assert!(keys.contains(&"session:abc"));
    let abc = list.iter().find(|e| e.key == "session:abc").unwrap();
    assert_eq!(abc.label.as_deref(), Some("My Chat"));
}

#[test]
fn test_list_pins_main_then_sorts_by_recency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path).unwrap();

    meta.upsert("main", None);
    meta.upsert("session:older", None);
    meta.upsert("session:newer", None);

    if let Some(entry) = meta.entries.get_mut("main") {
        entry.created_at = 1;
        entry.updated_at = 1;
    }
    if let Some(entry) = meta.entries.get_mut("session:older") {
        entry.created_at = 100;
        entry.updated_at = 100;
    }
    if let Some(entry) = meta.entries.get_mut("session:newer") {
        entry.created_at = 200;
        entry.updated_at = 200;
    }

    let keys: Vec<String> = meta.list().into_iter().map(|entry| entry.key).collect();
    assert_eq!(keys, vec!["main", "session:newer", "session:older"]);
}

#[test]
fn test_save_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");

    {
        let mut meta = SessionMetadata::load(path.clone()).unwrap();
        meta.upsert("main", Some("Main".to_string()));
        meta.save().unwrap();
    }

    let meta = SessionMetadata::load(path).unwrap();
    let entry = meta.get("main").unwrap();
    assert_eq!(entry.label.as_deref(), Some("Main"));
}

#[test]
fn test_remove() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").is_some());
    meta.remove("main");
    assert!(meta.get("main").is_none());
}

async fn sqlite_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    // sessions table references projects, so create a stub projects table.
    sqlx::query("CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    SqliteSessionMetadata::init(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn test_sqlite_upsert_and_list() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    meta.upsert("session:abc", Some("My Chat".to_string()))
        .await
        .unwrap();

    let list = meta.list().await;
    assert_eq!(list.len(), 2);
    let abc = list.iter().find(|e| e.key == "session:abc").unwrap();
    assert_eq!(abc.label.as_deref(), Some("My Chat"));
}

#[tokio::test]
async fn test_sqlite_list_pins_main_then_sorts_by_recency() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    meta.upsert("session:older", None).await.unwrap();
    meta.upsert("session:newer", None).await.unwrap();

    meta.set_timestamps_and_counts("main", 1, 1, 0, 0).await;
    meta.set_timestamps_and_counts("session:older", 100, 100, 0, 0)
        .await;
    meta.set_timestamps_and_counts("session:newer", 200, 200, 0, 0)
        .await;

    let keys: Vec<String> = meta
        .list()
        .await
        .into_iter()
        .map(|entry| entry.key)
        .collect();
    assert_eq!(keys, vec!["main", "session:newer", "session:older"]);
}

#[tokio::test]
async fn test_sqlite_remove() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(meta.get("main").await.is_some());
    meta.remove("main").await;
    assert!(meta.get("main").await.is_none());
}

#[tokio::test]
async fn test_sqlite_touch() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    meta.touch("main", 5).await;
    assert_eq!(meta.get("main").await.unwrap().message_count, 5);
}

#[tokio::test]
async fn test_sqlite_set_timestamps_and_counts() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    meta.set_timestamps_and_counts("main", 100, 200, 5, 3).await;

    let entry = meta.get("main").await.unwrap();
    assert_eq!(entry.created_at, 100);
    assert_eq!(entry.updated_at, 200);
    assert_eq!(entry.message_count, 5);
    assert_eq!(entry.last_seen_message_count, 3);
}

#[tokio::test]
async fn test_sqlite_mark_seen() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    // New session starts with last_seen_message_count = 0.
    assert_eq!(meta.get("main").await.unwrap().last_seen_message_count, 0);

    // Simulate receiving messages.
    meta.touch("main", 5).await;
    // touch does NOT change last_seen_message_count.
    assert_eq!(meta.get("main").await.unwrap().last_seen_message_count, 0);

    // Mark as seen.
    meta.mark_seen("main").await;
    let entry = meta.get("main").await.unwrap();
    assert_eq!(entry.last_seen_message_count, 5);
    assert_eq!(entry.message_count, 5);

    // More messages arrive — last_seen stays at previous value.
    meta.touch("main", 8).await;
    let entry = meta.get("main").await.unwrap();
    assert_eq!(entry.message_count, 8);
    assert_eq!(entry.last_seen_message_count, 5);
}

#[tokio::test]
async fn test_sqlite_mark_seen_emits_patched_event() {
    let pool = sqlite_pool().await;
    let bus = crate::session_events::SessionEventBus::new();
    let meta = SqliteSessionMetadata::with_event_bus(pool, bus.clone());
    let mut rx = bus.subscribe();

    meta.upsert("main", None).await.unwrap();
    let created = rx.recv().await.unwrap();
    assert!(
        matches!(
            created,
            crate::session_events::SessionEvent::Created { session_key } if session_key == "main"
        ),
        "expected created event after upsert"
    );

    meta.mark_seen("main").await;
    let patched = rx.recv().await.unwrap();
    assert!(
        matches!(
            patched,
            crate::session_events::SessionEvent::Patched { session_key } if session_key == "main"
        ),
        "expected patched event after mark_seen"
    );
}

#[test]
fn test_touch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path).unwrap();

    meta.upsert("main", None);
    meta.touch("main", 5);
    assert_eq!(meta.get("main").unwrap().message_count, 5);
}

#[test]
fn test_archived() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(!meta.get("main").unwrap().archived);

    meta.set_archived("main", true);
    assert!(meta.get("main").unwrap().archived);

    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert!(reloaded.get("main").unwrap().archived);
}

#[test]
fn test_sandbox_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").unwrap().sandbox_enabled.is_none());

    meta.set_sandbox_enabled("main", Some(true));
    assert_eq!(meta.get("main").unwrap().sandbox_enabled, Some(true));

    meta.set_sandbox_enabled("main", None);
    assert!(meta.get("main").unwrap().sandbox_enabled.is_none());

    // Verify it round-trips through save/load.
    meta.set_sandbox_enabled("main", Some(false));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(reloaded.get("main").unwrap().sandbox_enabled, Some(false));
}

#[test]
fn test_worktree_branch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").unwrap().worktree_branch.is_none());

    meta.set_worktree_branch("main", Some("moltis/abc".to_string()));
    assert_eq!(
        meta.get("main").unwrap().worktree_branch.as_deref(),
        Some("moltis/abc")
    );

    meta.set_worktree_branch("main", None);
    assert!(meta.get("main").unwrap().worktree_branch.is_none());

    // Round-trip through save/load.
    meta.set_worktree_branch("main", Some("moltis/xyz".to_string()));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(
        reloaded.get("main").unwrap().worktree_branch.as_deref(),
        Some("moltis/xyz")
    );
}

#[tokio::test]
async fn test_sqlite_worktree_branch() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(meta.get("main").await.unwrap().worktree_branch.is_none());

    meta.set_worktree_branch("main", Some("moltis/abc".to_string()))
        .await;
    assert_eq!(
        meta.get("main").await.unwrap().worktree_branch.as_deref(),
        Some("moltis/abc")
    );

    meta.set_worktree_branch("main", None).await;
    assert!(meta.get("main").await.unwrap().worktree_branch.is_none());
}

#[tokio::test]
async fn test_sqlite_archived() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(!meta.get("main").await.unwrap().archived);

    meta.set_archived("main", true).await;
    assert!(meta.get("main").await.unwrap().archived);

    meta.set_archived("main", false).await;
    assert!(!meta.get("main").await.unwrap().archived);
}

#[test]
fn test_sandbox_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").unwrap().sandbox_image.is_none());

    meta.set_sandbox_image("main", Some("custom:latest".to_string()));
    assert_eq!(
        meta.get("main").unwrap().sandbox_image.as_deref(),
        Some("custom:latest")
    );

    meta.set_sandbox_image("main", None);
    assert!(meta.get("main").unwrap().sandbox_image.is_none());

    // Round-trip through save/load.
    meta.set_sandbox_image("main", Some("alpine:3.20".to_string()));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(
        reloaded.get("main").unwrap().sandbox_image.as_deref(),
        Some("alpine:3.20")
    );
}

#[tokio::test]
async fn test_sqlite_sandbox_image() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(meta.get("main").await.unwrap().sandbox_image.is_none());

    meta.set_sandbox_image("main", Some("custom:latest".to_string()))
        .await;
    assert_eq!(
        meta.get("main").await.unwrap().sandbox_image.as_deref(),
        Some("custom:latest")
    );

    meta.set_sandbox_image("main", None).await;
    assert!(meta.get("main").await.unwrap().sandbox_image.is_none());
}

#[test]
fn test_sandbox_enabled_serde_compat() {
    // Existing metadata without sandbox_enabled should deserialize fine.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    fs::write(
        &path,
        r#"{"main":{"id":"1","key":"main","label":null,"created_at":0,"updated_at":0,"message_count":0}}"#,
    )
    .unwrap();
    let meta = SessionMetadata::load(path).unwrap();
    assert!(meta.get("main").unwrap().sandbox_enabled.is_none());
}

#[test]
fn test_channel_binding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("tg:bot1:123", None);
    assert!(meta.get("tg:bot1:123").unwrap().channel_binding.is_none());

    let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;
    meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()));
    assert_eq!(
        meta.get("tg:bot1:123").unwrap().channel_binding.as_deref(),
        Some(binding)
    );

    meta.set_channel_binding("tg:bot1:123", None);
    assert!(meta.get("tg:bot1:123").unwrap().channel_binding.is_none());

    // Round-trip through save/load.
    meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(
        reloaded
            .get("tg:bot1:123")
            .unwrap()
            .channel_binding
            .as_deref(),
        Some(binding)
    );
}

#[tokio::test]
async fn test_sqlite_active_session() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    // No active session initially.
    assert!(
        meta.get_active_session("telegram", "bot1", "123", None)
            .await
            .is_none()
    );

    // Set and get.
    meta.set_active_session("telegram", "bot1", "123", None, "session:abc")
        .await;
    assert_eq!(
        meta.get_active_session("telegram", "bot1", "123", None)
            .await
            .as_deref(),
        Some("session:abc")
    );

    // Overwrite.
    meta.set_active_session("telegram", "bot1", "123", None, "session:def")
        .await;
    assert_eq!(
        meta.get_active_session("telegram", "bot1", "123", None)
            .await
            .as_deref(),
        Some("session:def")
    );

    // Different chat_id is independent.
    assert!(
        meta.get_active_session("telegram", "bot1", "456", None)
            .await
            .is_none()
    );

    // Thread ID isolates sessions within the same chat.
    meta.set_active_session("telegram", "bot1", "123", Some("42"), "session:topic")
        .await;
    assert_eq!(
        meta.get_active_session("telegram", "bot1", "123", Some("42"))
            .await
            .as_deref(),
        Some("session:topic")
    );
    // Original chat without thread_id still has its own session.
    assert_eq!(
        meta.get_active_session("telegram", "bot1", "123", None)
            .await
            .as_deref(),
        Some("session:def")
    );
}

#[tokio::test]
async fn test_sqlite_list_channel_sessions() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();

    // Create two sessions with the same channel binding.
    meta.upsert("telegram:bot1:123", Some("Session 1".into()))
        .await
        .unwrap();
    meta.set_channel_binding("telegram:bot1:123", Some(binding.clone()))
        .await;

    meta.upsert("session:new1", Some("Session 2".into()))
        .await
        .unwrap();
    meta.set_channel_binding("session:new1", Some(binding.clone()))
        .await;

    let sessions = meta.list_channel_sessions("telegram", "bot1", "123").await;
    assert_eq!(sessions.len(), 2);
    let keys: Vec<&str> = sessions.iter().map(|s| s.key.as_str()).collect();
    assert!(keys.contains(&"telegram:bot1:123"));
    assert!(keys.contains(&"session:new1"));

    // Different chat should return empty.
    let other = meta.list_channel_sessions("telegram", "bot1", "999").await;
    assert!(other.is_empty());
}

#[tokio::test]
async fn test_sqlite_clear_active_session_mappings() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.set_active_session("telegram", "bot1", "123", None, "session:abc")
        .await;
    meta.set_active_session("telegram", "bot1", "456", None, "session:abc")
        .await;
    meta.set_active_session("telegram", "bot1", "789", None, "session:def")
        .await;

    meta.clear_active_session_mappings("session:abc").await;

    assert!(
        meta.get_active_session("telegram", "bot1", "123", None)
            .await
            .is_none()
    );
    assert!(
        meta.get_active_session("telegram", "bot1", "456", None)
            .await
            .is_none()
    );
    assert_eq!(
        meta.get_active_session("telegram", "bot1", "789", None)
            .await
            .as_deref(),
        Some("session:def")
    );
}

#[tokio::test]
async fn test_sqlite_channel_binding() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("tg:bot1:123", None).await.unwrap();
    assert!(
        meta.get("tg:bot1:123")
            .await
            .unwrap()
            .channel_binding
            .is_none()
    );

    let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;
    meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()))
        .await;
    assert_eq!(
        meta.get("tg:bot1:123")
            .await
            .unwrap()
            .channel_binding
            .as_deref(),
        Some(binding)
    );

    meta.set_channel_binding("tg:bot1:123", None).await;
    assert!(
        meta.get("tg:bot1:123")
            .await
            .unwrap()
            .channel_binding
            .is_none()
    );
}

#[test]
fn test_mcp_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").unwrap().mcp_disabled.is_none());

    meta.set_mcp_disabled("main", Some(true));
    assert_eq!(meta.get("main").unwrap().mcp_disabled, Some(true));

    meta.set_mcp_disabled("main", Some(false));
    assert_eq!(meta.get("main").unwrap().mcp_disabled, Some(false));

    meta.set_mcp_disabled("main", None);
    assert!(meta.get("main").unwrap().mcp_disabled.is_none());

    // Round-trip through save/load.
    meta.set_mcp_disabled("main", Some(true));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(reloaded.get("main").unwrap().mcp_disabled, Some(true));
}

#[tokio::test]
async fn test_sqlite_mcp_disabled() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(meta.get("main").await.unwrap().mcp_disabled.is_none());

    meta.set_mcp_disabled("main", Some(true)).await;
    assert_eq!(meta.get("main").await.unwrap().mcp_disabled, Some(true));

    meta.set_mcp_disabled("main", Some(false)).await;
    assert_eq!(meta.get("main").await.unwrap().mcp_disabled, Some(false));

    meta.set_mcp_disabled("main", None).await;
    assert!(meta.get("main").await.unwrap().mcp_disabled.is_none());
}

#[tokio::test]
async fn test_version_starts_at_zero() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    let entry = meta.upsert("main", None).await.unwrap();
    assert_eq!(entry.version, 0);
}

#[tokio::test]
async fn test_version_increments_on_mutation() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert_eq!(meta.get("main").await.unwrap().version, 0);

    meta.set_model("main", Some("gpt-4".to_string())).await;
    assert_eq!(meta.get("main").await.unwrap().version, 1);

    meta.touch("main", 5).await;
    assert_eq!(meta.get("main").await.unwrap().version, 2);

    // Insert a project row so the FK constraint is satisfied.
    sqlx::query("INSERT INTO projects (id) VALUES ('proj1')")
        .execute(&meta.pool)
        .await
        .unwrap();
    meta.set_project_id("main", Some("proj1".to_string())).await;
    assert_eq!(meta.get("main").await.unwrap().version, 3);

    meta.set_archived("main", true).await;
    assert_eq!(meta.get("main").await.unwrap().version, 4);

    meta.set_sandbox_enabled("main", Some(true)).await;
    assert_eq!(meta.get("main").await.unwrap().version, 5);

    meta.set_sandbox_image("main", Some("img:1".to_string()))
        .await;
    assert_eq!(meta.get("main").await.unwrap().version, 6);

    meta.set_worktree_branch("main", Some("branch".to_string()))
        .await;
    assert_eq!(meta.get("main").await.unwrap().version, 7);

    meta.set_mcp_disabled("main", Some(true)).await;
    assert_eq!(meta.get("main").await.unwrap().version, 8);

    meta.set_channel_binding("main", Some("{}".to_string()))
        .await;
    assert_eq!(meta.get("main").await.unwrap().version, 9);

    meta.set_parent("main", Some("parent".to_string()), Some(0))
        .await;
    assert_eq!(meta.get("main").await.unwrap().version, 10);

    meta.mark_seen("main").await;
    assert_eq!(meta.get("main").await.unwrap().version, 11);

    meta.set_preview("main", Some("hello")).await;
    assert_eq!(meta.get("main").await.unwrap().version, 12);

    meta.set_agent_id("main", Some("agent-1")).await.unwrap();
    assert_eq!(meta.get("main").await.unwrap().version, 13);
}

#[tokio::test]
async fn test_version_increments_on_upsert_update() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", Some("First".to_string()))
        .await
        .unwrap();
    assert_eq!(meta.get("main").await.unwrap().version, 0);

    // Upsert with existing key bumps version via ON CONFLICT.
    meta.upsert("main", Some("Second".to_string()))
        .await
        .unwrap();
    assert_eq!(meta.get("main").await.unwrap().version, 1);
}

#[tokio::test]
async fn test_version_in_list() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    meta.touch("main", 3).await;

    let list = meta.list().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].version, 1);
}

#[test]
fn test_json_backend_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert_eq!(meta.get("main").unwrap().version, 0);

    meta.set_model("main", Some("gpt-4".to_string()));
    assert_eq!(meta.get("main").unwrap().version, 1);

    meta.touch("main", 5);
    assert_eq!(meta.get("main").unwrap().version, 2);

    // Upsert with label change bumps version.
    meta.upsert("main", Some("New Label".to_string()));
    assert_eq!(meta.get("main").unwrap().version, 3);

    // Upsert without change does not bump version.
    meta.upsert("main", Some("New Label".to_string()));
    assert_eq!(meta.get("main").unwrap().version, 3);

    // Round-trip through save/load preserves version.
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(reloaded.get("main").unwrap().version, 3);
}

#[test]
fn test_agent_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path.clone()).unwrap();

    meta.upsert("main", None);
    assert!(meta.get("main").unwrap().agent_id.is_none());

    meta.set_agent_id("main", Some("agent-1".to_string()));
    assert_eq!(
        meta.get("main").unwrap().agent_id.as_deref(),
        Some("agent-1")
    );

    meta.set_agent_id("main", None);
    assert!(meta.get("main").unwrap().agent_id.is_none());

    // Round-trip through save/load.
    meta.set_agent_id("main", Some("agent-2".to_string()));
    meta.save().unwrap();
    let reloaded = SessionMetadata::load(path).unwrap();
    assert_eq!(
        reloaded.get("main").unwrap().agent_id.as_deref(),
        Some("agent-2")
    );
}

#[test]
fn test_list_by_agent_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path).unwrap();

    meta.upsert("s1", Some("Session 1".to_string()));
    meta.upsert("s2", Some("Session 2".to_string()));
    meta.upsert("s3", Some("Session 3".to_string()));

    meta.set_agent_id("s1", Some("agent-a".to_string()));
    meta.set_agent_id("s2", Some("agent-a".to_string()));
    meta.set_agent_id("s3", Some("agent-b".to_string()));

    let agent_a = meta.list_by_agent_id("agent-a");
    assert_eq!(agent_a.len(), 2);
    let keys: Vec<&str> = agent_a.iter().map(|e| e.key.as_str()).collect();
    assert!(keys.contains(&"s1"));
    assert!(keys.contains(&"s2"));

    let agent_b = meta.list_by_agent_id("agent-b");
    assert_eq!(agent_b.len(), 1);
    assert_eq!(agent_b[0].key, "s3");

    let none = meta.list_by_agent_id("agent-missing");
    assert!(none.is_empty());
}

#[test]
fn test_delete_by_agent_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    let mut meta = SessionMetadata::load(path).unwrap();

    meta.upsert("s1", None);
    meta.upsert("s2", None);
    meta.upsert("s3", None);

    meta.set_agent_id("s1", Some("agent-a".to_string()));
    meta.set_agent_id("s2", Some("agent-a".to_string()));
    meta.set_agent_id("s3", Some("agent-b".to_string()));

    let deleted = meta.delete_by_agent_id("agent-a");
    assert_eq!(deleted, 2);
    assert!(meta.get("s1").is_none());
    assert!(meta.get("s2").is_none());
    assert!(meta.get("s3").is_some());

    // Deleting a non-existent agent returns 0.
    let deleted = meta.delete_by_agent_id("agent-missing");
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn test_sqlite_agent_id() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("main", None).await.unwrap();
    assert!(meta.get("main").await.unwrap().agent_id.is_none());

    meta.set_agent_id("main", Some("agent-1")).await.unwrap();
    assert_eq!(
        meta.get("main").await.unwrap().agent_id.as_deref(),
        Some("agent-1")
    );

    meta.set_agent_id("main", None).await.unwrap();
    assert!(meta.get("main").await.unwrap().agent_id.is_none());
}

#[tokio::test]
async fn test_sqlite_list_by_agent_id() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("s1", Some("Session 1".to_string()))
        .await
        .unwrap();
    meta.upsert("s2", Some("Session 2".to_string()))
        .await
        .unwrap();
    meta.upsert("s3", Some("Session 3".to_string()))
        .await
        .unwrap();

    meta.set_agent_id("s1", Some("agent-a")).await.unwrap();
    meta.set_agent_id("s2", Some("agent-a")).await.unwrap();
    meta.set_agent_id("s3", Some("agent-b")).await.unwrap();

    let agent_a = meta.list_by_agent_id("agent-a").await.unwrap();
    assert_eq!(agent_a.len(), 2);
    let keys: Vec<&str> = agent_a.iter().map(|e| e.key.as_str()).collect();
    assert!(keys.contains(&"s1"));
    assert!(keys.contains(&"s2"));

    let agent_b = meta.list_by_agent_id("agent-b").await.unwrap();
    assert_eq!(agent_b.len(), 1);
    assert_eq!(agent_b[0].key, "s3");

    let none = meta.list_by_agent_id("agent-missing").await.unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
async fn test_sqlite_delete_by_agent_id() {
    let pool = sqlite_pool().await;
    let meta = SqliteSessionMetadata::new(pool);

    meta.upsert("s1", None).await.unwrap();
    meta.upsert("s2", None).await.unwrap();
    meta.upsert("s3", None).await.unwrap();

    meta.set_agent_id("s1", Some("agent-a")).await.unwrap();
    meta.set_agent_id("s2", Some("agent-a")).await.unwrap();
    meta.set_agent_id("s3", Some("agent-b")).await.unwrap();

    let deleted = meta.delete_by_agent_id("agent-a").await.unwrap();
    assert_eq!(deleted, 2);
    assert!(meta.get("s1").await.is_none());
    assert!(meta.get("s2").await.is_none());
    assert!(meta.get("s3").await.is_some());

    // Deleting a non-existent agent returns 0.
    let deleted = meta.delete_by_agent_id("agent-missing").await.unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn test_agent_id_serde_compat() {
    // Existing metadata without agent_id should deserialize fine.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.json");
    fs::write(
        &path,
        r#"{"main":{"id":"1","key":"main","label":null,"created_at":0,"updated_at":0,"message_count":0}}"#,
    )
    .unwrap();
    let meta = SessionMetadata::load(path).unwrap();
    assert!(meta.get("main").unwrap().agent_id.is_none());
}
