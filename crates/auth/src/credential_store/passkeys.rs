use crate::{
    Result,
    credential_store::{CredentialStore, PasskeyEntry},
};

impl CredentialStore {
    /// Store a new passkey credential.
    pub async fn store_passkey(
        &self,
        credential_id: &[u8],
        name: &str,
        passkey_data: &[u8],
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO passkeys (credential_id, name, passkey_data) VALUES (?, ?, ?)",
        )
        .bind(credential_id)
        .bind(name)
        .bind(passkey_data)
        .execute(&self.pool)
        .await?;
        self.mark_setup_complete().await?;
        Ok(result.last_insert_rowid())
    }

    /// List all registered passkeys.
    pub async fn list_passkeys(&self) -> Result<Vec<PasskeyEntry>> {
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
    pub async fn remove_passkey(&self, passkey_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM passkeys WHERE id = ?")
            .bind(passkey_id)
            .execute(&self.pool)
            .await?;
        self.recompute_setup_complete().await?;
        Ok(())
    }

    /// Rename a passkey.
    pub async fn rename_passkey(&self, passkey_id: i64, name: &str) -> Result<()> {
        sqlx::query("UPDATE passkeys SET name = ? WHERE id = ?")
            .bind(name)
            .bind(passkey_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Load all passkey data blobs (for WebAuthn authentication).
    pub async fn load_all_passkey_data(&self) -> Result<Vec<(i64, Vec<u8>)>> {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT id, passkey_data FROM passkeys")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Check if any passkeys are registered (for login page UI).
    pub async fn has_passkeys(&self) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM passkeys LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }
}
