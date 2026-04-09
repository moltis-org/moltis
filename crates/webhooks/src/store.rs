//! Persistence for webhooks, deliveries, and response actions.

use {
    async_trait::async_trait,
    sqlx::{Row, SqlitePool},
    time::OffsetDateTime,
};

use crate::{
    Error, Result,
    types::{
        AuthMode, DeliveryStatus, EventFilter, SessionMode, ToolPolicy, Webhook, WebhookCreate,
        WebhookDelivery, WebhookPatch, WebhookResponseAction, generate_public_id,
    },
};

/// Persistence trait for webhook data.
#[async_trait]
pub trait WebhookStore: Send + Sync {
    async fn list_webhooks(&self) -> Result<Vec<Webhook>>;
    async fn get_webhook(&self, id: i64) -> Result<Webhook>;
    async fn get_webhook_by_public_id(&self, public_id: &str) -> Result<Webhook>;
    async fn create_webhook(&self, create: WebhookCreate) -> Result<Webhook>;
    async fn update_webhook(&self, id: i64, patch: WebhookPatch) -> Result<Webhook>;
    async fn delete_webhook(&self, id: i64) -> Result<()>;
    async fn increment_delivery_count(&self, id: i64, received_at: &str) -> Result<()>;

    async fn insert_delivery(&self, delivery: &NewDelivery) -> Result<i64>;
    async fn update_delivery_status(
        &self,
        id: i64,
        status: DeliveryStatus,
        update: DeliveryUpdate,
    ) -> Result<()>;
    async fn get_delivery(&self, id: i64) -> Result<WebhookDelivery>;
    async fn list_deliveries(
        &self,
        webhook_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WebhookDelivery>>;
    async fn get_delivery_body(&self, delivery_id: i64) -> Result<Option<Vec<u8>>>;
    async fn find_by_delivery_key(&self, webhook_id: i64, key: &str) -> Result<Option<i64>>;
    async fn list_unprocessed_deliveries(&self) -> Result<Vec<i64>>;

    async fn insert_response_action(&self, action: &NewResponseAction) -> Result<i64>;
    async fn list_response_actions(&self, delivery_id: i64) -> Result<Vec<WebhookResponseAction>>;

    async fn prune_deliveries_before(&self, before: &str) -> Result<u64>;
}

/// Input for inserting a new delivery.
pub struct NewDelivery {
    pub webhook_id: i64,
    pub received_at: String,
    pub status: DeliveryStatus,
    pub event_type: Option<String>,
    pub entity_key: Option<String>,
    pub delivery_key: Option<String>,
    pub http_method: Option<String>,
    pub content_type: Option<String>,
    pub remote_ip: Option<String>,
    pub headers_json: Option<String>,
    pub body_size: usize,
    pub body_blob: Option<Vec<u8>>,
    pub rejection_reason: Option<String>,
}

/// Fields to update on a delivery.
#[derive(Default)]
pub struct DeliveryUpdate {
    pub session_key: Option<String>,
    pub run_error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

/// Input for inserting a response action.
pub struct NewResponseAction {
    pub delivery_id: i64,
    pub tool_name: String,
    pub input_json: Option<String>,
    pub output_json: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
}

/// SQLite-backed webhook store.
pub struct SqliteWebhookStore {
    pool: SqlitePool,
}

impl SqliteWebhookStore {
    /// Create a store using an existing pool (migrations must already be run).
    pub fn with_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn row_to_webhook(row: &sqlx::sqlite::SqliteRow) -> Result<Webhook> {
    let auth_mode_str: String = row.get("auth_mode");
    let auth_mode: AuthMode =
        serde_json::from_value(serde_json::Value::String(auth_mode_str)).unwrap_or_default();
    let session_mode_str: String = row.get("session_mode");
    let session_mode: SessionMode =
        serde_json::from_value(serde_json::Value::String(session_mode_str)).unwrap_or_default();
    let tool_policy: Option<ToolPolicy> = row
        .get::<Option<String>, _>("tool_policy_json")
        .and_then(|s| serde_json::from_str(&s).ok());
    let event_filter: EventFilter = row
        .get::<Option<String>, _>("event_filter_json")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let allowed_cidrs: Vec<String> = row
        .get::<Option<String>, _>("allowed_cidrs_json")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let auth_config: Option<serde_json::Value> = row
        .get::<Option<String>, _>("auth_config_json")
        .and_then(|s| serde_json::from_str(&s).ok());
    let source_config: Option<serde_json::Value> = row
        .get::<Option<String>, _>("source_config_json")
        .and_then(|s| serde_json::from_str(&s).ok());

    Ok(Webhook {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        enabled: row.get::<i32, _>("enabled") != 0,
        public_id: row.get("public_id"),
        agent_id: row.get("agent_id"),
        model: row.get("model"),
        system_prompt_suffix: row.get("system_prompt_suffix"),
        tool_policy,
        auth_mode,
        auth_config,
        source_profile: row.get("source_profile"),
        source_config,
        event_filter,
        session_mode,
        named_session_key: row.get("named_session_key"),
        allowed_cidrs,
        max_body_bytes: row.get::<i64, _>("max_body_bytes") as usize,
        rate_limit_per_minute: row.get::<i64, _>("rate_limit_per_minute") as u32,
        delivery_count: row.get::<i64, _>("delivery_count") as u64,
        last_delivery_at: row.get("last_delivery_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_delivery(row: &sqlx::sqlite::SqliteRow) -> Result<WebhookDelivery> {
    let status_str: String = row.get("status");
    let status: DeliveryStatus = serde_json::from_value(serde_json::Value::String(status_str))
        .unwrap_or(DeliveryStatus::Received);

    Ok(WebhookDelivery {
        id: row.get("id"),
        webhook_id: row.get("webhook_id"),
        received_at: row.get("received_at"),
        status,
        event_type: row.get("event_type"),
        entity_key: row.get("entity_key"),
        delivery_key: row.get("delivery_key"),
        http_method: row.get("http_method"),
        content_type: row.get("content_type"),
        remote_ip: row.get("remote_ip"),
        headers_json: row.get("headers_json"),
        body_size: row.get::<i64, _>("body_size") as usize,
        session_key: row.get("session_key"),
        rejection_reason: row.get("rejection_reason"),
        run_error: row.get("run_error"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        duration_ms: row.try_get("duration_ms").ok().flatten(),
        input_tokens: row.try_get("input_tokens").ok().flatten(),
        output_tokens: row.try_get("output_tokens").ok().flatten(),
    })
}

#[async_trait]
impl WebhookStore for SqliteWebhookStore {
    async fn list_webhooks(&self) -> Result<Vec<Webhook>> {
        let rows = sqlx::query("SELECT * FROM webhooks ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_webhook).collect()
    }

    async fn get_webhook(&self, id: i64) -> Result<Webhook> {
        let row = sqlx::query("SELECT * FROM webhooks WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::webhook_not_found(id.to_string()))?;
        row_to_webhook(&row)
    }

    async fn get_webhook_by_public_id(&self, public_id: &str) -> Result<Webhook> {
        let row = sqlx::query("SELECT * FROM webhooks WHERE public_id = ?")
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::webhook_not_found(public_id))?;
        row_to_webhook(&row)
    }

    async fn create_webhook(&self, create: WebhookCreate) -> Result<Webhook> {
        let public_id = generate_public_id();
        let now = now_iso();
        let auth_mode_str = serde_json::to_value(&create.auth_mode)?
            .as_str()
            .unwrap_or("static_header")
            .to_string();
        let session_mode_str = serde_json::to_value(&create.session_mode)?
            .as_str()
            .unwrap_or("per_delivery")
            .to_string();
        let tool_policy_json = create
            .tool_policy
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let event_filter_json = serde_json::to_string(&create.event_filter)?;
        let auth_config_json = create
            .auth_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let source_config_json = create
            .source_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let allowed_cidrs_json = serde_json::to_string(&create.allowed_cidrs)?;

        let result = sqlx::query(
            "INSERT INTO webhooks (name, description, public_id, agent_id, model, system_prompt_suffix, \
             tool_policy_json, auth_mode, auth_config_json, source_profile, source_config_json, \
             event_filter_json, session_mode, named_session_key, allowed_cidrs_json, \
             max_body_bytes, rate_limit_per_minute, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.name)
        .bind(&create.description)
        .bind(&public_id)
        .bind(&create.agent_id)
        .bind(&create.model)
        .bind(&create.system_prompt_suffix)
        .bind(&tool_policy_json)
        .bind(&auth_mode_str)
        .bind(&auth_config_json)
        .bind(&create.source_profile)
        .bind(&source_config_json)
        .bind(&event_filter_json)
        .bind(&session_mode_str)
        .bind(&create.named_session_key)
        .bind(&allowed_cidrs_json)
        .bind(create.max_body_bytes as i64)
        .bind(create.rate_limit_per_minute as i64)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_webhook(result.last_insert_rowid()).await
    }

    async fn update_webhook(&self, id: i64, patch: WebhookPatch) -> Result<Webhook> {
        let mut webhook = self.get_webhook(id).await?;
        let now = now_iso();

        if let Some(name) = patch.name {
            webhook.name = name;
        }
        if let Some(desc) = patch.description {
            webhook.description = desc;
        }
        if let Some(enabled) = patch.enabled {
            webhook.enabled = enabled;
        }
        if let Some(agent_id) = patch.agent_id {
            webhook.agent_id = agent_id;
        }
        if let Some(model) = patch.model {
            webhook.model = model;
        }
        if let Some(suffix) = patch.system_prompt_suffix {
            webhook.system_prompt_suffix = suffix;
        }
        if let Some(tp) = patch.tool_policy {
            webhook.tool_policy = tp;
        }
        if let Some(am) = patch.auth_mode {
            webhook.auth_mode = am;
        }
        if let Some(ac) = patch.auth_config {
            webhook.auth_config = ac;
        }
        if let Some(sc) = patch.source_config {
            webhook.source_config = sc;
        }
        if let Some(ef) = patch.event_filter {
            webhook.event_filter = ef;
        }
        if let Some(sm) = patch.session_mode {
            webhook.session_mode = sm;
        }
        if let Some(nsk) = patch.named_session_key {
            webhook.named_session_key = nsk;
        }
        if let Some(cidrs) = patch.allowed_cidrs {
            webhook.allowed_cidrs = cidrs;
        }
        if let Some(max) = patch.max_body_bytes {
            webhook.max_body_bytes = max;
        }
        if let Some(rl) = patch.rate_limit_per_minute {
            webhook.rate_limit_per_minute = rl;
        }

        let auth_mode_str = serde_json::to_value(&webhook.auth_mode)?
            .as_str()
            .unwrap_or("static_header")
            .to_string();
        let session_mode_str = serde_json::to_value(&webhook.session_mode)?
            .as_str()
            .unwrap_or("per_delivery")
            .to_string();
        let tool_policy_json = webhook
            .tool_policy
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let event_filter_json = serde_json::to_string(&webhook.event_filter)?;
        let auth_config_json = webhook
            .auth_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let source_config_json = webhook
            .source_config
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let allowed_cidrs_json = serde_json::to_string(&webhook.allowed_cidrs)?;

        sqlx::query(
            "UPDATE webhooks SET name = ?, description = ?, enabled = ?, agent_id = ?, model = ?, \
             system_prompt_suffix = ?, tool_policy_json = ?, auth_mode = ?, auth_config_json = ?, \
             source_config_json = ?, event_filter_json = ?, session_mode = ?, named_session_key = ?, \
             allowed_cidrs_json = ?, max_body_bytes = ?, rate_limit_per_minute = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&webhook.name)
        .bind(&webhook.description)
        .bind(webhook.enabled as i32)
        .bind(&webhook.agent_id)
        .bind(&webhook.model)
        .bind(&webhook.system_prompt_suffix)
        .bind(&tool_policy_json)
        .bind(&auth_mode_str)
        .bind(&auth_config_json)
        .bind(&source_config_json)
        .bind(&event_filter_json)
        .bind(&session_mode_str)
        .bind(&webhook.named_session_key)
        .bind(&allowed_cidrs_json)
        .bind(webhook.max_body_bytes as i64)
        .bind(webhook.rate_limit_per_minute as i64)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        webhook.updated_at = now;
        Ok(webhook)
    }

    async fn delete_webhook(&self, id: i64) -> Result<()> {
        // Transaction prevents partial data loss if a later DELETE fails.
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM webhook_response_actions WHERE delivery_id IN \
             (SELECT id FROM webhook_deliveries WHERE webhook_id = ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM webhook_deliveries WHERE webhook_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query("DELETE FROM webhooks WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Err(Error::webhook_not_found(id.to_string()));
        }
        tx.commit().await?;
        Ok(())
    }

    async fn increment_delivery_count(&self, id: i64, received_at: &str) -> Result<()> {
        sqlx::query(
            "UPDATE webhooks SET delivery_count = delivery_count + 1, last_delivery_at = ? WHERE id = ?",
        )
        .bind(received_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_delivery(&self, d: &NewDelivery) -> Result<i64> {
        let status_str = serde_json::to_value(&d.status)?
            .as_str()
            .unwrap_or("received")
            .to_string();
        let result = sqlx::query(
            "INSERT INTO webhook_deliveries (webhook_id, received_at, status, event_type, entity_key, \
             delivery_key, http_method, content_type, remote_ip, headers_json, body_size, body_blob, \
             rejection_reason) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(d.webhook_id)
        .bind(&d.received_at)
        .bind(&status_str)
        .bind(&d.event_type)
        .bind(&d.entity_key)
        .bind(&d.delivery_key)
        .bind(&d.http_method)
        .bind(&d.content_type)
        .bind(&d.remote_ip)
        .bind(&d.headers_json)
        .bind(d.body_size as i64)
        .bind(&d.body_blob)
        .bind(&d.rejection_reason)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn update_delivery_status(
        &self,
        id: i64,
        status: DeliveryStatus,
        update: DeliveryUpdate,
    ) -> Result<()> {
        let status_str = serde_json::to_value(&status)?
            .as_str()
            .unwrap_or("received")
            .to_string();
        sqlx::query(
            "UPDATE webhook_deliveries SET status = ?, session_key = COALESCE(?, session_key), \
             run_error = COALESCE(?, run_error), started_at = COALESCE(?, started_at), \
             finished_at = COALESCE(?, finished_at), duration_ms = COALESCE(?, duration_ms), \
             input_tokens = COALESCE(?, input_tokens), output_tokens = COALESCE(?, output_tokens) \
             WHERE id = ?",
        )
        .bind(&status_str)
        .bind(&update.session_key)
        .bind(&update.run_error)
        .bind(&update.started_at)
        .bind(&update.finished_at)
        .bind(update.duration_ms)
        .bind(update.input_tokens)
        .bind(update.output_tokens)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_delivery(&self, id: i64) -> Result<WebhookDelivery> {
        let row = sqlx::query("SELECT * FROM webhook_deliveries WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::DeliveryNotFound { delivery_id: id })?;
        row_to_delivery(&row)
    }

    async fn list_deliveries(
        &self,
        webhook_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WebhookDelivery>> {
        let rows = sqlx::query(
            "SELECT * FROM webhook_deliveries WHERE webhook_id = ? ORDER BY received_at DESC LIMIT ? OFFSET ?",
        )
        .bind(webhook_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_delivery).collect()
    }

    async fn get_delivery_body(&self, delivery_id: i64) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query("SELECT body_blob FROM webhook_deliveries WHERE id = ?")
            .bind(delivery_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| r.get("body_blob")))
    }

    async fn find_by_delivery_key(&self, webhook_id: i64, key: &str) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT id FROM webhook_deliveries WHERE webhook_id = ? AND delivery_key = ?",
        )
        .bind(webhook_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get("id")))
    }

    async fn list_unprocessed_deliveries(&self) -> Result<Vec<i64>> {
        let rows = sqlx::query(
            "SELECT id FROM webhook_deliveries WHERE status IN ('received', 'queued', 'processing') ORDER BY received_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn insert_response_action(&self, a: &NewResponseAction) -> Result<i64> {
        let now = now_iso();
        let result = sqlx::query(
            "INSERT INTO webhook_response_actions (delivery_id, tool_name, input_json, output_json, status, error_message, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(a.delivery_id)
        .bind(&a.tool_name)
        .bind(&a.input_json)
        .bind(&a.output_json)
        .bind(&a.status)
        .bind(&a.error_message)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn list_response_actions(&self, delivery_id: i64) -> Result<Vec<WebhookResponseAction>> {
        let rows = sqlx::query(
            "SELECT * FROM webhook_response_actions WHERE delivery_id = ? ORDER BY created_at ASC",
        )
        .bind(delivery_id)
        .fetch_all(&self.pool)
        .await?;
        let mut actions = Vec::with_capacity(rows.len());
        for row in &rows {
            actions.push(WebhookResponseAction {
                id: row.get("id"),
                delivery_id: row.get("delivery_id"),
                tool_name: row.get("tool_name"),
                input_json: row.get("input_json"),
                output_json: row.get("output_json"),
                status: row.get("status"),
                error_message: row.get("error_message"),
                created_at: row.get("created_at"),
            });
        }
        Ok(actions)
    }

    async fn prune_deliveries_before(&self, before: &str) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM webhook_response_actions WHERE delivery_id IN \
             (SELECT id FROM webhook_deliveries WHERE received_at < ?)",
        )
        .bind(before)
        .execute(&mut *tx)
        .await?;
        let result = sqlx::query("DELETE FROM webhook_deliveries WHERE received_at < ?")
            .bind(before)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::types::*};

    async fn make_store() -> SqliteWebhookStore {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::run_migrations(&pool).await.unwrap();
        SqliteWebhookStore::with_pool(pool)
    }

    fn make_create(name: &str) -> WebhookCreate {
        WebhookCreate {
            name: name.into(),
            description: None,
            agent_id: None,
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::StaticHeader,
            auth_config: Some(serde_json::json!({ "header": "X-Secret", "value": "test123" })),
            source_profile: "generic".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode: SessionMode::PerDelivery,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        }
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let store = make_store().await;
        store.create_webhook(make_create("hook1")).await.unwrap();
        store.create_webhook(make_create("hook2")).await.unwrap();
        let webhooks = store.list_webhooks().await.unwrap();
        assert_eq!(webhooks.len(), 2);
    }

    #[tokio::test]
    async fn test_get_by_public_id() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let fetched = store.get_webhook_by_public_id(&wh.public_id).await.unwrap();
        assert_eq!(fetched.id, wh.id);
        assert_eq!(fetched.name, "hook1");
    }

    #[tokio::test]
    async fn test_update() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let updated = store
            .update_webhook(wh.id, WebhookPatch {
                name: Some("renamed".into()),
                enabled: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert!(!updated.enabled);
    }

    #[tokio::test]
    async fn test_delete() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        store.delete_webhook(wh.id).await.unwrap();
        assert!(store.get_webhook(wh.id).await.is_err());
    }

    #[tokio::test]
    async fn test_delivery_roundtrip() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let delivery_id = store
            .insert_delivery(&NewDelivery {
                webhook_id: wh.id,
                received_at: "2026-04-07T00:00:00Z".into(),
                status: DeliveryStatus::Received,
                event_type: Some("push".into()),
                entity_key: None,
                delivery_key: Some("abc-123".into()),
                http_method: Some("POST".into()),
                content_type: Some("application/json".into()),
                remote_ip: Some("1.2.3.4".into()),
                headers_json: None,
                body_size: 100,
                body_blob: Some(b"{\"test\":true}".to_vec()),
                rejection_reason: None,
            })
            .await
            .unwrap();

        let delivery = store.get_delivery(delivery_id).await.unwrap();
        assert_eq!(delivery.event_type.as_deref(), Some("push"));
        assert_eq!(delivery.status, DeliveryStatus::Received);

        // Get body
        let body = store.get_delivery_body(delivery_id).await.unwrap();
        assert!(body.is_some());

        // Dedup check
        let dup = store.find_by_delivery_key(wh.id, "abc-123").await.unwrap();
        assert_eq!(dup, Some(delivery_id));
    }

    #[tokio::test]
    async fn test_delivery_status_update() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let delivery_id = store
            .insert_delivery(&NewDelivery {
                webhook_id: wh.id,
                received_at: "2026-04-07T00:00:00Z".into(),
                status: DeliveryStatus::Received,
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

        store
            .update_delivery_status(delivery_id, DeliveryStatus::Completed, DeliveryUpdate {
                session_key: Some("webhook:test:1".into()),
                duration_ms: Some(1500),
                input_tokens: Some(100),
                output_tokens: Some(200),
                ..Default::default()
            })
            .await
            .unwrap();

        let delivery = store.get_delivery(delivery_id).await.unwrap();
        assert_eq!(delivery.status, DeliveryStatus::Completed);
        assert_eq!(delivery.session_key.as_deref(), Some("webhook:test:1"));
    }

    #[tokio::test]
    async fn test_response_actions() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let delivery_id = store
            .insert_delivery(&NewDelivery {
                webhook_id: wh.id,
                received_at: "2026-04-07T00:00:00Z".into(),
                status: DeliveryStatus::Completed,
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

        store
            .insert_response_action(&NewResponseAction {
                delivery_id,
                tool_name: "github_post_comment".into(),
                input_json: Some("{\"body\":\"LGTM\"}".into()),
                output_json: Some("{\"id\":42}".into()),
                status: "success".into(),
                error_message: None,
            })
            .await
            .unwrap();

        let actions = store.list_response_actions(delivery_id).await.unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool_name, "github_post_comment");
    }

    #[tokio::test]
    async fn test_cascade_delete() {
        let store = make_store().await;
        let wh = store.create_webhook(make_create("hook1")).await.unwrap();
        let delivery_id = store
            .insert_delivery(&NewDelivery {
                webhook_id: wh.id,
                received_at: "2026-04-07T00:00:00Z".into(),
                status: DeliveryStatus::Received,
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
        store
            .insert_response_action(&NewResponseAction {
                delivery_id,
                tool_name: "test".into(),
                input_json: None,
                output_json: None,
                status: "success".into(),
                error_message: None,
            })
            .await
            .unwrap();

        // Deleting webhook should cascade to deliveries and actions
        store.delete_webhook(wh.id).await.unwrap();
        assert!(store.get_delivery(delivery_id).await.is_err());
    }
}
