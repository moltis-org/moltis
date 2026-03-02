//! Encryption-at-rest vault using XChaCha20-Poly1305.
//!
//! A random DEK (Data Encryption Key) is wrapped with a password-derived KEK
//! via Argon2id. The vault must be password-unlocked once per process start.
//! Trait-based [`Cipher`] design allows swapping the encryption backend.

pub mod error;
pub mod kdf;
pub mod key_wrap;
pub mod migration;
pub mod recovery;
pub mod traits;
pub mod vault;
pub mod xchacha20;

pub use {
    error::VaultError,
    recovery::RecoveryKey,
    traits::Cipher,
    vault::{Vault, VaultStatus},
    xchacha20::XChaCha20Poly1305Cipher,
};

/// Run database migrations for the vault crate.
///
/// Creates the `vault_metadata` table. Should be called at application startup
/// after cron migrations and before gateway migrations.
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
