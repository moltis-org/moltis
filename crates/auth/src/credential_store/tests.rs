#![allow(clippy::expect_used, clippy::unwrap_used)]

use {
    crate::credential_store::{
        CredentialStore, SshAuthMode,
        util::{generate_token, hash_password, is_loopback, sha256_hex, verify_password},
    },
    secrecy::ExposeSecret,
    sqlx::SqlitePool,
};

#[cfg(feature = "vault")]
use std::sync::Arc;

#[cfg(feature = "vault")]
use moltis_vault::Vault;

fn fixture_secret(_tag: &str) -> String {
    generate_token()
}

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
    let password = generate_token();
    let wrong_password = generate_token();
    let hash = hash_password(&password).unwrap();
    assert!(verify_password(&password, &hash));
    assert!(!verify_password(&wrong_password, &hash));
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
    assert_eq!(h, sha256_hex("hello"));
    assert_ne!(h, sha256_hex("world"));
}

#[tokio::test]
async fn test_credential_store_password() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();
    let initial_password = fixture_secret("credential-store-password-initial");
    let replacement_password = fixture_secret("credential-store-password-replacement");
    let duplicate_password = generate_token();
    let wrong_password = fixture_secret("credential-store-password-wrong");
    let bad_change_password = fixture_secret("credential-store-password-bad-change");
    let tiny_password = fixture_secret("credential-store-password-tiny");

    assert!(!store.is_setup_complete());
    assert!(!store.verify_password(&wrong_password).await.unwrap());

    store.set_initial_password(&initial_password).await.unwrap();
    assert!(store.is_setup_complete());
    assert!(store.verify_password(&initial_password).await.unwrap());
    assert!(!store.verify_password(&wrong_password).await.unwrap());

    assert!(
        store
            .set_initial_password(&duplicate_password)
            .await
            .is_err()
    );

    store
        .change_password(&initial_password, &replacement_password)
        .await
        .unwrap();
    assert!(store.verify_password(&replacement_password).await.unwrap());
    assert!(!store.verify_password(&initial_password).await.unwrap());
    assert!(
        store
            .change_password(&bad_change_password, &tiny_password)
            .await
            .is_err()
    );
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

    let (id, raw_key) = store.create_api_key("test key", None).await.unwrap();
    assert!(id > 0);
    assert!(raw_key.starts_with("mk_"));

    let verification = store.verify_api_key(&raw_key).await.unwrap();
    assert!(verification.is_some());
    let v = verification.unwrap();
    assert_eq!(v.key_id, id);
    assert!(v.scopes.is_empty());

    assert!(store.verify_api_key("mk_bogus").await.unwrap().is_none());

    let keys = store.list_api_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].label, "test key");
    assert!(keys[0].scopes.is_none());

    store.revoke_api_key(id).await.unwrap();
    assert!(store.verify_api_key(&raw_key).await.unwrap().is_none());
    assert!(store.list_api_keys().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_credential_store_api_keys_with_scopes() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let scopes = vec!["operator.read".to_string(), "operator.write".to_string()];
    let (id, raw_key) = store
        .create_api_key("scoped key", Some(&scopes))
        .await
        .unwrap();
    assert!(id > 0);

    let verification = store.verify_api_key(&raw_key).await.unwrap();
    assert!(verification.is_some());
    let v = verification.unwrap();
    assert_eq!(v.key_id, id);
    assert_eq!(v.scopes, scopes);

    let keys = store.list_api_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].scopes, Some(scopes.clone()));

    let (id2, raw_key2) = store.create_api_key("full access key", None).await.unwrap();
    let keys = store.list_api_keys().await.unwrap();
    assert_eq!(keys.len(), 2);

    let scoped = keys.iter().find(|k| k.id == id).unwrap();
    let full = keys.iter().find(|k| k.id == id2).unwrap();
    assert_eq!(scoped.scopes, Some(scopes));
    assert!(full.scopes.is_none());

    assert!(store.verify_api_key(&raw_key).await.unwrap().is_some());
    assert!(store.verify_api_key(&raw_key2).await.unwrap().is_some());
}

#[tokio::test]
async fn test_credential_store_reset_all() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();
    let initial_password = fixture_secret("credential-store-reset-initial");
    let reset_password = fixture_secret("credential-store-reset-replacement");

    store.set_initial_password(&initial_password).await.unwrap();
    let token = store.create_session().await.unwrap();
    let (_id, raw_key) = store.create_api_key("test", None).await.unwrap();
    store
        .store_passkey(b"cred-1", "test pk", b"data")
        .await
        .unwrap();

    store.reset_all().await.unwrap();

    assert!(store.is_auth_disabled());
    assert!(!store.is_setup_complete());
    assert!(!store.validate_session(&token).await.unwrap());
    assert!(store.verify_api_key(&raw_key).await.unwrap().is_none());
    assert!(!store.has_passkeys().await.unwrap());
    assert!(!store.verify_password(&initial_password).await.unwrap());

    store.set_initial_password(&reset_password).await.unwrap();
    assert!(store.is_setup_complete());
    assert!(!store.is_auth_disabled());
}

#[tokio::test]
async fn test_reset_all_removes_managed_ssh_material() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let key_id = store
        .create_ssh_key(
            "prod-key",
            "PRIVATE KEY",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltis test@example",
            "256 SHA256:test moltis:test (ED25519)",
        )
        .await
        .unwrap();
    store
        .create_ssh_target(
            "prod-box",
            "deploy@example.com",
            None,
            None,
            SshAuthMode::Managed,
            Some(key_id),
            true,
        )
        .await
        .unwrap();

    store.reset_all().await.unwrap();
    assert!(store.list_ssh_keys().await.unwrap().is_empty());
    assert!(store.list_ssh_targets().await.unwrap().is_empty());
    assert!(store.get_ssh_private_key(key_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_auth_disabled_persists_across_restart() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool.clone()).await.unwrap();
    let initial_password = fixture_secret("auth-disabled-persists-initial");
    let replacement_password = fixture_secret("auth-disabled-persists-replacement");

    store.set_initial_password(&initial_password).await.unwrap();
    store.reset_all().await.unwrap();
    assert!(store.is_auth_disabled());

    let store2 = CredentialStore::new(pool.clone()).await.unwrap();
    assert!(store2.is_auth_disabled());
    assert!(!store2.is_setup_complete());

    store2
        .set_initial_password(&replacement_password)
        .await
        .unwrap();
    let store3 = CredentialStore::new(pool).await.unwrap();
    assert!(!store3.is_auth_disabled());
    assert!(store3.is_setup_complete());
}

#[tokio::test]
async fn test_credential_store_env_vars() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    assert!(store.list_env_vars().await.unwrap().is_empty());

    let id = store.set_env_var("MY_KEY", "secret123").await.unwrap();
    assert!(id > 0);

    let vars = store.list_env_vars().await.unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].key, "MY_KEY");
    assert!(!vars[0].encrypted);

    let values = store.get_all_env_values().await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], ("MY_KEY".into(), "secret123".into()));

    store.set_env_var("MY_KEY", "updated").await.unwrap();
    let values = store.get_all_env_values().await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].1, "updated");

    store.set_env_var("OTHER", "val").await.unwrap();
    let vars = store.list_env_vars().await.unwrap();
    assert_eq!(vars.len(), 2);

    let first_id = vars.iter().find(|v| v.key == "MY_KEY").unwrap().id;
    store.delete_env_var(first_id).await.unwrap();
    let vars = store.list_env_vars().await.unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].key, "OTHER");
}

#[tokio::test]
async fn test_credential_store_ssh_keys_and_targets() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let key_id = store
        .create_ssh_key(
            "prod-key",
            "PRIVATE KEY",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltis test@example",
            "256 SHA256:test moltis:test (ED25519)",
        )
        .await
        .unwrap();
    let target_id = store
        .create_ssh_target(
            "prod-box",
            "deploy@example.com",
            Some(2222),
            Some("|1|salt= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltisHostPin"),
            SshAuthMode::Managed,
            Some(key_id),
            true,
        )
        .await
        .unwrap();

    let keys = store.list_ssh_keys().await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].id, key_id);
    assert_eq!(keys[0].target_count, 1);

    let targets = store.list_ssh_targets().await.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, target_id);
    assert_eq!(targets[0].label, "prod-box");
    assert_eq!(targets[0].port, Some(2222));
    assert_eq!(
        targets[0].known_host.as_deref(),
        Some("|1|salt= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltisHostPin")
    );
    assert_eq!(targets[0].auth_mode, SshAuthMode::Managed);
    assert_eq!(targets[0].key_name.as_deref(), Some("prod-key"));
    assert!(targets[0].is_default);

    let resolved = store.resolve_ssh_target("prod-box").await.unwrap().unwrap();
    assert_eq!(resolved.node_id, format!("ssh:target:{target_id}"));
    assert_eq!(resolved.target, "deploy@example.com");
    assert_eq!(
        resolved.known_host.as_deref(),
        Some("|1|salt= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltisHostPin")
    );

    let default_target = store.get_default_ssh_target().await.unwrap().unwrap();
    assert_eq!(default_target.id, target_id);

    let private_key = store.get_ssh_private_key(key_id).await.unwrap().unwrap();
    assert_eq!(private_key.expose_secret(), "PRIVATE KEY");

    store.delete_ssh_target(target_id).await.unwrap();
    assert!(
        store
            .resolve_ssh_target("prod-box")
            .await
            .unwrap()
            .is_none()
    );
    store.delete_ssh_key(key_id).await.unwrap();
    assert!(store.list_ssh_keys().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_first_ssh_target_becomes_default_and_delete_promotes_replacement() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let key_id = store
        .create_ssh_key(
            "prod-key",
            "PRIVATE KEY",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltis test@example",
            "256 SHA256:test moltis:test (ED25519)",
        )
        .await
        .unwrap();
    let first_target_id = store
        .create_ssh_target(
            "first-box",
            "deploy@first.example.com",
            None,
            None,
            SshAuthMode::Managed,
            Some(key_id),
            false,
        )
        .await
        .unwrap();
    let second_target_id = store
        .create_ssh_target(
            "second-box",
            "deploy@second.example.com",
            None,
            None,
            SshAuthMode::Managed,
            Some(key_id),
            false,
        )
        .await
        .unwrap();

    let default_before_delete = store.get_default_ssh_target().await.unwrap().unwrap();
    assert_eq!(default_before_delete.id, first_target_id);

    store.delete_ssh_target(first_target_id).await.unwrap();

    let default_after_delete = store.get_default_ssh_target().await.unwrap().unwrap();
    assert_eq!(default_after_delete.id, second_target_id);
}

#[tokio::test]
async fn test_delete_ssh_key_rejects_in_use_key() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let key_id = store
        .create_ssh_key(
            "prod-key",
            "PRIVATE KEY",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltis test@example",
            "256 SHA256:test moltis:test (ED25519)",
        )
        .await
        .unwrap();
    store
        .create_ssh_target(
            "prod-box",
            "deploy@example.com",
            None,
            None,
            SshAuthMode::Managed,
            Some(key_id),
            true,
        )
        .await
        .unwrap();

    let error = store.delete_ssh_key(key_id).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ssh key is still assigned to one or more targets")
    );
}

#[tokio::test]
async fn test_update_ssh_target_known_host_round_trips() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let target_id = store
        .create_ssh_target(
            "prod-box",
            "deploy@example.com",
            None,
            None,
            SshAuthMode::System,
            None,
            true,
        )
        .await
        .unwrap();

    store
        .update_ssh_target_known_host(
            target_id,
            Some("prod.example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltisHostPin"),
        )
        .await
        .unwrap();
    let pinned = store
        .resolve_ssh_target_by_id(target_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pinned.known_host.as_deref(),
        Some("prod.example.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltisHostPin")
    );

    store
        .update_ssh_target_known_host(target_id, None)
        .await
        .unwrap();
    let cleared = store
        .resolve_ssh_target_by_id(target_id)
        .await
        .unwrap()
        .unwrap();
    assert!(cleared.known_host.is_none());
}

#[cfg(feature = "vault")]
async fn vault_store(password: &str) -> (CredentialStore, Arc<Vault>) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_vault::run_migrations(&pool).await.unwrap();
    let vault = Vault::new(pool.clone()).await.unwrap();
    vault.initialize(password).await.unwrap();
    let vault = Arc::new(vault);
    let store = CredentialStore::with_vault(
        pool,
        &moltis_config::AuthConfig::default(),
        Some(vault.clone()),
    )
    .await
    .unwrap();
    (store, vault)
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn test_ssh_keys_encrypt_when_vault_is_unsealed() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    moltis_vault::run_migrations(&pool).await.unwrap();
    let vault = Arc::new(Vault::new(pool.clone()).await.unwrap());
    let vault_password = fixture_secret("vault-ssh-key-password");
    vault.initialize(&vault_password).await.unwrap();
    let store = CredentialStore::with_vault(
        pool.clone(),
        &moltis_config::AuthConfig::default(),
        Some(Arc::clone(&vault)),
    )
    .await
    .unwrap();

    let key_id = store
        .create_ssh_key(
            "enc-key",
            "TOP SECRET KEY",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMoltis enc@example",
            "256 SHA256:enc moltis:enc (ED25519)",
        )
        .await
        .unwrap();

    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT private_key, encrypted FROM ssh_keys WHERE id = ?")
            .bind(key_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    let (stored_value, encrypted) = row.unwrap();
    assert_ne!(stored_value, "TOP SECRET KEY");
    assert_eq!(encrypted, 1);

    let private_key = store.get_ssh_private_key(key_id).await.unwrap().unwrap();
    assert_eq!(private_key.expose_secret(), "TOP SECRET KEY");
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
async fn test_change_password_invalidates_sessions() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();
    let initial_password = fixture_secret("change-password-invalidates-initial");
    let replacement_password = fixture_secret("change-password-invalidates-replacement");

    store.set_initial_password(&initial_password).await.unwrap();

    let token1 = store.create_session().await.unwrap();
    let token2 = store.create_session().await.unwrap();
    assert!(store.validate_session(&token1).await.unwrap());
    assert!(store.validate_session(&token2).await.unwrap());

    store
        .change_password(&initial_password, &replacement_password)
        .await
        .unwrap();

    assert!(!store.validate_session(&token1).await.unwrap());
    assert!(!store.validate_session(&token2).await.unwrap());

    let token3 = store.create_session().await.unwrap();
    assert!(store.validate_session(&token3).await.unwrap());
}

#[tokio::test]
async fn test_add_password_marks_setup_complete_and_reenables_auth() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();
    let password = generate_token();

    store.reset_all().await.unwrap();
    assert!(store.is_auth_disabled());
    assert!(!store.is_setup_complete());

    store.add_password(&password).await.unwrap();
    assert!(store.has_password().await.unwrap());
    assert!(store.is_setup_complete());
    assert!(!store.is_auth_disabled());
}

#[tokio::test]
async fn test_store_passkey_marks_setup_complete_and_reenables_auth() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    store.reset_all().await.unwrap();
    assert!(store.is_auth_disabled());
    assert!(!store.is_setup_complete());

    store
        .store_passkey(b"cred-1", "My Passkey", b"data")
        .await
        .unwrap();
    assert!(store.has_passkeys().await.unwrap());
    assert!(store.is_setup_complete());
    assert!(!store.is_auth_disabled());
}

#[tokio::test]
async fn test_mark_setup_complete_with_passkey_only() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    assert!(!store.is_setup_complete());
    assert!(store.mark_setup_complete().await.is_err());

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

    let store2 = CredentialStore::new(pool).await.unwrap();
    assert!(store2.is_setup_complete());
}

#[tokio::test]
async fn test_removing_last_passkey_clears_setup_complete() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();

    let id = store
        .store_passkey(b"cred-1", "My Passkey", b"data")
        .await
        .unwrap();
    store.mark_setup_complete().await.unwrap();
    assert!(store.is_setup_complete());

    store.remove_passkey(id).await.unwrap();
    assert!(!store.has_passkeys().await.unwrap());
    assert!(!store.has_password().await.unwrap());
    assert!(!store.is_setup_complete());
}

#[tokio::test]
async fn test_removing_passkey_keeps_setup_when_password_exists() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let store = CredentialStore::new(pool).await.unwrap();
    let password = generate_token();

    store.set_initial_password(&password).await.unwrap();
    let id = store
        .store_passkey(b"cred-1", "My Passkey", b"data")
        .await
        .unwrap();
    assert!(store.is_setup_complete());

    store.remove_passkey(id).await.unwrap();
    assert!(!store.has_passkeys().await.unwrap());
    assert!(store.has_password().await.unwrap());
    assert!(store.is_setup_complete());
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn test_env_var_encryption_when_vault_unsealed() {
    let vault_password = generate_token();
    let secret_value = fixture_secret("vault-env-encrypted-secret");
    let (store, _vault) = vault_store(&vault_password).await;

    store
        .set_env_var("SECRET_KEY", &secret_value)
        .await
        .unwrap();

    let row: (String, i64) =
        sqlx::query_as("SELECT value, encrypted FROM env_variables WHERE key = 'SECRET_KEY'")
            .fetch_one(store.db_pool())
            .await
            .unwrap();
    assert_eq!(row.1, 1);
    assert_ne!(row.0, secret_value);
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn test_env_var_plaintext_when_vault_sealed() {
    let vault_password = generate_token();
    let visible_value = fixture_secret("vault-env-plaintext-visible");
    let (store, vault) = vault_store(&vault_password).await;
    vault.seal().await;

    store
        .set_env_var("PLAIN_KEY", &visible_value)
        .await
        .unwrap();

    let row: (String, i64) =
        sqlx::query_as("SELECT value, encrypted FROM env_variables WHERE key = 'PLAIN_KEY'")
            .fetch_one(store.db_pool())
            .await
            .unwrap();
    assert_eq!(row.1, 0);
    assert_eq!(row.0, visible_value);
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn test_env_var_decrypt_round_trip() {
    let vault_password = generate_token();
    let api_token = fixture_secret("vault-env-round-trip-api-token");
    let (store, _vault) = vault_store(&vault_password).await;

    store.set_env_var("API_TOKEN", &api_token).await.unwrap();
    store
        .set_env_var("WEBHOOK_URL", "https://example.com/hook")
        .await
        .unwrap();

    let values = store.get_all_env_values().await.unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0], ("API_TOKEN".into(), api_token));
    assert_eq!(
        values[1],
        ("WEBHOOK_URL".into(), "https://example.com/hook".into())
    );
}
