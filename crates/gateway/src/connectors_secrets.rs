//! Vault-backed handling of connector credentials.
//!
//! Connector account configs carry secrets (CalDAV passwords, Tesla refresh
//! tokens) that must be encrypted at rest whenever the vault is available, and
//! decrypted only at the point a provider needs them.

use super::*;

impl ConnectorManager {
    #[cfg(feature = "vault")]
    pub async fn migrate_plaintext_credentials(&self) -> Result<usize> {
        if !crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            return Ok(0);
        }
        let Some(vault) = self.vault.as_ref() else {
            return Ok(0);
        };
        if !vault.is_unsealed().await {
            return Ok(0);
        }

        let mut mutation = self.account_mutations.lock().await;
        let mut changed = 0;
        for account in self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?
        {
            let mut config = account.config.clone();
            if !has_plaintext_secret_fields(&config, SECRET_FIELDS)
                .map_err(|error| internal(error, "inspect connector password"))?
            {
                continue;
            }
            encrypt_secret_fields(
                &mut config,
                SECRET_FIELDS,
                &secret_aad_scope(&account.id),
                vault.as_ref(),
            )
            .await
            .map_err(|error| internal(error, "encrypt connector password"))?;
            self.store
                .update_account(&account.id, AccountUpdate {
                    name: account.name,
                    config,
                    enabled: account.enabled,
                })
                .await
                .map_err(ConnectorManagerError::from)?;
            changed += 1;
        }
        mutation.revision = mutation
            .revision
            .wrapping_add(u64::try_from(changed).unwrap_or(u64::MAX));
        Ok(changed)
    }

    #[cfg(feature = "vault")]
    pub async fn decrypt_credentials_and_disable_vault(
        &self,
        vault: &moltis_vault::Vault,
    ) -> Result<usize> {
        let mut mutation = self.account_mutations.lock().await;
        let changed = decrypt_credentials(&self.store, vault).await?;
        moltis_config::update_config(|config| {
            config.auth.vault_enabled = false;
        })
        .map_err(|error| internal(error, "persist disabled connector vault encryption"))?;
        crate::vault_lifecycle::set_vault_encryption_runtime_enabled(false);
        mutation.revision = mutation
            .revision
            .wrapping_add(u64::try_from(changed).unwrap_or(u64::MAX));
        Ok(changed)
    }

    #[cfg(feature = "vault")]
    pub async fn decrypt_all_credentials_at(
        data_dir: &Path,
        vault: &moltis_vault::Vault,
    ) -> Result<usize> {
        let db_path = data_dir.join("connectors.db");
        if !db_path.exists() {
            return Ok(0);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db_path)
                    .foreign_keys(true)
                    .busy_timeout(StdDuration::from_secs(5)),
            )
            .await
            .map_err(|error| internal(error, "open connector database for vault disable"))?;
        let store = SqliteConnectorStore::new(pool);
        decrypt_credentials(&store, vault).await
    }

    #[cfg(feature = "vault")]
    pub(super) async fn prepare_secret_for_storage(
        &self,
        id: &str,
        config: &mut Value,
    ) -> Result<()> {
        if !crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            return Ok(());
        }
        let Some(vault) = self.vault.as_ref() else {
            return Err(ConnectorManagerError::Unavailable(
                "vault encryption is enabled, but the vault is unavailable".to_owned(),
            ));
        };
        if vault.is_unsealed().await {
            encrypt_secret_fields(config, SECRET_FIELDS, &secret_aad_scope(id), vault.as_ref())
                .await
                .map_err(|error| internal(error, "encrypt connector password"))?;
            return Ok(());
        }
        let status = vault
            .status()
            .await
            .map_err(|error| internal(error, "check connector vault status"))?;
        if matches!(status, VaultStatus::Uninitialized) {
            return Ok(());
        }
        if has_plaintext_secret_fields(config, SECRET_FIELDS)
            .map_err(|error| internal(error, "inspect connector password"))?
        {
            return Err(ConnectorManagerError::Unavailable(
                "vault is sealed; connector passwords cannot be persisted".to_owned(),
            ));
        }
        Ok(())
    }

    #[cfg(not(feature = "vault"))]
    pub(super) async fn prepare_secret_for_storage(
        &self,
        _id: &str,
        _config: &mut Value,
    ) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "vault")]
    pub(super) async fn decrypt_secret_for_runtime(
        &self,
        id: &str,
        config: &mut Value,
    ) -> Result<()> {
        if crate::vault_lifecycle::is_vault_encryption_runtime_enabled() {
            let Some(vault) = self.vault.as_ref() else {
                return Err(ConnectorManagerError::Unavailable(
                    "vault encryption is enabled, but the vault is unavailable".to_owned(),
                ));
            };
            let status = vault
                .status()
                .await
                .map_err(|error| internal(error, "check connector vault status"))?;
            if matches!(status, VaultStatus::Sealed) {
                return Err(ConnectorManagerError::Unavailable(
                    "vault is sealed; connector passwords are unavailable".to_owned(),
                ));
            }
        }
        if !has_encrypted_secret_fields(config, SECRET_FIELDS)
            .map_err(|error| internal(error, "inspect encrypted connector password"))?
        {
            return Ok(());
        }
        let Some(vault) = self.vault.as_ref() else {
            return Err(ConnectorManagerError::Unavailable(
                "encrypted connector passwords require the vault".to_owned(),
            ));
        };
        if !vault.is_unsealed().await {
            return Err(ConnectorManagerError::Unavailable(
                "vault is sealed; connector passwords are unavailable".to_owned(),
            ));
        }
        decrypt_secret_fields(config, SECRET_FIELDS, &secret_aad_scope(id), vault.as_ref())
            .await
            .map_err(|error| internal(error, "decrypt connector password"))?;
        Ok(())
    }

    #[cfg(not(feature = "vault"))]
    pub(super) async fn decrypt_secret_for_runtime(
        &self,
        _id: &str,
        _config: &mut Value,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "vault")]
async fn decrypt_credentials(
    store: &SqliteConnectorStore,
    vault: &moltis_vault::Vault,
) -> Result<usize> {
    let mut changed = 0;
    for account in store
        .list_accounts()
        .await
        .map_err(ConnectorManagerError::from)?
    {
        let mut config = account.config.clone();
        if !has_encrypted_secret_fields(&config, SECRET_FIELDS)
            .map_err(|error| internal(error, "inspect connector secret"))?
        {
            continue;
        }
        decrypt_secret_fields(
            &mut config,
            SECRET_FIELDS,
            &secret_aad_scope(&account.id),
            vault,
        )
        .await
        .map_err(|error| internal(error, "decrypt connector secret"))?;
        store
            .update_account(&account.id, AccountUpdate {
                name: account.name,
                config,
                enabled: account.enabled,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        changed += 1;
    }
    Ok(changed)
}
