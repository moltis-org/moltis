use std::sync::atomic::Ordering;

#[cfg(feature = "vault")]
use std::sync::Arc;

use sqlx::SqlitePool;

#[cfg(feature = "vault")]
use moltis_vault::Vault;

use crate::{
    Error, Result,
    credential_store::{
        CredentialStore,
        util::{DUMMY_ARGON2_HASH, generate_token, hash_password, verify_password},
    },
};

impl CredentialStore {
    /// Maximum number of concurrent active sessions. Oldest sessions are evicted when the cap is reached.
    const MAX_SESSIONS: i64 = 10;

    /// Open a database at the given path, reset all auth, and close it.
    pub async fn reset_from_db_path(db_path: &std::path::Path) -> Result<()> {
        let db_url = format!("sqlite:{}", db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        let store = Self::new(pool).await?;
        store.reset_all().await
    }

    /// Create a new store and initialize tables.
    /// Reads `auth.disabled` from the discovered config file.
    pub async fn new(pool: SqlitePool) -> Result<Self> {
        let config = moltis_config::discover_and_load();
        Self::with_config(pool, &config.auth).await
    }

    /// Create a new store with explicit auth config (avoids reading from disk).
    pub async fn with_config(
        pool: SqlitePool,
        auth_config: &moltis_config::AuthConfig,
    ) -> Result<Self> {
        let store = Self {
            pool,
            setup_complete: std::sync::atomic::AtomicBool::new(false),
            auth_disabled: std::sync::atomic::AtomicBool::new(false),
            #[cfg(feature = "vault")]
            vault: None,
        };
        store.init().await?;
        let has = store.has_password().await? || store.has_passkeys().await?;
        store.setup_complete.store(has, Ordering::Relaxed);
        sqlx::query(
            "INSERT OR IGNORE INTO auth_state (id, auth_disabled, updated_at) VALUES (1, ?, datetime('now'))",
        )
        .bind(if auth_config.disabled { 1_i64 } else { 0_i64 })
        .execute(&store.pool)
        .await?;
        let db_disabled: Option<(i64,)> =
            sqlx::query_as("SELECT auth_disabled FROM auth_state WHERE id = 1")
                .fetch_optional(&store.pool)
                .await?;
        let disabled = db_disabled.map_or(auth_config.disabled, |(value,)| value != 0);
        store.auth_disabled.store(disabled, Ordering::Relaxed);
        Ok(store)
    }

    /// Create a new store with vault support for encrypting environment variables.
    #[cfg(feature = "vault")]
    pub async fn with_vault(
        pool: SqlitePool,
        auth_config: &moltis_config::AuthConfig,
        vault: Option<Arc<Vault>>,
    ) -> Result<Self> {
        let store = Self {
            pool,
            setup_complete: std::sync::atomic::AtomicBool::new(false),
            auth_disabled: std::sync::atomic::AtomicBool::new(false),
            vault,
        };
        store.init().await?;
        let has = store.has_password().await? || store.has_passkeys().await?;
        store.setup_complete.store(has, Ordering::Relaxed);
        sqlx::query(
            "INSERT OR IGNORE INTO auth_state (id, auth_disabled, updated_at) VALUES (1, ?, datetime('now'))",
        )
        .bind(if auth_config.disabled { 1_i64 } else { 0_i64 })
        .execute(&store.pool)
        .await?;
        let db_disabled: Option<(i64,)> =
            sqlx::query_as("SELECT auth_disabled FROM auth_state WHERE id = 1")
                .fetch_optional(&store.pool)
                .await?;
        let disabled = db_disabled.map_or(auth_config.disabled, |(value,)| value != 0);
        store.auth_disabled.store(disabled, Ordering::Relaxed);
        Ok(store)
    }

    /// Initialize auth tables.
    ///
    /// **Note**: Schema is now managed by sqlx migrations. This method is a no-op
    /// when running with the gateway (migrations have already run). It's retained
    /// for standalone tests that use in-memory databases.
    async fn init(&self) -> Result<()> {
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
                scopes     TEXT,
                key_salt   TEXT
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
            "CREATE TABLE IF NOT EXISTS auth_audit_log (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT    NOT NULL,
                client_ip  TEXT,
                detail     TEXT,
                created_at TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS env_variables (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                key        TEXT    NOT NULL UNIQUE,
                value      TEXT    NOT NULL,
                encrypted  INTEGER NOT NULL DEFAULT 0,
                created_at TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS ssh_keys (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT    NOT NULL UNIQUE,
                private_key TEXT    NOT NULL,
                public_key  TEXT    NOT NULL,
                fingerprint TEXT    NOT NULL,
                encrypted   INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS ssh_targets (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                label       TEXT    NOT NULL UNIQUE,
                target      TEXT    NOT NULL,
                port        INTEGER,
                known_host  TEXT,
                auth_mode   TEXT    NOT NULL DEFAULT 'system',
                key_id      INTEGER,
                is_default  INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(key_id) REFERENCES ssh_keys(id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth_state (
                id            INTEGER PRIMARY KEY CHECK (id = 1),
                auth_disabled INTEGER NOT NULL DEFAULT 0,
                updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Whether initial setup (password creation) has been completed.
    pub fn is_setup_complete(&self) -> bool {
        self.setup_complete.load(Ordering::Relaxed)
    }

    /// Whether authentication has been explicitly disabled via reset.
    pub fn is_auth_disabled(&self) -> bool {
        self.auth_disabled.load(Ordering::Relaxed)
    }

    /// Clear the auth-disabled flag (e.g. after completing localhost setup without a password).
    pub async fn clear_auth_disabled(&self) -> Result<()> {
        self.auth_disabled.store(false, Ordering::Relaxed);
        self.persist_auth_disabled(false).await
    }

    async fn persist_auth_disabled(&self, disabled: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO auth_state (id, auth_disabled, updated_at)
             VALUES (1, ?, datetime('now'))
             ON CONFLICT(id) DO UPDATE
             SET auth_disabled = excluded.auth_disabled, updated_at = excluded.updated_at",
        )
        .bind(if disabled {
            1_i64
        } else {
            0_i64
        })
        .execute(&self.pool)
        .await?;
        moltis_config::update_config(|c| c.auth.disabled = disabled)?;
        Ok(())
    }

    /// Whether a password has been set.
    pub async fn has_password(&self) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM auth_password WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Set the initial password (first-run setup). Fails if already set.
    pub async fn set_initial_password(&self, password: &str) -> Result<()> {
        if self.is_setup_complete() {
            return Err(Error::Validation("password already set".into()));
        }
        let hash = hash_password(password)?;
        sqlx::query("INSERT INTO auth_password (id, password_hash) VALUES (1, ?)")
            .bind(&hash)
            .execute(&self.pool)
            .await?;
        self.setup_complete.store(true, Ordering::Relaxed);
        self.auth_disabled.store(false, Ordering::Relaxed);
        self.persist_auth_disabled(false).await?;
        Ok(())
    }

    /// Add a password when none exists yet (e.g. after passkey-only setup).
    ///
    /// This marks setup complete so auth is enforced immediately.
    pub async fn add_password(&self, password: &str) -> Result<()> {
        if self.has_password().await? {
            return Err(Error::Validation("password already set".into()));
        }
        let hash = hash_password(password)?;
        sqlx::query("INSERT INTO auth_password (id, password_hash) VALUES (1, ?)")
            .bind(&hash)
            .execute(&self.pool)
            .await?;
        self.mark_setup_complete().await?;
        Ok(())
    }

    /// Mark initial setup as complete without setting a password (e.g. passkey-only setup).
    ///
    /// Requires at least one credential (password or passkey) to already exist.
    pub async fn mark_setup_complete(&self) -> Result<()> {
        let has_password = self.has_password().await?;
        let has_passkeys = self.has_passkeys().await?;
        if !has_password && !has_passkeys {
            return Err(Error::Validation(
                "cannot mark setup complete without any credentials".into(),
            ));
        }
        self.setup_complete.store(true, Ordering::Relaxed);
        self.auth_disabled.store(false, Ordering::Relaxed);
        self.persist_auth_disabled(false).await?;
        Ok(())
    }

    /// Recompute `setup_complete` from the current credentials in the database.
    pub(crate) async fn recompute_setup_complete(&self) -> Result<()> {
        let has = self.has_password().await? || self.has_passkeys().await?;
        self.setup_complete.store(has, Ordering::Relaxed);
        Ok(())
    }

    /// Verify a password against the stored hash.
    ///
    /// When no password is set, a dummy Argon2 verification is performed
    /// to prevent timing side channels that would reveal whether a password exists.
    pub async fn verify_password(&self, password: &str) -> Result<bool> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT password_hash FROM auth_password WHERE id = 1")
                .fetch_optional(&self.pool)
                .await?;
        let hash = match row {
            Some((h,)) => h,
            None => {
                let _ = verify_password(password, DUMMY_ARGON2_HASH);
                return Ok(false);
            },
        };
        Ok(verify_password(password, &hash))
    }

    /// Change the password (requires correct current password).
    ///
    /// Invalidates all existing sessions for defense-in-depth.
    pub async fn change_password(&self, current: &str, new_password: &str) -> Result<()> {
        if !self.verify_password(current).await? {
            return Err(Error::Validation("current password is incorrect".into()));
        }
        let hash = hash_password(new_password)?;
        sqlx::query(
            "UPDATE auth_password SET password_hash = ?, updated_at = datetime('now') WHERE id = 1",
        )
        .bind(&hash)
        .execute(&self.pool)
        .await?;

        sqlx::query("DELETE FROM auth_sessions")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Create a new session token (30-day expiry).
    ///
    /// Enforces a cap of [`MAX_SESSIONS`] active (non-expired) sessions.
    /// When the cap is reached, the oldest sessions are deleted to make room.
    pub async fn create_session(&self) -> Result<String> {
        sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM auth_sessions")
            .fetch_one(&self.pool)
            .await?;
        if count.0 >= Self::MAX_SESSIONS {
            let to_delete = count.0 - Self::MAX_SESSIONS + 1;
            sqlx::query(
                "DELETE FROM auth_sessions WHERE token IN (SELECT token FROM auth_sessions ORDER BY created_at ASC LIMIT ?)",
            )
            .bind(to_delete)
            .execute(&self.pool)
            .await?;
        }

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
    pub async fn validate_session(&self, token: &str) -> Result<bool> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT token FROM auth_sessions WHERE token = ? AND expires_at > datetime('now')",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Delete a session (logout).
    pub async fn delete_session(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clean up expired sessions.
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Remove all authentication data: password, sessions, passkeys, API keys.
    /// After this, `is_setup_complete()` returns false and the middleware passes all requests through.
    pub async fn reset_all(&self) -> Result<()> {
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
        sqlx::query("DELETE FROM ssh_targets")
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM ssh_keys")
            .execute(&self.pool)
            .await?;
        self.setup_complete.store(false, Ordering::Relaxed);
        self.auth_disabled.store(true, Ordering::Relaxed);
        self.persist_auth_disabled(true).await?;
        Ok(())
    }

    /// Get a reference to the vault (if configured).
    #[cfg(feature = "vault")]
    pub fn vault(&self) -> Option<&Arc<Vault>> {
        self.vault.as_ref()
    }

    /// Get a reference to the underlying database pool.
    pub fn db_pool(&self) -> &SqlitePool {
        &self.pool
    }
}
