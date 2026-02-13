//! Metrics history storage.
//!
//! This module provides a trait-based abstraction for persisting metrics history
//! to enable historical charts that survive restarts.

use {
    anyhow::Result,
    serde::{Deserialize, Serialize},
    std::{collections::HashMap, path::Path},
};

/// Per-provider token metrics for a single time point.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTokens {
    /// Input tokens for this provider.
    pub input_tokens: u64,
    /// Output tokens for this provider.
    pub output_tokens: u64,
    /// Completions count for this provider.
    pub completions: u64,
    /// Errors for this provider.
    pub errors: u64,
}

/// A historical metrics data point for time-series charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsHistoryPoint {
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// LLM completion count (aggregate).
    pub llm_completions: u64,
    /// LLM input tokens (aggregate).
    pub llm_input_tokens: u64,
    /// LLM output tokens (aggregate).
    pub llm_output_tokens: u64,
    /// LLM errors (aggregate).
    pub llm_errors: u64,
    /// Per-provider token breakdown.
    #[serde(default)]
    pub by_provider: HashMap<String, ProviderTokens>,
    /// HTTP requests total.
    pub http_requests: u64,
    /// Active HTTP requests (in-flight).
    pub http_active: u64,
    /// WebSocket connections total.
    pub ws_connections: u64,
    /// Active WebSocket connections.
    pub ws_active: u64,
    /// Tool executions total.
    pub tool_executions: u64,
    /// Tool errors.
    pub tool_errors: u64,
    /// MCP tool calls total.
    pub mcp_calls: u64,
    /// Active sessions.
    pub active_sessions: u64,
}

/// Trait for metrics history storage backends.
///
/// Implementations can store metrics history in SQLite, TimescaleDB,
/// or any other time-series database.
#[async_trait::async_trait]
pub trait MetricsStore: Send + Sync {
    /// Save a new metrics data point.
    async fn save_point(&self, point: &MetricsHistoryPoint) -> Result<()>;

    /// Load metrics history since a given timestamp (millis).
    ///
    /// Returns points ordered by timestamp ascending.
    /// If `since` is 0, returns all points up to `limit`.
    async fn load_history(&self, since: u64, limit: usize) -> Result<Vec<MetricsHistoryPoint>>;

    /// Delete metrics older than the given timestamp (millis).
    ///
    /// Returns the number of deleted rows.
    async fn cleanup_before(&self, before: u64) -> Result<u64>;

    /// Get the most recent data point, if any.
    async fn latest_point(&self) -> Result<Option<MetricsHistoryPoint>>;
}

/// SQLite-based metrics store.
///
/// Stores metrics history in a SQLite database file.
pub struct SqliteMetricsStore {
    pool: sqlx::SqlitePool,
}

impl SqliteMetricsStore {
    /// Create a new SQLite metrics store.
    ///
    /// Opens or creates the database at the given path.
    pub async fn new(path: &Path) -> Result<Self> {
        let db_url = format!("sqlite:{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&db_url).await?;

        // Run migrations
        Self::migrate(&pool).await?;

        Ok(Self { pool })
    }

    /// Create an in-memory SQLite metrics store (for testing).
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    #[cfg(test)]
    pub async fn in_memory() -> Result<Self> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &sqlx::SqlitePool) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS metrics_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                llm_completions INTEGER NOT NULL DEFAULT 0,
                llm_input_tokens INTEGER NOT NULL DEFAULT 0,
                llm_output_tokens INTEGER NOT NULL DEFAULT 0,
                llm_errors INTEGER NOT NULL DEFAULT 0,
                by_provider TEXT,
                http_requests INTEGER NOT NULL DEFAULT 0,
                http_active INTEGER NOT NULL DEFAULT 0,
                ws_connections INTEGER NOT NULL DEFAULT 0,
                ws_active INTEGER NOT NULL DEFAULT 0,
                tool_executions INTEGER NOT NULL DEFAULT 0,
                tool_errors INTEGER NOT NULL DEFAULT 0,
                mcp_calls INTEGER NOT NULL DEFAULT 0,
                active_sessions INTEGER NOT NULL DEFAULT 0
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_metrics_history_timestamp
            ON metrics_history(timestamp)
            "#,
        )
        .execute(pool)
        .await?;

        // Migration: add by_provider column if it doesn't exist (for existing databases).
        sqlx::query(
            r#"
            ALTER TABLE metrics_history ADD COLUMN by_provider TEXT
            "#,
        )
        .execute(pool)
        .await
        .ok(); // Ignore error if column already exists

        Ok(())
    }
}

#[async_trait::async_trait]
impl MetricsStore for SqliteMetricsStore {
    async fn save_point(&self, point: &MetricsHistoryPoint) -> Result<()> {
        let by_provider_json = if point.by_provider.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&point.by_provider)?)
        };

        sqlx::query(
            r#"
            INSERT INTO metrics_history (
                timestamp, llm_completions, llm_input_tokens, llm_output_tokens,
                llm_errors, by_provider, http_requests, http_active, ws_connections,
                ws_active, tool_executions, tool_errors, mcp_calls, active_sessions
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(point.timestamp as i64)
        .bind(point.llm_completions as i64)
        .bind(point.llm_input_tokens as i64)
        .bind(point.llm_output_tokens as i64)
        .bind(point.llm_errors as i64)
        .bind(by_provider_json)
        .bind(point.http_requests as i64)
        .bind(point.http_active as i64)
        .bind(point.ws_connections as i64)
        .bind(point.ws_active as i64)
        .bind(point.tool_executions as i64)
        .bind(point.tool_errors as i64)
        .bind(point.mcp_calls as i64)
        .bind(point.active_sessions as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn load_history(&self, since: u64, limit: usize) -> Result<Vec<MetricsHistoryPoint>> {
        let rows = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT timestamp, llm_completions, llm_input_tokens, llm_output_tokens,
                   llm_errors, by_provider, http_requests, http_active, ws_connections,
                   ws_active, tool_executions, tool_errors, mcp_calls, active_sessions
            FROM metrics_history
            WHERE timestamp >= ?
            ORDER BY timestamp ASC
            LIMIT ?
            "#,
        )
        .bind(since as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn cleanup_before(&self, before: u64) -> Result<u64> {
        let result = sqlx::query("DELETE FROM metrics_history WHERE timestamp < ?")
            .bind(before as i64)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    async fn latest_point(&self) -> Result<Option<MetricsHistoryPoint>> {
        let row = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT timestamp, llm_completions, llm_input_tokens, llm_output_tokens,
                   llm_errors, by_provider, http_requests, http_active, ws_connections,
                   ws_active, tool_executions, tool_errors, mcp_calls, active_sessions
            FROM metrics_history
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }
}

/// Internal row type for SQLite queries.
#[derive(sqlx::FromRow)]
struct MetricsRow {
    timestamp: i64,
    llm_completions: i64,
    llm_input_tokens: i64,
    llm_output_tokens: i64,
    llm_errors: i64,
    by_provider: Option<String>,
    http_requests: i64,
    http_active: i64,
    ws_connections: i64,
    ws_active: i64,
    tool_executions: i64,
    tool_errors: i64,
    mcp_calls: i64,
    active_sessions: i64,
}

impl From<MetricsRow> for MetricsHistoryPoint {
    fn from(row: MetricsRow) -> Self {
        let by_provider = row
            .by_provider
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        Self {
            timestamp: row.timestamp as u64,
            llm_completions: row.llm_completions as u64,
            llm_input_tokens: row.llm_input_tokens as u64,
            llm_output_tokens: row.llm_output_tokens as u64,
            llm_errors: row.llm_errors as u64,
            by_provider,
            http_requests: row.http_requests as u64,
            http_active: row.http_active as u64,
            ws_connections: row.ws_connections as u64,
            ws_active: row.ws_active as u64,
            tool_executions: row.tool_executions as u64,
            tool_errors: row.tool_errors as u64,
            mcp_calls: row.mcp_calls as u64,
            active_sessions: row.active_sessions as u64,
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn make_point(timestamp: u64, llm_completions: u64) -> MetricsHistoryPoint {
        MetricsHistoryPoint {
            timestamp,
            llm_completions,
            llm_input_tokens: 0,
            llm_output_tokens: 0,
            llm_errors: 0,
            by_provider: HashMap::new(),
            http_requests: 0,
            http_active: 0,
            ws_connections: 0,
            ws_active: 0,
            tool_executions: 0,
            tool_errors: 0,
            mcp_calls: 0,
            active_sessions: 0,
        }
    }

    #[tokio::test]
    async fn test_save_and_load() {
        let store = SqliteMetricsStore::in_memory().await.unwrap();

        let mut point = make_point(1000, 10);
        point.llm_input_tokens = 100;
        point.llm_output_tokens = 50;
        point.llm_errors = 1;
        point.http_requests = 200;
        point.http_active = 5;
        point.ws_connections = 20;
        point.ws_active = 3;
        point.tool_executions = 15;
        point.tool_errors = 2;
        point.mcp_calls = 8;
        point.active_sessions = 4;

        store.save_point(&point).await.unwrap();

        let history = store.load_history(0, 100).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].timestamp, 1000);
        assert_eq!(history[0].llm_completions, 10);
        assert_eq!(history[0].llm_input_tokens, 100);
    }

    #[tokio::test]
    async fn test_save_and_load_with_provider() {
        let store = SqliteMetricsStore::in_memory().await.unwrap();

        let mut point = make_point(1000, 10);
        point
            .by_provider
            .insert("anthropic".to_string(), ProviderTokens {
                input_tokens: 500,
                output_tokens: 200,
                completions: 5,
                errors: 0,
            });
        point
            .by_provider
            .insert("openai".to_string(), ProviderTokens {
                input_tokens: 300,
                output_tokens: 100,
                completions: 5,
                errors: 1,
            });

        store.save_point(&point).await.unwrap();

        let history = store.load_history(0, 100).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].by_provider.len(), 2);
        assert_eq!(history[0].by_provider["anthropic"].input_tokens, 500);
        assert_eq!(history[0].by_provider["openai"].errors, 1);
    }

    #[tokio::test]
    async fn test_load_since() {
        let store = SqliteMetricsStore::in_memory().await.unwrap();

        for i in 0..5 {
            store
                .save_point(&make_point(1000 + i * 100, i))
                .await
                .unwrap();
        }

        // Load since timestamp 1200 (should get points at 1200, 1300, 1400)
        let history = store.load_history(1200, 100).await.unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].timestamp, 1200);
    }

    #[tokio::test]
    async fn test_cleanup() {
        let store = SqliteMetricsStore::in_memory().await.unwrap();

        for i in 0..5 {
            store
                .save_point(&make_point(1000 + i * 100, 0))
                .await
                .unwrap();
        }

        // Cleanup before 1200 (should delete 1000, 1100)
        let deleted = store.cleanup_before(1200).await.unwrap();
        assert_eq!(deleted, 2);

        let history = store.load_history(0, 100).await.unwrap();
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn test_latest_point() {
        let store = SqliteMetricsStore::in_memory().await.unwrap();

        assert!(store.latest_point().await.unwrap().is_none());

        for i in 0..3 {
            store
                .save_point(&make_point(1000 + i * 100, i))
                .await
                .unwrap();
        }

        let latest = store.latest_point().await.unwrap().unwrap();
        assert_eq!(latest.timestamp, 1200);
        assert_eq!(latest.llm_completions, 2);
    }
}
