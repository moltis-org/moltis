//! Push notification support for PWA clients.
//!
//! Handles VAPID key generation/storage, subscription management, and sending
//! push notifications when the LLM responds while the user is not actively
//! viewing the chat.

#[path = "push_client.rs"]
mod push_client;
#[path = "push_order.rs"]
mod push_order;
#[path = "push_store_io.rs"]
mod push_store_io;
#[path = "push_subscriptions.rs"]
mod push_subscriptions;

use {
    anyhow::{Context, Result},
    base64::Engine,
    chrono::{DateTime, Utc},
    futures::{StreamExt, stream},
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
    tokio::sync::{RwLock, Semaphore},
    tracing::{debug, error, info, warn},
    web_push::{
        ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushClient,
        WebPushError, WebPushMessageBuilder,
    },
};

use push_client::PinnedWebPushClient;

/// How long a device's foreground presence report is trusted.
///
/// Clients heartbeat while focused, so this bounds how long a crashed or
/// force-quit client keeps suppressing its own notifications.
const PRESENCE_TTL: Duration = Duration::from_secs(120);

/// How long a push service should hold an undelivered message for an offline
/// device. Past this the notification is stale enough to not be worth showing.
const PUSH_TTL_SECONDS: u32 = 6 * 60 * 60;

const MAX_SUBSCRIPTIONS: usize = 100;
const MAX_ENDPOINT_LEN: usize = 2_048;
const MAX_P256DH_LEN: usize = 128;
const MAX_AUTH_LEN: usize = 64;
const MAX_USER_AGENT_LEN: usize = 512;
const MAX_IP_ADDRESS_LEN: usize = 64;
const MAX_CLIENT_ID_LEN: usize = 128;
const MAX_SESSION_KEY_LEN: usize = 512;
const MAX_PRESENCE_CLIENTS_PER_ENDPOINT: usize = 32;
const P256DH_KEY_LEN: usize = 65;
const AUTH_SECRET_LEN: usize = 16;
const FANOUT_CONCURRENCY: usize = 8;
const MAX_CONCURRENT_FANOUTS: usize = 4;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const FANOUT_TIMEOUT: Duration = Duration::from_secs(15);
const DNS_VALIDATION_TIMEOUT: Duration = Duration::from_secs(5);

/// A subscription was not safe or well-formed enough to retain.
#[derive(Debug, thiserror::Error)]
pub enum PushSubscriptionValidationError {
    #[error("invalid push subscription: {0}")]
    Invalid(String),
    #[error("push subscription limit reached ({MAX_SUBSCRIPTIONS})")]
    LimitReached,
    #[error("push subscription was revoked")]
    Revoked,
}

/// Outcome of an ordered client presence update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceUpdateResult {
    Recorded,
    UnknownEndpoint,
    Revoked,
    Stale,
    Invalid,
    TooManyClients,
}

/// Aggregate result of a push fanout.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct PushSendStats {
    pub targeted: usize,
    pub sent: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub expired: usize,
    pub suppressed: usize,
}

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
    /// Monotonic assistant message index within the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
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
            order: None,
            notification_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Attach a monotonic per-session delivery order.
    #[must_use]
    pub fn with_order(mut self, order: u64) -> Self {
        self.order = Some(order);
        self
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
    /// Monotonically increasing sequence number from this client.
    sequence: Option<u64>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PushStore {
    #[serde(skip_serializing_if = "Option::is_none")]
    vapid: Option<VapidKeys>,
    #[serde(default)]
    subscriptions: Vec<PushSubscription>,
    #[serde(default)]
    revoked_endpoints: Vec<RevokedEndpoint>,
}

impl PushStore {
    fn is_revoked(&self, endpoint: &str, replaces: Option<&str>) -> bool {
        self.revoked_endpoints.iter().any(|revoked| {
            revoked.endpoint == endpoint || replaces == Some(revoked.endpoint.as_str())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RevokedEndpoint {
    endpoint: String,
    revoked_at: DateTime<Utc>,
}

/// Push notification service.
pub struct PushService {
    store: RwLock<PushStore>,
    store_path: PathBuf,
    client: Box<dyn WebPushClient + Send + Sync>,
    /// Foreground leases per endpoint and client. In-memory only — presence is
    /// meaningless across a restart because every client reconnects anyway.
    presence: RwLock<HashMap<String, HashMap<String, Presence>>>,
    send_timeout: Duration,
    fanout_timeout: Duration,
    fanout_slots: Semaphore,
    ordered_deliveries: push_order::OrderedDeliveryGates,
}

fn invalid_subscription(message: impl Into<String>) -> PushSubscriptionValidationError {
    PushSubscriptionValidationError::Invalid(message.into())
}

fn validate_bounded_field(
    name: &str,
    value: &str,
    max_len: usize,
) -> std::result::Result<(), PushSubscriptionValidationError> {
    if value.is_empty() {
        return Err(invalid_subscription(format!("{name} must not be empty")));
    }
    if value.len() > max_len {
        return Err(invalid_subscription(format!(
            "{name} exceeds the {max_len}-byte limit"
        )));
    }
    Ok(())
}

fn validate_endpoint(
    endpoint: &str,
) -> std::result::Result<url::Url, PushSubscriptionValidationError> {
    validate_bounded_field("endpoint", endpoint, MAX_ENDPOINT_LEN)?;
    let parsed = url::Url::parse(endpoint).map_err(|error| {
        invalid_subscription(format!("endpoint is not an absolute URL: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid_subscription(
            "endpoint must be an absolute HTTPS URL",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid_subscription(
            "endpoint must not contain embedded credentials",
        ));
    }
    if parsed.fragment().is_some() {
        return Err(invalid_subscription("endpoint must not contain a fragment"));
    }

    // web-push's VAPID signer unwraps the parsed URI's scheme and host. Check
    // the exact URI parser it uses before the subscription can be retained.
    let uri = endpoint
        .parse::<http::Uri>()
        .map_err(|_| invalid_subscription("endpoint is not a valid HTTP URI"))?;
    if uri.scheme_str() != Some("https") || uri.host().is_none() {
        return Err(invalid_subscription(
            "endpoint must be an absolute HTTPS URI",
        ));
    }
    Ok(parsed)
}

async fn validate_endpoint_destination(
    endpoint: &url::Url,
) -> std::result::Result<(), PushSubscriptionValidationError> {
    tokio::time::timeout(
        DNS_VALIDATION_TIMEOUT,
        moltis_common::ssrf::ssrf_check(endpoint, &[]),
    )
    .await
    .map_err(|_| invalid_subscription("endpoint DNS resolution timed out"))?
    .map_err(|error| invalid_subscription(error.to_string()))
}

fn prune_presence(presence: &mut HashMap<String, HashMap<String, Presence>>) {
    presence.retain(|_, clients| {
        clients.retain(|_, lease| lease.reported_at.elapsed() < PRESENCE_TTL);
        !clients.is_empty()
    });
}

fn is_expired_endpoint_error(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<WebPushError>(),
        Some(WebPushError::EndpointNotFound(_) | WebPushError::EndpointNotValid(_))
    )
}

fn same_subscription_material(current: &PushSubscription, attempted: &PushSubscription) -> bool {
    current.endpoint == attempted.endpoint
        && current.p256dh == attempted.p256dh
        && current.auth == attempted.auth
}

impl PushService {
    /// Create a new push service, loading or generating VAPID keys.
    pub async fn new(data_dir: &std::path::Path) -> Result<Arc<Self>> {
        let client: Box<dyn WebPushClient + Send + Sync> = Box::new(PinnedWebPushClient);
        Self::new_with_client(data_dir, client, SEND_TIMEOUT, FANOUT_TIMEOUT).await
    }

    async fn new_with_client(
        data_dir: &std::path::Path,
        client: Box<dyn WebPushClient + Send + Sync>,
        send_timeout: Duration,
        fanout_timeout: Duration,
    ) -> Result<Arc<Self>> {
        let store_path = data_dir.join("push.json");
        let store = push_store_io::load(&store_path).await?;

        let service = Arc::new(Self {
            store: RwLock::new(store),
            store_path,
            client,
            presence: RwLock::new(HashMap::new()),
            send_timeout,
            fanout_timeout,
            fanout_slots: Semaphore::new(MAX_CONCURRENT_FANOUTS),
            ordered_deliveries: push_order::OrderedDeliveryGates::default(),
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

        let mut store = self.store.write().await;
        let mut candidate = store.clone();
        candidate.vapid = Some(keys);
        self.commit_store(&mut store, candidate).await?;
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

    /// Validate all attacker-controlled subscription data before retaining it.
    pub async fn validate_subscription(
        sub: &PushSubscription,
    ) -> std::result::Result<(), PushSubscriptionValidationError> {
        let endpoint = Self::validate_subscription_shape(sub)?;
        validate_endpoint_destination(&endpoint).await?;
        Ok(())
    }

    fn validate_subscription_shape(
        sub: &PushSubscription,
    ) -> std::result::Result<url::Url, PushSubscriptionValidationError> {
        let endpoint = validate_endpoint(&sub.endpoint)?;
        validate_bounded_field("p256dh", &sub.p256dh, MAX_P256DH_LEN)?;
        validate_bounded_field("auth", &sub.auth, MAX_AUTH_LEN)?;
        if let Some(user_agent) = &sub.user_agent {
            validate_bounded_field("user_agent", user_agent, MAX_USER_AGENT_LEN)?;
        }
        if let Some(ip_address) = &sub.ip_address {
            validate_bounded_field("ip_address", ip_address, MAX_IP_ADDRESS_LEN)?;
        }

        let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&sub.p256dh)
            .map_err(|_| invalid_subscription("p256dh must be unpadded base64url"))?;
        if p256dh.len() != P256DH_KEY_LEN
            || p256dh.first() != Some(&0x04)
            || PublicKey::from_sec1_bytes(&p256dh).is_err()
        {
            return Err(invalid_subscription(
                "p256dh must be a valid 65-byte uncompressed P-256 public key",
            ));
        }

        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&sub.auth)
            .map_err(|_| invalid_subscription("auth must be unpadded base64url"))?;
        if auth.len() != AUTH_SECRET_LEN {
            return Err(invalid_subscription("auth must decode to a 16-byte secret"));
        }

        Ok(endpoint)
    }

    async fn commit_store(&self, store: &mut PushStore, candidate: PushStore) -> Result<()> {
        push_store_io::save(&self.store_path, &candidate).await?;
        *store = candidate;
        Ok(())
    }

    /// Record what a device is currently looking at.
    ///
    /// Unknown endpoints are ignored so an unsubscribed or spoofed endpoint
    /// cannot grow the presence map without bound.
    pub async fn record_presence(
        &self,
        endpoint: &str,
        client_id: &str,
        sequence: Option<u64>,
        session_key: Option<String>,
        visible: bool,
    ) -> PresenceUpdateResult {
        if endpoint.len() > MAX_ENDPOINT_LEN
            || client_id.is_empty()
            || client_id.len() > MAX_CLIENT_ID_LEN
            || session_key
                .as_ref()
                .is_some_and(|key| key.len() > MAX_SESSION_KEY_LEN)
        {
            return PresenceUpdateResult::Invalid;
        }

        // Keep the store guard until the lease is written. Subscription
        // removal follows the same store -> presence lock order, so it cannot
        // remove an endpoint and then race with an already-authorized insert.
        let store = self.store.read().await;
        if !store
            .subscriptions
            .iter()
            .any(|subscription| subscription.endpoint == endpoint)
        {
            let endpoint_id = push_client::endpoint_identifier(endpoint);
            if store
                .revoked_endpoints
                .iter()
                .any(|revoked| revoked.endpoint == endpoint)
            {
                debug!(%endpoint_id, "ignoring presence for revoked push subscription");
                return PresenceUpdateResult::Revoked;
            }
            debug!(%endpoint_id, "ignoring presence for unknown push subscription");
            return PresenceUpdateResult::UnknownEndpoint;
        }

        let mut presence = self.presence.write().await;
        prune_presence(&mut presence);
        let clients = presence.entry(endpoint.to_string()).or_default();
        if sequence.is_some_and(|incoming| {
            clients
                .get(client_id)
                .and_then(|current| current.sequence)
                .is_some_and(|current| incoming <= current)
        }) {
            return PresenceUpdateResult::Stale;
        }
        if !clients.contains_key(client_id) && clients.len() >= MAX_PRESENCE_CLIENTS_PER_ENDPOINT {
            return PresenceUpdateResult::TooManyClients;
        }
        clients.insert(client_id.to_string(), Presence {
            session_key,
            visible,
            reported_at: Instant::now(),
            sequence,
        });
        PresenceUpdateResult::Recorded
    }

    /// Endpoints that reported themselves actively viewing `session_key`.
    async fn endpoints_watching(&self, session_key: &str) -> Vec<String> {
        let mut presence = self.presence.write().await;
        prune_presence(&mut presence);
        presence
            .iter()
            .filter(|(_, clients)| {
                clients
                    .values()
                    .any(|presence| presence.is_watching(session_key))
            })
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
        Ok(self.send_to_all_with_stats(payload).await?.sent)
    }

    /// Send to all eligible subscriptions and return aggregate delivery stats.
    pub async fn send_to_all_with_stats(&self, payload: &PushPayload) -> Result<PushSendStats> {
        let _ordered_delivery = if let (Some(session_key), Some(order)) =
            (payload.session_key.as_deref(), payload.order)
        {
            let Some(guard) = self.ordered_deliveries.start(session_key, order).await? else {
                debug!(session_key, order, "skipping stale push notification");
                return Ok(PushSendStats::default());
            };
            Some(guard)
        } else {
            None
        };

        let (vapid, subscriptions) = {
            let store = self.store.read().await;
            (store.vapid.clone(), store.subscriptions.clone())
        };

        let Some(vapid) = vapid else {
            warn!("No VAPID keys configured, cannot send push notifications");
            return Ok(PushSendStats::default());
        };

        if subscriptions.is_empty() {
            debug!("No push subscriptions, skipping notification");
            return Ok(PushSendStats::default());
        }

        // Skip the device the user is reading this very message on.
        let watching = match payload.session_key.as_deref() {
            Some(key) => self.endpoints_watching(key).await,
            None => Vec::new(),
        };
        let suppressed = watching.len();
        let targets: Vec<PushSubscription> = subscriptions
            .into_iter()
            .filter(|sub| !watching.contains(&sub.endpoint))
            .collect();

        if targets.is_empty() {
            debug!(
                suppressed,
                "all subscribed devices are viewing this session, skipping push"
            );
            return Ok(PushSendStats {
                suppressed,
                ..PushSendStats::default()
            });
        }

        let payload_json = serde_json::to_vec(payload)?;
        let topic = payload.topic();
        let targeted = targets.len();
        let deadline = tokio::time::sleep(self.fanout_timeout);
        tokio::pin!(deadline);
        let _fanout_slot = tokio::select! {
            permit = self.fanout_slots.acquire() => match permit {
                Ok(permit) => permit,
                Err(error) => {
                    error!(%error, targeted, "push fanout semaphore closed");
                    return Ok(PushSendStats {
                        targeted,
                        failed: targeted,
                        suppressed,
                        ..PushSendStats::default()
                    });
                },
            },
            () = &mut deadline => {
                warn!(targeted, "push fanout timed out waiting for capacity");
                return Ok(PushSendStats {
                    targeted,
                    timed_out: targeted,
                    suppressed,
                    ..PushSendStats::default()
                });
            },
        };

        // Bound fanout and every individual send. The upstream web-push client
        // explicitly never times out, so an endpoint must not be able to hold
        // this completion path forever.
        let vapid = &vapid;
        let mut sends = Box::pin(
            stream::iter(targets.into_iter().map(|sub| {
                let topic = topic.clone();
                let payload_json = payload_json.as_slice();
                async move {
                    let result = tokio::time::timeout(
                        self.send_timeout,
                        self.send_to_subscription(vapid, &sub, payload_json, topic),
                    )
                    .await;
                    (sub, result)
                }
            }))
            .buffer_unordered(FANOUT_CONCURRENCY),
        );
        let mut results = Vec::with_capacity(targeted);
        loop {
            tokio::select! {
                result = sends.next() => {
                    let Some(result) = result else {
                        break;
                    };
                    results.push(result);
                },
                () = &mut deadline => {
                    break;
                },
            }
        }
        let fanout_timed_out = targeted.saturating_sub(results.len());

        let mut stats = PushSendStats {
            targeted,
            suppressed,
            timed_out: fanout_timed_out,
            ..PushSendStats::default()
        };
        let mut expired_subscriptions = Vec::new();
        for (subscription, result) in results {
            let endpoint_id = push_client::endpoint_identifier(&subscription.endpoint);
            match result {
                Ok(Ok(())) => stats.sent += 1,
                Ok(Err(e)) => {
                    // Match the typed error rather than sniffing the message for
                    // "410": the push service's wording is not an API contract.
                    if is_expired_endpoint_error(&e) {
                        info!(%endpoint_id, "push endpoint expired, removing subscription");
                        expired_subscriptions.push(subscription);
                        stats.expired += 1;
                    } else {
                        let error_kind = e
                            .downcast_ref::<WebPushError>()
                            .map(WebPushError::short_description)
                            .unwrap_or("push_error");
                        error!(%endpoint_id, error_kind, "Failed to send push notification");
                        stats.failed += 1;
                    }
                },
                Err(_) => {
                    warn!(%endpoint_id, timeout_secs = self.send_timeout.as_secs_f64(), "push notification send timed out");
                    stats.timed_out += 1;
                },
            }
        }

        // Clean up invalid subscriptions.
        if !expired_subscriptions.is_empty() {
            let mut store = self.store.write().await;
            let mut candidate = store.clone();
            let mut removed_endpoints = Vec::new();
            for expired in &expired_subscriptions {
                if let Some(index) = candidate
                    .subscriptions
                    .iter()
                    .position(|current| same_subscription_material(current, expired))
                {
                    candidate.subscriptions.remove(index);
                    removed_endpoints.push(expired.endpoint.clone());
                }
            }
            if !removed_endpoints.is_empty() {
                self.commit_store(&mut store, candidate).await?;
                let mut presence = self.presence.write().await;
                for endpoint in removed_endpoints {
                    presence.remove(&endpoint);
                }
            }
        }

        if stats.failed > 0 || stats.timed_out > 0 || stats.expired > 0 {
            warn!(
                targeted = stats.targeted,
                sent = stats.sent,
                failed = stats.failed,
                timed_out = stats.timed_out,
                expired = stats.expired,
                suppressed = stats.suppressed,
                "push notification fanout completed with partial delivery"
            );
        } else {
            debug!(
                targeted = stats.targeted,
                sent = stats.sent,
                suppressed = stats.suppressed,
                "push notification fanout completed"
            );
        }

        Ok(stats)
    }

    /// Send a push notification to a single subscription.
    async fn send_to_subscription(
        &self,
        vapid: &VapidKeys,
        sub: &PushSubscription,
        payload: &[u8],
        topic: Option<String>,
    ) -> Result<()> {
        // Re-check syntax and crypto before message construction. The custom
        // client performs the send-time DNS resolution and pins those exact
        // addresses into the actual HTTP request.
        Self::validate_subscription_shape(sub)?;
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

        let endpoint_id = push_client::endpoint_identifier(&sub.endpoint);
        debug!(%endpoint_id, "Sent push notification");
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

/// Send an ordered push notification to all subscribers.
pub async fn send_ordered_push_notification(
    push_service: &Arc<PushService>,
    title: &str,
    body: &str,
    url: Option<&str>,
    session_key: Option<&str>,
    order: u64,
) -> Result<usize> {
    let payload = PushPayload::new(
        title,
        body,
        url.map(String::from),
        session_key.map(String::from),
    )
    .with_order(order);

    push_service.send_to_all(&payload).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "concurrency_tests.rs"]
mod concurrency_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        std::sync::{
            Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
        web_push::WebPushMessage,
    };

    #[derive(Default)]
    struct MockClientState {
        active: AtomicUsize,
        max_active: AtomicUsize,
        sent: AtomicUsize,
        endpoints: StdMutex<Vec<String>>,
    }

    struct MockClient {
        state: Arc<MockClientState>,
        delay: Duration,
    }

    struct ActiveSend(Arc<MockClientState>);

    impl Drop for ActiveSend {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl WebPushClient for MockClient {
        async fn send(&self, message: WebPushMessage) -> std::result::Result<(), WebPushError> {
            let active = self.state.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.state.max_active.fetch_max(active, Ordering::SeqCst);
            let _active = ActiveSend(Arc::clone(&self.state));
            if let Ok(mut endpoints) = self.state.endpoints.lock() {
                endpoints.push(message.endpoint.to_string());
            }
            if message.endpoint.path() == "/hang" {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(self.delay).await;
            self.state.sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn subscription(endpoint: &str) -> PushSubscription {
        let signing_key = SigningKey::random(&mut OsRng);
        let public_key = PublicKey::from(signing_key.verifying_key());
        PushSubscription {
            endpoint: endpoint.to_string(),
            p256dh: base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(public_key.to_sec1_bytes()),
            auth: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; AUTH_SECRET_LEN]),
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

    async fn mock_service(
        send_timeout: Duration,
        delay: Duration,
    ) -> (Arc<PushService>, Arc<MockClientState>, tempfile::TempDir) {
        mock_service_with_timeouts(send_timeout, Duration::from_secs(1), delay).await
    }

    async fn mock_service_with_timeouts(
        send_timeout: Duration,
        fanout_timeout: Duration,
        delay: Duration,
    ) -> (Arc<PushService>, Arc<MockClientState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = Arc::new(MockClientState::default());
        let client = Box::new(MockClient {
            state: Arc::clone(&state),
            delay,
        });
        let service =
            PushService::new_with_client(dir.path(), client, send_timeout, fanout_timeout)
                .await
                .expect("push service");
        (service, state, dir)
    }

    #[test]
    fn presence_matches_only_the_watched_session() {
        let presence = Presence {
            session_key: Some("main".to_string()),
            visible: true,
            reported_at: Instant::now(),
            sequence: Some(1),
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
            sequence: Some(1),
        };
        assert!(!presence.is_watching("main"));
    }

    #[test]
    fn stale_presence_stops_suppressing_notifications() {
        let presence = Presence {
            session_key: Some("main".to_string()),
            visible: true,
            reported_at: Instant::now() - PRESENCE_TTL - Duration::from_secs(1),
            sequence: Some(1),
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
        )
        .with_order(42);
        let value = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(value["title"], "Title");
        assert_eq!(value["sessionKey"], "main");
        assert_eq!(value["order"], 42);
        assert!(value["notificationId"].is_string());
        assert!(value["timestamp"].is_string());
    }

    #[test]
    fn each_payload_gets_a_distinct_notification_id() {
        let a = PushPayload::new("t", "b", None, None);
        let b = PushPayload::new("t", "b", None, None);
        assert_ne!(a.notification_id, b.notification_id);
        assert_eq!(a.order, None);
        assert_eq!(b.with_order(9).order, Some(9));
    }

    #[tokio::test]
    async fn subscription_validation_accepts_browser_shaped_data() {
        assert!(
            PushService::validate_subscription(&subscription("https://8.8.8.8/push"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn subscription_validation_rejects_unsafe_endpoints() {
        for endpoint in [
            "http://8.8.8.8/push",
            "https://user:secret@8.8.8.8/push",
            "https://127.0.0.1/push",
            "https://10.0.0.1/push",
            "https://169.254.169.254/push",
            "https://100.64.0.1/push",
            "https://[::1]/push",
            "https://[fe80::1]/push",
            "https://localhost/push",
        ] {
            assert!(
                PushService::validate_subscription(&subscription(endpoint))
                    .await
                    .is_err(),
                "unsafe endpoint was accepted: {endpoint}"
            );
        }
    }

    #[tokio::test]
    async fn subscription_validation_rejects_malformed_keys_and_oversized_fields() {
        let mut malformed_p256dh = subscription("https://8.8.8.8/push");
        malformed_p256dh.p256dh =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 64]);
        assert!(
            PushService::validate_subscription(&malformed_p256dh)
                .await
                .is_err()
        );

        let mut malformed_auth = subscription("https://8.8.8.8/push");
        malformed_auth.auth = "not base64!".to_string();
        assert!(
            PushService::validate_subscription(&malformed_auth)
                .await
                .is_err()
        );

        let mut oversized = subscription("https://8.8.8.8/push");
        oversized.user_agent = Some("x".repeat(MAX_USER_AGENT_LEN + 1));
        assert!(
            PushService::validate_subscription(&oversized)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn invalid_rotation_is_atomic() {
        let (service, _dir) = service().await;
        let old = "https://8.8.8.8/old";
        let new = "https://8.8.8.8/new";
        service
            .add_subscription(subscription(old), None)
            .await
            .expect("add old");

        let mut invalid_new = subscription(new);
        invalid_new.auth = "bad".to_string();
        let error = service
            .add_subscription(invalid_new, Some("not a URL"))
            .await
            .expect_err("invalid new subscription");
        assert!(
            error.to_string().contains("auth"),
            "the new subscription must be validated before replaces: {error}"
        );
        assert_eq!(
            service
                .list_subscriptions()
                .await
                .into_iter()
                .map(|sub| sub.endpoint)
                .collect::<Vec<_>>(),
            vec![old.to_string()]
        );

        assert!(
            service
                .add_subscription(subscription(new), Some("not a URL"))
                .await
                .is_err()
        );
        service
            .add_subscription(subscription(new), Some("https://127.0.0.1/old"))
            .await
            .expect("an old deletion key does not need to resolve publicly");
        assert_eq!(service.subscription_count().await, 2);
        service.remove_subscription(new).await.expect("remove new");
        assert_eq!(service.subscription_count().await, 1);
        assert_eq!(service.list_subscriptions().await[0].endpoint, old);
    }

    #[tokio::test]
    async fn subscription_count_is_capped_but_replacement_is_allowed_at_the_cap() {
        let (service, _dir) = service().await;
        let subscriptions = (0..MAX_SUBSCRIPTIONS)
            .map(|index| subscription(&format!("https://8.8.8.8/{index}")))
            .collect();
        service.store.write().await.subscriptions = subscriptions;

        let error = service
            .add_subscription(subscription("https://8.8.8.8/overflow"), None)
            .await
            .expect_err("subscription cap");
        assert!(
            matches!(
                error.downcast_ref::<PushSubscriptionValidationError>(),
                Some(PushSubscriptionValidationError::LimitReached)
            ),
            "unexpected error: {error}"
        );

        service
            .add_subscription(
                subscription("https://8.8.8.8/replacement"),
                Some("https://8.8.8.8/0"),
            )
            .await
            .expect("replacement at cap");
        assert_eq!(service.subscription_count().await, MAX_SUBSCRIPTIONS);
    }

    #[tokio::test]
    async fn presence_is_rejected_for_unknown_endpoints() {
        let (service, _dir) = service().await;
        assert_eq!(
            service
                .record_presence("https://8.8.8.8/unknown", "client-a", Some(1), None, true,)
                .await,
            PresenceUpdateResult::UnknownEndpoint,
            "an unknown endpoint must not be able to grow the presence map"
        );
        assert!(service.presence.read().await.is_empty());
    }

    #[tokio::test]
    async fn presence_is_recorded_for_known_endpoints() {
        let (service, _dir) = service().await;
        let endpoint = "https://8.8.8.8/abc";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("add subscription");

        assert_eq!(
            service
                .record_presence(
                    endpoint,
                    "client-a",
                    Some(1),
                    Some("main".to_string()),
                    true,
                )
                .await,
            PresenceUpdateResult::Recorded,
        );
        assert_eq!(service.endpoints_watching("main").await, vec![endpoint]);
        assert!(service.endpoints_watching("other").await.is_empty());
    }

    #[tokio::test]
    async fn presence_aggregates_clients_and_rejects_stale_updates() {
        let (service, _dir) = service().await;
        let endpoint = "https://8.8.8.8/presence";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("add subscription");

        assert_eq!(
            service
                .record_presence(endpoint, "tab-a", Some(10), Some("main".to_string()), true,)
                .await,
            PresenceUpdateResult::Recorded
        );
        assert_eq!(
            service
                .record_presence(endpoint, "tab-b", Some(1), Some("other".to_string()), true,)
                .await,
            PresenceUpdateResult::Recorded
        );
        assert_eq!(service.endpoints_watching("main").await, vec![endpoint]);
        assert_eq!(service.endpoints_watching("other").await, vec![endpoint]);

        assert_eq!(
            service
                .record_presence(endpoint, "tab-a", Some(9), None, false)
                .await,
            PresenceUpdateResult::Stale
        );
        assert_eq!(
            service.endpoints_watching("main").await,
            vec![endpoint],
            "a stale hidden report must not overwrite newer focused state"
        );

        assert_eq!(
            service
                .record_presence(endpoint, "tab-a", Some(11), None, false)
                .await,
            PresenceUpdateResult::Recorded
        );
        assert!(service.endpoints_watching("main").await.is_empty());
        assert_eq!(
            service.endpoints_watching("other").await,
            vec![endpoint],
            "another focused tab must continue suppressing its session"
        );
    }

    #[tokio::test]
    async fn stale_presence_leases_are_pruned() {
        let (service, _dir) = service().await;
        let endpoint = "https://8.8.8.8/stale";
        service
            .add_subscription(subscription(endpoint), None)
            .await
            .expect("add subscription");
        service
            .record_presence(endpoint, "tab-a", Some(1), Some("main".to_string()), true)
            .await;
        service
            .presence
            .write()
            .await
            .get_mut(endpoint)
            .and_then(|clients| clients.get_mut("tab-a"))
            .expect("presence lease")
            .reported_at = Instant::now() - PRESENCE_TTL - Duration::from_secs(1);

        assert!(service.endpoints_watching("main").await.is_empty());
        assert!(service.presence.read().await.is_empty());
    }

    #[tokio::test]
    async fn resubscribing_retires_the_replaced_endpoint() {
        let (service, _dir) = service().await;
        let old = "https://8.8.8.8/old";
        let new = "https://8.8.8.8/new";

        service
            .add_subscription(subscription(old), None)
            .await
            .expect("add old");
        service
            .record_presence(old, "client-a", Some(1), Some("main".to_string()), true)
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
    async fn send_skips_devices_watching_the_session() {
        let (service, _dir) = service().await;
        let watching = "https://8.8.8.8/watching";
        service
            .add_subscription(subscription(watching), None)
            .await
            .expect("add subscription");
        service
            .record_presence(
                watching,
                "client-a",
                Some(1),
                Some("main".to_string()),
                true,
            )
            .await;

        // The only subscriber is watching, so nothing is dispatched and no
        // network call is attempted.
        let payload = PushPayload::new("t", "b", None, Some("main".to_string()));
        assert_eq!(service.send_to_all(&payload).await.expect("send"), 0);
    }

    // The counts below tell a hung endpoint from a healthy one by whether its
    // send crosses `send_timeout`, and the mock's delay only sits 6x inside
    // that deadline. On the real clock a loaded machine can close that gap and
    // bill a healthy endpoint as timed out. A paused clock only moves when the
    // runtime is idle, so the delay stays well inside the timeout however long
    // the test really takes, while the hung endpoint still never completes.
    #[tokio::test(start_paused = true)]
    async fn fanout_is_bounded_and_times_out_a_hung_endpoint() {
        let (service, client, _dir) =
            mock_service(Duration::from_millis(30), Duration::from_millis(5)).await;
        for index in 0..12 {
            let path = if index == 0 {
                "hang".to_string()
            } else {
                format!("device-{index}")
            };
            service
                .add_subscription(subscription(&format!("https://8.8.8.8/{path}")), None)
                .await
                .expect("add subscription");
        }

        let payload = PushPayload::new("title", "body", None, None);
        let stats = service
            .send_to_all_with_stats(&payload)
            .await
            .expect("fanout completes");

        assert_eq!(stats.targeted, 12);
        assert_eq!(stats.sent, 11);
        assert_eq!(stats.timed_out, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(client.sent.load(Ordering::SeqCst), 11);
        assert!(
            client.max_active.load(Ordering::SeqCst) <= FANOUT_CONCURRENCY,
            "fanout exceeded its concurrency bound"
        );
        assert_eq!(client.active.load(Ordering::SeqCst), 0);
    }
}
