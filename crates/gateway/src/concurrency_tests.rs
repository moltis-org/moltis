use {
    super::*,
    std::sync::atomic::{AtomicUsize, Ordering},
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
