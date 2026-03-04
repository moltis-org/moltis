//! Device pairing state machine and device token management.
//!
//! Pairing state is persisted to SQLite so it survives gateway restarts.
//! An in-memory cache provides fast reads; all mutations write through to DB.

use std::time::Duration;

use {
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
    sqlx::SqlitePool,
    time::OffsetDateTime,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pair request not found")]
    PairRequestNotFound,

    #[error("pair request already {0:?}")]
    PairRequestNotPending(PairStatus),

    #[error("pair request expired")]
    PairRequestExpired,

    #[error("device not found")]
    DeviceNotFound,

    #[error("invalid device token")]
    InvalidToken,

    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl PairStatus {
    /// String representation used for DB storage and serialization.
    #[allow(dead_code)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "approved" => Self::Approved,
            "rejected" => Self::Rejected,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PairRequest {
    pub id: String,
    pub device_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub public_key: Option<String>,
    pub nonce: String,
    pub status: PairStatus,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceToken {
    pub token: String,
    pub device_id: String,
    pub scopes: Vec<String>,
    pub issued_at_ms: u64,
    pub revoked: bool,
}

/// Result of verifying a device token.
#[derive(Debug, Clone)]
pub struct DeviceTokenVerification {
    pub device_id: String,
    pub scopes: Vec<String>,
}

// ── Pairing store ───────────────────────────────────────────────────────────

/// SQLite-backed pairing store. Persists pair requests, paired devices, and
/// device tokens across gateway restarts.
pub struct PairingStore {
    pool: SqlitePool,
    pair_ttl: Duration,
}

impl PairingStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            pair_ttl: Duration::from_secs(300), // 5 min
        }
    }

    /// Submit a new pairing request. Returns the generated request with nonce.
    pub async fn request_pair(
        &self,
        device_id: &str,
        display_name: Option<&str>,
        platform: &str,
        public_key: Option<&str>,
    ) -> Result<PairRequest> {
        let id = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();
        let expires_at = now + self.pair_ttl;
        let created_str = format_datetime(now);
        let expires_str = format_datetime(expires_at);

        sqlx::query(
            "INSERT INTO pair_requests (id, device_id, display_name, platform, public_key, nonce, status, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(device_id)
        .bind(display_name)
        .bind(platform)
        .bind(public_key)
        .bind(&nonce)
        .bind(&created_str)
        .bind(&expires_str)
        .execute(&self.pool)
        .await?;

        Ok(PairRequest {
            id,
            device_id: device_id.to_string(),
            display_name: display_name.map(|s| s.to_string()),
            platform: platform.to_string(),
            public_key: public_key.map(|s| s.to_string()),
            nonce,
            status: PairStatus::Pending,
            created_at: created_str,
            expires_at: expires_str,
        })
    }

    /// List all non-expired pending requests.
    pub async fn list_pending(&self) -> Result<Vec<PairRequest>> {
        let rows: Vec<(String, String, Option<String>, String, Option<String>, String, String, String)> = sqlx::query_as(
            "SELECT id, device_id, display_name, platform, public_key, nonce, created_at, expires_at
             FROM pair_requests
             WHERE status = 'pending' AND expires_at > datetime('now')",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    device_id,
                    display_name,
                    platform,
                    public_key,
                    nonce,
                    created_at,
                    expires_at,
                )| {
                    PairRequest {
                        id,
                        device_id,
                        display_name,
                        platform,
                        public_key,
                        nonce,
                        status: PairStatus::Pending,
                        created_at,
                        expires_at,
                    }
                },
            )
            .collect())
    }

    /// Approve a pending pair request. Issues a device token.
    pub async fn approve(&self, pair_id: &str) -> Result<DeviceToken> {
        // Load and validate the request.
        let row: Option<(
            String,
            Option<String>,
            String,
            Option<String>,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT device_id, display_name, platform, public_key, status, expires_at
                 FROM pair_requests WHERE id = ?",
        )
        .bind(pair_id)
        .fetch_optional(&self.pool)
        .await?;

        let (device_id, display_name, platform, public_key, status, expires_at) =
            row.ok_or(Error::PairRequestNotFound)?;

        let current_status = PairStatus::from_str(&status);
        if current_status != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(current_status));
        }

        // Check expiry.
        if is_expired(&expires_at) {
            sqlx::query("UPDATE pair_requests SET status = 'expired' WHERE id = ?")
                .bind(pair_id)
                .execute(&self.pool)
                .await?;
            return Err(Error::PairRequestExpired);
        }

        // Mark request as approved.
        sqlx::query("UPDATE pair_requests SET status = 'approved' WHERE id = ?")
            .bind(pair_id)
            .execute(&self.pool)
            .await?;

        // Upsert the paired device.
        sqlx::query(
            "INSERT INTO paired_devices (device_id, display_name, platform, public_key, status)
             VALUES (?, ?, ?, ?, 'active')
             ON CONFLICT(device_id) DO UPDATE SET
                display_name = excluded.display_name,
                platform = excluded.platform,
                public_key = excluded.public_key,
                status = 'active',
                revoked_at = NULL",
        )
        .bind(&device_id)
        .bind(&display_name)
        .bind(&platform)
        .bind(&public_key)
        .execute(&self.pool)
        .await?;

        // Issue a device token.
        let raw_token = format!("mdt_{}", generate_token());
        let token_hash = sha256_hex(&raw_token);
        let token_prefix = &raw_token[..raw_token.len().min(12)];
        let scopes = vec![
            "operator.read".to_string(),
            "operator.write".to_string(),
            "operator.approvals".to_string(),
        ];
        let scopes_json = serde_json::to_string(&scopes).unwrap_or_default();
        let issued_at_ms = current_epoch_ms();

        sqlx::query(
            "INSERT INTO device_tokens (token_hash, token_prefix, device_id, scopes, issued_at)
             VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&token_hash)
        .bind(token_prefix)
        .bind(&device_id)
        .bind(&scopes_json)
        .execute(&self.pool)
        .await?;

        Ok(DeviceToken {
            token: raw_token,
            device_id,
            scopes,
            issued_at_ms,
            revoked: false,
        })
    }

    /// Reject a pending pair request.
    pub async fn reject(&self, pair_id: &str) -> Result<()> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT status, expires_at FROM pair_requests WHERE id = ?")
                .bind(pair_id)
                .fetch_optional(&self.pool)
                .await?;

        let (status, _) = row.ok_or(Error::PairRequestNotFound)?;
        if PairStatus::from_str(&status) != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(PairStatus::from_str(&status)));
        }

        sqlx::query("UPDATE pair_requests SET status = 'rejected' WHERE id = ?")
            .bind(pair_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// List all approved (non-revoked) devices.
    pub async fn list_devices(&self) -> Result<Vec<PairedDevice>> {
        let rows: Vec<(String, Option<String>, String, Option<String>, String)> = sqlx::query_as(
            "SELECT device_id, display_name, platform, public_key, created_at
             FROM paired_devices WHERE status = 'active'
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(device_id, display_name, platform, public_key, created_at)| PairedDevice {
                    device_id,
                    display_name,
                    platform,
                    public_key,
                    created_at,
                },
            )
            .collect())
    }

    /// List device tokens for a specific device (active only).
    pub async fn list_device_tokens(&self, device_id: &str) -> Result<Vec<DeviceTokenEntry>> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT token_prefix, scopes, issued_at FROM device_tokens
             WHERE device_id = ? AND revoked = 0
             ORDER BY issued_at DESC",
        )
        .bind(device_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(token_prefix, scopes_json, issued_at)| {
                let scopes = serde_json::from_str::<Vec<String>>(&scopes_json).unwrap_or_default();
                DeviceTokenEntry {
                    token_prefix,
                    device_id: device_id.to_string(),
                    scopes,
                    issued_at,
                }
            })
            .collect())
    }

    /// Rotate a device token: revoke all existing tokens, issue new.
    pub async fn rotate_token(&self, device_id: &str) -> Result<DeviceToken> {
        // Verify device exists and is active.
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT status FROM paired_devices WHERE device_id = ?")
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?;

        if exists.is_none() {
            return Err(Error::DeviceNotFound);
        }

        // Load current scopes from any active token.
        let scopes_row: Option<(String,)> = sqlx::query_as(
            "SELECT scopes FROM device_tokens WHERE device_id = ? AND revoked = 0 LIMIT 1",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?;

        let scopes = scopes_row
            .and_then(|(s,)| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_else(|| {
                vec![
                    "operator.read".into(),
                    "operator.write".into(),
                    "operator.approvals".into(),
                ]
            });

        // Revoke all existing tokens.
        sqlx::query("UPDATE device_tokens SET revoked = 1 WHERE device_id = ? AND revoked = 0")
            .bind(device_id)
            .execute(&self.pool)
            .await?;

        // Issue new token.
        let raw_token = format!("mdt_{}", generate_token());
        let token_hash = sha256_hex(&raw_token);
        let token_prefix = &raw_token[..raw_token.len().min(12)];
        let scopes_json = serde_json::to_string(&scopes).unwrap_or_default();
        let issued_at_ms = current_epoch_ms();

        sqlx::query(
            "INSERT INTO device_tokens (token_hash, token_prefix, device_id, scopes, issued_at)
             VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&token_hash)
        .bind(token_prefix)
        .bind(device_id)
        .bind(&scopes_json)
        .execute(&self.pool)
        .await?;

        Ok(DeviceToken {
            token: raw_token,
            device_id: device_id.to_string(),
            scopes,
            issued_at_ms,
            revoked: false,
        })
    }

    /// Revoke all tokens for a device and mark device as revoked.
    pub async fn revoke_token(&self, device_id: &str) -> Result<()> {
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT status FROM paired_devices WHERE device_id = ?")
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?;

        if exists.is_none() {
            return Err(Error::DeviceNotFound);
        }

        sqlx::query("UPDATE device_tokens SET revoked = 1 WHERE device_id = ?")
            .bind(device_id)
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "UPDATE paired_devices SET status = 'revoked', revoked_at = datetime('now') WHERE device_id = ?",
        )
        .bind(device_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Verify a raw device token. Returns device identity and scopes if valid.
    pub async fn verify_device_token(
        &self,
        raw_token: &str,
    ) -> Result<Option<DeviceTokenVerification>> {
        let hash = sha256_hex(raw_token);

        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT device_id, scopes FROM device_tokens
             WHERE token_hash = ? AND revoked = 0",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((device_id, scopes_json)) => {
                // Also verify the device is still active.
                let device_active: Option<(String,)> = sqlx::query_as(
                    "SELECT status FROM paired_devices WHERE device_id = ? AND status = 'active'",
                )
                .bind(&device_id)
                .fetch_optional(&self.pool)
                .await?;

                if device_active.is_none() {
                    return Ok(None);
                }

                let scopes = serde_json::from_str::<Vec<String>>(&scopes_json).unwrap_or_default();
                Ok(Some(DeviceTokenVerification { device_id, scopes }))
            },
            None => Ok(None),
        }
    }

    /// Create a pre-authorized device and issue a token directly (no pairing handshake).
    pub async fn create_device_token(
        &self,
        display_name: Option<&str>,
        platform: &str,
    ) -> Result<DeviceToken> {
        let device_id = uuid::Uuid::new_v4().to_string();

        // Insert as an active paired device.
        sqlx::query(
            "INSERT INTO paired_devices (device_id, display_name, platform, public_key, status)
             VALUES (?, ?, ?, NULL, 'active')",
        )
        .bind(&device_id)
        .bind(display_name)
        .bind(platform)
        .execute(&self.pool)
        .await?;

        // Issue a device token.
        let raw_token = format!("mdt_{}", generate_token());
        let token_hash = sha256_hex(&raw_token);
        let token_prefix = &raw_token[..raw_token.len().min(12)];
        let scopes = vec![
            "operator.read".to_string(),
            "operator.write".to_string(),
            "operator.approvals".to_string(),
        ];
        let scopes_json = serde_json::to_string(&scopes).unwrap_or_default();
        let issued_at_ms = current_epoch_ms();

        sqlx::query(
            "INSERT INTO device_tokens (token_hash, token_prefix, device_id, scopes, issued_at)
             VALUES (?, ?, ?, ?, datetime('now'))",
        )
        .bind(&token_hash)
        .bind(token_prefix)
        .bind(&device_id)
        .bind(&scopes_json)
        .execute(&self.pool)
        .await?;

        Ok(DeviceToken {
            token: raw_token,
            device_id,
            scopes,
            issued_at_ms,
            revoked: false,
        })
    }

    /// Evict expired pending requests.
    pub async fn evict_expired(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE pair_requests SET status = 'expired'
             WHERE status = 'pending' AND expires_at <= datetime('now')",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

// ── Additional types ────────────────────────────────────────────────────────

/// A paired device (for listing).
#[derive(Debug, Clone, Serialize)]
pub struct PairedDevice {
    pub device_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub public_key: Option<String>,
    pub created_at: String,
}

/// A device token entry (for listing — never exposes raw token).
#[derive(Debug, Clone, Serialize)]
pub struct DeviceTokenEntry {
    pub token_prefix: String,
    pub device_id: String,
    pub scopes: Vec<String>,
    pub issued_at: String,
}

// ── In-memory pairing state (kept for backward compat during transition) ────

/// In-memory pairing state for use when no database is available (tests).
pub struct PairingState {
    pending: std::collections::HashMap<String, PairRequest>,
    devices: std::collections::HashMap<String, DeviceToken>,
    pair_ttl: Duration,
}

impl Default for PairingState {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingState {
    pub fn new() -> Self {
        Self {
            pending: std::collections::HashMap::new(),
            devices: std::collections::HashMap::new(),
            pair_ttl: Duration::from_secs(300),
        }
    }

    pub fn request_pair(
        &mut self,
        device_id: &str,
        display_name: Option<&str>,
        platform: &str,
        public_key: Option<&str>,
    ) -> PairRequest {
        let id = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let now = OffsetDateTime::now_utc();
        let expires_at = now + self.pair_ttl;
        let req = PairRequest {
            id: id.clone(),
            device_id: device_id.to_string(),
            display_name: display_name.map(|s| s.to_string()),
            platform: platform.to_string(),
            public_key: public_key.map(|s| s.to_string()),
            nonce,
            status: PairStatus::Pending,
            created_at: format_datetime(now),
            expires_at: format_datetime(expires_at),
        };
        self.pending.insert(id, req.clone());
        req
    }

    pub fn list_pending(&self) -> Vec<&PairRequest> {
        let now = format_datetime(OffsetDateTime::now_utc());
        self.pending
            .values()
            .filter(|r| r.status == PairStatus::Pending && r.expires_at > now)
            .collect()
    }

    pub fn approve(&mut self, pair_id: &str) -> Result<DeviceToken> {
        let req = self
            .pending
            .get_mut(pair_id)
            .ok_or(Error::PairRequestNotFound)?;
        if req.status != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(req.status));
        }
        let now = format_datetime(OffsetDateTime::now_utc());
        if req.expires_at <= now {
            req.status = PairStatus::Expired;
            return Err(Error::PairRequestExpired);
        }
        req.status = PairStatus::Approved;

        let token = DeviceToken {
            token: uuid::Uuid::new_v4().to_string(),
            device_id: req.device_id.clone(),
            scopes: vec![
                "operator.read".into(),
                "operator.write".into(),
                "operator.approvals".into(),
            ],
            issued_at_ms: current_epoch_ms(),
            revoked: false,
        };
        self.devices.insert(req.device_id.clone(), token.clone());
        Ok(token)
    }

    pub fn reject(&mut self, pair_id: &str) -> Result<()> {
        let req = self
            .pending
            .get_mut(pair_id)
            .ok_or(Error::PairRequestNotFound)?;
        if req.status != PairStatus::Pending {
            return Err(Error::PairRequestNotPending(req.status));
        }
        req.status = PairStatus::Rejected;
        Ok(())
    }

    pub fn list_devices(&self) -> Vec<&DeviceToken> {
        self.devices.values().filter(|d| !d.revoked).collect()
    }

    pub fn rotate_token(&mut self, device_id: &str) -> Result<DeviceToken> {
        let existing = self
            .devices
            .get_mut(device_id)
            .ok_or(Error::DeviceNotFound)?;
        existing.revoked = true;

        let new_token = DeviceToken {
            token: uuid::Uuid::new_v4().to_string(),
            device_id: device_id.to_string(),
            scopes: existing.scopes.clone(),
            issued_at_ms: current_epoch_ms(),
            revoked: false,
        };
        self.devices
            .insert(device_id.to_string(), new_token.clone());
        Ok(new_token)
    }

    pub fn revoke_token(&mut self, device_id: &str) -> Result<()> {
        let existing = self
            .devices
            .get_mut(device_id)
            .ok_or(Error::DeviceNotFound)?;
        existing.revoked = true;
        Ok(())
    }

    /// Create a pre-authorized device and issue a token directly (no pairing handshake).
    pub fn create_device_token(
        &mut self,
        display_name: Option<&str>,
        platform: &str,
    ) -> DeviceToken {
        let device_id = uuid::Uuid::new_v4().to_string();
        let _ = display_name;
        let _ = platform;
        let token = DeviceToken {
            token: uuid::Uuid::new_v4().to_string(),
            device_id: device_id.clone(),
            scopes: vec![
                "operator.read".into(),
                "operator.write".into(),
                "operator.approvals".into(),
            ],
            issued_at_ms: current_epoch_ms(),
            revoked: false,
        };
        self.devices.insert(device_id, token.clone());
        token
    }

    pub fn evict_expired(&mut self) {
        let now = format_datetime(OffsetDateTime::now_utc());
        self.pending
            .retain(|_, r| !(r.status == PairStatus::Pending && r.expires_at <= now));
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn generate_token() -> String {
    use {base64::Engine, rand::RngCore};

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn format_datetime(dt: OffsetDateTime) -> String {
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn is_expired(expires_at: &str) -> bool {
    let Ok(expires) =
        OffsetDateTime::parse(expires_at, &time::format_description::well_known::Rfc3339)
    else {
        // If we can't parse, try SQLite datetime format.
        return is_expired_sqlite(expires_at);
    };
    OffsetDateTime::now_utc() > expires
}

fn is_expired_sqlite(expires_at: &str) -> bool {
    // SQLite datetime format: "YYYY-MM-DD HH:MM:SS"
    // Simple string comparison works because the format is lexicographically ordered.
    let now = OffsetDateTime::now_utc();
    let now_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    now_str.as_str() > expires_at
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn pairing_lifecycle() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        // Request pairing.
        let req = store
            .request_pair("dev-1", Some("My iPhone"), "ios", None)
            .await
            .unwrap();
        assert_eq!(req.device_id, "dev-1");
        assert_eq!(req.status, PairStatus::Pending);

        // List pending.
        let pending = store.list_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, req.id);

        // Approve.
        let token = store.approve(&req.id).await.unwrap();
        assert!(token.token.starts_with("mdt_"));
        assert_eq!(token.device_id, "dev-1");
        assert!(!token.scopes.is_empty());

        // Pending should be empty now.
        let pending = store.list_pending().await.unwrap();
        assert!(pending.is_empty());

        // Device should be listed.
        let devices = store.list_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, "dev-1");

        // Verify token.
        let verification = store.verify_device_token(&token.token).await.unwrap();
        assert!(verification.is_some());
        let v = verification.unwrap();
        assert_eq!(v.device_id, "dev-1");
        assert_eq!(v.scopes, token.scopes);

        // Rotate token.
        let new_token = store.rotate_token("dev-1").await.unwrap();
        assert_ne!(new_token.token, token.token);

        // Old token should be invalid.
        let old_verify = store.verify_device_token(&token.token).await.unwrap();
        assert!(old_verify.is_none());

        // New token should be valid.
        let new_verify = store.verify_device_token(&new_token.token).await.unwrap();
        assert!(new_verify.is_some());

        // Revoke device.
        store.revoke_token("dev-1").await.unwrap();
        let revoked_verify = store.verify_device_token(&new_token.token).await.unwrap();
        assert!(revoked_verify.is_none());

        // Device should not be listed.
        let devices = store.list_devices().await.unwrap();
        assert!(devices.is_empty());
    }

    #[tokio::test]
    async fn reject_pair_request() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        let req = store
            .request_pair("dev-2", None, "android", None)
            .await
            .unwrap();
        store.reject(&req.id).await.unwrap();

        // Should not be in pending.
        let pending = store.list_pending().await.unwrap();
        assert!(pending.is_empty());

        // Reject again should fail.
        let result = store.reject(&req.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn approve_nonexistent_request() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        let result = store.approve("nonexistent").await;
        assert!(matches!(result, Err(Error::PairRequestNotFound)));
    }

    #[tokio::test]
    async fn verify_invalid_token() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        let result = store.verify_device_token("invalid_token").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn create_device_token_directly() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        // Create a device token without the pairing handshake.
        let token = store
            .create_device_token(Some("My Server"), "linux")
            .await
            .unwrap();
        assert!(token.token.starts_with("mdt_"));
        assert!(!token.scopes.is_empty());

        // Device should be listed.
        let devices = store.list_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, token.device_id);
        assert_eq!(devices[0].display_name.as_deref(), Some("My Server"));
        assert_eq!(devices[0].platform, "linux");

        // Token should verify.
        let verification = store.verify_device_token(&token.token).await.unwrap();
        assert!(verification.is_some());
        let v = verification.unwrap();
        assert_eq!(v.device_id, token.device_id);

        // Can revoke it.
        store.revoke_token(&token.device_id).await.unwrap();
        let revoked_verify = store.verify_device_token(&token.token).await.unwrap();
        assert!(revoked_verify.is_none());
    }

    #[tokio::test]
    async fn rotate_nonexistent_device() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);

        let result = store.rotate_token("nonexistent").await;
        assert!(matches!(result, Err(Error::DeviceNotFound)));
    }
}
