//! Tests for clearing a session's channel binding (`sessions.patch` with
//! `channelBinding: null`).
//!
//! A session attached to a chat runs every non-gateway turn without tools or
//! private context, so releasing it is the only way back. These pin both
//! directions: an attached session can be released, a chat's own session
//! cannot.

use super::*;

const ATTACH_BINDING: &str = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;

async fn service_with_attached_session(
    dir: &tempfile::TempDir,
    key: &str,
) -> (LiveSessionService, Arc<SqliteSessionMetadata>) {
    let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    metadata.upsert(key, None).await.unwrap();
    metadata
        .set_channel_binding(key, Some(ATTACH_BINDING.to_string()))
        .await;
    metadata
        .set_active_session("telegram", "bot1", "123", None, key)
        .await;
    let service = LiveSessionService::new(store, Arc::clone(&metadata));
    (service, metadata)
}

/// A session attached to a chat runs every turn without tools or private
/// context, so there must be a way back. Unbinding must also drop the
/// chat→session override: leaving it would keep routing channel traffic
/// into a session that no longer advertises a binding, which is the one
/// combination the channel-bound ceiling cannot detect.
#[tokio::test]
async fn patch_clears_an_attached_channel_binding_and_its_active_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, metadata) = service_with_attached_session(&dir, "session:attached").await;

    let result = svc
        .patch(serde_json::json!({
            "key": "session:attached",
            "channelBinding": null,
        }))
        .await
        .unwrap();

    assert!(result["channelBinding"].is_null());
    assert!(
        metadata
            .get("session:attached")
            .await
            .unwrap()
            .channel_binding
            .is_none()
    );
    assert_eq!(
        metadata
            .get_active_session("telegram", "bot1", "123", None)
            .await,
        None,
        "the chat must fall back to its own default session"
    );
}

/// A channel's own session *is* the room's conversation. Unbinding it would
/// hand a web turn full trust over other people's messages, and the next
/// inbound message would re-bind it anyway.
#[tokio::test]
async fn patch_refuses_to_unbind_a_channels_own_session() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, metadata) = service_with_attached_session(&dir, "telegram:bot1:123").await;

    let error = svc
        .patch(serde_json::json!({
            "key": "telegram:bot1:123",
            "channelBinding": null,
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("cannot be unbound"), "{error}");
    assert!(
        metadata
            .get("telegram:bot1:123")
            .await
            .unwrap()
            .channel_binding
            .is_some(),
        "the binding must survive a refused unbind"
    );
}

/// Creating a binding routes a chat's traffic into a session — a privileged
/// channel-side action. Rejecting the value outright is better than
/// ignoring it, which would let a caller believe it succeeded.
#[tokio::test]
async fn patch_rejects_setting_a_channel_binding() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, metadata) = service_with_attached_session(&dir, "session:attached").await;

    let error = svc
        .patch(serde_json::json!({
            "key": "session:attached",
            "channelBinding": {"channel_type": "telegram", "account_id": "bot1", "chat_id": "999"},
        }))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("only be set to null"), "{error}");
    assert_eq!(
        metadata
            .get("session:attached")
            .await
            .unwrap()
            .channel_binding
            .as_deref(),
        Some(ATTACH_BINDING),
        "a rejected patch must not alter the binding"
    );
}

/// Absent field must stay a no-op — every other `patch` field behaves that
/// way, and an accidental unbind here silently widens a session's trust.
#[tokio::test]
async fn patch_without_channel_binding_leaves_it_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, metadata) = service_with_attached_session(&dir, "session:attached").await;

    svc.patch(serde_json::json!({ "key": "session:attached", "label": "renamed" }))
        .await
        .unwrap();

    assert_eq!(
        metadata
            .get("session:attached")
            .await
            .unwrap()
            .channel_binding
            .as_deref(),
        Some(ATTACH_BINDING)
    );
}
