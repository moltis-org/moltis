//! Per-session key-value state store.
//!
//! Provides a SQLite-backed KV store scoped to `(session_key, namespace, key)`
//! so that skills and extensions can persist context across messages.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

/// A single state entry.
#[derive(Debug, Clone)]
pub struct StateEntry {
    pub namespace: String,
    pub key: String,
    pub value: String,
    pub updated_at: u64,
}

#[derive(sqlx::FromRow)]
struct StateRow {
    namespace: String,
    key: String,
    value: String,
    updated_at: i64,
}

impl From<StateRow> for StateEntry {
    fn from(r: StateRow) -> Self {
        Self {
            namespace: r.namespace,
            key: r.key,
            value: r.value,
            updated_at: r.updated_at as u64,
        }
    }
}

/// SQLite-backed per-session state store.
pub struct SessionStateStore {
    pool: sqlx::SqlitePool,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl SessionStateStore {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// Get a value by session key, namespace, and key.
    pub async fn get(
        &self,
        session_key: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT value FROM session_state WHERE session_key = ? AND namespace = ? AND key = ?",
        )
        .bind(session_key)
        .bind(namespace)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Set a value. Inserts or updates the entry.
    pub async fn set(
        &self,
        session_key: &str,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            r#"INSERT INTO session_state (session_key, namespace, key, value, updated_at)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(session_key, namespace, key) DO UPDATE SET
                 value = excluded.value,
                 updated_at = excluded.updated_at"#,
        )
        .bind(session_key)
        .bind(namespace)
        .bind(key)
        .bind(value)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a single key.
    pub async fn delete(&self, session_key: &str, namespace: &str, key: &str) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM session_state WHERE session_key = ? AND namespace = ? AND key = ?",
        )
        .bind(session_key)
        .bind(namespace)
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List all entries in a namespace for a session.
    pub async fn list(&self, session_key: &str, namespace: &str) -> Result<Vec<StateEntry>> {
        let rows = sqlx::query_as::<_, StateRow>(
            "SELECT namespace, key, value, updated_at FROM session_state \
             WHERE session_key = ? AND namespace = ? ORDER BY key",
        )
        .bind(session_key)
        .bind(namespace)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Delete all entries in a namespace for a session.
    pub async fn delete_all(&self, session_key: &str, namespace: &str) -> Result<u64> {
        let result =
            sqlx::query("DELETE FROM session_state WHERE session_key = ? AND namespace = ?")
                .bind(session_key)
                .bind(namespace)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected())
    }

    /// Delete all state for a session (cascade on session delete).
    pub async fn delete_session(&self, session_key: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM session_state WHERE session_key = ?")
            .bind(session_key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS session_state (
                session_key TEXT NOT NULL,
                namespace   TEXT NOT NULL,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL,
                updated_at  INTEGER NOT NULL,
                PRIMARY KEY (session_key, namespace, key)
            )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store
            .set("session:1", "my-skill", "count", "42")
            .await
            .unwrap();
        let val = store.get("session:1", "my-skill", "count").await.unwrap();
        assert_eq!(val.as_deref(), Some("42"));
    }

    #[tokio::test]
    async fn test_get_missing() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        let val = store.get("session:1", "ns", "missing").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_set_overwrites() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns", "k", "v1").await.unwrap();
        store.set("s1", "ns", "k", "v2").await.unwrap();
        let val = store.get("s1", "ns", "k").await.unwrap();
        assert_eq!(val.as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns", "k", "v").await.unwrap();
        let deleted = store.delete("s1", "ns", "k").await.unwrap();
        assert!(deleted);
        assert!(store.get("s1", "ns", "k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_missing() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        let deleted = store.delete("s1", "ns", "k").await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_list() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns", "a", "1").await.unwrap();
        store.set("s1", "ns", "b", "2").await.unwrap();
        store.set("s1", "other", "c", "3").await.unwrap();

        let entries = store.list("s1", "ns").await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "a");
        assert_eq!(entries[1].key, "b");
    }

    #[tokio::test]
    async fn test_delete_all() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns", "a", "1").await.unwrap();
        store.set("s1", "ns", "b", "2").await.unwrap();
        store.set("s1", "other", "c", "3").await.unwrap();

        let count = store.delete_all("s1", "ns").await.unwrap();
        assert_eq!(count, 2);
        // "other" namespace untouched.
        assert!(store.get("s1", "other", "c").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns1", "a", "1").await.unwrap();
        store.set("s1", "ns2", "b", "2").await.unwrap();
        store.set("s2", "ns1", "a", "3").await.unwrap();

        let count = store.delete_session("s1").await.unwrap();
        assert_eq!(count, 2);
        // s2 untouched.
        assert!(store.get("s2", "ns1", "a").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_namespace_isolation() {
        let pool = test_pool().await;
        let store = SessionStateStore::new(pool);

        store.set("s1", "ns-a", "key", "val-a").await.unwrap();
        store.set("s1", "ns-b", "key", "val-b").await.unwrap();

        assert_eq!(
            store.get("s1", "ns-a", "key").await.unwrap().as_deref(),
            Some("val-a")
        );
        assert_eq!(
            store.get("s1", "ns-b", "key").await.unwrap().as_deref(),
            Some("val-b")
        );
    }
}
