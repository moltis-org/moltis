//! Push notification support for PWA clients.
//!
//! Handles VAPID key generation/storage, subscription management, and sending
//! push notifications when the LLM responds while the user is not actively
//! viewing the chat.

use {
    anyhow::{Context, Result},
    base64::Engine,
    chrono::{DateTime, Utc},
    p256::{
        PublicKey, ecdsa::SigningKey, elliptic_curve::rand_core::OsRng, pkcs8::EncodePrivateKey,
    },
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    },
    tokio::sync::RwLock,
    tracing::{debug, error, info, warn},
    web_push::{
        ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushClient,
        WebPushError, WebPushMessageBuilder,
    },
};

/// How long a device's foreground presence report is trusted.
///
/// Reports are event-driven (visibility, focus, session switch), not polled, so
/// this only bounds how long a crashed or force-quit client keeps suppressing
/// its own notifications.
const PRESENCE_TTL: Duration = Duration::from_secs(120);

/// How long a push service should hold an undelivered message for an offline
/// device. Past this the notification is stale enough to not be worth showing.
const PUSH_TTL_SECONDS: u32 = 6 * 60 * 60;

/// VAPID keys for push notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VapidKeys {
    /// Base64 URL-safe encoded public key (for the browser).
    pub public_key: String,
    /// PEM-encoded private key (for signing).
    pub private_key_pem: String,
}

/// A push subscription from a browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscription {
    /// The push endpoint URL.
    pub endpoint: String,
    /// The p256dh key (base64 URL-safe encoded).
    pub p256dh: String,
    /// The auth secret (base64 URL-safe encoded).
    pub auth: String,
    /// User agent string (for debugging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Client IP address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    /// When the subscription was created.
    pub created_at: DateTime<Utc>,
}

/// Payload for a push notification.
#[derive(Debug, Clone, Serialize)]
pub struct PushPayload {
    /// Notification title.
    pub title: String,
    /// Notification body text.
    pub body: String,
    /// URL to open when clicked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Session key for deduplication.
    #[serde(rename = "sessionKey", skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    /// Unique id for this notification, so the service worker can tell a fresh
    /// notification apart from a redelivery of one it already showed.
    #[serde(rename = "notificationId")]
    pub notification_id: String,
    /// When the underlying event happened (ISO 8601).
    pub timestamp: DateTime<Utc>,
}

impl PushPayload {
    /// Build a payload, stamping it with a fresh id and the current time.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        url: Option<String>,
        session_key: Option<String>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            url,
            session_key,
            notification_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Collapse key for the push service.
    ///
    /// Messages sharing a topic supersede each other while the device is
    /// offline, so a phone that was away for an hour wakes to the latest
    /// message per session instead of a backlog of every one it missed.
    ///
    /// The Topic header is capped at 32 base64url characters, which is shorter
    /// than many session keys. Truncating the encoded key itself would make any
    /// two keys sharing a long prefix — `telegram:bot123:chat…`, or the nested
    /// project/session keys this app generates — collapse onto one another, so
    /// one chat would silently swallow another chat's pending notification.
    /// Hashing first keeps the whole key significant.
    fn topic(&self) -> Option<String> {
        use sha2::{Digest, Sha256};

        self.session_key.as_ref().map(|key| {
            let digest = Sha256::digest(key.as_bytes());
            let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
            // 32 base64url chars of SHA-256 keeps 192 bits — collisions are not
            // a practical concern.
            encoded.chars().take(32).collect()
        })
    }
}

/// What a subscribed device reported it is currently looking at.
#[derive(Debug, Clone)]
struct Presence {
    /// The session on screen, if the app is in the foreground.
    session_key: Option<String>,
    /// Whether the app is visible and focused.
    visible: bool,
    /// When the report arrived, used to expire stale presence.
    reported_at: Instant,
}

impl Presence {
    /// True when this device is actively watching `session_key` right now.
    fn is_watching(&self, session_key: &str) -> bool {
        self.visible
            && self.reported_at.elapsed() < PRESENCE_TTL
            && self.session_key.as_deref() == Some(session_key)
    }
}

/// Stored push data (VAPID keys + subscriptions).
#[derive(Debug, Default, Serialize, Deserialize)]
struct PushStore {
    #[serde(skip_serializing_if = "Option::is_none")]
    vapid: Option<VapidKeys>,
    #[serde(default)]
    subscriptions: Vec<PushSubscription>,
}

/// Push notification service.
pub struct PushService {
    store: RwLock<PushStore>,
    store_path: PathBuf,
    client: Box<dyn WebPushClient + Send + Sync>,
    /// Foreground presence per subscription endpoint. In-memory only — presence
    /// is meaningless across a restart because every client reconnects anyway.
    presence: RwLock<HashMap<String, Presence>>,
}

impl PushService {
    /// Create a new push service, loading or generating VAPID keys.
    pub async fn new(data_dir: &std::path::Path) -> Result<Arc<Self>> {
        let store_path = data_dir.join("push.json");
        let store = if store_path.exists() {
            let content = tokio::fs::read_to_string(&store_path)
                .await
                .context("Failed to read push store")?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            PushStore::default()
        };

        let client: Box<dyn WebPushClient + Send + Sync> =
            Box::new(web_push::IsahcWebPushClient::new()?);

        let service = Arc::new(Self {
            store: RwLock::new(store),
            store_path,
            client,
            presence: RwLock::new(HashMap::new()),
        });

        // Generate VAPID keys if not present.
        if service.store.read().await.vapid.is_none() {
            service.generate_vapid_keys().await?;
        }

        Ok(service)
    }

    /// Generate new VAPID keys and save them.
    async fn generate_vapid_keys(&self) -> Result<()> {
        info!("Generating new VAPID keys for push notifications");

        // Generate a new ECDSA P-256 key pair.
        let signing_key = SigningKey::random(&mut OsRng);
        let public_key = PublicKey::from(signing_key.verifying_key());

        // Get the public key in uncompressed point format and encode as base64 URL-safe.
        let public_key_bytes = public_key.to_sec1_bytes();
        let public_key_b64 =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&public_key_bytes);

        // Get the private key as PEM.
        let private_key_pem = signing_key
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .context("Failed to encode private key as PEM")?;

        let keys = VapidKeys {
            public_key: public_key_b64,
            private_key_pem: private_key_pem.to_string(),
        };

        {
            let mut store = self.store.write().await;
            store.vapid = Some(keys);
        }

        self.save_store().await?;
        info!("VAPID keys generated and saved");
        Ok(())
    }

    /// Get the VAPID public key for clients.
    pub async fn vapid_public_key(&self) -> Option<String> {
        self.store
            .read()
            .await
            .vapid
            .as_ref()
            .map(|v| v.public_key.clone())
    }

    /// Add a new push subscription.
    ///
    /// `replaces` carries the endpoint a browser just rotated away from, so a
    /// `pushsubscriptionchange` re-registration retires the dead endpoint
    /// instead of leaving it to accumulate until its next 410.
    pub async fn add_subscription(
        &self,
        sub: PushSubscription,
        replaces: Option<&str>,
    ) -> Result<()> {
        {
            let mut store = self.store.write().await;
            // Remove any existing subscription with the same endpoint.
            store.subscriptions.retain(|s| s.endpoint != sub.endpoint);
            if let Some(old) = replaces {
                store.subscriptions.retain(|s| s.endpoint != old);
            }
            store.subscriptions.push(sub);
        }
        if let Some(old) = replaces {
            self.presence.write().await.remove(old);
        }
        self.save_store().await?;
        info!("Added push subscription");
        Ok(())
    }

    /// Remove a subscription by endpoint.
    pub async fn remove_subscription(&self, endpoint: &str) -> Result<()> {
        {
            let mut store = self.store.write().await;
            let before = store.subscriptions.len();
            store.subscriptions.retain(|s| s.endpoint != endpoint);
            if store.subscriptions.len() < before {
                info!("Removed push subscription");
            }
        }
        self.presence.write().await.remove(endpoint);
        self.save_store().await?;
        Ok(())
    }

    /// Record what a device is currently looking at.
    ///
    /// Unknown endpoints are ignored so an unsubscribed or spoofed endpoint
    /// cannot grow the presence map without bound.
    pub async fn record_presence(
        &self,
        endpoint: &str,
        session_key: Option<String>,
        visible: bool,
    ) -> bool {
        let known = self
            .store
            .read()
            .await
            .subscriptions
            .iter()
            .any(|s| s.endpoint == endpoint);
        if !known {
            debug!(endpoint, "ignoring presence for unknown push subscription");
            return false;
        }

        self.presence
            .write()
            .await
            .insert(endpoint.to_string(), Presence {
                session_key,
                visible,
                reported_at: Instant::now(),
            });
        true
    }

    /// Endpoints that reported themselves actively viewing `session_key`.
    async fn endpoints_watching(&self, session_key: &str) -> Vec<String> {
        self.presence
            .read()
            .await
            .iter()
            .filter(|(_, presence)| presence.is_watching(session_key))
            .map(|(endpoint, _)| endpoint.clone())
            .collect()
    }

    /// Get the number of active subscriptions.
    pub async fn subscription_count(&self) -> usize {
        self.store.read().await.subscriptions.len()
    }

    /// Get all subscriptions (for admin display).
    pub async fn list_subscriptions(&self) -> Vec<PushSubscription> {
        self.store.read().await.subscriptions.clone()
    }

    /// Send a push notification to every subscription that is not already
    /// looking at the session the notification is about.
    ///
    /// Returns the number of endpoints the push was accepted for. Devices
    /// suppressed by presence are not counted as sends.
    pub async fn send_to_all(&self, payload: &PushPayload) -> Result<usize> {
        let (vapid, subscriptions) = {
            let store = self.store.read().await;
            (store.vapid.clone(), store.subscriptions.clone())
        };

        let Some(vapid) = vapid else {
            warn!("No VAPID keys configured, cannot send push notifications");
            return Ok(0);
        };

        if subscriptions.is_empty() {
            debug!("No push subscriptions, skipping notification");
            return Ok(0);
        }

        // Skip the device the user is reading this very message on.
        let watching = match payload.session_key.as_deref() {
            Some(key) => self.endpoints_watching(key).await,
            None => Vec::new(),
        };
        let targets: Vec<&PushSubscription> = subscriptions
            .iter()
            .filter(|sub| !watching.contains(&sub.endpoint))
            .collect();

        if targets.is_empty() {
            debug!(
                suppressed = watching.len(),
                "all subscribed devices are viewing this session, skipping push"
            );
            return Ok(0);
        }

        let payload_json = serde_json::to_vec(payload)?;
        let topic = payload.topic();

        // Fan out concurrently — a slow or unreachable push service must not
        // hold up delivery to every other device.
        let vapid = &vapid;
        let results = futures::future::join_all(targets.iter().map(|sub| {
            let payload_json = payload_json.as_slice();
            let topic = topic.clone();
            async move {
                (
                    sub.endpoint.clone(),
                    self.send_to_subscription(vapid, sub, payload_json, topic)
                        .await,
                )
            }
        }))
        .await;

        let mut sent = 0;
        let mut expired_endpoints = Vec::new();
        for (endpoint, result) in results {
            match result {
                Ok(()) => sent += 1,
                Err(e) => {
                    // Match the typed error rather than sniffing the message for
                    // "410": the push service's wording is not an API contract.
                    let expired = matches!(
                        e.downcast_ref::<WebPushError>(),
                        Some(WebPushError::EndpointNotFound(_) | WebPushError::EndpointNotValid(_))
                    );
                    if expired {
                        info!(%endpoint, "push endpoint expired, removing subscription");
                        expired_endpoints.push(endpoint);
                    } else {
                        error!(%endpoint, error = %e, "Failed to send push notification");
                    }
                },
            }
        }

        // Clean up invalid subscriptions.
        if !expired_endpoints.is_empty() {
            {
                let mut store = self.store.write().await;
                store
                    .subscriptions
                    .retain(|s| !expired_endpoints.contains(&s.endpoint));
            }
            {
                let mut presence = self.presence.write().await;
                for endpoint in &expired_endpoints {
                    presence.remove(endpoint);
                }
            }
            self.save_store().await?;
        }

        Ok(sent)
    }

    /// Send a push notification to a single subscription.
    async fn send_to_subscription(
        &self,
        vapid: &VapidKeys,
        sub: &PushSubscription,
        payload: &[u8],
        topic: Option<String>,
    ) -> Result<()> {
        let subscription_info = SubscriptionInfo {
            endpoint: sub.endpoint.clone(),
            keys: web_push::SubscriptionKeys {
                p256dh: sub.p256dh.clone(),
                auth: sub.auth.clone(),
            },
        };

        let sig_builder =
            VapidSignatureBuilder::from_pem(vapid.private_key_pem.as_bytes(), &subscription_info)?
                .build()?;

        let mut builder = WebPushMessageBuilder::new(&subscription_info);
        builder.set_payload(ContentEncoding::Aes128Gcm, payload);
        builder.set_vapid_signature(sig_builder);
        // Without a TTL some push services drop the message immediately when the
        // device is offline; without an urgency they may batch it for hours.
        builder.set_ttl(PUSH_TTL_SECONDS);
        builder.set_urgency(Urgency::High);
        if let Some(topic) = topic {
            builder.set_topic(topic);
        }

        let message = builder.build()?;
        self.client.send(message).await?;

        debug!(endpoint = %sub.endpoint, "Sent push notification");
        Ok(())
    }

    /// Save the store to disk.
    async fn save_store(&self) -> Result<()> {
        let store = self.store.read().await;
        let content = serde_json::to_string_pretty(&*store)?;
        tokio::fs::write(&self.store_path, content).await?;
        Ok(())
    }
}

/// Send a push notification to all subscribers.
pub async fn send_push_notification(
    push_service: &Arc<PushService>,
    title: &str,
    body: &str,
    url: Option<&str>,
    session_key: Option<&str>,
) -> Result<usize> {
    let payload = PushPayload::new(
        title,
        body,
        url.map(String::from),
        session_key.map(String::from),
    );

    push_service.send_to_all(&payload).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn subscription(endpoint: &str) -> PushSubscription {
        PushSubscription {
            endpoint: endpoint.to_string(),
            p256dh: "p256dh".to_string(),
            auth: "auth".to_string(),
            user_agent: None,
            ip_address: None,
            created_at: Utc::now(),
        }
    }

    /// The `TempDir` guard is returned alongside the service — dropping it
    /// would delete the directory the store writes into.
    async fn service() -> (Arc<PushService>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let service = PushService::new(dir.path()).await.expect("push service");
        (service, dir)
    }

    #[test]
    fn presence_matches_only_the_watched_session() {
        let presence = Presence {
            session_key: Some("main".to_string()),
            visible: true,
            reported_at: Instant::now(),
        };
        assert!(presence.is_watching("main"));
        assert!(!presence.is_watching("other"));
    }

    #[test]
    fn hidden_device_is_never_watching() {
        let presence = Presence {
            session_key: Some("main".to_string()),
            visible: false,
            reported_at: Instant::now(),
        };
        assert!(!presence.is_watching("main"));
    }

    #[test]
    fn stale_presence_stops_suppressing_notifications() {
        let presence = Presence {
            session_key: Some("main".to_string()),
            visible: true,
            reported_at: Instant::now() - PRESENCE_TTL - Duration::from_secs(1),
        };
        assert!(
            !presence.is_watching("main"),
            "a client that stopped reporting must not suppress push forever"
        );
    }

    #[test]
    fn topic_is_stable_per_session_and_within_header_limits() {
        let a = PushPayload::new("t", "b", None, Some("telegram:bot:chat".to_string()));
        let b = PushPayload::new("t2", "b2", None, Some("telegram:bot:chat".to_string()));
        assert_eq!(a.topic(), b.topic(), "same session must collapse");

        let long = PushPayload::new("t", "b", None, Some("x".repeat(200)));
        let topic = long.topic().expect("topic");
        assert!(topic.len() <= 32, "topic header is capped at 32 chars");
    }

    #[test]
    fn topic_is_absent_without_a_session() {
        assert!(PushPayload::new("t", "b", None, None).topic().is_none());
    }

    #[test]
    fn topics_differ_for_session_keys_sharing_a_long_prefix() {
        // Encoding the key directly and truncating to the header's 32-character
        // limit made any two keys agreeing on their first ~24 bytes collapse
        // onto one topic, so one chat's pending notification would supersede
        // another's. Nested and channel-scoped keys share prefixes routinely.
        let a = PushPayload::new(
            "t",
            "b",
            None,
            Some("telegram:bot123456789:chat-aaaa".into()),
        );
        let b = PushPayload::new(
            "t",
            "b",
            None,
            Some("telegram:bot123456789:chat-bbbb".into()),
        );
        assert_ne!(a.topic(), b.topic());

        let long_a = PushPayload::new("t", "b", None, Some(format!("{}-a", "x".repeat(200))));
        let long_b = PushPayload::new("t", "b", None, Some(format!("{}-b", "x".repeat(200))));
        assert_ne!(
            long_a.topic(),
            long_b.topic(),
            "keys must stay distinct however long they get"
        );
    }

    #[test]
    fn payload_serializes_with_client_field_names() {
        let payload = PushPayload::new(
            "Title",
            "Body",
            Some("/chats/main".into()),
            Some("main".into()),
        );
        let value = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(value["title"], "Title");
        assert_eq!(value["sessionKey"], "main");
        assert!(value["notificationId"].is_string());
        assert!(value["timestamp"].is_string());
    }

    #[test]
    fn each_payload_gets_a_distinct_notification_id() {
        let a = PushPayload::new("t", "b", None, None);
        let b = PushPayload::new("t", "b", None, None);
        assert_ne!(a.notification_id, b.notification_id);
    }

    #[tokio::test]
    async fn presence_is_rejected_for_unknown_endpoints() {
        let (service, _dir) = service().await;
        assert!(
            !service
                .record_presence("https://push.example/unknown", None, true)
                .await,
            "an unknown endpoint must not be able to grow the presence map"
        );
        assert!(service.presence.read().await.is_empty());
    }

    #[tokio::test]
    async fn presence_is_recorded_for_known_endpoints() {
        let (service, _dir) = service().await;
        let endpoint = "https://push.example/abc";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("add subscription");

        assert!(
            service
                .record_presence(endpoint, Some("main".to_string()), true)
                .await
        );
        assert_eq!(service.endpoints_watching("main").await, vec![endpoint]);
        assert!(service.endpoints_watching("other").await.is_empty());
    }

    #[tokio::test]
    async fn resubscribing_retires_the_replaced_endpoint() {
        let (service, _dir) = service().await;
        let old = "https://push.example/old";
        let new = "https://push.example/new";

        service
            .add_subscription(subscription(old), None)
            .await
            .expect("add old");
        service
            .record_presence(old, Some("main".to_string()), true)
            .await;

        service
            .add_subscription(subscription(new), Some(old))
            .await
            .expect("add new");

        let endpoints: Vec<String> = service
            .list_subscriptions()
            .await
            .into_iter()
            .map(|s| s.endpoint)
            .collect();
        assert_eq!(endpoints, vec![new.to_string()]);
        assert!(
            service.presence.read().await.get(old).is_none(),
            "presence for the rotated endpoint must not linger"
        );
    }

    #[tokio::test]
    async fn subscribing_twice_with_the_same_endpoint_does_not_duplicate() {
        let (service, _dir) = service().await;
        let endpoint = "https://push.example/abc";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("first");
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("second");
        assert_eq!(service.subscription_count().await, 1);
    }

    #[tokio::test]
    async fn send_skips_devices_watching_the_session() {
        let (service, _dir) = service().await;
        let watching = "https://push.example/watching";
        service
            .add_subscription(subscription(watching), None)
            .await
            .expect("add subscription");
        service
            .record_presence(watching, Some("main".to_string()), true)
            .await;

        // The only subscriber is watching, so nothing is dispatched and no
        // network call is attempted.
        let payload = PushPayload::new("t", "b", None, Some("main".to_string()));
        assert_eq!(service.send_to_all(&payload).await.expect("send"), 0);
    }

    #[tokio::test]
    async fn removing_a_subscription_drops_its_presence() {
        let (service, _dir) = service().await;
        let endpoint = "https://push.example/abc";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("add");
        service
            .record_presence(endpoint, Some("main".to_string()), true)
            .await;

        service.remove_subscription(endpoint).await.expect("remove");

        assert_eq!(service.subscription_count().await, 0);
        assert!(service.presence.read().await.is_empty());
    }

    /// Subscriptions are dropped only on a typed `WebPushError`, never on the
    /// text of an error message.
    ///
    /// `web_push` does not export `ErrorInfo`, so the expired variants cannot be
    /// constructed here and only the negative cases are covered. Those are the
    /// ones that matter: the previous implementation matched on
    /// `e.to_string().contains("410")`, which threw away working subscriptions
    /// whenever an unrelated error happened to mention that number.
    #[test]
    fn transient_errors_never_expire_a_subscription() {
        fn is_expired(e: &anyhow::Error) -> bool {
            matches!(
                e.downcast_ref::<WebPushError>(),
                Some(WebPushError::EndpointNotFound(_) | WebPushError::EndpointNotValid(_))
            )
        }

        assert!(!is_expired(&anyhow::Error::from(WebPushError::Unspecified)));
        assert!(!is_expired(&anyhow::Error::from(WebPushError::InvalidUri)));
        assert!(
            !is_expired(&anyhow::anyhow!("request failed with status 410")),
            "an untyped error must not be read as an expired endpoint"
        );
        assert!(!is_expired(&anyhow::anyhow!("Gone")));
    }

    #[tokio::test]
    async fn vapid_keys_persist_across_restarts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = PushService::new(dir.path()).await.expect("first");
        let key = first.vapid_public_key().await.expect("key");
        drop(first);

        let second = PushService::new(dir.path()).await.expect("second");
        assert_eq!(
            second.vapid_public_key().await.as_deref(),
            Some(key.as_str()),
            "rotating VAPID keys on restart would invalidate every subscription"
        );
    }
}
