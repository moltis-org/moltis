use {async_trait::async_trait, sqlx::SqlitePool};

use moltis_channels::{
    Error as ChannelError, Result as ChannelResult,
    store::{ChannelStore, StoredChannel},
};

fn channel_db_error(context: &'static str, source: sqlx::Error) -> ChannelError {
    ChannelError::external(context, source)
}

/// Internal row type for sqlx mapping.
#[derive(sqlx::FromRow)]
struct ChannelRow {
    account_id: String,
    channel_type: String,
    config: String,
    created_at: i64,
    updated_at: i64,
}

impl TryFrom<ChannelRow> for StoredChannel {
    type Error = ChannelError;

    fn try_from(r: ChannelRow) -> ChannelResult<Self> {
        Ok(Self {
            account_id: r.account_id,
            channel_type: r.channel_type,
            config: serde_json::from_str(&r.config)?,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

/// SQLite-backed channel store.
pub struct SqliteChannelStore {
    pool: SqlitePool,
}

impl SqliteChannelStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Initialize the channels table schema.
    ///
    /// **Deprecated**: Schema is now managed by sqlx migrations.
    /// This method is retained for tests that use in-memory databases.
    #[doc(hidden)]
    pub async fn init(pool: &SqlitePool) -> ChannelResult<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS channels (
                channel_type TEXT    NOT NULL DEFAULT 'telegram',
                account_id   TEXT    NOT NULL,
                config       TEXT    NOT NULL,
                created_at   INTEGER NOT NULL,
                updated_at   INTEGER NOT NULL,
                PRIMARY KEY (channel_type, account_id)
            )"#,
        )
        .execute(pool)
        .await
        .map_err(|e| channel_db_error("init channels table", e))?;
        Ok(())
    }
}

#[async_trait]
impl ChannelStore for SqliteChannelStore {
    async fn list(&self) -> ChannelResult<Vec<StoredChannel>> {
        let rows =
            sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels ORDER BY updated_at DESC")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| channel_db_error("list channels", e))?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn get(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> ChannelResult<Option<StoredChannel>> {
        let row = sqlx::query_as::<_, ChannelRow>(
            "SELECT * FROM channels WHERE channel_type = ? AND account_id = ?",
        )
        .bind(channel_type)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| channel_db_error("get channel", e))?;
        row.map(TryInto::try_into).transpose()
    }

    async fn upsert(&self, channel: StoredChannel) -> ChannelResult<()> {
        let config_json = serde_json::to_string(&channel.config)?;
        sqlx::query(
            r#"INSERT INTO channels (account_id, channel_type, config, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(channel_type, account_id) DO UPDATE SET
                 channel_type = excluded.channel_type,
                 config = excluded.config,
                 updated_at = excluded.updated_at"#,
        )
        .bind(&channel.account_id)
        .bind(&channel.channel_type)
        .bind(&config_json)
        .bind(channel.created_at)
        .bind(channel.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| channel_db_error("upsert channel", e))?;
        Ok(())
    }

    async fn delete(&self, channel_type: &str, account_id: &str) -> ChannelResult<()> {
        sqlx::query("DELETE FROM channels WHERE channel_type = ? AND account_id = ?")
            .bind(channel_type)
            .bind(account_id)
            .execute(&self.pool)
            .await
            .map_err(|e| channel_db_error("delete channel", e))?;
        Ok(())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        SqliteChannelStore::init(&pool).await.unwrap();
        pool
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[tokio::test]
    async fn test_upsert_and_get() {
        let pool = test_pool().await;
        let store = SqliteChannelStore::new(pool);

        let ch = StoredChannel {
            account_id: "bot1".into(),
            channel_type: "telegram".into(),
            config: serde_json::json!({"token": "abc"}),
            created_at: now(),
            updated_at: now(),
        };
        store.upsert(ch).await.unwrap();

        let got = store.get("telegram", "bot1").await.unwrap().unwrap();
        assert_eq!(got.account_id, "bot1");
        assert_eq!(got.config["token"], "abc");
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let pool = test_pool().await;
        let store = SqliteChannelStore::new(pool);
        let t = now();

        store
            .upsert(StoredChannel {
                account_id: "bot1".into(),
                channel_type: "telegram".into(),
                config: serde_json::json!({"token": "old"}),
                created_at: t,
                updated_at: t,
            })
            .await
            .unwrap();

        store
            .upsert(StoredChannel {
                account_id: "bot1".into(),
                channel_type: "telegram".into(),
                config: serde_json::json!({"token": "new"}),
                created_at: t,
                updated_at: t + 1,
            })
            .await
            .unwrap();

        let got = store.get("telegram", "bot1").await.unwrap().unwrap();
        assert_eq!(got.config["token"], "new");

        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = test_pool().await;
        let store = SqliteChannelStore::new(pool);

        store
            .upsert(StoredChannel {
                account_id: "bot1".into(),
                channel_type: "telegram".into(),
                config: serde_json::json!({}),
                created_at: now(),
                updated_at: now(),
            })
            .await
            .unwrap();

        store.delete("telegram", "bot1").await.unwrap();
        assert!(store.get("telegram", "bot1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_list_order() {
        let pool = test_pool().await;
        let store = SqliteChannelStore::new(pool);

        store
            .upsert(StoredChannel {
                account_id: "old".into(),
                channel_type: "telegram".into(),
                config: serde_json::json!({}),
                created_at: 100,
                updated_at: 100,
            })
            .await
            .unwrap();

        store
            .upsert(StoredChannel {
                account_id: "new".into(),
                channel_type: "telegram".into(),
                config: serde_json::json!({}),
                created_at: 200,
                updated_at: 200,
            })
            .await
            .unwrap();

        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].account_id, "new");
        assert_eq!(all[1].account_id, "old");
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let pool = test_pool().await;
        let store = SqliteChannelStore::new(pool);
        assert!(store.get("telegram", "nope").await.unwrap().is_none());
    }
}
