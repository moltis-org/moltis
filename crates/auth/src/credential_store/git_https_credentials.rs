use {
    secrecy::{ExposeSecret, Secret},
    tokio::sync::{Mutex, MutexGuard},
};

use crate::{
    Error, Result,
    credential_store::{CredentialStore, GitHttpsCredential, GitHttpsCredentialEntry},
};

/// Serialize recoverable Git credential writes with vault migration/disable.
pub async fn git_credential_mutation_guard() -> MutexGuard<'static, u64> {
    static MUTATIONS: Mutex<u64> = Mutex::const_new(0);
    let mut generation = MUTATIONS.lock().await;
    *generation = generation.wrapping_add(1);
    generation
}

impl CredentialStore {
    /// Create a host-bound HTTPS Git credential.
    pub async fn create_git_https_credential(
        &self,
        host: &str,
        username: &str,
        token: Secret<String>,
    ) -> Result<i64> {
        let _mutation = git_credential_mutation_guard().await;
        let host = validate_host(host)?;
        let username = validate_username(username)?;
        validate_token(&token)?;

        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "INSERT INTO git_https_credentials (host, username, token) VALUES (?, ?, '')",
        )
        .bind(&host)
        .bind(&username)
        .execute(&mut *tx)
        .await?;
        let id = result.last_insert_rowid();
        let (stored_token, encrypted) = self.prepare_git_https_token(id, token).await?;
        sqlx::query("UPDATE git_https_credentials SET token = ?, encrypted = ? WHERE id = ?")
            .bind(stored_token.expose_secret())
            .bind(encrypted)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        Ok(id)
    }

    /// Replace the host, username, and token for an HTTPS Git credential.
    pub async fn update_git_https_credential(
        &self,
        id: i64,
        host: &str,
        username: &str,
        token: Secret<String>,
    ) -> Result<()> {
        let _mutation = git_credential_mutation_guard().await;
        let host = validate_host(host)?;
        let username = validate_username(username)?;
        validate_token(&token)?;
        let (stored_token, encrypted) = self.prepare_git_https_token(id, token).await?;

        let updated = sqlx::query(
            "UPDATE git_https_credentials
             SET host = ?, username = ?, token = ?, encrypted = ?, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(host)
        .bind(username)
        .bind(stored_token.expose_secret())
        .bind(encrypted)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(Error::GitCredential(
                "HTTPS Git credential not found".into(),
            ));
        }
        Ok(())
    }

    /// List HTTPS Git credential metadata without exposing tokens.
    pub async fn list_git_https_credentials(&self) -> Result<Vec<GitHttpsCredentialEntry>> {
        let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
            "SELECT
                id,
                host,
                username,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at),
                COALESCE(encrypted, 0)
             FROM git_https_credentials
             ORDER BY host ASC, username ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, host, username, created_at, updated_at, encrypted)| {
                GitHttpsCredentialEntry {
                    id,
                    host,
                    username,
                    created_at,
                    updated_at,
                    encrypted: encrypted != 0,
                }
            })
            .collect())
    }

    /// Get an HTTPS Git credential, decrypting its token when necessary.
    pub async fn get_git_https_credential(&self, id: i64) -> Result<Option<GitHttpsCredential>> {
        let row: Option<(String, String, String, i64)> = sqlx::query_as(
            "SELECT host, username, token, COALESCE(encrypted, 0)
             FROM git_https_credentials WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((host, username, token, encrypted)) = row else {
            return Ok(None);
        };
        let token = self
            .resolve_git_https_token(id, Secret::new(token), encrypted)
            .await?;
        Ok(Some(GitHttpsCredential {
            id,
            host,
            username,
            token,
        }))
    }

    /// Delete a credential unless repositories still reference it.
    ///
    /// `repository_reference_count` is supplied by the repository owner so this
    /// store remains usable before and after its reference table is introduced.
    pub async fn delete_git_https_credential(
        &self,
        id: i64,
        repository_reference_count: usize,
    ) -> Result<()> {
        let _mutation = git_credential_mutation_guard().await;
        if repository_reference_count != 0 {
            return Err(Error::GitCredential(
                "HTTPS Git credential is still assigned to one or more repositories".into(),
            ));
        }

        sqlx::query("DELETE FROM git_https_credentials WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn prepare_git_https_token(
        &self,
        id: i64,
        token: Secret<String>,
    ) -> Result<(Secret<String>, i64)> {
        #[cfg(feature = "vault")]
        {
            if self.is_vault_encryption_enabled() {
                let Some(ref vault) = self.vault else {
                    return Err(Error::Crypto(
                        "vault not available for HTTPS Git credential encryption".into(),
                    ));
                };
                let encrypted = vault
                    .encrypt_string(token.expose_secret(), &git_https_aad(id))
                    .await
                    .map_err(|error| Error::Crypto(error.to_string()))?;
                return Ok((Secret::new(encrypted), 1));
            }
        }

        let _ = id;
        Ok((token, 0))
    }

    async fn resolve_git_https_token(
        &self,
        id: i64,
        token: Secret<String>,
        encrypted: i64,
    ) -> Result<Secret<String>> {
        #[cfg(feature = "vault")]
        {
            if encrypted != 0 {
                let Some(ref vault) = self.vault else {
                    return Err(Error::Crypto(
                        "vault not available for encrypted HTTPS Git credential".into(),
                    ));
                };
                let decrypted = vault
                    .decrypt_string(token.expose_secret(), &git_https_aad(id))
                    .await
                    .map_err(|error| Error::Crypto(error.to_string()))?;
                return Ok(Secret::new(decrypted));
            }
        }

        let _ = id;
        let _ = encrypted;
        Ok(token)
    }
}

fn validate_host(host: &str) -> Result<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(Error::GitCredential(
            "HTTPS Git credential host is required".into(),
        ));
    }
    if host.contains("://")
        || host.contains('/')
        || host.contains('\\')
        || host.contains('@')
        || host.chars().any(char::is_whitespace)
    {
        return Err(Error::GitCredential(
            "HTTPS Git credential host must be a host name, not a URL".into(),
        ));
    }
    Ok(host)
}

fn validate_username(username: &str) -> Result<String> {
    let username = username.trim();
    if username.is_empty() {
        return Err(Error::GitCredential(
            "HTTPS Git credential username is required".into(),
        ));
    }
    Ok(username.to_owned())
}

fn validate_token(token: &Secret<String>) -> Result<()> {
    if token.expose_secret().is_empty() {
        return Err(Error::GitCredential(
            "HTTPS Git credential token is required".into(),
        ));
    }
    Ok(())
}

#[cfg(feature = "vault")]
fn git_https_aad(id: i64) -> String {
    format!("git-https-credential:{id}")
}
