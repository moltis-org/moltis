use {
    crate::{auth::CredentialStore, state::GatewayState},
    anyhow::Context,
    secrecy::{ExposeSecret, Secret},
    std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    },
};

pub const AUTO_UNSEAL_KEY_ENV: &str = "MOLTIS_VAULT_AUTO_UNSEAL_KEY";
pub const AUTO_UNSEAL_KEY_FILE_ENV: &str = "MOLTIS_VAULT_AUTO_UNSEAL_KEY_FILE";

static VAULT_ENCRYPTION_RUNTIME_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_vault_encryption_runtime_enabled(enabled: bool) {
    VAULT_ENCRYPTION_RUNTIME_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_vault_encryption_runtime_enabled() -> bool {
    VAULT_ENCRYPTION_RUNTIME_ENABLED.load(Ordering::Relaxed)
}

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VaultDisableReport {
    pub env_vars: usize,
    pub ssh_keys: usize,
    pub channels: usize,
    pub webhooks: usize,
    pub provider_keys: bool,
}

/// Decrypt all known vault-backed data and disable vault use in config.
///
/// This must only run while the vault is unsealed. The config flag is written
/// after all decryptions succeed, so a partial failure leaves vault mode intact.
#[tracing::instrument(skip(vault, pool))]
pub async fn disable_vault_and_decrypt_all(
    vault: &moltis_vault::Vault,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<VaultDisableReport> {
    if !vault.is_unsealed().await {
        anyhow::bail!("vault must be unlocked before disabling encryption at rest");
    }

    let report = VaultDisableReport {
        env_vars: decrypt_env_vars(vault, pool).await?,
        ssh_keys: decrypt_ssh_keys(vault, pool).await?,
        channels: decrypt_channels(vault, pool).await?,
        webhooks: decrypt_webhooks(vault, pool).await?,
        provider_keys: decrypt_provider_keys(vault).await?,
    };

    moltis_config::update_config(|config| {
        config.auth.vault_enabled = false;
    })
    .context("failed to persist auth.vault_enabled=false")?;
    set_vault_encryption_runtime_enabled(false);

    tracing::info!(?report, "vault disabled after decrypting stored secrets");
    Ok(report)
}

async fn decrypt_env_vars(
    vault: &moltis_vault::Vault,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<usize> {
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, key, value FROM env_variables WHERE encrypted = 1")
            .fetch_all(pool)
            .await?;
    let count = rows.len();
    for (id, key, ciphertext) in rows {
        let plaintext = vault
            .decrypt_string(&ciphertext, &format!("env:{key}"))
            .await?;
        sqlx::query("UPDATE env_variables SET value = ?, encrypted = 0, updated_at = datetime('now') WHERE id = ?")
            .bind(plaintext)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(count)
}

async fn decrypt_ssh_keys(
    vault: &moltis_vault::Vault,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<usize> {
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, name, private_key FROM ssh_keys WHERE encrypted = 1")
            .fetch_all(pool)
            .await?;
    let count = rows.len();
    for (id, name, ciphertext) in rows {
        let plaintext = vault
            .decrypt_string(&ciphertext, &format!("ssh-key:{name}"))
            .await?;
        sqlx::query("UPDATE ssh_keys SET private_key = ?, encrypted = 0, updated_at = datetime('now') WHERE id = ?")
            .bind(plaintext)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(count)
}

async fn decrypt_channels(
    vault: &moltis_vault::Vault,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<usize> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT channel_type, account_id, config FROM channels")
            .fetch_all(pool)
            .await?;
    let mut changed = 0;
    for (channel_type, account_id, config_json) in rows {
        let channel_type_enum = match channel_type.parse::<moltis_channels::plugin::ChannelType>() {
            Ok(channel_type) => channel_type,
            Err(_) => {
                let config: serde_json::Value =
                    serde_json::from_str(&config_json).with_context(|| {
                        format!("invalid channel config for {channel_type}:{account_id}")
                    })?;
                if contains_vault_encrypted_secret(&config) {
                    anyhow::bail!(
                        "cannot disable vault: channel {channel_type}:{account_id} has encrypted secrets but the channel type is unavailable"
                    );
                }
                continue;
            },
        };
        let secret_fields = channel_type_enum.secret_fields();
        if secret_fields.is_empty() {
            continue;
        }
        let mut config: serde_json::Value = serde_json::from_str(&config_json)
            .with_context(|| format!("invalid channel config for {channel_type}:{account_id}"))?;
        if !moltis_secret_store::has_encrypted_secret_fields(&config, secret_fields)? {
            continue;
        }
        moltis_secret_store::decrypt_secret_fields(
            &mut config,
            secret_fields,
            &format!("channel:{channel_type}:{account_id}"),
            vault,
        )
        .await?;
        sqlx::query("UPDATE channels SET config = ?, updated_at = ? WHERE channel_type = ? AND account_id = ?")
            .bind(serde_json::to_string(&config)?)
            .bind(unix_now_i64())
            .bind(&channel_type)
            .bind(&account_id)
            .execute(pool)
            .await?;
        changed += 1;
    }
    Ok(changed)
}

fn contains_vault_encrypted_secret(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.get("kind").and_then(serde_json::Value::as_str) == Some("vault_encrypted")
                || map.values().any(contains_vault_encrypted_secret)
        },
        serde_json::Value::Array(values) => values.iter().any(contains_vault_encrypted_secret),
        _ => false,
    }
}

async fn decrypt_webhooks(
    vault: &moltis_vault::Vault,
    pool: &sqlx::SqlitePool,
) -> anyhow::Result<usize> {
    let rows: Vec<(i64, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, auth_mode, auth_config_json, source_config_json FROM webhooks")
            .fetch_all(pool)
            .await?;
    let mut changed = 0;
    for (id, auth_mode, auth_config_json, source_config_json) in rows {
        let mut auth_config_update = None;
        let mut source_config_update = None;
        if let Some(config_json) = auth_config_json {
            let mut config: serde_json::Value = serde_json::from_str(&config_json)
                .with_context(|| format!("invalid auth_config_json for webhook {id}"))?;
            let fields = webhook_auth_secret_fields(&auth_mode);
            if !fields.is_empty()
                && moltis_secret_store::has_encrypted_secret_fields(&config, fields)?
            {
                moltis_secret_store::decrypt_secret_fields(
                    &mut config,
                    fields,
                    "webhook:config",
                    vault,
                )
                .await?;
                auth_config_update = Some(serde_json::to_string(&config)?);
            }
        }
        if let Some(config_json) = source_config_json {
            let mut config: serde_json::Value = serde_json::from_str(&config_json)
                .with_context(|| format!("invalid source_config_json for webhook {id}"))?;
            let fields = webhook_source_secret_fields();
            if moltis_secret_store::has_encrypted_secret_fields(&config, fields)? {
                moltis_secret_store::decrypt_secret_fields(
                    &mut config,
                    fields,
                    "webhook:config",
                    vault,
                )
                .await?;
                source_config_update = Some(serde_json::to_string(&config)?);
            }
        }

        match (auth_config_update, source_config_update) {
            (Some(auth_config), Some(source_config)) => {
                sqlx::query("UPDATE webhooks SET auth_config_json = ?, source_config_json = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(auth_config)
                    .bind(source_config)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            },
            (Some(auth_config), None) => {
                sqlx::query("UPDATE webhooks SET auth_config_json = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(auth_config)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            },
            (None, Some(source_config)) => {
                sqlx::query("UPDATE webhooks SET source_config_json = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(source_config)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            },
            (None, None) => {},
        }
    }
    Ok(changed)
}

async fn decrypt_provider_keys(vault: &moltis_vault::Vault) -> anyhow::Result<bool> {
    let Some(config_dir) = moltis_config::config_dir() else {
        return Ok(false);
    };
    let path = config_dir.join("provider_keys.json");
    let enc_path = path.with_extension("json.enc");
    if !enc_path.exists() {
        return Ok(false);
    }
    let encrypted = tokio::fs::read_to_string(&enc_path)
        .await
        .with_context(|| format!("failed to read {}", enc_path.display()))?;
    let plaintext = vault.decrypt_string(&encrypted, "provider_keys").await?;
    write_secret_file(&path, &plaintext).await?;
    tokio::fs::remove_file(&enc_path)
        .await
        .with_context(|| format!("failed to remove {}", enc_path.display()))?;
    Ok(true)
}

async fn write_secret_file(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    let path = path.to_path_buf();
    let content = content.to_owned();
    tokio::task::spawn_blocking(move || write_secret_file_blocking(&path, &content))
        .await
        .context("secret file write task failed")?
}

fn write_secret_file_blocking(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to chmod {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        let mut file = options
            .open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

fn webhook_auth_secret_fields(auth_mode: &str) -> &'static [&'static str] {
    match auth_mode {
        "static_header" => &["value"],
        "bearer" | "gitlab_token" => &["token"],
        "github_hmac_sha256"
        | "stripe_webhook_signature"
        | "linear_webhook_signature"
        | "pagerduty_v2_signature"
        | "sentry_webhook_signature" => &["secret"],
        _ => &[],
    }
}

fn webhook_source_secret_fields() -> &'static [&'static str] {
    &[
        "access_token",
        "api_key",
        "api_token",
        "bearer_token",
        "client_secret",
        "secret",
        "signing_secret",
        "token",
        "webhook_secret",
    ]
}

fn unix_now_i64() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
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
            disable_vault_and_decrypt_all, is_vault_encryption_runtime_enabled,
            set_vault_encryption_runtime_enabled,
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

    struct VaultRuntimeFlagGuard;

    impl VaultRuntimeFlagGuard {
        fn enabled() -> Self {
            set_vault_encryption_runtime_enabled(true);
            Self
        }
    }

    impl Drop for VaultRuntimeFlagGuard {
        fn drop(&mut self) {
            set_vault_encryption_runtime_enabled(true);
        }
    }

    fn test_password() -> String {
        format!(
            "test-password-{}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        )
    }

    #[tokio::test]
    async fn auto_unseal_with_recovery_key_unseals_vault() {
        let vault = test_vault().await;
        let recovery_key = vault.initialize(&test_password()).await.unwrap();
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
        let recovery_key = vault.initialize(&test_password()).await.unwrap();
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
        vault.initialize(&test_password()).await.unwrap();
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
        vault.initialize(&test_password()).await.unwrap();
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

    #[tokio::test]
    #[serial_test::serial(vault_runtime)]
    async fn disable_vault_decrypts_stored_secrets_before_flipping_config() {
        let _runtime_flag = VaultRuntimeFlagGuard::enabled();
        let config_dir = tempfile::tempdir().unwrap();
        moltis_config::set_config_dir(config_dir.path().to_path_buf());
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_vault::run_migrations(&pool).await.unwrap();
        create_disable_test_tables(&pool).await;
        let vault = Arc::new(moltis_vault::Vault::new(pool.clone()).await.unwrap());
        vault.initialize(&test_password()).await.unwrap();

        let env_ciphertext = vault
            .encrypt_string("env-secret", "env:API_KEY")
            .await
            .unwrap();
        sqlx::query("INSERT INTO env_variables (id, key, value, encrypted, updated_at) VALUES (1, 'API_KEY', ?, 1, datetime('now'))")
            .bind(env_ciphertext)
            .execute(&pool)
            .await
            .unwrap();

        let ssh_ciphertext = vault
            .encrypt_string("private-key", "ssh-key:deploy")
            .await
            .unwrap();
        sqlx::query("INSERT INTO ssh_keys (id, name, private_key, encrypted, updated_at) VALUES (1, 'deploy', ?, 1, datetime('now'))")
            .bind(ssh_ciphertext)
            .execute(&pool)
            .await
            .unwrap();

        let mut channel_config = serde_json::json!({ "token": "Bot discord-token" });
        moltis_secret_store::encrypt_secret_fields(
            &mut channel_config,
            &["token"],
            "channel:discord:main",
            vault.as_ref(),
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO channels (channel_type, account_id, config, created_at, updated_at) VALUES ('discord', 'main', ?, 1, 1)")
            .bind(serde_json::to_string(&channel_config).unwrap())
            .execute(&pool)
            .await
            .unwrap();

        let mut webhook_config = serde_json::json!({ "token": "webhook-token" });
        moltis_secret_store::encrypt_secret_fields(
            &mut webhook_config,
            &["token"],
            "webhook:config",
            vault.as_ref(),
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO webhooks (id, auth_mode, auth_config_json, source_profile, source_config_json, updated_at) VALUES (1, 'bearer', ?, 'generic', NULL, datetime('now'))")
            .bind(serde_json::to_string(&webhook_config).unwrap())
            .execute(&pool)
            .await
            .unwrap();

        set_vault_encryption_runtime_enabled(true);
        let report = disable_vault_and_decrypt_all(&vault, &pool).await.unwrap();
        assert_eq!(report.env_vars, 1);
        assert_eq!(report.ssh_keys, 1);
        assert_eq!(report.channels, 1);
        assert_eq!(report.webhooks, 1);

        let env: (String, i64) =
            sqlx::query_as("SELECT value, encrypted FROM env_variables WHERE key = 'API_KEY'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(env, ("env-secret".to_owned(), 0));
        let channel: (String,) =
            sqlx::query_as("SELECT config FROM channels WHERE account_id = 'main'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let channel_json: serde_json::Value = serde_json::from_str(&channel.0).unwrap();
        assert_eq!(channel_json["token"], "Bot discord-token");
    }

    #[tokio::test]
    #[serial_test::serial(vault_runtime)]
    async fn disable_vault_fails_on_unknown_channel_with_encrypted_secret() {
        let _runtime_flag = VaultRuntimeFlagGuard::enabled();
        let config_dir = tempfile::tempdir().unwrap();
        moltis_config::set_config_dir(config_dir.path().to_path_buf());
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_vault::run_migrations(&pool).await.unwrap();
        create_disable_test_tables(&pool).await;
        let vault = Arc::new(moltis_vault::Vault::new(pool.clone()).await.unwrap());
        vault.initialize(&test_password()).await.unwrap();

        let config = serde_json::json!({
            "token": {
                "kind": "vault_encrypted",
                "ciphertext": "unavailable-channel-ciphertext"
            }
        });
        sqlx::query("INSERT INTO channels (channel_type, account_id, config, created_at, updated_at) VALUES ('future-channel', 'main', ?, 1, 1)")
            .bind(serde_json::to_string(&config).unwrap())
            .execute(&pool)
            .await
            .unwrap();

        set_vault_encryption_runtime_enabled(true);
        let error = disable_vault_and_decrypt_all(&vault, &pool)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("channel future-channel:main has encrypted secrets")
        );
        assert!(is_vault_encryption_runtime_enabled());
    }

    async fn create_disable_test_tables(pool: &SqlitePool) {
        for sql in [
            "CREATE TABLE env_variables (id INTEGER PRIMARY KEY, key TEXT NOT NULL, value TEXT NOT NULL, encrypted INTEGER NOT NULL DEFAULT 0, updated_at TEXT)",
            "CREATE TABLE ssh_keys (id INTEGER PRIMARY KEY, name TEXT NOT NULL, private_key TEXT NOT NULL, encrypted INTEGER NOT NULL DEFAULT 0, updated_at TEXT)",
            "CREATE TABLE channels (channel_type TEXT NOT NULL, account_id TEXT NOT NULL, config TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY (channel_type, account_id))",
            "CREATE TABLE webhooks (id INTEGER PRIMARY KEY, auth_mode TEXT NOT NULL, auth_config_json TEXT, source_profile TEXT NOT NULL, source_config_json TEXT, updated_at TEXT)",
        ] {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    }
}
