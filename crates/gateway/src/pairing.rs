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

/// Result of a TOFU key pinning check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPinningResult {
    /// Presented key matches the stored (pinned) key.
    Match,
    /// A different key is already pinned for this device.
    Mismatch { expected: String },
    /// No key is pinned yet (new device or no stored key).
    NoPinnedKey,
    /// The device has been revoked — reject regardless of key.
    Revoked,
}

/// Result of pinning a public key to an existing active device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinPublicKeyResult {
    /// The device had no key and this call pinned it.
    Pinned,
    /// The same key was already pinned.
    AlreadyPinned,
    /// A different key is already pinned for this device.
    Mismatch { expected: String },
    /// The device has been revoked.
    Revoked,
    /// No device exists for the supplied ID.
    DeviceNotFound,
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

    /// Look up an active paired device by its Ed25519 public key (base64).
    ///
    /// Returns `None` if no device is associated with this key, or if the
    /// device has been revoked.
    pub async fn find_device_by_public_key(
        &self,
        public_key: &str,
    ) -> Result<Option<PairedDevice>> {
        let row: Option<(String, Option<String>, String, Option<String>, String)> = sqlx::query_as(
            "SELECT device_id, display_name, platform, public_key, created_at
                 FROM paired_devices
                 WHERE public_key = ? AND status = 'active'",
        )
        .bind(public_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(device_id, display_name, platform, public_key, created_at)| PairedDevice {
                device_id,
                display_name,
                platform,
                public_key,
                created_at,
            },
        ))
    }

    /// Check whether a public key has been revoked (not just absent).
    pub async fn is_public_key_revoked(&self, public_key: &str) -> Result<bool> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT status FROM paired_devices WHERE public_key = ? AND status = 'revoked'",
        )
        .bind(public_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Pin a public key to an existing device (used during token→key migration).
    /// Only sets the key if the device has no key pinned yet, and reports
    /// conflicts explicitly so callers do not mistake a zero-row update for a
    /// successful migration.
    pub async fn pin_public_key(
        &self,
        device_id: &str,
        public_key: &str,
    ) -> Result<PinPublicKeyResult> {
        let row: Option<(Option<String>, String)> =
            sqlx::query_as("SELECT public_key, status FROM paired_devices WHERE device_id = ?")
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            None => Ok(PinPublicKeyResult::DeviceNotFound),
            Some((_, status)) if status == "revoked" => Ok(PinPublicKeyResult::Revoked),
            Some((Some(stored_key), status)) if status == "active" => {
                if stored_key == public_key {
                    Ok(PinPublicKeyResult::AlreadyPinned)
                } else {
                    Ok(PinPublicKeyResult::Mismatch {
                        expected: stored_key,
                    })
                }
            },
            Some((None, status)) if status == "active" => {
                let result = sqlx::query(
                    "UPDATE paired_devices SET public_key = ?
                     WHERE device_id = ? AND status = 'active' AND public_key IS NULL",
                )
                .bind(public_key)
                .bind(device_id)
                .execute(&self.pool)
                .await?;

                if result.rows_affected() > 0 {
                    Ok(PinPublicKeyResult::Pinned)
                } else {
                    let refreshed: Option<(Option<String>, String)> = sqlx::query_as(
                        "SELECT public_key, status FROM paired_devices WHERE device_id = ?",
                    )
                    .bind(device_id)
                    .fetch_optional(&self.pool)
                    .await?;
                    match refreshed {
                        Some((Some(stored_key), status))
                            if status == "active" && stored_key == public_key =>
                        {
                            Ok(PinPublicKeyResult::AlreadyPinned)
                        },
                        Some((Some(stored_key), status)) if status == "active" => {
                            Ok(PinPublicKeyResult::Mismatch {
                                expected: stored_key,
                            })
                        },
                        Some((_, status)) if status == "revoked" => Ok(PinPublicKeyResult::Revoked),
                        None => Ok(PinPublicKeyResult::DeviceNotFound),
                        Some(..) => Ok(PinPublicKeyResult::DeviceNotFound),
                    }
                }
            },
            Some(..) => Ok(PinPublicKeyResult::DeviceNotFound),
        }
    }

    /// TOFU key pinning check: verify that a device's presented public key
    /// matches its stored key. Returns `Ok(true)` if the keys match or no key
    /// is pinned yet, `Ok(false)` if a different key is already pinned, and
    /// the expected fingerprint for the security warning.
    pub async fn check_key_pinning(
        &self,
        device_id: &str,
        presented_key: &str,
    ) -> Result<KeyPinningResult> {
        let row: Option<(Option<String>, String)> =
            sqlx::query_as("SELECT public_key, status FROM paired_devices WHERE device_id = ?")
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some((Some(stored_key), status)) if status == "active" => {
                if stored_key == presented_key {
                    Ok(KeyPinningResult::Match)
                } else {
                    Ok(KeyPinningResult::Mismatch {
                        expected: stored_key,
                    })
                }
            },
            Some((None, status)) if status == "active" => Ok(KeyPinningResult::NoPinnedKey),
            Some((_, status)) if status == "revoked" => Ok(KeyPinningResult::Revoked),
            Some(..) => Ok(KeyPinningResult::NoPinnedKey),
            None => Ok(KeyPinningResult::NoPinnedKey), // unknown device
        }
    }

    /// Get the current status of a pair request by ID.
    pub async fn get_pair_status(&self, pair_id: &str) -> Result<Option<PairStatus>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM pair_requests WHERE id = ?")
                .bind(pair_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(s,)| PairStatus::from_str(&s)))
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
    use {base64::Engine, rand::Rng};

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

/// Verify an Ed25519 challenge-response: check that `signature` is a valid
/// signature over `nonce` by the holder of `public_key_b64`.
///
/// All three values are base64-encoded. Returns `Ok(true)` on valid signature,
/// `Ok(false)` on invalid, `Err` on malformed inputs.
pub fn verify_ed25519_challenge(
    public_key_b64: &str,
    nonce: &[u8],
    signature_b64: &str,
) -> std::result::Result<bool, String> {
    use {base64::Engine, ed25519_dalek::Verifier};

    let pk_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| format!("invalid public key base64: {e}"))?;

    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| "public key must be exactly 32 bytes".to_string())?;

    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| format!("invalid Ed25519 public key: {e}"))?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64)
        .map_err(|e| format!("invalid signature base64: {e}"))?;

    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| "signature must be exactly 64 bytes".to_string())?;

    let signature = ed25519_dalek::Signature::from_bytes(&sig_array);

    Ok(verifying_key.verify(nonce, &signature).is_ok())
}

/// Generate a 32-byte cryptographic challenge nonce, returned as base64.
pub fn generate_challenge_nonce() -> (Vec<u8>, String) {
    use {base64::Engine, rand::Rng};

    let mut nonce = [0u8; 32];
    rand::rng().fill_bytes(&mut nonce);
    let b64 = base64::engine::general_purpose::STANDARD.encode(nonce);
    (nonce.to_vec(), b64)
}

/// Compute a fingerprint from a base64-encoded Ed25519 public key:
/// `SHA256:<base64 of SHA-256 hash>`.
pub fn public_key_fingerprint(public_key_b64: &str) -> std::result::Result<String, String> {
    use base64::Engine;

    let pk_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| format!("invalid public key base64: {e}"))?;
    let pk_array: [u8; 32] = pk_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "public key must be exactly 32 bytes".to_string())?;
    ed25519_dalek::VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| format!("invalid Ed25519 public key: {e}"))?;
    let hash = Sha256::digest(&pk_bytes);
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD.encode(hash)
    ))
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

    // ── Ed25519 identity tests ──────────────────────────────────────────

    fn make_test_keypair() -> (String, ed25519_dalek::SigningKey) {
        use {base64::Engine, rand::Rng};

        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let pk_b64 = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().as_bytes());
        (pk_b64, signing_key)
    }

    #[tokio::test]
    async fn find_device_by_public_key_lifecycle() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);
        let (pk_b64, _) = make_test_keypair();

        // Not found before pairing.
        assert!(
            store
                .find_device_by_public_key(&pk_b64)
                .await
                .unwrap()
                .is_none()
        );

        // Pair with public key.
        let req = store
            .request_pair("dev-pk-1", Some("Key Node"), "linux", Some(&pk_b64))
            .await
            .unwrap();
        store.approve(&req.id).await.unwrap();

        // Now found.
        let device = store
            .find_device_by_public_key(&pk_b64)
            .await
            .unwrap()
            .expect("device should be found");
        assert_eq!(device.device_id, "dev-pk-1");
        assert_eq!(device.public_key.as_deref(), Some(pk_b64.as_str()));

        // Revoke and verify it's no longer found.
        store.revoke_token("dev-pk-1").await.unwrap();
        assert!(
            store
                .find_device_by_public_key(&pk_b64)
                .await
                .unwrap()
                .is_none()
        );

        // But it IS marked as revoked.
        assert!(store.is_public_key_revoked(&pk_b64).await.unwrap());
    }

    #[test]
    fn ed25519_challenge_response_valid() {
        use {base64::Engine, ed25519_dalek::Signer};

        let (pk_b64, signing_key) = make_test_keypair();
        let (nonce, _nonce_b64) = generate_challenge_nonce();

        let signature = signing_key.sign(&nonce);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        assert!(verify_ed25519_challenge(&pk_b64, &nonce, &sig_b64).unwrap());
    }

    #[test]
    fn ed25519_challenge_response_wrong_key() {
        use {base64::Engine, ed25519_dalek::Signer};

        let (_pk_b64, signing_key) = make_test_keypair();
        let (other_pk_b64, _) = make_test_keypair();
        let (nonce, _) = generate_challenge_nonce();

        let signature = signing_key.sign(&nonce);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        // Verify with the wrong public key should fail.
        assert!(!verify_ed25519_challenge(&other_pk_b64, &nonce, &sig_b64).unwrap());
    }

    #[test]
    fn ed25519_challenge_response_wrong_nonce() {
        use {base64::Engine, ed25519_dalek::Signer};

        let (pk_b64, signing_key) = make_test_keypair();
        let (nonce, _) = generate_challenge_nonce();
        let (wrong_nonce, _) = generate_challenge_nonce();

        let signature = signing_key.sign(&nonce);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

        // Verify with wrong nonce should fail.
        assert!(!verify_ed25519_challenge(&pk_b64, &wrong_nonce, &sig_b64).unwrap());
    }

    #[tokio::test]
    async fn key_pinning_enforcement() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);
        let (pk_b64, _) = make_test_keypair();
        let (other_pk_b64, _) = make_test_keypair();

        // No pinned key for unknown device.
        assert_eq!(
            store.check_key_pinning("dev-pin-1", &pk_b64).await.unwrap(),
            KeyPinningResult::NoPinnedKey
        );

        // Pair with first key.
        let req = store
            .request_pair("dev-pin-1", Some("Pinned Node"), "linux", Some(&pk_b64))
            .await
            .unwrap();
        store.approve(&req.id).await.unwrap();

        // Same key should match.
        assert_eq!(
            store.check_key_pinning("dev-pin-1", &pk_b64).await.unwrap(),
            KeyPinningResult::Match
        );

        // Different key should be rejected.
        match store
            .check_key_pinning("dev-pin-1", &other_pk_b64)
            .await
            .unwrap()
        {
            KeyPinningResult::Mismatch { expected } => {
                assert_eq!(expected, pk_b64);
            },
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pin_public_key_reports_actual_outcome() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);
        let (pk_b64, _) = make_test_keypair();
        let (other_pk_b64, _) = make_test_keypair();

        assert_eq!(
            store
                .pin_public_key("missing-device", &pk_b64)
                .await
                .unwrap(),
            PinPublicKeyResult::DeviceNotFound
        );

        let token = store
            .create_device_token(Some("Migrating Node"), "linux")
            .await
            .unwrap();

        assert_eq!(
            store
                .pin_public_key(&token.device_id, &pk_b64)
                .await
                .unwrap(),
            PinPublicKeyResult::Pinned
        );
        assert_eq!(
            store
                .pin_public_key(&token.device_id, &pk_b64)
                .await
                .unwrap(),
            PinPublicKeyResult::AlreadyPinned
        );

        match store
            .pin_public_key(&token.device_id, &other_pk_b64)
            .await
            .unwrap()
        {
            PinPublicKeyResult::Mismatch { expected } => assert_eq!(expected, pk_b64),
            other => panic!("expected Mismatch, got {other:?}"),
        }

        store.revoke_token(&token.device_id).await.unwrap();
        assert_eq!(
            store
                .pin_public_key(&token.device_id, &pk_b64)
                .await
                .unwrap(),
            PinPublicKeyResult::Revoked
        );
    }

    #[tokio::test]
    async fn key_pinning_reports_revoked_device() {
        let pool = test_pool().await;
        let store = PairingStore::new(pool);
        let (pk_b64, _) = make_test_keypair();

        let req = store
            .request_pair(
                "dev-revoked-pin",
                Some("Pinned Node"),
                "linux",
                Some(&pk_b64),
            )
            .await
            .unwrap();
        store.approve(&req.id).await.unwrap();
        store.revoke_token("dev-revoked-pin").await.unwrap();

        assert_eq!(
            store
                .check_key_pinning("dev-revoked-pin", &pk_b64)
                .await
                .unwrap(),
            KeyPinningResult::Revoked
        );
    }

    #[test]
    fn public_key_fingerprint_format() {
        let (pk_b64, _) = make_test_keypair();
        let fp = public_key_fingerprint(&pk_b64).unwrap();
        assert!(fp.starts_with("SHA256:"));
        assert_eq!(fp.len(), 7 + 44); // SHA256: prefix + 44 base64 chars
    }

    #[test]
    fn public_key_fingerprint_rejects_malformed_keys() {
        use base64::Engine;

        assert!(public_key_fingerprint("not base64").is_err());
        assert!(public_key_fingerprint("").is_err());

        let too_short = base64::engine::general_purpose::STANDARD.encode([1u8; 31]);
        assert!(public_key_fingerprint(&too_short).is_err());
    }
}
