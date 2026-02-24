//! Vault state machine: initialization, seal/unseal, encrypt/decrypt.

use {base64::Engine, sqlx::SqlitePool, tokio::sync::RwLock, zeroize::Zeroizing};

use crate::{
    error::VaultError,
    kdf::{self, KdfParams},
    key_wrap,
    recovery::{self, RecoveryKey},
    traits::Cipher,
    xchacha20::XChaCha20Poly1305Cipher,
};

/// Vault status exposed to the API / frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultStatus {
    /// No password has been set — vault doesn't exist yet.
    Uninitialized,
    /// Vault exists but the DEK is not in memory (needs password).
    Sealed,
    /// Vault is unlocked — DEK is held in memory.
    Unsealed,
}

/// Row from the `vault_metadata` table.
#[derive(Debug)]
struct VaultRow {
    kdf_salt: String,
    kdf_params: String,
    wrapped_dek: String,
    recovery_wrapped_dek: Option<String>,
    #[allow(dead_code)]
    recovery_key_hash: Option<String>,
}

/// Encryption-at-rest vault.
///
/// Generic over [`Cipher`] but defaults to [`XChaCha20Poly1305Cipher`].
/// The DEK is held in memory behind a `RwLock` — `None` means sealed.
pub struct Vault<C: Cipher = XChaCha20Poly1305Cipher> {
    pool: SqlitePool,
    cipher: C,
    dek: RwLock<Option<Zeroizing<[u8; 32]>>>,
}

impl Vault<XChaCha20Poly1305Cipher> {
    /// Create a vault with the default XChaCha20-Poly1305 cipher.
    pub async fn new(pool: SqlitePool) -> Result<Self, VaultError> {
        Self::with_cipher(pool, XChaCha20Poly1305Cipher).await
    }
}

impl<C: Cipher> Vault<C> {
    /// Create a vault with a custom cipher.
    pub async fn with_cipher(pool: SqlitePool, cipher: C) -> Result<Self, VaultError> {
        Ok(Self {
            pool,
            cipher,
            dek: RwLock::new(None),
        })
    }

    /// Query the current vault status.
    pub async fn status(&self) -> Result<VaultStatus, VaultError> {
        let row = self.load_metadata().await?;
        match row {
            None => Ok(VaultStatus::Uninitialized),
            Some(_) => {
                let guard = self.dek.read().await;
                if guard.is_some() {
                    Ok(VaultStatus::Unsealed)
                } else {
                    Ok(VaultStatus::Sealed)
                }
            },
        }
    }

    /// Initialize the vault with a password.
    ///
    /// Generates a random DEK, wraps it with the password-derived KEK,
    /// generates a recovery key, and stores everything in the database.
    /// Returns the recovery key (shown to the user exactly once).
    pub async fn initialize(&self, password: &str) -> Result<RecoveryKey, VaultError> {
        // Ensure vault doesn't already exist.
        if self.load_metadata().await?.is_some() {
            return Err(VaultError::AlreadyInitialized);
        }

        // Generate random DEK.
        let mut dek = Zeroizing::new([0u8; 32]);
        rand::RngCore::fill_bytes(&mut rand::rng(), dek.as_mut());

        // Derive KEK from password.
        let salt_b64 = kdf::generate_salt();
        let salt = kdf::decode_salt(&salt_b64)?;
        let params = KdfParams::default();
        let kek = kdf::derive_key(password.as_bytes(), &salt, &params)?;

        // Wrap DEK with password KEK.
        let wrapped_dek = key_wrap::wrap_dek(&self.cipher, &kek, &dek)?;

        // Generate recovery key and wrap DEK with it.
        let recovery_key = recovery::generate_recovery_key();
        let (recovery_wrapped, recovery_hash) =
            recovery::wrap_with_recovery(&self.cipher, &dek, recovery_key.phrase())?;

        // Store in database.
        let params_json = serde_json::to_string(&params)?;
        sqlx::query(
            "INSERT INTO vault_metadata (id, kdf_salt, kdf_params, wrapped_dek, recovery_wrapped_dek, recovery_key_hash)
             VALUES (1, ?, ?, ?, ?, ?)",
        )
        .bind(&salt_b64)
        .bind(&params_json)
        .bind(&wrapped_dek)
        .bind(&recovery_wrapped)
        .bind(&recovery_hash)
        .execute(&self.pool)
        .await?;

        // Hold DEK in memory (vault is now unsealed).
        *self.dek.write().await = Some(dek);

        #[cfg(feature = "tracing")]
        tracing::info!("vault initialized");

        Ok(recovery_key)
    }

    /// Unseal the vault using a password.
    pub async fn unseal(&self, password: &str) -> Result<(), VaultError> {
        let row = self
            .load_metadata()
            .await?
            .ok_or(VaultError::NotInitialized)?;

        let salt = kdf::decode_salt(&row.kdf_salt)?;
        let params: KdfParams = serde_json::from_str(&row.kdf_params)?;
        let kek = kdf::derive_key(password.as_bytes(), &salt, &params)?;

        let dek = key_wrap::unwrap_dek(&self.cipher, &kek, &row.wrapped_dek)
            .map_err(|_| VaultError::BadCredential)?;

        *self.dek.write().await = Some(dek);

        #[cfg(feature = "tracing")]
        tracing::info!("vault unsealed");

        Ok(())
    }

    /// Unseal the vault using a recovery phrase.
    pub async fn unseal_with_recovery(&self, phrase: &str) -> Result<(), VaultError> {
        let row = self
            .load_metadata()
            .await?
            .ok_or(VaultError::NotInitialized)?;

        let recovery_wrapped = row.recovery_wrapped_dek.ok_or(VaultError::CipherError(
            "no recovery key configured".to_string(),
        ))?;

        let dek = recovery::unwrap_with_recovery(&self.cipher, &recovery_wrapped, phrase)
            .map_err(|_| VaultError::BadCredential)?;

        *self.dek.write().await = Some(dek);

        #[cfg(feature = "tracing")]
        tracing::info!("vault unsealed via recovery key");

        Ok(())
    }

    /// Seal the vault (clear the DEK from memory).
    pub async fn seal(&self) {
        *self.dek.write().await = None;

        #[cfg(feature = "tracing")]
        tracing::info!("vault sealed");
    }

    /// Change the password. Re-wraps the DEK with the new password's KEK.
    ///
    /// The vault must already be unsealed (DEK in memory).
    pub async fn change_password(&self, old: &str, new: &str) -> Result<(), VaultError> {
        let row = self
            .load_metadata()
            .await?
            .ok_or(VaultError::NotInitialized)?;

        // Verify old password by unwrapping DEK.
        let salt = kdf::decode_salt(&row.kdf_salt)?;
        let params: KdfParams = serde_json::from_str(&row.kdf_params)?;
        let old_kek = kdf::derive_key(old.as_bytes(), &salt, &params)?;
        let _dek = key_wrap::unwrap_dek(&self.cipher, &old_kek, &row.wrapped_dek)
            .map_err(|_| VaultError::BadCredential)?;

        // Read current DEK from memory.
        let guard = self.dek.read().await;
        let dek = guard.as_ref().ok_or(VaultError::Sealed)?;

        // Derive new KEK with fresh salt.
        let new_salt_b64 = kdf::generate_salt();
        let new_salt = kdf::decode_salt(&new_salt_b64)?;
        let new_kek = kdf::derive_key(new.as_bytes(), &new_salt, &params)?;

        // Re-wrap DEK.
        let new_wrapped = key_wrap::wrap_dek(&self.cipher, &new_kek, dek)?;
        let params_json = serde_json::to_string(&params)?;

        drop(guard);

        sqlx::query(
            "UPDATE vault_metadata SET kdf_salt = ?, kdf_params = ?, wrapped_dek = ?, updated_at = datetime('now') WHERE id = 1",
        )
        .bind(&new_salt_b64)
        .bind(&params_json)
        .bind(&new_wrapped)
        .execute(&self.pool)
        .await?;

        #[cfg(feature = "tracing")]
        tracing::info!("vault password changed (DEK re-wrapped)");

        Ok(())
    }

    /// Encrypt a string and return a versioned base64 blob.
    ///
    /// The AAD (additional authenticated data) should identify the context,
    /// e.g. `"env:MY_KEY"` or `"provider_keys"`.
    pub async fn encrypt_string(&self, plaintext: &str, aad: &str) -> Result<String, VaultError> {
        let guard = self.dek.read().await;
        let dek = guard.as_ref().ok_or(VaultError::Sealed)?;

        let encrypted = self
            .cipher
            .encrypt(dek, plaintext.as_bytes(), aad.as_bytes())?;

        // Prepend version tag.
        let mut blob = Vec::with_capacity(1 + encrypted.len());
        blob.push(self.cipher.version_tag());
        blob.extend_from_slice(&encrypted);

        Ok(base64::engine::general_purpose::STANDARD.encode(blob))
    }

    /// Decrypt a base64 blob back to a string.
    pub async fn decrypt_string(&self, b64: &str, aad: &str) -> Result<String, VaultError> {
        let guard = self.dek.read().await;
        let dek = guard.as_ref().ok_or(VaultError::Sealed)?;

        let blob = base64::engine::general_purpose::STANDARD.decode(b64)?;
        if blob.is_empty() {
            return Err(VaultError::CipherError("empty blob".to_string()));
        }

        let version = blob[0];
        if version != self.cipher.version_tag() {
            return Err(VaultError::CipherError(format!(
                "unsupported cipher version: {version:#04x}"
            )));
        }

        let plaintext = self.cipher.decrypt(dek, &blob[1..], aad.as_bytes())?;
        String::from_utf8(plaintext).map_err(|e| VaultError::CipherError(e.to_string()))
    }

    /// Whether the vault is currently unsealed (DEK in memory).
    pub async fn is_unsealed(&self) -> bool {
        self.dek.read().await.is_some()
    }

    /// Load vault metadata from the database.
    async fn load_metadata(&self) -> Result<Option<VaultRow>, VaultError> {
        let row: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT kdf_salt, kdf_params, wrapped_dek, recovery_wrapped_dek, recovery_key_hash
                 FROM vault_metadata WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(kdf_salt, kdf_params, wrapped_dek, recovery_wrapped_dek, recovery_key_hash)| {
                VaultRow {
                    kdf_salt,
                    kdf_params,
                    wrapped_dek,
                    recovery_wrapped_dek,
                    recovery_key_hash,
                }
            },
        ))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::xchacha20::XChaCha20Poly1305Cipher};

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS vault_metadata (
                id                   INTEGER PRIMARY KEY CHECK (id = 1),
                version              INTEGER NOT NULL DEFAULT 1,
                kdf_salt             TEXT NOT NULL,
                kdf_params           TEXT NOT NULL,
                wrapped_dek          TEXT NOT NULL,
                recovery_wrapped_dek TEXT,
                recovery_key_hash    TEXT,
                created_at           TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn status_uninitialized() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();
        assert_eq!(vault.status().await.unwrap(), VaultStatus::Uninitialized);
    }

    #[tokio::test]
    async fn initialize_and_unseal() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        let rk = vault.initialize("testpassword123").await.unwrap();
        assert!(!rk.phrase().is_empty());
        assert_eq!(vault.status().await.unwrap(), VaultStatus::Unsealed);

        // Seal.
        vault.seal().await;
        assert_eq!(vault.status().await.unwrap(), VaultStatus::Sealed);

        // Unseal with password.
        vault.unseal("testpassword123").await.unwrap();
        assert_eq!(vault.status().await.unwrap(), VaultStatus::Unsealed);
    }

    #[tokio::test]
    async fn wrong_password_fails() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("correctpassword").await.unwrap();
        vault.seal().await;

        let result = vault.unseal("wrongpassword").await;
        assert!(matches!(result, Err(VaultError::BadCredential)));
    }

    #[tokio::test]
    async fn recovery_key_unseal() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        let rk = vault.initialize("testpassword123").await.unwrap();
        let phrase = rk.phrase().to_string();
        vault.seal().await;

        vault.unseal_with_recovery(&phrase).await.unwrap();
        assert_eq!(vault.status().await.unwrap(), VaultStatus::Unsealed);
    }

    #[tokio::test]
    async fn encrypt_decrypt_string() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("password").await.unwrap();

        let encrypted = vault
            .encrypt_string("my secret api key", "env:OPENAI_API_KEY")
            .await
            .unwrap();
        let decrypted = vault
            .decrypt_string(&encrypted, "env:OPENAI_API_KEY")
            .await
            .unwrap();
        assert_eq!(decrypted, "my secret api key");
    }

    #[tokio::test]
    async fn encrypt_sealed_fails() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("password").await.unwrap();
        vault.seal().await;

        let result = vault.encrypt_string("data", "aad").await;
        assert!(matches!(result, Err(VaultError::Sealed)));
    }

    #[tokio::test]
    async fn wrong_aad_decrypt_fails() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("password").await.unwrap();

        let encrypted = vault.encrypt_string("secret", "env:KEY1").await.unwrap();
        let result = vault.decrypt_string(&encrypted, "env:KEY2").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn change_password() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("oldpass").await.unwrap();

        // Encrypt something with the old key.
        let encrypted = vault.encrypt_string("secret", "test").await.unwrap();

        // Change password.
        vault.change_password("oldpass", "newpass").await.unwrap();

        // Seal and unseal with new password.
        vault.seal().await;
        vault.unseal("newpass").await.unwrap();

        // Old data should still be decryptable (same DEK).
        let decrypted = vault.decrypt_string(&encrypted, "test").await.unwrap();
        assert_eq!(decrypted, "secret");

        // Old password should no longer work.
        vault.seal().await;
        let result = vault.unseal("oldpass").await;
        assert!(matches!(result, Err(VaultError::BadCredential)));
    }

    #[tokio::test]
    async fn double_initialize_fails() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        vault.initialize("pass1").await.unwrap();
        let result = vault.initialize("pass2").await;
        assert!(matches!(result, Err(VaultError::AlreadyInitialized)));
    }

    #[tokio::test]
    async fn unseal_not_initialized_fails() {
        let pool = test_pool().await;
        let vault = Vault::with_cipher(pool, XChaCha20Poly1305Cipher)
            .await
            .unwrap();

        let result = vault.unseal("password").await;
        assert!(matches!(result, Err(VaultError::NotInitialized)));
    }
}
