#[cfg(feature = "vault")]
use std::sync::Arc;

#[cfg(feature = "vault")]
use moltis_vault::Vault;

#[cfg(feature = "vault")]
use crate::Error;
use crate::{
    Result,
    credential_store::{CredentialStore, EnvVarEntry},
};

impl CredentialStore {
    /// List all environment variables (names only, no values).
    pub async fn list_env_vars(&self) -> Result<Vec<EnvVarEntry>> {
        let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
            "SELECT id, key, strftime('%Y-%m-%dT%H:%M:%SZ', created_at), strftime('%Y-%m-%dT%H:%M:%SZ', updated_at), COALESCE(encrypted, 0) FROM env_variables ORDER BY key ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, key, created_at, updated_at, encrypted)| EnvVarEntry {
                id,
                key,
                created_at,
                updated_at,
                encrypted: encrypted != 0,
            })
            .collect())
    }

    /// Set (upsert) an environment variable.
    ///
    /// When the vault feature is enabled and the vault is unsealed, the value is encrypted before storage.
    pub async fn set_env_var(&self, key: &str, value: &str) -> Result<i64> {
        #[cfg(feature = "vault")]
        let (store_value, encrypted) = {
            if let Some(ref vault) = self.vault {
                if vault.is_unsealed().await {
                    let aad = format!("env:{key}");
                    let enc = vault
                        .encrypt_string(value, &aad)
                        .await
                        .map_err(|e| Error::Crypto(e.to_string()))?;
                    (enc, 1_i64)
                } else {
                    (value.to_owned(), 0_i64)
                }
            } else {
                (value.to_owned(), 0_i64)
            }
        };
        #[cfg(not(feature = "vault"))]
        let (store_value, encrypted) = (value.to_owned(), 0_i64);

        let result = sqlx::query(
            "INSERT INTO env_variables (key, value, encrypted) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, encrypted = excluded.encrypted, updated_at = datetime('now')",
        )
        .bind(key)
        .bind(&store_value)
        .bind(encrypted)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    /// Delete an environment variable by id. Returns the key name if found.
    pub async fn delete_env_var(&self, id: i64) -> Result<Option<String>> {
        let key: Option<(String,)> = sqlx::query_as("SELECT key FROM env_variables WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        sqlx::query("DELETE FROM env_variables WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(key.map(|(k,)| k))
    }

    /// Get all environment variable key-value pairs (internal use for sandbox injection).
    pub async fn get_all_env_values(&self) -> Result<Vec<(String, String)>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT key, value, COALESCE(encrypted, 0) FROM env_variables ORDER BY key ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for (key, value, encrypted) in rows {
            #[cfg(feature = "vault")]
            let plaintext = {
                if encrypted != 0 {
                    if let Some(ref vault) = self.vault {
                        let aad = format!("env:{key}");
                        match vault.decrypt_string(&value, &aad).await {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(key = %key, error = %e, "failed to decrypt env var, skipping");
                                continue;
                            },
                        }
                    } else {
                        tracing::warn!(key = %key, "encrypted env var but no vault available, skipping");
                        continue;
                    }
                } else {
                    value
                }
            };
            #[cfg(not(feature = "vault"))]
            let plaintext = {
                let _ = encrypted;
                value
            };

            result.push((key, plaintext));
        }
        Ok(result)
    }

    #[cfg(feature = "vault")]
    pub fn vault_for_env(&self) -> Option<&Arc<Vault>> {
        self.vault.as_ref()
    }

    pub async fn audit_log(&self, event_type: &str, client_ip: Option<&str>, detail: Option<&str>) {
        let result = sqlx::query(
            "INSERT INTO auth_audit_log (event_type, client_ip, detail) VALUES (?, ?, ?)",
        )
        .bind(event_type)
        .bind(client_ip)
        .bind(detail)
        .execute(&self.pool)
        .await;
        if let Err(e) = result {
            tracing::debug!(error = %e, "failed to write audit log");
        }

        let _ = sqlx::query(
            "DELETE FROM auth_audit_log WHERE created_at < datetime('now', '-90 days')",
        )
        .execute(&self.pool)
        .await;
    }
}
