use {
    crate::{auth::CredentialStore, state::GatewayState},
    secrecy::{ExposeSecret, Secret},
    std::{path::PathBuf, sync::Arc},
};

pub const AUTO_UNSEAL_KEY_ENV: &str = "MOLTIS_VAULT_AUTO_UNSEAL_KEY";
pub const AUTO_UNSEAL_KEY_FILE_ENV: &str = "MOLTIS_VAULT_AUTO_UNSEAL_KEY_FILE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoUnsealResult {
    NotConfigured,
    AlreadyUnsealed,
    Unsealed,
    NotInitialized,
    BadCredential,
    EmptySecret,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoUnsealSourceKind {
    Env,
    File,
}

struct AutoUnsealSecret {
    value: Secret<String>,
    kind: AutoUnsealSourceKind,
}

#[tracing::instrument(skip(vault))]
pub async fn auto_unseal_from_env(vault: &moltis_vault::Vault) -> AutoUnsealResult {
    let Some(secret) = auto_unseal_secret_from_env().await else {
        return AutoUnsealResult::NotConfigured;
    };
    auto_unseal_with_secret(vault, secret).await
}

async fn auto_unseal_with_secret(
    vault: &moltis_vault::Vault,
    secret: AutoUnsealSecret,
) -> AutoUnsealResult {
    if vault.is_unsealed().await {
        tracing::debug!("vault auto-unseal skipped: already unsealed");
        return AutoUnsealResult::AlreadyUnsealed;
    }

    let phrase = secret.value.expose_secret().trim();
    if phrase.is_empty() {
        tracing::warn!(
            source = ?secret.kind,
            "vault auto-unseal skipped: configured recovery key is empty"
        );
        return AutoUnsealResult::EmptySecret;
    }

    match vault.unseal_with_recovery(phrase).await {
        Ok(()) => {
            tracing::info!(source = ?secret.kind, "vault auto-unsealed");
            AutoUnsealResult::Unsealed
        },
        Err(moltis_vault::VaultError::NotInitialized) => {
            tracing::debug!("vault auto-unseal skipped: vault is not initialized");
            AutoUnsealResult::NotInitialized
        },
        Err(moltis_vault::VaultError::BadCredential) => {
            tracing::warn!(
                source = ?secret.kind,
                "vault auto-unseal failed: recovery key was rejected"
            );
            AutoUnsealResult::BadCredential
        },
        Err(error) => {
            tracing::warn!(source = ?secret.kind, %error, "vault auto-unseal failed");
            AutoUnsealResult::Error
        },
    }
}

async fn auto_unseal_secret_from_env() -> Option<AutoUnsealSecret> {
    let key = std::env::var(AUTO_UNSEAL_KEY_ENV).ok();
    let key_file = std::env::var(AUTO_UNSEAL_KEY_FILE_ENV).ok();

    match (key, key_file) {
        (None, None) => None,
        (Some(value), None) => {
            tracing::warn!(
                key_env = AUTO_UNSEAL_KEY_ENV,
                file_env = AUTO_UNSEAL_KEY_FILE_ENV,
                "vault auto-unseal recovery key supplied directly through the process environment; use a secret file when possible"
            );
            Some(AutoUnsealSecret {
                value: Secret::new(value),
                kind: AutoUnsealSourceKind::Env,
            })
        },
        (env_value, Some(path)) => {
            if env_value.is_some() {
                tracing::warn!(
                    key_env = AUTO_UNSEAL_KEY_ENV,
                    file_env = AUTO_UNSEAL_KEY_FILE_ENV,
                    "both vault auto-unseal env vars are set; using recovery key file"
                );
            }
            read_auto_unseal_secret_file(PathBuf::from(path)).await
        },
    }
}

async fn read_auto_unseal_secret_file(path: PathBuf) -> Option<AutoUnsealSecret> {
    match tokio::fs::read_to_string(&path).await {
        Ok(value) => Some(AutoUnsealSecret {
            value: Secret::new(value),
            kind: AutoUnsealSourceKind::File,
        }),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "vault auto-unseal recovery key file could not be read"
            );
            None
        },
    }
}

/// Migrate plaintext secrets to encrypted storage after vault unseal.
pub async fn run_vault_env_migration(credential_store: &CredentialStore) {
    if let Some(vault) = credential_store.vault() {
        let pool = credential_store.db_pool();
        match moltis_vault::migration::migrate_env_vars(vault, pool).await {
            Ok(n) if n > 0 => {
                tracing::info!(count = n, "migrated env vars to encrypted");
            },
            Ok(_) => {},
            Err(error) => {
                tracing::warn!(%error, "env var migration failed");
            },
        }
        match moltis_vault::migration::migrate_ssh_keys(vault, pool).await {
            Ok(n) if n > 0 => {
                tracing::info!(count = n, "migrated ssh keys to encrypted");
            },
            Ok(_) => {},
            Err(error) => {
                tracing::warn!(%error, "ssh key migration failed");
            },
        }

        if let Some(config_dir) = moltis_config::config_dir() {
            let provider_keys_path = config_dir.join("provider_keys.json");
            match moltis_vault::migration::encrypt_json_file(
                vault,
                &provider_keys_path,
                "provider_keys",
            )
            .await
            {
                Ok(true) => {
                    tracing::info!("encrypted provider_keys.json to vault storage");
                },
                Ok(false) => {},
                Err(error) => {
                    tracing::warn!(%error, "provider_keys.json encryption failed");
                },
            }
        }
    }
}

/// Start stored channel accounts after vault unseal.
///
/// When the vault is unsealed, previously encrypted channel configs become
/// decryptable. This handles the case where the vault was sealed at startup
/// and channels could not be started until a later manual unlock.
#[tracing::instrument(skip(state))]
pub async fn start_stored_channels_on_vault_unseal(state: &Arc<GatewayState>) {
    let Some(registry) = state.services.channel_registry.as_ref() else {
        tracing::debug!("no channel registry available, skipping channel startup on vault unseal");
        return;
    };
    let Some(store) = state.services.channel_store.as_ref() else {
        tracing::debug!("no channel store available, skipping channel startup on vault unseal");
        return;
    };

    let stored = match store.list().await {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(%error, "failed to list stored channels on vault unseal");
            return;
        },
    };

    if stored.is_empty() {
        return;
    }

    for channel in stored {
        if registry.get(&channel.channel_type).is_none() {
            tracing::debug!(
                account_id = channel.account_id,
                channel_type = channel.channel_type,
                "unsupported channel type on vault unseal, skipping stored account"
            );
            continue;
        }

        if registry.resolve_channel_type(&channel.account_id).is_some() {
            continue;
        }

        tracing::info!(
            account_id = channel.account_id,
            channel_type = channel.channel_type,
            "starting stored channel on vault unseal"
        );

        if let Err(error) = registry
            .start_account(&channel.channel_type, &channel.account_id, channel.config)
            .await
        {
            tracing::warn!(
                account_id = channel.account_id,
                channel_type = channel.channel_type,
                %error,
                "failed to start stored channel on vault unseal"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use {
        crate::vault_lifecycle::{
            AutoUnsealResult, AutoUnsealSecret, AutoUnsealSourceKind, auto_unseal_with_secret,
        },
        secrecy::Secret,
        sqlx::SqlitePool,
        std::sync::Arc,
    };

    async fn test_vault() -> Arc<moltis_vault::Vault> {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_vault::run_migrations(&pool).await.unwrap();
        Arc::new(moltis_vault::Vault::new(pool).await.unwrap())
    }

    #[tokio::test]
    async fn auto_unseal_with_recovery_key_unseals_vault() {
        let vault = test_vault().await;
        let recovery_key = vault.initialize("test-password-123").await.unwrap();
        let recovery_phrase = recovery_key.phrase().to_owned();
        vault.seal().await;

        let result = auto_unseal_with_secret(&vault, AutoUnsealSecret {
            value: Secret::new(recovery_phrase),
            kind: AutoUnsealSourceKind::Env,
        })
        .await;

        assert_eq!(result, AutoUnsealResult::Unsealed);
        assert_eq!(
            vault.status().await.unwrap(),
            moltis_vault::VaultStatus::Unsealed
        );
    }

    #[tokio::test]
    async fn auto_unseal_already_unsealed_is_successful_noop() {
        let vault = test_vault().await;
        let recovery_key = vault.initialize("test-password-123").await.unwrap();
        let recovery_phrase = recovery_key.phrase().to_owned();

        let result = auto_unseal_with_secret(&vault, AutoUnsealSecret {
            value: Secret::new(recovery_phrase),
            kind: AutoUnsealSourceKind::Env,
        })
        .await;

        assert_eq!(result, AutoUnsealResult::AlreadyUnsealed);
        assert_eq!(
            vault.status().await.unwrap(),
            moltis_vault::VaultStatus::Unsealed
        );
    }

    #[tokio::test]
    async fn auto_unseal_rejects_wrong_recovery_key() {
        let vault = test_vault().await;
        vault.initialize("test-password-123").await.unwrap();
        vault.seal().await;

        let result = auto_unseal_with_secret(&vault, AutoUnsealSecret {
            value: Secret::new("WRNG-WRNG-WRNG-WRNG-WRNG-WRNG-WRNG-WRNG".to_string()),
            kind: AutoUnsealSourceKind::Env,
        })
        .await;

        assert_eq!(result, AutoUnsealResult::BadCredential);
        assert_eq!(
            vault.status().await.unwrap(),
            moltis_vault::VaultStatus::Sealed
        );
    }

    #[tokio::test]
    async fn auto_unseal_empty_secret_is_noop() {
        let vault = test_vault().await;
        vault.initialize("test-password-123").await.unwrap();
        vault.seal().await;

        let result = auto_unseal_with_secret(&vault, AutoUnsealSecret {
            value: Secret::new(" \n ".to_string()),
            kind: AutoUnsealSourceKind::File,
        })
        .await;

        assert_eq!(result, AutoUnsealResult::EmptySecret);
        assert_eq!(
            vault.status().await.unwrap(),
            moltis_vault::VaultStatus::Sealed
        );
    }
}
