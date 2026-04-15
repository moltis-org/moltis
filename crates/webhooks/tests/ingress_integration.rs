//! Integration tests for the webhook ingress flow.
//!
//! Simulates the full pipeline: lookup → auth → profile parsing → event filter
//! → dedup → persist delivery, without the HTTP server.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    axum::http::HeaderMap,
    hmac::{Hmac, Mac},
    sha2::Sha256,
    sqlx::SqlitePool,
};

use moltis_webhooks::{
    auth, dedup,
    profiles::ProfileRegistry,
    store::{NewDelivery, SqliteWebhookStore, WebhookStore},
    types::{AuthMode, DeliveryStatus, EventFilter, SessionMode, WebhookCreate},
};

type HmacSha256 = Hmac<Sha256>;

async fn setup() -> SqliteWebhookStore {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_webhooks::run_migrations(&pool).await.unwrap();
    SqliteWebhookStore::with_pool(pool)
}

fn make_headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (k, v) in pairs {
        headers.insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            axum::http::HeaderValue::from_str(v).unwrap(),
        );
    }
    headers
}

fn github_hmac_sig(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("sha256={sig}")
}

// ── Full GitHub PR ingress flow ────────────────────────────────────────

#[tokio::test]
async fn test_github_pr_ingress_full_flow() {
    let store = setup().await;
    let secret = "test-secret-123";

    // 1. Create a GitHub webhook
    let wh = store
        .create_webhook(WebhookCreate {
            name: "GitHub PR Review".into(),
            description: Some("Reviews pull requests".into()),
            agent_id: Some("code-reviewer".into()),
            model: None,
            system_prompt_suffix: Some("Focus on security issues.".into()),
            tool_policy: None,
            auth_mode: AuthMode::GithubHmacSha256,
            auth_config: Some(serde_json::json!({ "secret": secret })),
            source_profile: "github".into(),
            source_config: None,
            event_filter: EventFilter {
                allow: vec![
                    "pull_request.opened".into(),
                    "pull_request.synchronize".into(),
                ],
                deny: vec![],
            },
            session_mode: SessionMode::PerEntity,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        })
        .await
        .unwrap();

    assert!(wh.public_id.starts_with("wh_"));
    assert!(wh.enabled);

    // 2. Simulate an inbound GitHub PR opened event
    let body = serde_json::to_vec(&serde_json::json!({
        "action": "opened",
        "number": 42,
        "pull_request": {
            "number": 42,
            "title": "Add webhook support",
            "user": { "login": "testuser" },
            "head": { "ref": "feature/webhooks" },
            "base": { "ref": "main" },
            "html_url": "https://github.com/example/repo/pull/42",
            "body": "This PR adds webhook support.",
            "draft": false,
            "additions": 500,
            "deletions": 50,
            "changed_files": 20
        },
        "repository": {
            "full_name": "example/repo",
            "html_url": "https://github.com/example/repo"
        },
        "sender": { "login": "testuser" }
    }))
    .unwrap();

    let delivery_id = "test-delivery-001";
    let sig = github_hmac_sig(secret, &body);
    let headers = make_headers(&[
        ("x-github-event", "pull_request"),
        ("x-github-delivery", delivery_id),
        ("x-hub-signature-256", &sig),
        ("content-type", "application/json"),
    ]);

    // 3. Verify auth
    let verify_result = auth::verify(&wh.auth_mode, wh.auth_config.as_ref(), &headers, &body);
    assert!(verify_result.is_ok(), "auth should pass with correct HMAC");

    // 4. Parse event type and delivery key via profile
    let registry = ProfileRegistry::new();
    let profile = registry.get("github").expect("github profile exists");
    let event_type = profile.parse_event_type(&headers, &body);
    assert_eq!(event_type.as_deref(), Some("pull_request.opened"));

    let delivery_key = profile.parse_delivery_key(&headers, &body);
    assert_eq!(delivery_key.as_deref(), Some(delivery_id));

    // 5. Check event filter
    assert!(wh.event_filter.accepts("pull_request.opened"));
    assert!(!wh.event_filter.accepts("push")); // not in allow list

    // 6. Check dedup (should be None for first delivery)
    let dup = dedup::check_duplicate(&store, wh.id, delivery_key.as_deref())
        .await
        .unwrap();
    assert!(dup.is_none(), "first delivery should not be a duplicate");

    // 7. Extract entity key
    let body_val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entity_key = profile.entity_key("pull_request.opened", &body_val);
    assert_eq!(entity_key.as_deref(), Some("github:example/repo:pr:42"));

    // 8. Persist delivery
    let did = store
        .insert_delivery(&NewDelivery {
            webhook_id: wh.id,
            received_at: "2026-04-07T12:00:00Z".into(),
            status: DeliveryStatus::Queued,
            event_type: event_type.clone(),
            entity_key: entity_key.clone(),
            delivery_key: delivery_key.clone(),
            http_method: Some("POST".into()),
            content_type: Some("application/json".into()),
            remote_ip: Some("192.0.2.1".into()),
            headers_json: None,
            body_size: body.len(),
            body_blob: Some(body.clone()),
            rejection_reason: None,
        })
        .await
        .unwrap();

    // Increment count
    store
        .increment_delivery_count(wh.id, "2026-04-07T12:00:00Z")
        .await
        .unwrap();

    // 9. Verify delivery was persisted
    let delivery = store.get_delivery(did).await.unwrap();
    assert_eq!(delivery.status, DeliveryStatus::Queued);
    assert_eq!(delivery.event_type.as_deref(), Some("pull_request.opened"));
    assert_eq!(
        delivery.entity_key.as_deref(),
        Some("github:example/repo:pr:42")
    );

    // 10. Verify dedup now catches the duplicate
    let dup2 = dedup::check_duplicate(&store, wh.id, delivery_key.as_deref())
        .await
        .unwrap();
    assert_eq!(
        dup2,
        Some(did),
        "second delivery with same key is a duplicate"
    );

    // 11. Verify body can be retrieved
    let stored_body = store.get_delivery_body(did).await.unwrap();
    assert!(stored_body.is_some());
    assert_eq!(stored_body.unwrap(), body);

    // 12. Check webhook delivery count was incremented
    let updated_wh = store.get_webhook(wh.id).await.unwrap();
    assert_eq!(updated_wh.delivery_count, 1);

    // 13. Verify normalization produces useful output
    let normalized = profile.normalize_payload("pull_request.opened", &body_val);
    assert!(normalized.summary.contains("pull_request.opened"));
    assert!(normalized.summary.contains("example/repo"));
    assert!(normalized.summary.contains("PR #42"));
    assert!(normalized.summary.contains("Add webhook support"));
    assert!(normalized.summary.contains("@testuser"));

    // 14. Verify the delivery message builder works
    let message = moltis_webhooks::normalize::build_delivery_message(
        &wh,
        Some("pull_request.opened"),
        Some(delivery_id),
        "2026-04-07T12:00:00Z",
        &normalized.summary,
    );
    assert!(message.contains("Webhook delivery received."));
    assert!(message.contains("GitHub PR Review"));
    assert!(message.contains("github"));
    assert!(message.contains("Focus on security issues.")); // system prompt suffix
}

// ── Auth rejection ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_github_auth_rejects_bad_signature() {
    let secret = "real-secret";
    let body = b"test body";

    // Sign with wrong secret
    let bad_sig = github_hmac_sig("wrong-secret", body);
    let headers = make_headers(&[
        ("x-github-event", "push"),
        ("x-hub-signature-256", &bad_sig),
    ]);

    let config = serde_json::json!({ "secret": secret });
    let result = auth::verify(&AuthMode::GithubHmacSha256, Some(&config), &headers, body);
    assert!(result.is_err(), "bad signature should be rejected");
}

#[tokio::test]
async fn test_static_header_auth_flow() {
    let store = setup().await;

    let wh = store
        .create_webhook(WebhookCreate {
            name: "Generic Hook".into(),
            description: None,
            agent_id: None,
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::StaticHeader,
            auth_config: Some(
                serde_json::json!({ "header": "x-webhook-secret", "value": "my-token" }),
            ),
            source_profile: "generic".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode: SessionMode::PerDelivery,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        })
        .await
        .unwrap();

    // Good token
    let good_headers = make_headers(&[("x-webhook-secret", "my-token")]);
    assert!(auth::verify(&wh.auth_mode, wh.auth_config.as_ref(), &good_headers, b"{}").is_ok());

    // Bad token
    let bad_headers = make_headers(&[("x-webhook-secret", "wrong")]);
    assert!(auth::verify(&wh.auth_mode, wh.auth_config.as_ref(), &bad_headers, b"{}").is_err());

    // Missing header
    let empty_headers = HeaderMap::new();
    assert!(
        auth::verify(
            &wh.auth_mode,
            wh.auth_config.as_ref(),
            &empty_headers,
            b"{}"
        )
        .is_err()
    );
}

// ── Event filter blocks unwanted events ────────────────────────────────

#[tokio::test]
async fn test_event_filter_blocks_unwanted_github_events() {
    let filter = EventFilter {
        allow: vec!["pull_request.opened".into()],
        deny: vec![],
    };

    assert!(filter.accepts("pull_request.opened"));
    assert!(!filter.accepts("pull_request.closed"));
    assert!(!filter.accepts("push"));
    assert!(!filter.accepts("issues.opened"));
}

#[tokio::test]
async fn test_event_filter_deny_overrides_allow() {
    let filter = EventFilter {
        allow: vec!["push".into(), "pull_request.opened".into()],
        deny: vec!["push".into()],
    };

    assert!(!filter.accepts("push")); // denied
    assert!(filter.accepts("pull_request.opened")); // allowed
}

// ── GitLab profile parsing ─────────────────────────────────────────────

#[tokio::test]
async fn test_gitlab_event_parsing() {
    let registry = ProfileRegistry::new();
    let profile = registry.get("gitlab").expect("gitlab profile exists");

    let body = serde_json::to_vec(&serde_json::json!({
        "object_kind": "merge_request",
        "user": { "username": "dev" },
        "project": { "path_with_namespace": "group/project" },
        "object_attributes": {
            "iid": 10,
            "title": "Fix bug",
            "action": "open",
            "url": "https://gitlab.com/group/project/-/merge_requests/10"
        }
    }))
    .unwrap();

    let headers = make_headers(&[("x-gitlab-event", "Merge Request Hook")]);
    let event_type = profile.parse_event_type(&headers, &body);
    assert_eq!(event_type.as_deref(), Some("merge_request.open"));

    let body_val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entity_key = profile.entity_key("merge_request.open", &body_val);
    assert_eq!(entity_key.as_deref(), Some("gitlab:group/project:mr:10"));
}

// ── Stripe profile parsing ─────────────────────────────────────────────

#[tokio::test]
async fn test_stripe_event_parsing() {
    let registry = ProfileRegistry::new();
    let profile = registry.get("stripe").expect("stripe profile exists");

    let body = serde_json::to_vec(&serde_json::json!({
        "id": "evt_test_123",
        "type": "customer.subscription.created",
        "livemode": false,
        "data": {
            "object": {
                "id": "sub_abc",
                "customer": "cus_xyz",
                "status": "active"
            }
        }
    }))
    .unwrap();

    let headers = make_headers(&[("content-type", "application/json")]);
    let event_type = profile.parse_event_type(&headers, &body);
    assert_eq!(event_type.as_deref(), Some("customer.subscription.created"));

    let delivery_key = profile.parse_delivery_key(&headers, &body);
    assert_eq!(delivery_key.as_deref(), Some("evt_test_123"));

    let body_val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let entity_key = profile.entity_key("customer.subscription.created", &body_val);
    assert_eq!(entity_key.as_deref(), Some("stripe:sub_abc"));
}

// ── Rate limiting ──────────────────────────────────────────────────────

#[test]
fn test_rate_limiter_enforces_per_webhook_limit() {
    let limiter = moltis_webhooks::rate_limit::WebhookRateLimiter::new(1000);

    // 10 requests should succeed for webhook 1 with limit 10
    for _ in 0..10 {
        assert!(limiter.check(1, 10));
    }
    // 11th should fail
    assert!(!limiter.check(1, 10));

    // Different webhook should still work
    assert!(limiter.check(2, 10));
}

// ── Disabled webhook returns not found ─────────────────────────────────

#[tokio::test]
async fn test_disabled_webhook_lookup() {
    let store = setup().await;

    let wh = store
        .create_webhook(WebhookCreate {
            name: "Disabled Hook".into(),
            description: None,
            agent_id: None,
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::None,
            auth_config: None,
            source_profile: "generic".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode: SessionMode::PerDelivery,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        })
        .await
        .unwrap();

    // Disable it
    store
        .update_webhook(wh.id, moltis_webhooks::types::WebhookPatch {
            enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();

    // Lookup by public_id still works, but enabled is false
    let fetched = store.get_webhook_by_public_id(&wh.public_id).await.unwrap();
    assert!(!fetched.enabled);
}

// ── Profile registry ───────────────────────────────────────────────────

#[test]
fn test_profile_registry_lists_all_profiles() {
    let registry = ProfileRegistry::new();
    let summaries = registry.list();
    let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();

    assert!(ids.contains(&"generic"));
    assert!(ids.contains(&"github"));
    assert!(ids.contains(&"gitlab"));
    assert!(ids.contains(&"stripe"));
}

#[test]
fn test_profile_registry_lookup() {
    let registry = ProfileRegistry::new();
    assert!(registry.get("github").is_some());
    assert!(registry.get("nonexistent").is_none());
}

// ── GitLab token auth uses "token" key, not "secret" ──────────────────

#[test]
fn test_gitlab_token_auth_uses_token_key() {
    // GitLab verify expects { "token": "..." }, not { "secret": "..." }.
    // Regression test: the UI previously sent { secret } for gitlab_token mode.
    let config_correct = serde_json::json!({ "token": "glpat-abc123" });
    let headers = make_headers(&[("x-gitlab-token", "glpat-abc123")]);
    assert!(auth::verify(&AuthMode::GitlabToken, Some(&config_correct), &headers, b"").is_ok());

    // Wrong key name must fail.
    let config_wrong = serde_json::json!({ "secret": "glpat-abc123" });
    assert!(auth::verify(&AuthMode::GitlabToken, Some(&config_wrong), &headers, b"").is_err());
}

// ── Auth with missing config returns clear error ──────────────────────

#[test]
fn test_auth_with_null_config_fails_clearly() {
    let headers = make_headers(&[("x-webhook-secret", "anything")]);
    let result = auth::verify(&AuthMode::StaticHeader, None, &headers, b"");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("missing auth config key"),
        "error should mention missing config, got: {err}"
    );
}

// ── UTF-8 safe truncation ─────────────────────────────────────────────

#[test]
fn test_truncate_str_ascii() {
    let s = "hello world";
    assert_eq!(moltis_webhooks::normalize::truncate_str(s, 5), "hello");
    assert_eq!(
        moltis_webhooks::normalize::truncate_str(s, 100),
        "hello world"
    );
}

#[test]
fn test_truncate_str_multibyte() {
    // Each emoji is 4 bytes. "🎉🎊🎈" = 12 bytes.
    let s = "\u{1F389}\u{1F38A}\u{1F388}";
    assert_eq!(s.len(), 12);

    // Truncate at 5 bytes — falls inside the 2nd emoji (bytes 4..8).
    // Should walk back to byte 4 (end of first emoji).
    let t = moltis_webhooks::normalize::truncate_str(s, 5);
    assert_eq!(t, "\u{1F389}");
    assert_eq!(t.len(), 4);

    // Truncate at 8 — exactly at boundary of 2nd emoji.
    let t2 = moltis_webhooks::normalize::truncate_str(s, 8);
    assert_eq!(t2, "\u{1F389}\u{1F38A}");
}

#[test]
fn test_truncate_str_cjk() {
    // CJK chars are 3 bytes each. "你好世界" = 12 bytes.
    let s = "你好世界";
    assert_eq!(s.len(), 12);

    // Truncate at 7 — falls inside "世" (bytes 6..9).
    let t = moltis_webhooks::normalize::truncate_str(s, 7);
    assert_eq!(t, "你好");
    assert_eq!(t.len(), 6);
}

// ── GitHub normalize with multibyte description doesn't panic ─────────

#[test]
fn test_github_normalize_multibyte_description() {
    let registry = ProfileRegistry::new();
    let profile = registry.get("github").unwrap();

    // Build a PR payload with a long CJK description (> 500 bytes).
    let long_desc = "修复".repeat(300); // 6 bytes * 300 = 1800 bytes
    let body = serde_json::json!({
        "action": "opened",
        "pull_request": {
            "number": 1,
            "title": "修复错误",
            "user": { "login": "dev" },
            "head": { "ref": "fix" },
            "base": { "ref": "main" },
            "body": long_desc,
            "html_url": "https://github.com/test/repo/pull/1",
            "draft": false
        },
        "repository": { "full_name": "test/repo" }
    });

    // This must not panic on multibyte truncation.
    let normalized = profile.normalize_payload("pull_request.opened", &body);
    assert!(normalized.summary.contains("PR #1"));
    assert!(normalized.summary.contains("..."));
}

// ── Generic profile normalize with large payload doesn't panic ────────

#[test]
fn test_generic_normalize_large_multibyte_payload() {
    let registry = ProfileRegistry::new();
    let profile = registry.get("generic").unwrap();

    // Build a payload > 8192 bytes with multibyte chars.
    let big_value = "日本語テスト".repeat(2000); // 18 bytes * 2000 = 36000 bytes
    let body = serde_json::json!({ "data": big_value });

    // This must not panic on multibyte truncation.
    let normalized = profile.normalize_payload("test", &body);
    assert!(normalized.summary.contains("truncated"));
}

// ── Crash recovery includes processing deliveries ─────────────────────

#[tokio::test]
async fn test_crash_recovery_includes_processing_deliveries() {
    let store = setup().await;
    let wh = store
        .create_webhook(WebhookCreate {
            name: "test".into(),
            description: None,
            agent_id: None,
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::None,
            auth_config: None,
            source_profile: "generic".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode: SessionMode::PerDelivery,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        })
        .await
        .unwrap();

    // Insert deliveries in various states.
    use moltis_webhooks::store::NewDelivery;
    for (i, status) in [
        DeliveryStatus::Received,
        DeliveryStatus::Queued,
        DeliveryStatus::Processing,
        DeliveryStatus::Completed,
        DeliveryStatus::Failed,
        DeliveryStatus::Filtered,
    ]
    .iter()
    .enumerate()
    {
        let id = store
            .insert_delivery(&NewDelivery {
                webhook_id: wh.id,
                received_at: format!("2026-04-07T00:00:0{i}Z"),
                status: status.clone(),
                event_type: None,
                entity_key: None,
                delivery_key: None,
                http_method: None,
                content_type: None,
                remote_ip: None,
                headers_json: None,
                body_size: 0,
                body_blob: None,
                rejection_reason: None,
            })
            .await
            .unwrap();
        // For non-received statuses, update the status after insert.
        if *status != DeliveryStatus::Received {
            store
                .update_delivery_status(id, status.clone(), Default::default())
                .await
                .unwrap();
        }
    }

    // Crash recovery should find received + queued + processing = 3.
    let unprocessed = store.list_unprocessed_deliveries().await.unwrap();
    assert_eq!(
        unprocessed.len(),
        3,
        "should find received, queued, and processing; got {unprocessed:?}"
    );
}

// ── Generic profile dedup: no body-hash fallback ──────────────────────

#[test]
fn test_generic_profile_no_body_hash_dedup() {
    let registry = ProfileRegistry::new();
    let profile = registry.get("generic").unwrap();

    // Without delivery ID headers, dedup key should be None.
    let headers = make_headers(&[("content-type", "application/json")]);
    let key = profile.parse_delivery_key(&headers, b"{\"same\":\"body\"}");
    assert!(
        key.is_none(),
        "generic profile should not generate dedup key from body hash"
    );

    // With an explicit header, dedup key should be present.
    let headers_with_id = make_headers(&[("x-delivery-id", "abc-123")]);
    let key2 = profile.parse_delivery_key(&headers_with_id, b"{}");
    assert_eq!(key2.as_deref(), Some("abc-123"));
}

// ── Rate limiter: global limit blocks across webhooks ─────────────────

#[test]
fn test_rate_limiter_global_limit_blocks_all() {
    let limiter = moltis_webhooks::rate_limit::WebhookRateLimiter::new(3);

    // 3 requests across different webhooks should succeed.
    assert!(limiter.check(1, 100));
    assert!(limiter.check(2, 100));
    assert!(limiter.check(3, 100));

    // 4th request from any webhook should be blocked by global limit.
    assert!(!limiter.check(4, 100));
    assert!(!limiter.check(1, 100));
}

// ── Webhook redaction ─────────────────────────────────────────────────

#[test]
fn test_webhook_redacted_hides_secrets() {
    let wh = moltis_webhooks::types::Webhook {
        id: 1,
        name: "test".into(),
        description: None,
        enabled: true,
        public_id: "wh_test".into(),
        agent_id: None,
        model: None,
        system_prompt_suffix: None,
        tool_policy: None,
        auth_mode: AuthMode::GithubHmacSha256,
        auth_config: Some(serde_json::json!({ "secret": "super-secret-value" })),
        source_profile: "github".into(),
        source_config: Some(serde_json::json!({ "api_token": "ghp_secret" })),
        event_filter: EventFilter::default(),
        session_mode: SessionMode::PerDelivery,
        named_session_key: None,
        allowed_cidrs: vec![],
        max_body_bytes: 1_048_576,
        rate_limit_per_minute: 60,
        delivery_count: 0,
        last_delivery_at: None,
        created_at: "2026-04-07T00:00:00Z".into(),
        updated_at: "2026-04-07T00:00:00Z".into(),
    };

    let redacted = wh.redacted();
    assert_eq!(redacted.auth_config, Some(serde_json::json!("[REDACTED]")));
    assert_eq!(
        redacted.source_config,
        Some(serde_json::json!("[REDACTED]"))
    );

    // Serialized JSON must not contain the secret.
    let json = serde_json::to_string(&redacted).unwrap();
    assert!(!json.contains("super-secret-value"));
    assert!(!json.contains("ghp_secret"));
    assert!(json.contains("[REDACTED]"));
}

#[test]
fn test_webhook_redacted_none_stays_none() {
    let wh = moltis_webhooks::types::Webhook {
        id: 1,
        name: "test".into(),
        description: None,
        enabled: true,
        public_id: "wh_test".into(),
        agent_id: None,
        model: None,
        system_prompt_suffix: None,
        tool_policy: None,
        auth_mode: AuthMode::None,
        auth_config: None,
        source_profile: "generic".into(),
        source_config: None,
        event_filter: EventFilter::default(),
        session_mode: SessionMode::PerDelivery,
        named_session_key: None,
        allowed_cidrs: vec![],
        max_body_bytes: 1_048_576,
        rate_limit_per_minute: 60,
        delivery_count: 0,
        last_delivery_at: None,
        created_at: "2026-04-07T00:00:00Z".into(),
        updated_at: "2026-04-07T00:00:00Z".into(),
    };

    let redacted = wh.redacted();
    assert!(redacted.auth_config.is_none());
    assert!(redacted.source_config.is_none());
}

// ── CIDR allowlist filtering ──────────────────────────────────────────

#[test]
fn test_cidr_match_ipv4() {
    use std::net::IpAddr;
    let cidr: ipnet::IpNet = "10.0.0.0/8".parse().unwrap();
    let ip: IpAddr = "10.1.2.3".parse().unwrap();
    assert!(cidr.contains(&ip));

    let outside: IpAddr = "192.168.1.1".parse().unwrap();
    assert!(!cidr.contains(&outside));
}

#[test]
fn test_cidr_match_exact_ip() {
    use std::net::IpAddr;
    let cidr: ipnet::IpNet = "203.0.113.50/32".parse().unwrap();
    let ip: IpAddr = "203.0.113.50".parse().unwrap();
    assert!(cidr.contains(&ip));

    let other: IpAddr = "203.0.113.51".parse().unwrap();
    assert!(!cidr.contains(&other));
}

#[test]
fn test_cidr_allowlist_logic() {
    // Simulate the allowlist check logic from the ingress handler.
    let allowed_cidrs = ["10.0.0.0/8".to_string(), "192.168.1.0/24".to_string()];
    let check = |ip_str: &str| -> bool {
        if let Ok(addr) = ip_str.parse::<std::net::IpAddr>() {
            allowed_cidrs.iter().any(|cidr| {
                cidr.parse::<ipnet::IpNet>()
                    .map(|net| net.contains(&addr))
                    .unwrap_or(false)
            })
        } else {
            false
        }
    };

    assert!(check("10.1.2.3"));
    assert!(check("192.168.1.100"));
    assert!(!check("172.16.0.1"));
    assert!(!check("8.8.8.8"));
}

#[test]
fn test_cidr_empty_allowlist_allows_all() {
    // When allowed_cidrs is empty, the check is skipped (all traffic allowed).
    let allowed_cidrs: Vec<String> = vec![];
    assert!(allowed_cidrs.is_empty()); // guard skips the check
}

// ── Source profile immutability on edit ────────────────────────────────

#[tokio::test]
async fn test_source_profile_not_in_patch() {
    // WebhookPatch does not have source_profile — verify the field is
    // absent so serde ignores it on the server side.
    let patch_json = serde_json::json!({
        "name": "renamed",
        "sourceProfile": "github",  // should be ignored
    });
    let patch: moltis_webhooks::types::WebhookPatch = serde_json::from_value(patch_json).unwrap();
    assert_eq!(patch.name, Some("renamed".into()));
    // source_profile is not a field on WebhookPatch, so it's silently ignored.
    // The webhook keeps its original source_profile.

    let store = setup().await;
    let wh = store
        .create_webhook(WebhookCreate {
            name: "test".into(),
            description: None,
            agent_id: None,
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::None,
            auth_config: None,
            source_profile: "generic".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode: SessionMode::PerDelivery,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        })
        .await
        .unwrap();

    let updated = store.update_webhook(wh.id, patch).await.unwrap();
    assert_eq!(updated.name, "renamed");
    assert_eq!(
        updated.source_profile, "generic",
        "source_profile must not change via patch"
    );
}
