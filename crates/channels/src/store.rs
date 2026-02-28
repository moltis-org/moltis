use {async_trait::async_trait, serde::Serialize};

use crate::Result;

/// A persisted channel configuration.
#[derive(Debug, Clone, Serialize)]
pub struct StoredChannel {
    pub account_id: String,
    pub channel_type: String,
    pub config: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Persistent storage for channel configurations.
#[async_trait]
pub trait ChannelStore: Send + Sync {
    async fn list(&self) -> Result<Vec<StoredChannel>>;
    async fn get(&self, channel_type: &str, account_id: &str) -> Result<Option<StoredChannel>>;
    async fn upsert(&self, channel: StoredChannel) -> Result<()>;
    async fn delete(&self, channel_type: &str, account_id: &str) -> Result<()>;
}
