//! SQLite-backed reply/trace correlation.

use {
    async_trait::async_trait,
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        trace_link::{TraceLink, TraceLinkStore},
    },
    sqlx::SqlitePool,
};

fn db_error(context: &'static str, source: sqlx::Error) -> ChannelError {
    ChannelError::external(context, source)
}

/// SQLite-backed [`TraceLinkStore`].
pub struct SqliteTraceLinkStore {
    pool: SqlitePool,
}

impl SqliteTraceLinkStore {
    /// Build a store over `pool`.
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create the schema directly.
    ///
    /// **Deprecated**: schema is managed by sqlx migrations. Retained for tests
    /// on in-memory databases, matching the other channel stores.
    #[doc(hidden)]
    pub async fn init(pool: &SqlitePool) -> ChannelResult<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS trace_links (
                channel_type TEXT    NOT NULL,
                account_id   TEXT    NOT NULL,
                chat_id      TEXT    NOT NULL,
                message_id   TEXT    NOT NULL,
                trace_id     TEXT    NOT NULL,
                session_key  TEXT,
                created_at   INTEGER NOT NULL,
                PRIMARY KEY (channel_type, account_id, chat_id, message_id)
            )",
        )
        .execute(pool)
        .await
        .map_err(|e| db_error("init trace_links table", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_trace_links_created
             ON trace_links (created_at)",
        )
        .execute(pool)
        .await
        .map_err(|e| db_error("init trace_links index", e))?;

        Ok(())
    }
}

#[async_trait]
impl TraceLinkStore for SqliteTraceLinkStore {
    async fn link(&self, link: TraceLink) -> ChannelResult<()> {
        // Replace rather than ignore: an edited or resent reply keeps the same
        // message id but belongs to the newer turn, and a reaction on it should
        // score that turn.
        sqlx::query(
            "INSERT INTO trace_links
             (channel_type, account_id, chat_id, message_id, trace_id, session_key, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (channel_type, account_id, chat_id, message_id)
             DO UPDATE SET trace_id = excluded.trace_id,
                           session_key = excluded.session_key,
                           created_at = excluded.created_at",
        )
        .bind(&link.channel_type)
        .bind(&link.account_id)
        .bind(&link.chat_id)
        .bind(&link.message_id)
        .bind(&link.trace_id)
        .bind(&link.session_key)
        .bind(link.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| db_error("record trace link", e))?;
        Ok(())
    }

    async fn lookup(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> ChannelResult<Option<TraceLink>> {
        let row = sqlx::query_as::<_, (String, Option<String>, i64)>(
            "SELECT trace_id, session_key, created_at
             FROM trace_links
             WHERE channel_type = ? AND account_id = ? AND chat_id = ? AND message_id = ?",
        )
        .bind(channel_type)
        .bind(account_id)
        .bind(chat_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| db_error("look up trace link", e))?;

        Ok(row.map(|(trace_id, session_key, created_at)| TraceLink {
            channel_type: channel_type.to_string(),
            account_id: account_id.to_string(),
            chat_id: chat_id.to_string(),
            message_id: message_id.to_string(),
            trace_id,
            session_key,
            created_at,
        }))
    }

    async fn prune(&self, cutoff: i64) -> ChannelResult<u64> {
        let result = sqlx::query("DELETE FROM trace_links WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|e| db_error("prune trace links", e))?;
        Ok(result.rows_affected())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> SqliteTraceLinkStore {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        SqliteTraceLinkStore::init(&pool).await.unwrap();
        SqliteTraceLinkStore::new(pool)
    }

    fn link(message_id: &str, trace_id: &str) -> TraceLink {
        TraceLink {
            channel_type: "telegram".into(),
            account_id: "bot-1".into(),
            chat_id: "chat-1".into(),
            message_id: message_id.into(),
            trace_id: trace_id.into(),
            session_key: Some("agent:main:main".into()),
            created_at: 1_700_000_000,
        }
    }

    #[tokio::test]
    async fn a_linked_reply_resolves_to_its_trace() {
        let store = store().await;
        store.link(link("42", "trace-1")).await.unwrap();

        let found = store
            .lookup("telegram", "bot-1", "chat-1", "42")
            .await
            .unwrap()
            .expect("link should resolve");

        assert_eq!(found.trace_id, "trace-1");
        assert_eq!(found.session_key.as_deref(), Some("agent:main:main"));
    }

    #[tokio::test]
    async fn an_unknown_message_resolves_to_nothing() {
        let store = store().await;
        let found = store
            .lookup("telegram", "bot-1", "chat-1", "missing")
            .await
            .unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn message_ids_are_scoped_per_channel_and_chat() {
        // Message ids are only unique within a chat; Telegram numbers restart
        // per conversation, so a global lookup would attribute a reaction in
        // one chat to a completely unrelated turn in another.
        let store = store().await;
        store.link(link("42", "trace-1")).await.unwrap();

        let other_chat = TraceLink {
            chat_id: "chat-2".into(),
            trace_id: "trace-2".into(),
            ..link("42", "trace-2")
        };
        store.link(other_chat).await.unwrap();

        let first = store
            .lookup("telegram", "bot-1", "chat-1", "42")
            .await
            .unwrap()
            .expect("present");
        let second = store
            .lookup("telegram", "bot-1", "chat-2", "42")
            .await
            .unwrap()
            .expect("present");

        assert_eq!(first.trace_id, "trace-1");
        assert_eq!(second.trace_id, "trace-2");
    }

    #[tokio::test]
    async fn the_same_message_id_on_another_channel_is_a_separate_link() {
        let store = store().await;
        store.link(link("42", "trace-1")).await.unwrap();
        store
            .link(TraceLink {
                channel_type: "discord".into(),
                trace_id: "trace-discord".into(),
                ..link("42", "trace-discord")
            })
            .await
            .unwrap();

        let discord = store
            .lookup("discord", "bot-1", "chat-1", "42")
            .await
            .unwrap()
            .expect("present");

        assert_eq!(discord.trace_id, "trace-discord");
    }

    #[tokio::test]
    async fn relinking_a_message_points_at_the_newer_turn() {
        let store = store().await;
        store.link(link("42", "trace-old")).await.unwrap();
        store
            .link(TraceLink {
                created_at: 1_700_000_500,
                ..link("42", "trace-new")
            })
            .await
            .unwrap();

        let found = store
            .lookup("telegram", "bot-1", "chat-1", "42")
            .await
            .unwrap()
            .expect("present");

        assert_eq!(found.trace_id, "trace-new");
        assert_eq!(found.created_at, 1_700_000_500);
    }

    #[tokio::test]
    async fn pruning_drops_only_links_older_than_the_cutoff() {
        let store = store().await;
        store.link(link("old", "trace-old")).await.unwrap();
        store
            .link(TraceLink {
                created_at: 1_700_009_000,
                ..link("new", "trace-new")
            })
            .await
            .unwrap();

        let removed = store.prune(1_700_005_000).await.unwrap();

        assert_eq!(removed, 1);
        assert!(
            store
                .lookup("telegram", "bot-1", "chat-1", "old")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .lookup("telegram", "bot-1", "chat-1", "new")
                .await
                .unwrap()
                .is_some(),
            "pruning must not drop links still inside the retention window"
        );
    }

    #[tokio::test]
    async fn pruning_an_empty_table_is_a_no_op() {
        let store = store().await;
        assert_eq!(store.prune(1_700_000_000).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_link_without_a_session_is_still_usable() {
        // Channel replies that are not bound to an agent session still deserve
        // feedback attribution.
        let store = store().await;
        store
            .link(TraceLink {
                session_key: None,
                ..link("42", "trace-1")
            })
            .await
            .unwrap();

        let found = store
            .lookup("telegram", "bot-1", "chat-1", "42")
            .await
            .unwrap()
            .expect("present");

        assert_eq!(found.trace_id, "trace-1");
        assert!(found.session_key.is_none());
    }
}
