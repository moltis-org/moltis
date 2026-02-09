use {
    async_trait::async_trait,
    moltis_channels::message_log::{MessageLog, MessageLogEntry, SenderSummary},
    sqlx::SqlitePool,
};

/// SQLite-backed message log.
pub struct SqliteMessageLog {
    pool: SqlitePool,
}

impl SqliteMessageLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Initialize the message_log table schema.
    ///
    /// **Deprecated**: Schema is now managed by sqlx migrations.
    /// This method is retained for tests that use in-memory databases.
    #[doc(hidden)]
    pub async fn init(pool: &SqlitePool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS message_log (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id     TEXT    NOT NULL,
                channel_type   TEXT    NOT NULL,
                peer_id        TEXT    NOT NULL,
                username       TEXT,
                sender_name    TEXT,
                chat_id        TEXT    NOT NULL,
                chat_type      TEXT    NOT NULL,
                body           TEXT    NOT NULL,
                access_granted INTEGER NOT NULL DEFAULT 0,
                created_at     INTEGER NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_message_log_account_created
             ON message_log (account_id, created_at DESC)",
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[async_trait]
impl MessageLog for SqliteMessageLog {
    async fn log(&self, entry: MessageLogEntry) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO message_log
             (account_id, channel_type, peer_id, username, sender_name,
              chat_id, chat_type, body, access_granted, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&entry.account_id)
        .bind(&entry.channel_type)
        .bind(&entry.peer_id)
        .bind(&entry.username)
        .bind(&entry.sender_name)
        .bind(&entry.chat_id)
        .bind(&entry.chat_type)
        .bind(&entry.body)
        .bind(entry.access_granted)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_by_account(
        &self,
        account_id: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<MessageLogEntry>> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                String,
                String,
                String,
                bool,
                i64,
            ),
        >(
            "SELECT id, account_id, channel_type, peer_id, username, sender_name,
                    chat_id, chat_type, body, access_granted, created_at
             FROM message_log
             WHERE account_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(account_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| MessageLogEntry {
                id: r.0,
                account_id: r.1,
                channel_type: r.2,
                peer_id: r.3,
                username: r.4,
                sender_name: r.5,
                chat_id: r.6,
                chat_type: r.7,
                body: r.8,
                access_granted: r.9,
                created_at: r.10,
            })
            .collect())
    }

    async fn unique_senders(&self, account_id: &str) -> anyhow::Result<Vec<SenderSummary>> {
        let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64, i64, bool)>(
            "SELECT peer_id, username, sender_name,
                    COUNT(*) as message_count,
                    MAX(created_at) as last_seen,
                    MAX(CASE WHEN access_granted THEN 1 ELSE 0 END) as last_access_granted
             FROM message_log
             WHERE account_id = ?
             GROUP BY peer_id
             ORDER BY last_seen DESC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SenderSummary {
                peer_id: r.0,
                username: r.1,
                sender_name: r.2,
                message_count: r.3,
                last_seen: r.4,
                last_access_granted: r.5,
            })
            .collect())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        SqliteMessageLog::init(&pool).await.unwrap();
        pool
    }

    fn sample_entry(account_id: &str, peer_id: &str, granted: bool) -> MessageLogEntry {
        MessageLogEntry {
            id: 0,
            account_id: account_id.into(),
            channel_type: "telegram".into(),
            peer_id: peer_id.into(),
            username: Some("testuser".into()),
            sender_name: Some("Test User".into()),
            chat_id: "12345".into(),
            chat_type: "dm".into(),
            body: "hello".into(),
            access_granted: granted,
            created_at: 1700000000,
        }
    }

    #[tokio::test]
    async fn log_and_list() {
        let pool = test_pool().await;
        let store = SqliteMessageLog::new(pool);

        store
            .log(sample_entry("bot1", "user1", true))
            .await
            .unwrap();
        store
            .log(sample_entry("bot1", "user2", false))
            .await
            .unwrap();
        store
            .log(sample_entry("bot2", "user3", true))
            .await
            .unwrap();

        let entries = store.list_by_account("bot1", 10).await.unwrap();
        assert_eq!(entries.len(), 2);

        let entries = store.list_by_account("bot2", 10).await.unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let pool = test_pool().await;
        let store = SqliteMessageLog::new(pool);

        for i in 0..5 {
            let mut e = sample_entry("bot1", "user1", true);
            e.created_at = 1700000000 + i;
            store.log(e).await.unwrap();
        }

        let entries = store.list_by_account("bot1", 3).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Most recent first
        assert!(entries[0].created_at > entries[1].created_at);
    }

    #[tokio::test]
    async fn unique_senders_groups() {
        let pool = test_pool().await;
        let store = SqliteMessageLog::new(pool);

        store
            .log(sample_entry("bot1", "user1", true))
            .await
            .unwrap();
        store
            .log(sample_entry("bot1", "user1", false))
            .await
            .unwrap();
        store
            .log(sample_entry("bot1", "user2", true))
            .await
            .unwrap();

        let senders = store.unique_senders("bot1").await.unwrap();
        assert_eq!(senders.len(), 2);
        let user1 = senders.iter().find(|s| s.peer_id == "user1").unwrap();
        assert_eq!(user1.message_count, 2);
    }
}
