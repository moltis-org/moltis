#[cfg(feature = "vault")]
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[cfg(feature = "vault")]
use moltis_vault::Vault;
use sqlx::SqlitePool;

mod api_keys;
mod env_vars;
mod git_https_credentials;
mod legacy;
mod passkeys;
mod sessions;
mod ssh;
#[cfg(test)]
mod tests;
mod types;
mod util;

pub use {
    git_https_credentials::git_credential_mutation_guard,
    legacy::{AuthMode, AuthResult, ResolvedAuth, authorize_connect, resolve_auth},
    types::{
        ApiKeyEntry, ApiKeyVerification, AuthIdentity, AuthMethod, EnvVarEntry, GitHttpsCredential,
        GitHttpsCredentialEntry, PasskeyEntry, SshAuthMode, SshKeyEntry, SshResolvedTarget,
        SshTargetEntry, VALID_SCOPES,
    },
    util::is_loopback,
};

#[cfg(feature = "vault")]
pub use sessions::{PasswordVaultChangeError, VaultInitializeError, VaultInitializeOutcome};

/// Single-user credential store backed by SQLite.
pub struct CredentialStore {
    pool: SqlitePool,
    setup_complete: AtomicBool,
    /// When true, auth has been explicitly disabled via "remove all auth".
    /// The middleware and status endpoint treat this as "no auth configured".
    auth_disabled: AtomicBool,
    /// Encryption-at-rest vault for managed secrets.
    #[cfg(feature = "vault")]
    vault: Option<Arc<Vault>>,
    #[cfg(feature = "vault")]
    vault_encryption_enabled: AtomicBool,
}
