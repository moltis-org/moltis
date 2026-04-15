//! Live webhooks service implementation wiring the webhooks crate into gateway services.

use std::sync::Arc;

use {async_trait::async_trait, base64::Engine as _, serde_json::Value, tracing::error};

#[cfg(feature = "vault")]
use {
    moltis_secret_store::{
        decrypt_secret_fields, encrypt_secret_fields, has_encrypted_secret_fields,
        has_plaintext_secret_fields,
    },
    moltis_vault::VaultStatus,
};

use moltis_webhooks::{
    profiles::ProfileRegistry,
    store::{DeliveryUpdate, NewDelivery, NewResponseAction, WebhookStore},
    types::{
        AuthMode, DeliveryStatus, Webhook, WebhookCreate, WebhookPatch, WebhookResponseAction,
    },
};

use crate::services::{ServiceError, ServiceResult, WebhooksService as WebhooksServiceTrait};

const WEBHOOK_SECRET_AAD_SCOPE: &str = "webhook:config";

fn auth_secret_fields(mode: &AuthMode) -> &'static [&'static str] {
    match mode {
        AuthMode::None => &[],
        AuthMode::StaticHeader => &["value"],
        AuthMode::Bearer | AuthMode::GitlabToken => &["token"],
        AuthMode::GithubHmacSha256
        | AuthMode::StripeWebhookSignature
        | AuthMode::LinearWebhookSignature
        | AuthMode::PagerdutyV2Signature
        | AuthMode::SentryWebhookSignature => &["secret"],
    }
}

fn source_secret_fields(_source_profile: &str) -> &'static [&'static str] {
    &[
        "access_token",
        "api_key",
        "api_token",
        "bearer_token",
        "client_secret",
        "secret",
        "signing_secret",
        "token",
        "webhook_secret",
    ]
}

#[cfg(feature = "vault")]
fn secret_store_error(
    context: &'static str,
    source: moltis_secret_store::Error,
) -> moltis_webhooks::Error {
    moltis_webhooks::Error::external(context, source)
}

#[cfg(feature = "vault")]
async fn encrypt_secret_config(
    config: &mut Option<Value>,
    secret_fields: &[&str],
    vault: Option<&Arc<moltis_vault::Vault>>,
) -> moltis_webhooks::Result<()> {
    if secret_fields.is_empty() {
        return Ok(());
    }

    let Some(config) = config.as_mut() else {
        return Ok(());
    };

    let Some(vault) = vault else {
        return Ok(());
    };

    if vault.is_unsealed().await {
        encrypt_secret_fields(
            config,
            secret_fields,
            WEBHOOK_SECRET_AAD_SCOPE,
            vault.as_ref(),
        )
        .await
        .map_err(|error| secret_store_error("encrypt webhook secrets", error))?;
        return Ok(());
    }

    let status = vault
        .status()
        .await
        .map_err(|error| moltis_webhooks::Error::external("check vault status", error))?;
    if matches!(status, VaultStatus::Uninitialized) {
        return Ok(());
    }

    let has_plaintext = has_plaintext_secret_fields(config, secret_fields)
        .map_err(|error| secret_store_error("inspect plaintext webhook secrets", error))?;
    if has_plaintext {
        return Err(moltis_webhooks::Error::message(
            "vault is sealed; webhook secrets cannot be persisted",
        ));
    }

    Ok(())
}

#[cfg(feature = "vault")]
pub async fn decrypt_webhook_secrets(
    webhook: &mut Webhook,
    vault: Option<&Arc<moltis_vault::Vault>>,
) -> anyhow::Result<()> {
    decrypt_secret_config(
        &mut webhook.auth_config,
        auth_secret_fields(&webhook.auth_mode),
        vault,
    )
    .await?;
    decrypt_secret_config(
        &mut webhook.source_config,
        source_secret_fields(&webhook.source_profile),
        vault,
    )
    .await?;
    Ok(())
}

#[cfg(feature = "vault")]
async fn decrypt_secret_config(
    config: &mut Option<Value>,
    fields: &[&str],
    vault: Option<&Arc<moltis_vault::Vault>>,
) -> anyhow::Result<()> {
    if fields.is_empty() {
        return Ok(());
    }

    let Some(config) = config.as_mut() else {
        return Ok(());
    };

    let has_encrypted = has_encrypted_secret_fields(config, fields)?;
    if !has_encrypted {
        return Ok(());
    }

    let Some(vault) = vault else {
        anyhow::bail!("encrypted webhook secrets require the vault");
    };

    if !vault.is_unsealed().await {
        anyhow::bail!("vault is sealed; encrypted webhook secrets are unavailable");
    }

    decrypt_secret_fields(config, fields, WEBHOOK_SECRET_AAD_SCOPE, vault.as_ref()).await?;
    Ok(())
}

/// Vault-aware wrapper that encrypts secret-bearing webhook configs before persistence.
#[cfg(feature = "vault")]
pub struct VaultWebhookStore {
    inner: Arc<dyn WebhookStore>,
    vault: Option<Arc<moltis_vault::Vault>>,
}

#[cfg(feature = "vault")]
impl VaultWebhookStore {
    pub fn new(inner: Arc<dyn WebhookStore>, vault: Option<Arc<moltis_vault::Vault>>) -> Self {
        Self { inner, vault }
    }
}

#[cfg(feature = "vault")]
#[async_trait]
impl WebhookStore for VaultWebhookStore {
    async fn list_webhooks(&self) -> moltis_webhooks::Result<Vec<Webhook>> {
        self.inner.list_webhooks().await
    }

    async fn get_webhook(&self, id: i64) -> moltis_webhooks::Result<Webhook> {
        self.inner.get_webhook(id).await
    }

    async fn get_webhook_by_public_id(&self, public_id: &str) -> moltis_webhooks::Result<Webhook> {
        self.inner.get_webhook_by_public_id(public_id).await
    }

    async fn create_webhook(&self, mut create: WebhookCreate) -> moltis_webhooks::Result<Webhook> {
        encrypt_secret_config(
            &mut create.auth_config,
            auth_secret_fields(&create.auth_mode),
            self.vault.as_ref(),
        )
        .await?;
        encrypt_secret_config(
            &mut create.source_config,
            source_secret_fields(&create.source_profile),
            self.vault.as_ref(),
        )
        .await?;
        self.inner.create_webhook(create).await
    }

    async fn update_webhook(
        &self,
        id: i64,
        mut patch: WebhookPatch,
    ) -> moltis_webhooks::Result<Webhook> {
        let existing = self.inner.get_webhook(id).await?;
        let auth_mode = patch
            .auth_mode
            .clone()
            .unwrap_or_else(|| existing.auth_mode.clone());

        if let Some(config) = patch.auth_config.as_mut() {
            encrypt_secret_config(config, auth_secret_fields(&auth_mode), self.vault.as_ref())
                .await?;
        }
        if let Some(config) = patch.source_config.as_mut() {
            encrypt_secret_config(
                config,
                source_secret_fields(&existing.source_profile),
                self.vault.as_ref(),
            )
            .await?;
        }

        self.inner.update_webhook(id, patch).await
    }

    async fn delete_webhook(&self, id: i64) -> moltis_webhooks::Result<()> {
        self.inner.delete_webhook(id).await
    }

    async fn increment_delivery_count(
        &self,
        id: i64,
        received_at: &str,
    ) -> moltis_webhooks::Result<()> {
        self.inner.increment_delivery_count(id, received_at).await
    }

    async fn insert_delivery(&self, delivery: &NewDelivery) -> moltis_webhooks::Result<i64> {
        self.inner.insert_delivery(delivery).await
    }

    async fn update_delivery_status(
        &self,
        id: i64,
        status: DeliveryStatus,
        update: DeliveryUpdate,
    ) -> moltis_webhooks::Result<()> {
        self.inner.update_delivery_status(id, status, update).await
    }

    async fn get_delivery(
        &self,
        id: i64,
    ) -> moltis_webhooks::Result<moltis_webhooks::types::WebhookDelivery> {
        self.inner.get_delivery(id).await
    }

    async fn list_deliveries(
        &self,
        webhook_id: i64,
        limit: usize,
        offset: usize,
    ) -> moltis_webhooks::Result<Vec<moltis_webhooks::types::WebhookDelivery>> {
        self.inner.list_deliveries(webhook_id, limit, offset).await
    }

    async fn get_delivery_body(
        &self,
        delivery_id: i64,
    ) -> moltis_webhooks::Result<Option<Vec<u8>>> {
        self.inner.get_delivery_body(delivery_id).await
    }

    async fn find_by_delivery_key(
        &self,
        webhook_id: i64,
        key: &str,
    ) -> moltis_webhooks::Result<Option<i64>> {
        self.inner.find_by_delivery_key(webhook_id, key).await
    }

    async fn list_unprocessed_deliveries(&self) -> moltis_webhooks::Result<Vec<i64>> {
        self.inner.list_unprocessed_deliveries().await
    }

    async fn insert_response_action(
        &self,
        action: &NewResponseAction,
    ) -> moltis_webhooks::Result<i64> {
        self.inner.insert_response_action(action).await
    }

    async fn list_response_actions(
        &self,
        delivery_id: i64,
    ) -> moltis_webhooks::Result<Vec<WebhookResponseAction>> {
        self.inner.list_response_actions(delivery_id).await
    }

    async fn prune_deliveries_before(&self, before: &str) -> moltis_webhooks::Result<u64> {
        self.inner.prune_deliveries_before(before).await
    }
}

/// Gateway-facing webhooks service backed by the real store.
pub struct LiveWebhooksService {
    store: Arc<dyn WebhookStore>,
    profiles: ProfileRegistry,
}

impl LiveWebhooksService {
    pub fn new(store: Arc<dyn WebhookStore>) -> Self {
        Self {
            store,
            profiles: ProfileRegistry::new(),
        }
    }

    pub fn store(&self) -> &Arc<dyn WebhookStore> {
        &self.store
    }
}

#[async_trait]
impl WebhooksServiceTrait for LiveWebhooksService {
    async fn list(&self) -> ServiceResult {
        let webhooks: Vec<_> = self
            .store
            .list_webhooks()
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks list failed");
                ServiceError::message(e)
            })?
            .into_iter()
            .map(|w| w.redacted())
            .collect();
        Ok(serde_json::to_value(webhooks)?)
    }

    async fn get(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'id'"))?;
        let webhook = self
            .store
            .get_webhook(id)
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks get failed");
                ServiceError::message(e)
            })?
            .redacted();
        Ok(serde_json::to_value(webhook)?)
    }

    async fn create(&self, params: Value) -> ServiceResult {
        let create: WebhookCreate = serde_json::from_value(params)
            .map_err(|e| ServiceError::message(format!("invalid webhook spec: {e}")))?;
        let webhook = self
            .store
            .create_webhook(create)
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks create failed");
                ServiceError::message(e)
            })?
            .redacted();
        Ok(serde_json::to_value(webhook)?)
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'id'"))?;
        let patch: WebhookPatch = serde_json::from_value(
            params
                .get("patch")
                .cloned()
                .unwrap_or(Value::Object(Default::default())),
        )
        .map_err(|e| ServiceError::message(format!("invalid patch: {e}")))?;
        let webhook = self
            .store
            .update_webhook(id, patch)
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks update failed");
                ServiceError::message(e)
            })?
            .redacted();
        Ok(serde_json::to_value(webhook)?)
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'id'"))?;
        self.store.delete_webhook(id).await.map_err(|e| {
            error!(error = %e, "webhooks delete failed");
            ServiceError::message(e)
        })?;
        Ok(serde_json::json!({ "deleted": id }))
    }

    async fn deliveries(&self, params: Value) -> ServiceResult {
        let webhook_id = params
            .get("webhookId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'webhookId'"))?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let deliveries = self
            .store
            .list_deliveries(webhook_id, limit, offset)
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks deliveries list failed");
                ServiceError::message(e)
            })?;
        Ok(serde_json::to_value(deliveries)?)
    }

    async fn delivery_get(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'id'"))?;
        let delivery = self.store.get_delivery(id).await.map_err(|e| {
            error!(error = %e, "webhooks delivery get failed");
            ServiceError::message(e)
        })?;
        Ok(serde_json::to_value(delivery)?)
    }

    async fn delivery_payload(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'id'"))?;
        let body = self.store.get_delivery_body(id).await.map_err(|e| {
            error!(error = %e, "webhooks delivery payload failed");
            ServiceError::message(e)
        })?;
        match body {
            Some(bytes) => {
                // Try to parse as JSON for pretty display
                if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
                    Ok(json)
                } else {
                    Ok(serde_json::json!({
                        "raw": base64::engine::general_purpose::STANDARD.encode(&bytes),
                        "size": bytes.len(),
                    }))
                }
            },
            None => Ok(serde_json::json!(null)),
        }
    }

    async fn delivery_actions(&self, params: Value) -> ServiceResult {
        let delivery_id = params
            .get("deliveryId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ServiceError::message("missing 'deliveryId'"))?;
        let actions = self
            .store
            .list_response_actions(delivery_id)
            .await
            .map_err(|e| {
                error!(error = %e, "webhooks delivery actions failed");
                ServiceError::message(e)
            })?;
        Ok(serde_json::to_value(actions)?)
    }

    async fn profiles(&self) -> ServiceResult {
        let profiles = self.profiles.list();
        Ok(serde_json::to_value(profiles)?)
    }
}

#[cfg(all(test, feature = "vault"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use sqlx::SqlitePool;

    use super::*;

    async fn test_vault(pool: SqlitePool) -> Arc<moltis_vault::Vault> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS vault_metadata (
                id                   INTEGER PRIMARY KEY CHECK (id = 1),
                version              INTEGER NOT NULL DEFAULT 1,
                kdf_salt             TEXT NOT NULL,
                kdf_params           TEXT NOT NULL,
                wrapped_dek          TEXT NOT NULL,
                recovery_wrapped_dek TEXT,
                recovery_key_hash    TEXT,
                created_at           TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let vault = Arc::new(moltis_vault::Vault::new(pool).await.unwrap());
        vault.initialize("test-password").await.unwrap();
        vault
    }

    async fn make_store() -> (
        Arc<dyn WebhookStore>,
        Arc<moltis_webhooks::store::SqliteWebhookStore>,
        Arc<moltis_vault::Vault>,
    ) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_webhooks::run_migrations(&pool).await.unwrap();
        let raw = Arc::new(moltis_webhooks::store::SqliteWebhookStore::with_pool(
            pool.clone(),
        ));
        let vault = test_vault(pool).await;
        let wrapped: Arc<dyn WebhookStore> =
            Arc::new(VaultWebhookStore::new(raw.clone(), Some(vault.clone())));
        (wrapped, raw, vault)
    }

    #[tokio::test]
    async fn vault_store_encrypts_webhook_secret_configs() {
        let (store, raw, vault) = make_store().await;

        let webhook = store
            .create_webhook(WebhookCreate {
                name: "hook".into(),
                description: None,
                agent_id: None,
                model: None,
                system_prompt_suffix: None,
                tool_policy: Some(moltis_webhooks::types::ToolPolicy {
                    allow: vec!["web_fetch".into()],
                    deny: vec!["exec".into()],
                }),
                auth_mode: AuthMode::GithubHmacSha256,
                auth_config: Some(serde_json::json!({ "secret": "super-secret-value" })),
                source_profile: "github".into(),
                source_config: Some(serde_json::json!({ "api_token": "ghp_secret" })),
                event_filter: Default::default(),
                session_mode: moltis_webhooks::types::SessionMode::PerDelivery,
                named_session_key: None,
                allowed_cidrs: Vec::new(),
                max_body_bytes: 1024,
                rate_limit_per_minute: 60,
            })
            .await
            .unwrap();

        let raw_webhook = raw.get_webhook(webhook.id).await.unwrap();
        assert_eq!(raw_webhook.tool_policy.clone().unwrap().deny, vec!["exec"]);
        assert_ne!(
            raw_webhook.auth_config,
            Some(serde_json::json!({ "secret": "super-secret-value" }))
        );
        assert_ne!(
            raw_webhook.source_config,
            Some(serde_json::json!({ "api_token": "ghp_secret" }))
        );

        let mut runtime_webhook = raw_webhook.clone();
        decrypt_webhook_secrets(&mut runtime_webhook, Some(&vault))
            .await
            .unwrap();
        assert_eq!(
            runtime_webhook.auth_config,
            Some(serde_json::json!({ "secret": "super-secret-value" }))
        );
        assert_eq!(
            runtime_webhook.source_config,
            Some(serde_json::json!({ "api_token": "ghp_secret" }))
        );
    }

    #[tokio::test]
    async fn sealed_vault_rejects_plaintext_secret_updates() {
        let (store, _raw, vault) = make_store().await;
        let webhook = store
            .create_webhook(WebhookCreate {
                name: "hook".into(),
                description: None,
                agent_id: None,
                model: None,
                system_prompt_suffix: None,
                tool_policy: None,
                auth_mode: AuthMode::Bearer,
                auth_config: Some(serde_json::json!({ "token": "initial" })),
                source_profile: "generic".into(),
                source_config: None,
                event_filter: Default::default(),
                session_mode: moltis_webhooks::types::SessionMode::PerDelivery,
                named_session_key: None,
                allowed_cidrs: Vec::new(),
                max_body_bytes: 1024,
                rate_limit_per_minute: 60,
            })
            .await
            .unwrap();

        vault.seal().await;

        let error = store
            .update_webhook(webhook.id, WebhookPatch {
                auth_config: Some(Some(serde_json::json!({ "token": "rotated" }))),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("vault is sealed; webhook secrets cannot be persisted")
        );
    }
}
