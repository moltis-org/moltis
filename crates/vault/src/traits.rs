//! Cipher trait for swappable authenticated encryption backends.

use crate::error::VaultError;

/// Trait for authenticated encryption with associated data (AEAD).
///
/// Implementations can be swapped without changing the rest of the vault.
/// Each implementation has a unique version tag stored in the encrypted blob,
/// enabling future cipher migrations.
pub trait Cipher: Send + Sync {
    /// Unique identifier for this cipher (stored as the first byte of the blob).
    fn version_tag(&self) -> u8;

    /// Encrypt `plaintext` with `key` and `aad` (additional authenticated data).
    ///
    /// Returns `[nonce || ciphertext || tag]` — the exact layout is
    /// cipher-specific but must be parseable by [`decrypt`](Self::decrypt).
    fn encrypt(&self, key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, VaultError>;

    /// Decrypt a blob previously produced by [`encrypt`](Self::encrypt).
    fn decrypt(&self, key: &[u8; 32], ciphertext: &[u8], aad: &[u8])
    -> Result<Vec<u8>, VaultError>;
}
