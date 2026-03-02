use async_trait::async_trait;

use crate::Result;

/// A single logged inbound message.
#[derive(Debug, Clone)]
pub struct MessageLogEntry {
    pub id: i64,
    pub account_id: String,
    pub channel_type: String,
    pub peer_id: String,
    pub username: Option<String>,
    pub sender_name: Option<String>,
    pub chat_id: String,
    pub chat_type: String,
    pub body: String,
    pub access_granted: bool,
    pub created_at: i64,
}

/// Summary of a unique sender across logged messages.
#[derive(Debug, Clone)]
pub struct SenderSummary {
    pub peer_id: String,
    pub username: Option<String>,
    pub sender_name: Option<String>,
    pub message_count: i64,
    pub last_seen: i64,
    pub last_access_granted: bool,
}

/// Persistent log of every inbound message for forensics.
#[async_trait]
pub trait MessageLog: Send + Sync {
    async fn log(&self, entry: MessageLogEntry) -> Result<()>;
    async fn list_by_account(
        &self,
        channel_type: &str,
        account_id: &str,
        limit: u32,
    ) -> Result<Vec<MessageLogEntry>>;
    async fn unique_senders(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> Result<Vec<SenderSummary>>;
}
