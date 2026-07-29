use {
    super::*,
    std::sync::atomic::{AtomicUsize, Ordering},
    tokio::sync::Semaphore as TokioSemaphore,
    web_push::WebPushMessage,
};

struct HangingClient {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

struct DelayedClient {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

struct ControlledClient {
    started: Arc<TokioSemaphore>,
    release: Arc<TokioSemaphore>,
    calls: Arc<AtomicUsize>,
    expire: bool,
}

struct ActiveSend(Arc<AtomicUsize>);

impl Drop for ActiveSend {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl WebPushClient for HangingClient {
    async fn send(&self, _message: WebPushMessage) -> std::result::Result<(), WebPushError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        let _active = ActiveSend(Arc::clone(&self.active));
        std::future::pending().await
    }
}

#[async_trait::async_trait]
impl WebPushClient for DelayedClient {
    async fn send(&self, _message: WebPushMessage) -> std::result::Result<(), WebPushError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        let _active = ActiveSend(Arc::clone(&self.active));
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl WebPushClient for ControlledClient {
    async fn send(&self, _message: WebPushMessage) -> std::result::Result<(), WebPushError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.add_permits(1);
        let permit = self
            .release
            .acquire()
            .await
            .map_err(|_| WebPushError::Unspecified)?;
        permit.forget();
        if self.expire {
            Err(push_client::expired_endpoint_error())
        } else {
            Ok(())
        }
    }
}

fn subscription(index: usize) -> PushSubscription {
    let signing_key = SigningKey::random(&mut OsRng);
    let public_key = PublicKey::from(signing_key.verifying_key());
    PushSubscription {
        endpoint: format!("https://8.8.8.8/device-{index}"),
        p256dh: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public_key.to_sec1_bytes()),
        auth: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; AUTH_SECRET_LEN]),
        user_agent: None,
        ip_address: None,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn total_fanout_deadline_bounds_all_waves() {
    let dir = tempfile::tempdir().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let client = Box::new(HangingClient {
        active: Arc::clone(&active),
        max_active: Arc::clone(&max_active),
    });
    let service = PushService::new_with_client(
        dir.path(),
        client,
        Duration::from_secs(5),
        Duration::from_millis(30),
    )
    .await
    .unwrap();
    service.store.write().await.subscriptions = (0..20).map(subscription).collect();

    let started = Instant::now();
    let stats = service
        .send_to_all_with_stats(&PushPayload::new("title", "body", None, None))
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(stats.targeted, 20);
    assert_eq!(stats.timed_out, 20);
    assert!(max_active.load(Ordering::SeqCst) <= FANOUT_CONCURRENCY);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn concurrent_fanouts_are_globally_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let client = Box::new(DelayedClient {
        active: Arc::clone(&active),
        max_active: Arc::clone(&max_active),
    });
    let service = PushService::new_with_client(
        dir.path(),
        client,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    service.store.write().await.subscriptions = vec![subscription(0)];
    let payload = PushPayload::new("title", "body", None, None);

    let results = futures::future::join_all(
        (0..=MAX_CONCURRENT_FANOUTS).map(|_| service.send_to_all_with_stats(&payload)),
    )
    .await;

    assert!(results.into_iter().all(|result| result.unwrap().sent == 1));
    assert_eq!(max_active.load(Ordering::SeqCst), MAX_CONCURRENT_FANOUTS);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn subscription_removal_and_presence_update_do_not_deadlock_or_leave_a_lease() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    let endpoint = "https://8.8.8.8/lock-order";
    let mut stored = subscription(0);
    stored.endpoint = endpoint.to_string();
    service.add_subscription(stored, None).await.unwrap();

    let presence_service = Arc::clone(&service);
    let remove_service = Arc::clone(&service);
    tokio::time::timeout(Duration::from_secs(1), async move {
        let (_, removed) = tokio::join!(
            presence_service.record_presence(
                endpoint,
                "tab-a",
                Some(1),
                Some("main".to_string()),
                true,
            ),
            remove_service.remove_subscription(endpoint),
        );
        removed.unwrap();
    })
    .await
    .unwrap();

    assert_eq!(service.subscription_count().await, 0);
    assert!(service.presence.read().await.is_empty());
}

#[tokio::test]
async fn newer_order_prevents_waiting_older_delivery_from_becoming_final() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(TokioSemaphore::new(0));
    let release = Arc::new(TokioSemaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(ControlledClient {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
            expire: false,
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    service.store.write().await.subscriptions = vec![subscription(0)];

    let blocker = service
        .ordered_deliveries
        .start("ordered-session", 0)
        .await
        .unwrap()
        .unwrap();
    let older_service = Arc::clone(&service);
    let older = tokio::spawn(async move {
        older_service
            .send_to_all_with_stats(
                &PushPayload::new("older", "older", None, Some("ordered-session".into()))
                    .with_order(1),
            )
            .await
            .unwrap()
    });
    while service
        .ordered_deliveries
        .latest_order("ordered-session")
        .await
        != Some(1)
    {
        tokio::task::yield_now().await;
    }

    let newer_service = Arc::clone(&service);
    let newer = tokio::spawn(async move {
        newer_service
            .send_to_all_with_stats(
                &PushPayload::new("newer", "newer", None, Some("ordered-session".into()))
                    .with_order(2),
            )
            .await
            .unwrap()
    });
    while service
        .ordered_deliveries
        .latest_order("ordered-session")
        .await
        != Some(2)
    {
        tokio::task::yield_now().await;
    }

    drop(blocker);
    started.acquire().await.unwrap().forget();
    release.add_permits(1);
    assert_eq!(older.await.unwrap().sent, 0);
    assert_eq!(newer.await.unwrap().sent, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn completed_newer_delivery_rejects_a_late_older_task() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(TokioSemaphore::new(0));
    let release = Arc::new(TokioSemaphore::new(1));
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(ControlledClient {
            started,
            release,
            calls: Arc::clone(&calls),
            expire: false,
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    service.store.write().await.subscriptions = vec![subscription(0)];

    let newer =
        PushPayload::new("newer", "newer", None, Some("completed-session".into())).with_order(2);
    assert_eq!(
        service.send_to_all_with_stats(&newer).await.unwrap().sent,
        1
    );
    let older =
        PushPayload::new("older", "older", None, Some("completed-session".into())).with_order(1);
    assert_eq!(
        service.send_to_all_with_stats(&older).await.unwrap().sent,
        0
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ordered_delivery_high_water_marks_are_not_evicted() {
    let gates = push_order::OrderedDeliveryGates::default();
    for index in 0..1_025 {
        drop(
            gates
                .start(&format!("session-{index}"), index)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(gates.latest_order("session-0").await, Some(0));
}

#[tokio::test]
async fn ordered_deliveries_for_different_sessions_remain_concurrent() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(TokioSemaphore::new(0));
    let release = Arc::new(TokioSemaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(ControlledClient {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::clone(&calls),
            expire: false,
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    service.store.write().await.subscriptions = vec![subscription(0)];

    let tasks = ["session-a", "session-b"].map(|session| {
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            service
                .send_to_all_with_stats(
                    &PushPayload::new("title", "body", None, Some(session.to_string()))
                        .with_order(1),
                )
                .await
                .unwrap()
        })
    });
    started.acquire_many(2).await.unwrap().forget();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    release.add_permits(2);
    for task in tasks {
        assert_eq!(task.await.unwrap().sent, 1);
    }
}

#[tokio::test]
async fn expiry_cleanup_preserves_a_concurrently_refreshed_subscription() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(TokioSemaphore::new(0));
    let release = Arc::new(TokioSemaphore::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(ControlledClient {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::new(AtomicUsize::new(0)),
            expire: true,
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let endpoint = "https://8.8.8.8/refreshed";
    let attempted = subscription(0);
    let attempted_key = attempted.p256dh.clone();
    service.store.write().await.subscriptions = vec![PushSubscription {
        endpoint: endpoint.to_string(),
        ..attempted
    }];

    let sending_service = Arc::clone(&service);
    let sending = tokio::spawn(async move {
        sending_service
            .send_to_all_with_stats(&PushPayload::new("title", "body", None, None))
            .await
            .unwrap()
    });
    started.acquire().await.unwrap().forget();

    let mut refreshed = subscription(1);
    refreshed.endpoint = endpoint.to_string();
    assert_ne!(refreshed.p256dh, attempted_key);
    let refreshed_key = refreshed.p256dh.clone();
    service.add_subscription(refreshed, None).await.unwrap();
    release.add_permits(1);

    assert_eq!(sending.await.unwrap().expired, 1);
    let subscriptions = service.list_subscriptions().await;
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].endpoint, endpoint);
    assert_eq!(subscriptions[0].p256dh, refreshed_key);
}

#[tokio::test]
async fn revoked_endpoints_persist_and_require_explicit_revival() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    service
        .add_subscription(subscription(0), None)
        .await
        .unwrap();
    let stored_endpoint = service.list_subscriptions().await[0].endpoint.clone();
    service.remove_subscription(&stored_endpoint).await.unwrap();
    service.remove_subscription(&stored_endpoint).await.unwrap();
    drop(service);

    let service = PushService::new(dir.path()).await.unwrap();
    let mut automatic = subscription(1);
    automatic.endpoint = stored_endpoint.clone();
    let error = service
        .add_subscription(automatic.clone(), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PushSubscriptionValidationError>(),
        Some(PushSubscriptionValidationError::Revoked)
    ));
    assert_eq!(
        service
            .record_presence(&stored_endpoint, "tab-a", Some(1), None, true)
            .await,
        PresenceUpdateResult::Revoked
    );

    service
        .add_subscription_with_revival(automatic, None, true)
        .await
        .unwrap();
    assert_eq!(service.subscription_count().await, 1);

    service.remove_subscription(&stored_endpoint).await.unwrap();
    let mut rotated = subscription(2);
    rotated.endpoint = "https://8.8.8.8/rotated".to_string();
    let error = service
        .add_subscription(rotated.clone(), Some(&stored_endpoint))
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PushSubscriptionValidationError>(),
        Some(PushSubscriptionValidationError::Revoked)
    ));
    service
        .add_subscription_with_revival(rotated, Some(&stored_endpoint), true)
        .await
        .unwrap();
    assert_eq!(service.subscription_count().await, 1);

    let mut delayed_old = subscription(3);
    delayed_old.endpoint = stored_endpoint.clone();
    let error = service
        .add_subscription(delayed_old, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<PushSubscriptionValidationError>(),
        Some(PushSubscriptionValidationError::Revoked)
    ));
    assert_eq!(
        service
            .store
            .read()
            .await
            .revoked_endpoints
            .iter()
            .filter(|revoked| revoked.endpoint == stored_endpoint)
            .count(),
        1
    );
}

#[tokio::test]
async fn malformed_store_is_a_contextual_startup_error() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("push.json"), b"{not-json")
        .await
        .unwrap();
    let error = match PushService::new(dir.path()).await {
        Ok(_) => panic!("malformed store must fail startup"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains("failed to parse push store"), "{message}");
    assert!(message.contains("push.json"), "{message}");
}

async fn block_store_directory(dir: &tempfile::TempDir) {
    tokio::fs::remove_dir_all(dir.path()).await.unwrap();
    tokio::fs::write(dir.path(), b"not a directory")
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_add_persistence_does_not_mutate_the_live_store() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    block_store_directory(&dir).await;

    assert!(
        service
            .add_subscription(subscription(0), None)
            .await
            .is_err()
    );
    let store = service.store.read().await;
    assert!(store.subscriptions.is_empty());
    assert!(store.revoked_endpoints.is_empty());
}

#[tokio::test]
async fn failed_revival_persistence_preserves_the_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    let endpoint = "https://8.8.8.8/revive-save-failure";
    service.store.write().await.revoked_endpoints = vec![RevokedEndpoint {
        endpoint: endpoint.to_string(),
        revoked_at: Utc::now(),
    }];
    block_store_directory(&dir).await;

    let mut revived = subscription(0);
    revived.endpoint = endpoint.to_string();
    assert!(
        service
            .add_subscription_with_revival(revived, None, true)
            .await
            .is_err()
    );
    let store = service.store.read().await;
    assert!(store.subscriptions.is_empty());
    assert_eq!(store.revoked_endpoints.len(), 1);
    assert_eq!(store.revoked_endpoints[0].endpoint, endpoint);
}

#[tokio::test]
async fn failed_remove_persistence_preserves_subscription_presence_and_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    let endpoint = "https://8.8.8.8/remove-save-failure";
    let mut stored = subscription(0);
    stored.endpoint = endpoint.to_string();
    service.add_subscription(stored, None).await.unwrap();
    assert_eq!(
        service
            .record_presence(endpoint, "tab-a", Some(1), Some("main".into()), true)
            .await,
        PresenceUpdateResult::Recorded
    );
    block_store_directory(&dir).await;

    assert!(service.remove_subscription(endpoint).await.is_err());
    let store = service.store.read().await;
    assert_eq!(store.subscriptions.len(), 1);
    assert!(store.revoked_endpoints.is_empty());
    drop(store);
    assert_eq!(service.endpoints_watching("main").await, vec![endpoint]);
}

#[tokio::test]
async fn failed_key_persistence_does_not_install_generated_keys() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    service.store.write().await.vapid = None;
    block_store_directory(&dir).await;

    assert!(service.generate_vapid_keys().await.is_err());
    assert!(service.store.read().await.vapid.is_none());
}

#[tokio::test]
async fn failed_expiry_persistence_preserves_subscription_and_presence() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(TokioSemaphore::new(0));
    let release = Arc::new(TokioSemaphore::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(ControlledClient {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            calls: Arc::new(AtomicUsize::new(0)),
            expire: true,
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let endpoint = "https://8.8.8.8/expiry-save-failure";
    let mut stored = subscription(0);
    stored.endpoint = endpoint.to_string();
    service.store.write().await.subscriptions = vec![stored];
    assert_eq!(
        service
            .record_presence(endpoint, "tab-a", Some(1), Some("main".into()), true)
            .await,
        PresenceUpdateResult::Recorded
    );

    let sending_service = Arc::clone(&service);
    let sending = tokio::spawn(async move {
        sending_service
            .send_to_all_with_stats(&PushPayload::new("title", "body", None, None))
            .await
    });
    started.acquire().await.unwrap().forget();
    block_store_directory(&dir).await;
    release.add_permits(1);

    assert!(sending.await.unwrap().is_err());
    assert_eq!(service.subscription_count().await, 1);
    assert_eq!(service.endpoints_watching("main").await, vec![endpoint]);
}

#[tokio::test]
async fn legacy_and_ordered_public_send_apis_both_work() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();

    assert_eq!(
        send_push_notification(&service, "title", "body", None, Some("main"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        send_ordered_push_notification(&service, "title", "body", None, Some("main"), 7)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service.ordered_deliveries.latest_order("main").await,
        Some(7)
    );
}

#[tokio::test]
async fn legacy_store_without_tombstones_loads_and_persists_subscriptions() {
    let dir = tempfile::tempdir().unwrap();
    let first = PushService::new(dir.path()).await.unwrap();
    first.add_subscription(subscription(0), None).await.unwrap();
    drop(first);

    let path = dir.path().join("push.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("revoked_endpoints");
    tokio::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap())
        .await
        .unwrap();

    let second = PushService::new(dir.path()).await.unwrap();
    assert_eq!(second.subscription_count().await, 1);
    let endpoint = second.list_subscriptions().await[0].endpoint.clone();
    second.remove_subscription(&endpoint).await.unwrap();
    drop(second);
    let third = PushService::new(dir.path()).await.unwrap();
    assert_eq!(
        third
            .record_presence(&endpoint, "tab-a", Some(1), None, true)
            .await,
        PresenceUpdateResult::Revoked
    );
}

#[tokio::test]
async fn malformed_retained_subscription_is_failed_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let service = PushService::new_with_client(
        dir.path(),
        Box::new(DelayedClient {
            active: Arc::clone(&active),
            max_active,
        }),
        Duration::from_millis(30),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let mut malformed = subscription(0);
    malformed.endpoint = "not a URI".to_string();
    service.store.write().await.subscriptions.push(malformed);

    let stats = service
        .send_to_all_with_stats(&PushPayload::new("title", "body", None, None))
        .await
        .unwrap();
    assert_eq!(stats.failed, 1);
    assert_eq!(stats.sent, 0);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn removing_a_subscription_drops_its_presence() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    let endpoint = "https://8.8.8.8/abc";
    let mut stored = subscription(0);
    stored.endpoint = endpoint.to_string();
    service.add_subscription(stored, None).await.unwrap();
    service
        .record_presence(
            endpoint,
            "client-a",
            Some(1),
            Some("main".to_string()),
            true,
        )
        .await;

    service.remove_subscription(endpoint).await.unwrap();
    assert_eq!(service.subscription_count().await, 0);
    assert!(service.presence.read().await.is_empty());
}

#[test]
fn transient_errors_never_expire_a_subscription() {
    assert!(!is_expired_endpoint_error(&anyhow::Error::from(
        WebPushError::Unspecified
    )));
    assert!(!is_expired_endpoint_error(&anyhow::Error::from(
        WebPushError::InvalidUri
    )));
    assert!(!is_expired_endpoint_error(&anyhow::anyhow!(
        "request failed with status 410"
    )));
    assert!(!is_expired_endpoint_error(&anyhow::anyhow!("Gone")));
}

#[tokio::test]
async fn vapid_keys_persist_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let first = PushService::new(dir.path()).await.unwrap();
    let key = first.vapid_public_key().await.unwrap();
    drop(first);

    let second = PushService::new(dir.path()).await.unwrap();
    assert_eq!(
        second.vapid_public_key().await.as_deref(),
        Some(key.as_str())
    );
}

#[tokio::test]
async fn subscribing_twice_with_the_same_endpoint_does_not_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let service = PushService::new(dir.path()).await.unwrap();
    let endpoint = "https://8.8.8.8/duplicate";
    for index in 0..2 {
        let mut stored = subscription(index);
        stored.endpoint = endpoint.to_string();
        service.add_subscription(stored, None).await.unwrap();
    }
    assert_eq!(service.subscription_count().await, 1);
}
