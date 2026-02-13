use {
    anyhow::Result,
    base64::Engine,
    rand::RngCore,
    sha2::{Digest, Sha256},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareVisibility {
    Public,
    Private,
}

impl ShareVisibility {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl std::str::FromStr for ShareVisibility {
    type Err = &'static str;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            _ => Err("invalid share visibility"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SharedMessageRole {
    User,
    Assistant,
    #[serde(rename = "tool_result")]
    ToolResult,
    System,
    Notice,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SharedMapLinks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apple_maps: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_maps: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openstreetmap: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedImageAsset {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedImageSet {
    pub preview: SharedImageAsset,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<SharedImageAsset>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMessage {
    pub role: SharedMessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<SharedImageSet>,
    // Backward compatibility for snapshots created before image variants existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_links: Option<SharedMapLinks>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSnapshot {
    pub session_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_label: Option<String>,
    pub cutoff_message_count: u32,
    pub created_at: u64,
    pub messages: Vec<SharedMessage>,
}

#[derive(Debug, Clone)]
pub struct SessionShare {
    pub id: String,
    pub session_key: String,
    pub visibility: ShareVisibility,
    pub snapshot_json: String,
    pub snapshot_message_count: u32,
    pub token_hash: Option<String>,
    pub views: u64,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CreatedShare {
    pub share: SessionShare,
    pub access_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ShareRow {
    id: String,
    session_key: String,
    visibility: String,
    snapshot_json: String,
    snapshot_message_count: i64,
    token_hash: Option<String>,
    views: i64,
    created_at: i64,
    revoked_at: Option<i64>,
}

impl TryFrom<ShareRow> for SessionShare {
    type Error = anyhow::Error;

    fn try_from(row: ShareRow) -> Result<Self, Self::Error> {
        let visibility = row
            .visibility
            .parse::<ShareVisibility>()
            .map_err(|_| anyhow::anyhow!("invalid visibility '{}'", row.visibility))?;
        Ok(Self {
            id: row.id,
            session_key: row.session_key,
            visibility,
            snapshot_json: row.snapshot_json,
            snapshot_message_count: row.snapshot_message_count.max(0) as u32,
            token_hash: row.token_hash,
            views: row.views.max(0) as u64,
            created_at: row.created_at.max(0) as u64,
            revoked_at: row.revoked_at.map(|v| v.max(0) as u64),
        })
    }
}

pub struct ShareStore {
    pool: sqlx::SqlitePool,
}

impl ShareStore {
    #[must_use]
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    /// Deprecated: schema is managed by sqlx migrations. Kept for tests.
    #[doc(hidden)]
    pub async fn init(pool: &sqlx::SqlitePool) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS session_shares (
                id                     TEXT    PRIMARY KEY,
                session_key            TEXT    NOT NULL,
                visibility             TEXT    NOT NULL,
                snapshot_json          TEXT    NOT NULL,
                snapshot_message_count INTEGER NOT NULL,
                token_hash             TEXT,
                views                  INTEGER NOT NULL DEFAULT 0,
                created_at             INTEGER NOT NULL,
                revoked_at             INTEGER
            )"#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_shares_session_created ON session_shares(session_key, created_at DESC)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_shares_active ON session_shares(id, revoked_at)",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_session_shares_one_active_per_session ON session_shares(session_key) WHERE revoked_at IS NULL",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn create_or_replace(
        &self,
        session_key: &str,
        visibility: ShareVisibility,
        snapshot_json: String,
        snapshot_message_count: u32,
    ) -> Result<CreatedShare> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms() as i64;
        let access_key = (visibility == ShareVisibility::Private).then(generate_access_key);
        let token_hash = access_key.as_deref().map(hash_token);

        let mut tx = self.pool.begin().await?;

        // Keep at most one active share per session by revoking previous links.
        sqlx::query(
            "UPDATE session_shares SET revoked_at = ? WHERE session_key = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(session_key)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO session_shares (
                id, session_key, visibility, snapshot_json, snapshot_message_count,
                token_hash, views, created_at, revoked_at
            ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, NULL)"#,
        )
        .bind(&id)
        .bind(session_key)
        .bind(visibility.as_str())
        .bind(&snapshot_json)
        .bind(snapshot_message_count as i64)
        .bind(&token_hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        let share = self
            .get_by_id(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("newly created share not found"))?;

        Ok(CreatedShare { share, access_key })
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<SessionShare>> {
        let row = sqlx::query_as::<_, ShareRow>("SELECT * FROM session_shares WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(SessionShare::try_from).transpose()
    }

    pub async fn get_active_by_id(&self, id: &str) -> Result<Option<SessionShare>> {
        let row = sqlx::query_as::<_, ShareRow>(
            "SELECT * FROM session_shares WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(SessionShare::try_from).transpose()
    }

    pub async fn list_for_session(&self, session_key: &str) -> Result<Vec<SessionShare>> {
        let rows = sqlx::query_as::<_, ShareRow>(
            "SELECT * FROM session_shares WHERE session_key = ? ORDER BY created_at DESC",
        )
        .bind(session_key)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(SessionShare::try_from).collect()
    }

    pub async fn revoke(&self, id: &str) -> Result<bool> {
        let now = now_ms() as i64;
        let result = sqlx::query(
            "UPDATE session_shares SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn increment_views(&self, id: &str) -> Result<u64> {
        sqlx::query(
            "UPDATE session_shares SET views = views + 1 WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        let views = sqlx::query_scalar::<_, i64>("SELECT views FROM session_shares WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(views.max(0) as u64)
    }

    #[must_use]
    pub fn verify_access_key(share: &SessionShare, access_key: &str) -> bool {
        match (share.visibility, share.token_hash.as_deref()) {
            (ShareVisibility::Public, _) => true,
            (ShareVisibility::Private, Some(hash)) => {
                let provided_hash = hash_token(access_key);
                constant_time_eq(hash.as_bytes(), provided_hash.as_bytes())
            },
            (ShareVisibility::Private, None) => false,
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn generate_access_key() -> String {
    let mut bytes = [0_u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    out
}

fn nibble_to_hex(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        _ => (b'a' + (v - 10)) as char,
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> ShareStore {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        ShareStore::init(&pool).await.unwrap();
        ShareStore::new(pool)
    }

    #[tokio::test]
    async fn create_public_share_replaces_previous_active_share() {
        let store = test_store().await;
        let snapshot = serde_json::json!({"messages": []}).to_string();

        let first = store
            .create_or_replace("main", ShareVisibility::Public, snapshot.clone(), 3)
            .await
            .unwrap();
        let second = store
            .create_or_replace("main", ShareVisibility::Public, snapshot, 5)
            .await
            .unwrap();

        assert_ne!(first.share.id, second.share.id);

        let first_row = store.get_by_id(&first.share.id).await.unwrap().unwrap();
        assert!(first_row.revoked_at.is_some());

        let second_row = store.get_by_id(&second.share.id).await.unwrap().unwrap();
        assert!(second_row.revoked_at.is_none());
    }

    #[tokio::test]
    async fn private_share_requires_valid_access_key() {
        let store = test_store().await;
        let snapshot = serde_json::json!({"messages": []}).to_string();

        let created = store
            .create_or_replace("main", ShareVisibility::Private, snapshot, 2)
            .await
            .unwrap();

        let key = created.access_key.clone().expect("private share key");
        assert!(ShareStore::verify_access_key(&created.share, &key));
        assert!(!ShareStore::verify_access_key(&created.share, "wrong-key"));
    }

    #[tokio::test]
    async fn increment_views_counts_only_active_share() {
        let store = test_store().await;
        let snapshot = serde_json::json!({"messages": []}).to_string();

        let created = store
            .create_or_replace("main", ShareVisibility::Public, snapshot, 1)
            .await
            .unwrap();

        let views_1 = store.increment_views(&created.share.id).await.unwrap();
        let views_2 = store.increment_views(&created.share.id).await.unwrap();

        assert_eq!(views_1, 1);
        assert_eq!(views_2, 2);
    }
}
