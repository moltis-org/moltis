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
    std::{path::PathBuf, sync::Arc},
    tokio::sync::RwLock,
    tracing::{debug, error, info, warn},
    web_push::{
        ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
        WebPushMessageBuilder,
    },
};

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
    pub async fn add_subscription(&self, sub: PushSubscription) -> Result<()> {
        {
            let mut store = self.store.write().await;
            // Remove any existing subscription with the same endpoint.
            store.subscriptions.retain(|s| s.endpoint != sub.endpoint);
            store.subscriptions.push(sub);
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
        self.save_store().await?;
        Ok(())
    }

    /// Get the number of active subscriptions.
    pub async fn subscription_count(&self) -> usize {
        self.store.read().await.subscriptions.len()
    }

    /// Get all subscriptions (for admin display).
    pub async fn list_subscriptions(&self) -> Vec<PushSubscription> {
        self.store.read().await.subscriptions.clone()
    }

    /// Send a push notification to all subscriptions.
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

        let payload_json = serde_json::to_vec(payload)?;
        let mut sent = 0;
        let mut failed_endpoints = Vec::new();

        for sub in &subscriptions {
            match self.send_to_subscription(&vapid, sub, &payload_json).await {
                Ok(()) => sent += 1,
                Err(e) => {
                    error!(endpoint = %sub.endpoint, error = %e, "Failed to send push notification");
                    // If the subscription is invalid (410 Gone), mark for removal.
                    if e.to_string().contains("410") || e.to_string().contains("Gone") {
                        failed_endpoints.push(sub.endpoint.clone());
                    }
                },
            }
        }

        // Clean up invalid subscriptions.
        if !failed_endpoints.is_empty() {
            let mut store = self.store.write().await;
            store
                .subscriptions
                .retain(|s| !failed_endpoints.contains(&s.endpoint));
            drop(store);
            let _ = self.save_store().await;
        }

        Ok(sent)
    }

    /// Send a push notification to a single subscription.
    async fn send_to_subscription(
        &self,
        vapid: &VapidKeys,
        sub: &PushSubscription,
        payload: &[u8],
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
