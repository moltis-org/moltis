use std::sync::atomic::{AtomicBool, Ordering};

use {
    argon2::{
        Argon2,
        password_hash::{
            PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
        },
    },
    secrecy::ExposeSecret,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
    sqlx::SqlitePool,
};

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Password,
    Passkey,
    ApiKey,
    Loopback,
}

/// A verified identity after successful authentication.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub method: AuthMethod,
}

/// A registered passkey entry (for listing in the UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyEntry {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

/// An API key entry (for listing in the UI — never exposes the full key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub id: i64,
    pub label: String,
    pub key_prefix: String,
    pub created_at: String,
    /// Scopes granted to this API key. Empty/None means no access (must specify scopes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

/// Result of verifying an API key, including granted scopes.
#[derive(Debug, Clone)]
pub struct ApiKeyVerification {
    pub key_id: i64,
    /// Scopes granted to this key. Empty means no access (key must specify scopes).
    pub scopes: Vec<String>,
}

/// All valid API key scopes.
pub const VALID_SCOPES: &[&str] = &[
    "operator.admin",
    "operator.read",
    "operator.write",
    "operator.approvals",
    "operator.pairing",
];

/// An environment variable entry (for listing in the UI — never exposes the value).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarEntry {
    pub id: i64,
    pub key: String,
    pub created_at: String,
    pub updated_at: String,
}

// ── Credential store ─────────────────────────────────────────────────────────

/// Single-user credential store backed by SQLite.
pub struct CredentialStore {
    pool: SqlitePool,
    setup_complete: AtomicBool,
    /// When true, auth has been explicitly disabled via "remove all auth".
    /// The middleware and status endpoint treat this as "no auth configured".
    auth_disabled: AtomicBool,
}

impl CredentialStore {
    /// Open a database at the given path, reset all auth, and close it.
    pub async fn reset_from_db_path(db_path: &std::path::Path) -> anyhow::Result<()> {
        let db_url = format!("sqlite:{}", db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        let store = Self::new(pool).await?;
        store.reset_all().await
    }

    /// Create a new store and initialize tables.
    /// Reads `auth.disabled` from the discovered config file.
    pub async fn new(pool: SqlitePool) -> anyhow::Result<Self> {
        let config = moltis_config::discover_and_load();
        Self::with_config(pool, &config.auth).await
    }

    /// Create a new store with explicit auth config (avoids reading from disk).
    pub async fn with_config(
        pool: SqlitePool,
        auth_config: &moltis_config::AuthConfig,
    ) -> anyhow::Result<Self> {
        let store = Self {
            pool,
            setup_complete: AtomicBool::new(false),
            auth_disabled: AtomicBool::new(false),
        };
        store.init().await?;
        let has = store.has_password().await? || store.has_passkeys().await?;
        store.setup_complete.store(has, Ordering::Relaxed);
        store
            .auth_disabled
            .store(auth_config.disabled, Ordering::Relaxed);
        Ok(store)
    }

    /// Initialize auth tables.
    ///
    /// **Note**: Schema is now managed by sqlx migrations. This method is a no-op
    /// when running with the gateway (migrations have already run). It's retained
    /// for standalone tests that use in-memory databases.
    async fn init(&self) -> anyhow::Result<()> {
        // Tables are created by migrations in production. For tests using
        // in-memory databases, create them here.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth_password (
                id            INTEGER PRIMARY KEY CHECK (id = 1),
                password_hash TEXT    NOT NULL,
                created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS passkeys (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                credential_id BLOB    NOT NULL UNIQUE,
                name          TEXT    NOT NULL,
                passkey_data  BLOB    NOT NULL,
                created_at    TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS api_keys (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                label      TEXT    NOT NULL,
                key_hash   TEXT    NOT NULL,
                key_prefix TEXT    NOT NULL,
                created_at TEXT    NOT NULL DEFAULT (datetime('now')),
                revoked_at TEXT,
                scopes     TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth_sessions (
                token      TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS env_variables (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                key        TEXT    NOT NULL UNIQUE,
                value      TEXT    NOT NULL,
                created_at TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── Setup ────────────────────────────────────────────────────────────

    /// Whether initial setup (password creation) has been completed.
    pub fn is_setup_complete(&self) -> bool {
        self.setup_complete.load(Ordering::Relaxed)
    }

    /// Whether authentication has been explicitly disabled via reset.
    pub fn is_auth_disabled(&self) -> bool {
        self.auth_disabled.load(Ordering::Relaxed)
    }

    /// Clear the auth-disabled flag (e.g. after completing localhost setup without a password).
    pub fn clear_auth_disabled(&self) {
        self.auth_disabled.store(false, Ordering::Relaxed);
        let _ = self.persist_auth_disabled(false);
    }

    fn persist_auth_disabled(&self, disabled: bool) -> anyhow::Result<()> {
        moltis_config::update_config(|c| c.auth.disabled = disabled)?;
        Ok(())
    }

    /// Whether a password has been set.
    pub async fn has_password(&self) -> anyhow::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM auth_password WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    // ── Password ─────────────────────────────────────────────────────────

    /// Set the initial password (first-run setup). Fails if already set.
    pub async fn set_initial_password(&self, password: &str) -> anyhow::Result<()> {
        if self.is_setup_complete() {
            anyhow::bail!("password already set");
        }
        let hash = hash_password(password)?;
        sqlx::query("INSERT INTO auth_password (id, password_hash) VALUES (1, ?)")
            .bind(&hash)
            .execute(&self.pool)
            .await?;
        self.setup_complete.store(true, Ordering::Relaxed);
        self.auth_disabled.store(false, Ordering::Relaxed);
        self.persist_auth_disabled(false)?;
        Ok(())
    }

    /// Add a password when none exists yet (e.g. after passkey-only setup).
    ///
    /// Unlike [`set_initial_password`] this does **not** check or modify the
    /// setup-complete flag — it only inserts the password row.
    pub async fn add_password(&self, password: &str) -> anyhow::Result<()> {
        if self.has_password().await? {
            anyhow::bail!("password already set");
        }
        let hash = hash_password(password)?;
        sqlx::query("INSERT INTO auth_password (id, password_hash) VALUES (1, ?)")
            .bind(&hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark initial setup as complete without setting a password (e.g. passkey-only setup).
    ///
    /// Requires at least one credential (password or passkey) to already exist.
    pub async fn mark_setup_complete(&self) -> anyhow::Result<()> {
        let has_password = self.has_password().await?;
        let has_passkeys = self.has_passkeys().await?;
        if !has_password && !has_passkeys {
            anyhow::bail!("cannot mark setup complete without any credentials");
        }
        self.setup_complete.store(true, Ordering::Relaxed);
        self.auth_disabled.store(false, Ordering::Relaxed);
        self.persist_auth_disabled(false)?;
        Ok(())
    }

    /// Verify a password against the stored hash.
    pub async fn verify_password(&self, password: &str) -> anyhow::Result<bool> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT password_hash FROM auth_password WHERE id = 1")
                .fetch_optional(&self.pool)
                .await?;
        let Some((hash,)) = row else {
            return Ok(false);
        };
        Ok(verify_password(password, &hash))
    }

    /// Change the password (requires correct current password).
    pub async fn change_password(&self, current: &str, new_password: &str) -> anyhow::Result<()> {
        if !self.verify_password(current).await? {
            anyhow::bail!("current password is incorrect");
        }
        let hash = hash_password(new_password)?;
        sqlx::query(
            "UPDATE auth_password SET password_hash = ?, updated_at = datetime('now') WHERE id = 1",
        )
        .bind(&hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Sessions ─────────────────────────────────────────────────────────

    /// Create a new session token (30-day expiry).
    pub async fn create_session(&self) -> anyhow::Result<String> {
        let token = generate_token();
        sqlx::query(
            "INSERT INTO auth_sessions (token, expires_at) VALUES (?, datetime('now', '+30 days'))",
        )
        .bind(&token)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Validate a session token. Returns true if valid and not expired.
    pub async fn validate_session(&self, token: &str) -> anyhow::Result<bool> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT token FROM auth_sessions WHERE token = ? AND expires_at > datetime('now')",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Delete a session (logout).
    pub async fn delete_session(&self, token: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clean up expired sessions.
    pub async fn cleanup_expired_sessions(&self) -> anyhow::Result<u64> {
        let result = sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    // ── API Keys ─────────────────────────────────────────────────────────

    /// Generate a new API key with optional scopes. Returns (id, raw_key).
    /// The raw key is only shown once — we store only its SHA-256 hash.
    ///
    /// If `scopes` is None or empty, the key will have no access until scopes are set.
    pub async fn create_api_key(
        &self,
        label: &str,
        scopes: Option<&[String]>,
    ) -> anyhow::Result<(i64, String)> {
        let raw_key = format!("mk_{}", generate_token());
        let prefix = &raw_key[..raw_key.len().min(11)]; // "mk_" + 8 chars
        let hash = sha256_hex(&raw_key);

        // Store scopes as JSON array, or NULL for full access
        let scopes_json = scopes
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::to_string(s).unwrap_or_default());

        let result = sqlx::query(
            "INSERT INTO api_keys (label, key_hash, key_prefix, scopes) VALUES (?, ?, ?, ?)",
        )
        .bind(label)
        .bind(&hash)
        .bind(prefix)
        .bind(&scopes_json)
        .execute(&self.pool)
        .await?;
        Ok((result.last_insert_rowid(), raw_key))
    }

    /// List all API keys (active only, not revoked).
    pub async fn list_api_keys(&self) -> anyhow::Result<Vec<ApiKeyEntry>> {
        let rows: Vec<(i64, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, label, key_prefix, strftime('%Y-%m-%dT%H:%M:%SZ', created_at), scopes FROM api_keys WHERE revoked_at IS NULL ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, label, key_prefix, created_at, scopes_json)| {
                let scopes = scopes_json
                    .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                    .filter(|v| !v.is_empty());
                ApiKeyEntry {
                    id,
                    label,
                    key_prefix,
                    created_at,
                    scopes,
                }
            })
            .collect())
    }

    /// Revoke an API key by id.
    pub async fn revoke_api_key(&self, key_id: i64) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE api_keys SET revoked_at = datetime('now') WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Verify a raw API key. Returns `Some(ApiKeyVerification)` if valid,
    /// `None` if invalid or revoked.
    pub async fn verify_api_key(
        &self,
        raw_key: &str,
    ) -> anyhow::Result<Option<ApiKeyVerification>> {
        let hash = sha256_hex(raw_key);
        let row: Option<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, scopes FROM api_keys WHERE key_hash = ? AND revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(key_id, scopes_json)| {
            let scopes = scopes_json
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                .unwrap_or_default();
            ApiKeyVerification { key_id, scopes }
        }))
    }

    // ── Environment Variables ─────────────────────────────────────────────

    /// List all environment variables (names only, no values).
    pub async fn list_env_vars(&self) -> anyhow::Result<Vec<EnvVarEntry>> {
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id, key, strftime('%Y-%m-%dT%H:%M:%SZ', created_at), strftime('%Y-%m-%dT%H:%M:%SZ', updated_at) FROM env_variables ORDER BY key ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, key, created_at, updated_at)| EnvVarEntry {
                id,
                key,
                created_at,
                updated_at,
            })
            .collect())
    }

    /// Set (upsert) an environment variable.
    pub async fn set_env_var(&self, key: &str, value: &str) -> anyhow::Result<i64> {
        let result = sqlx::query(
            "INSERT INTO env_variables (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// Delete an environment variable by id.
    pub async fn delete_env_var(&self, id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM env_variables WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Get all environment variable key-value pairs (internal use for sandbox injection).
    pub async fn get_all_env_values(&self) -> anyhow::Result<Vec<(String, String)>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM env_variables ORDER BY key ASC")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    // ── Reset (remove all auth) ─────────────────────────────────────────

    /// Remove all authentication data: password, sessions, passkeys, API keys.
    /// After this, `is_setup_complete()` returns false and the middleware
    /// passes all requests through (no auth required).
    pub async fn reset_all(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_password")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM auth_sessions")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM passkeys")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM api_keys")
            .execute(&self.pool)
            .await?;
        self.setup_complete.store(false, Ordering::Relaxed);
        self.auth_disabled.store(true, Ordering::Relaxed);
        self.persist_auth_disabled(true)?;
        Ok(())
    }

    // ── Passkeys ─────────────────────────────────────────────────────────

    /// Store a new passkey credential.
    pub async fn store_passkey(
        &self,
        credential_id: &[u8],
        name: &str,
        passkey_data: &[u8],
    ) -> anyhow::Result<i64> {
        let result = sqlx::query(
            "INSERT INTO passkeys (credential_id, name, passkey_data) VALUES (?, ?, ?)",
        )
        .bind(credential_id)
        .bind(name)
        .bind(passkey_data)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// List all registered passkeys.
    pub async fn list_passkeys(&self) -> anyhow::Result<Vec<PasskeyEntry>> {
        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, name, strftime('%Y-%m-%dT%H:%M:%SZ', created_at) FROM passkeys ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(id, name, created_at)| PasskeyEntry {
                id,
                name,
                created_at,
            })
            .collect())
    }

    /// Remove a passkey by id.
    pub async fn remove_passkey(&self, passkey_id: i64) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM passkeys WHERE id = ?")
            .bind(passkey_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Rename a passkey.
    pub async fn rename_passkey(&self, passkey_id: i64, name: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE passkeys SET name = ? WHERE id = ?")
            .bind(name)
            .bind(passkey_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Load all passkey data blobs (for WebAuthn authentication).
    pub async fn load_all_passkey_data(&self) -> anyhow::Result<Vec<(i64, Vec<u8>)>> {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT id, passkey_data FROM passkeys")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Check if any passkeys are registered (for login page UI).
    pub async fn has_passkeys(&self) -> anyhow::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM passkeys LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }
}

// ── EnvVarProvider impl ─────────────────────────────────────────────────────

#[async_trait::async_trait]
impl moltis_tools::exec::EnvVarProvider for CredentialStore {
    async fn get_env_vars(&self) -> Vec<(String, secrecy::Secret<String>)> {
        self.get_all_env_values()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, secrecy::Secret::new(v)))
            .collect()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

pub fn is_loopback(ip: &str) -> bool {
    ip == "127.0.0.1" || ip.starts_with("127.") || ip == "::1" || ip.starts_with("::ffff:127.")
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?;
    Ok(hash.to_string())
}

fn verify_password(password: &str, hash_str: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash_str) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

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

// ── Legacy compat ────────────────────────────────────────────────────────────

/// Result of an authentication attempt.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub ok: bool,
    pub reason: Option<String>,
}

/// Constant-time string comparison (prevents timing attacks).
fn safe_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a
        .as_bytes()
        .iter()
        .zip(b.as_bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

/// Authenticate an incoming WebSocket connect request against legacy env-var auth.
pub fn authorize_connect(
    auth: &ResolvedAuth,
    provided_token: Option<&str>,
    provided_password: Option<&str>,
    _remote_ip: Option<&str>,
) -> AuthResult {
    match auth.mode {
        AuthMode::Token => {
            let Some(ref expected) = auth.token else {
                return AuthResult {
                    ok: true,
                    reason: None,
                };
            };
            match provided_token {
                Some(t) if safe_equal(t, expected.expose_secret()) => AuthResult {
                    ok: true,
                    reason: None,
                },
                Some(_) => AuthResult {
                    ok: false,
                    reason: Some("invalid token".into()),
                },
                None => AuthResult {
                    ok: false,
                    reason: Some("token required".into()),
                },
            }
        },
        AuthMode::Password => {
            let Some(ref expected) = auth.password else {
                return AuthResult {
                    ok: true,
                    reason: None,
                };
            };
            match provided_password {
                Some(p) if safe_equal(p, expected.expose_secret()) => AuthResult {
                    ok: true,
                    reason: None,
                },
                Some(_) => AuthResult {
                    ok: false,
                    reason: Some("invalid password".into()),
                },
                None => AuthResult {
                    ok: false,
                    reason: Some("password required".into()),
                },
            }
        },
    }
}

/// Legacy resolved auth from environment vars (kept for migration).
#[derive(Clone)]
pub struct ResolvedAuth {
    pub mode: AuthMode,
    pub token: Option<secrecy::Secret<String>>,
    pub password: Option<secrecy::Secret<String>>,
}

impl std::fmt::Debug for ResolvedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAuth")
            .field("mode", &self.mode)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Token,
    Password,
}

/// Resolve auth config from environment / config values.
pub fn resolve_auth(token: Option<String>, password: Option<String>) -> ResolvedAuth {
    let mode = if password.is_some() {
        AuthMode::Password
    } else {
        AuthMode::Token
    };
    ResolvedAuth {
        mode,
        token: token.map(secrecy::Secret::new),
        password: password.map(secrecy::Secret::new),
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_loopback() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.0.2"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("::ffff:127.0.0.1"));
        assert!(!is_loopback("192.168.1.1"));
        assert!(!is_loopback("10.0.0.1"));
    }

    #[test]
    fn test_password_hash_verify() {
        let hash = hash_password("test123").unwrap();
        assert!(verify_password("test123", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn test_generate_token() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
        assert!(t1.len() >= 40);
    }

    #[test]
    fn test_sha256_hex() {
        let h = sha256_hex("hello");
        assert_eq!(h.len(), 64);
        // deterministic
        assert_eq!(h, sha256_hex("hello"));
        assert_ne!(h, sha256_hex("world"));
    }

    #[tokio::test]
    async fn test_credential_store_password() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        assert!(!store.is_setup_complete());
        assert!(!store.verify_password("test").await.unwrap());

        store.set_initial_password("mypassword").await.unwrap();
        assert!(store.is_setup_complete());
        assert!(store.verify_password("mypassword").await.unwrap());
        assert!(!store.verify_password("wrong").await.unwrap());

        // Can't set again
        assert!(store.set_initial_password("another").await.is_err());

        // Change password
        store
            .change_password("mypassword", "newpass")
            .await
            .unwrap();
        assert!(store.verify_password("newpass").await.unwrap());
        assert!(!store.verify_password("mypassword").await.unwrap());

        // Wrong current password
        assert!(store.change_password("wrong", "x").await.is_err());
    }

    #[tokio::test]
    async fn test_credential_store_sessions() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        let token = store.create_session().await.unwrap();
        assert!(store.validate_session(&token).await.unwrap());
        assert!(!store.validate_session("bogus").await.unwrap());

        store.delete_session(&token).await.unwrap();
        assert!(!store.validate_session(&token).await.unwrap());
    }

    #[tokio::test]
    async fn test_credential_store_api_keys() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        // Create API key without scopes (full access)
        let (id, raw_key) = store.create_api_key("test key", None).await.unwrap();
        assert!(id > 0);
        assert!(raw_key.starts_with("mk_"));

        // Verify returns Some with empty scopes (no access — key must specify scopes)
        let verification = store.verify_api_key(&raw_key).await.unwrap();
        assert!(verification.is_some());
        let v = verification.unwrap();
        assert_eq!(v.key_id, id);
        assert!(v.scopes.is_empty()); // empty = no access

        // Invalid key returns None
        assert!(store.verify_api_key("mk_bogus").await.unwrap().is_none());

        let keys = store.list_api_keys().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "test key");
        assert!(keys[0].scopes.is_none()); // None = full access

        store.revoke_api_key(id).await.unwrap();
        assert!(store.verify_api_key(&raw_key).await.unwrap().is_none());

        let keys = store.list_api_keys().await.unwrap();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn test_credential_store_api_keys_with_scopes() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        // Create API key with specific scopes
        let scopes = vec!["operator.read".to_string(), "operator.write".to_string()];
        let (id, raw_key) = store
            .create_api_key("scoped key", Some(&scopes))
            .await
            .unwrap();
        assert!(id > 0);

        // Verify returns the scopes
        let verification = store.verify_api_key(&raw_key).await.unwrap();
        assert!(verification.is_some());
        let v = verification.unwrap();
        assert_eq!(v.key_id, id);
        assert_eq!(v.scopes, scopes);

        // List includes scopes
        let keys = store.list_api_keys().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].scopes, Some(scopes.clone()));

        // Create another key without scopes for comparison
        let (id2, raw_key2) = store.create_api_key("full access key", None).await.unwrap();

        let keys = store.list_api_keys().await.unwrap();
        assert_eq!(keys.len(), 2);

        // Find each key and verify scopes
        let scoped = keys.iter().find(|k| k.id == id).unwrap();
        let full = keys.iter().find(|k| k.id == id2).unwrap();
        assert_eq!(scoped.scopes, Some(scopes));
        assert!(full.scopes.is_none()); // None = no scopes specified (no access)

        // Verify both keys work
        assert!(store.verify_api_key(&raw_key).await.unwrap().is_some());
        assert!(store.verify_api_key(&raw_key2).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_credential_store_reset_all() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        // Set up password, session, API key, passkey.
        store.set_initial_password("testpass").await.unwrap();
        assert!(store.is_setup_complete());

        let token = store.create_session().await.unwrap();
        assert!(store.validate_session(&token).await.unwrap());

        let (_id, raw_key) = store.create_api_key("test", None).await.unwrap();
        assert!(store.verify_api_key(&raw_key).await.unwrap().is_some());

        store
            .store_passkey(b"cred-1", "test pk", b"data")
            .await
            .unwrap();
        assert!(store.has_passkeys().await.unwrap());

        // Reset everything.
        store.reset_all().await.unwrap();

        assert!(store.is_auth_disabled());
        assert!(!store.is_setup_complete());
        assert!(!store.validate_session(&token).await.unwrap());
        assert!(store.verify_api_key(&raw_key).await.unwrap().is_none());
        assert!(!store.has_passkeys().await.unwrap());
        assert!(!store.verify_password("testpass").await.unwrap());

        // Can set up again — re-enables auth.
        store.set_initial_password("newpass").await.unwrap();
        assert!(store.is_setup_complete());
        assert!(!store.is_auth_disabled());
    }

    #[tokio::test]
    async fn test_auth_disabled_persists_across_restart() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool.clone()).await.unwrap();

        store.set_initial_password("testpass").await.unwrap();
        store.reset_all().await.unwrap();
        assert!(store.is_auth_disabled());

        // Simulate restart: create a new CredentialStore from the same DB.
        let store2 = CredentialStore::new(pool.clone()).await.unwrap();
        assert!(store2.is_auth_disabled());
        assert!(!store2.is_setup_complete());

        // Re-enable auth.
        store2.set_initial_password("newpass").await.unwrap();

        // Another restart: disabled flag should be cleared.
        let store3 = CredentialStore::new(pool).await.unwrap();
        assert!(!store3.is_auth_disabled());
        assert!(store3.is_setup_complete());
    }

    #[tokio::test]
    async fn test_credential_store_env_vars() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        // Empty initially.
        let vars = store.list_env_vars().await.unwrap();
        assert!(vars.is_empty());

        // Set a variable.
        let id = store.set_env_var("MY_KEY", "secret123").await.unwrap();
        assert!(id > 0);

        let vars = store.list_env_vars().await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "MY_KEY");

        // Values returned by get_all_env_values.
        let values = store.get_all_env_values().await.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], ("MY_KEY".into(), "secret123".into()));

        // Upsert overwrites.
        store.set_env_var("MY_KEY", "updated").await.unwrap();
        let values = store.get_all_env_values().await.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].1, "updated");

        // Add a second variable.
        store.set_env_var("OTHER", "val").await.unwrap();
        let vars = store.list_env_vars().await.unwrap();
        assert_eq!(vars.len(), 2);

        // Delete by id.
        let first_id = vars.iter().find(|v| v.key == "MY_KEY").unwrap().id;
        store.delete_env_var(first_id).await.unwrap();
        let vars = store.list_env_vars().await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "OTHER");
    }

    #[tokio::test]
    async fn test_credential_store_passkeys() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        assert!(!store.has_passkeys().await.unwrap());

        let cred_id = b"credential-123";
        let data = b"serialized-passkey-data";
        let id = store
            .store_passkey(cred_id, "MacBook Touch ID", data)
            .await
            .unwrap();
        assert!(id > 0);

        assert!(store.has_passkeys().await.unwrap());

        let entries = store.list_passkeys().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "MacBook Touch ID");

        let all_data = store.load_all_passkey_data().await.unwrap();
        assert_eq!(all_data.len(), 1);
        assert_eq!(all_data[0].1, data);

        store.remove_passkey(id).await.unwrap();
        assert!(!store.has_passkeys().await.unwrap());
    }

    #[tokio::test]
    async fn test_mark_setup_complete_with_passkey_only() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool).await.unwrap();

        assert!(!store.is_setup_complete());

        // Cannot mark complete without any credentials.
        assert!(store.mark_setup_complete().await.is_err());

        // Store a passkey, then mark complete.
        store
            .store_passkey(b"cred-1", "My Passkey", b"data")
            .await
            .unwrap();
        store.mark_setup_complete().await.unwrap();
        assert!(store.is_setup_complete());
        assert!(!store.is_auth_disabled());
    }

    #[tokio::test]
    async fn test_setup_complete_persists_with_passkey_only() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = CredentialStore::new(pool.clone()).await.unwrap();

        store
            .store_passkey(b"cred-1", "My Passkey", b"data")
            .await
            .unwrap();
        store.mark_setup_complete().await.unwrap();
        assert!(store.is_setup_complete());

        // Simulate restart: create a new store from the same DB.
        let store2 = CredentialStore::new(pool).await.unwrap();
        assert!(store2.is_setup_complete());
    }
}
