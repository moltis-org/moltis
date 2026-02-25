//! Recovery key generation, wrapping, and verification.
//!
//! The recovery key is a 128-bit random value formatted as
//! `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX` (alphanumeric groups).
//! A KEK is derived from it via Argon2id with a fixed salt, then used to
//! wrap the DEK independently of the password-derived KEK.

use {
    sha2::{Digest, Sha256},
    zeroize::Zeroizing,
};

use crate::{
    error::VaultError,
    kdf::{self, KdfParams},
    key_wrap,
    traits::Cipher,
};

/// Fixed salt for recovery key derivation (domain separation).
const RECOVERY_SALT: &[u8] = b"moltis-vault-recovery-key-salt!!"; // 32 bytes

/// Recovery KDF params: lighter than password KDF since the recovery key
/// already has 128 bits of entropy.
fn recovery_kdf_params() -> KdfParams {
    KdfParams {
        m_cost: 16384, // 16 MiB
        t_cost: 2,
        p_cost: 1,
    }
}

/// A generated recovery key with its formatted string representation.
pub struct RecoveryKey {
    /// Human-readable formatted recovery key (e.g. `ABCD-EFGH-...`).
    phrase: String,
}

impl RecoveryKey {
    /// The formatted recovery phrase. Shown to the user exactly once.
    pub fn phrase(&self) -> &str {
        &self.phrase
    }
}

/// Character set for recovery keys (alphanumeric, uppercase).
const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // 32 chars, no I/O/0/1

/// Generate a new random recovery key.
pub fn generate_recovery_key() -> RecoveryKey {
    use rand::RngCore;

    let mut entropy = [0u8; 16]; // 128 bits
    rand::rng().fill_bytes(&mut entropy);

    // Encode 128 bits as 32 chars from a 32-char alphabet (5 bits per char).
    // 128 / 5 = 25.6 → we generate 26 chars, but we'll use all 16 bytes differently.
    // Simpler approach: map each byte to 2 chars from the 32-char alphabet.
    let mut chars = Vec::with_capacity(32);
    for byte in &entropy {
        chars.push(CHARSET[(byte >> 4) as usize % 32] as char);
        chars.push(CHARSET[(byte & 0x0F) as usize % 32] as char);
    }

    // Format as XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX (8 groups of 4).
    let phrase: String = chars
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-");

    RecoveryKey { phrase }
}

/// Derive a KEK from a recovery phrase using Argon2id with the fixed salt.
pub fn derive_recovery_kek(phrase: &str) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    // Normalize: strip dashes, uppercase.
    let normalized: String = phrase
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_uppercase();
    kdf::derive_key(normalized.as_bytes(), RECOVERY_SALT, &recovery_kdf_params())
}

/// Wrap the DEK with a recovery key, returning `(wrapped_dek_b64, recovery_key_hash)`.
pub fn wrap_with_recovery<C: Cipher>(
    cipher: &C,
    dek: &[u8; 32],
    phrase: &str,
) -> Result<(String, String), VaultError> {
    let recovery_kek = derive_recovery_kek(phrase)?;
    let wrapped = key_wrap::wrap_dek(cipher, &recovery_kek, dek)?;
    let hash = sha256_hex(phrase);
    Ok((wrapped, hash))
}

/// Unwrap the DEK using a recovery phrase.
pub fn unwrap_with_recovery<C: Cipher>(
    cipher: &C,
    wrapped_b64: &str,
    phrase: &str,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let recovery_kek = derive_recovery_kek(phrase)?;
    key_wrap::unwrap_dek(cipher, &recovery_kek, wrapped_b64)
}

/// Verify that a recovery phrase matches the stored hash.
pub fn verify_recovery_hash(phrase: &str, stored_hash: &str) -> bool {
    sha256_hex(phrase) == stored_hash
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::xchacha20::XChaCha20Poly1305Cipher};

    #[test]
    fn recovery_key_format() {
        let rk = generate_recovery_key();
        let phrase = rk.phrase();

        // 8 groups of 4 chars separated by dashes.
        let groups: Vec<&str> = phrase.split('-').collect();
        assert_eq!(groups.len(), 8);
        for group in &groups {
            assert_eq!(group.len(), 4);
            for c in group.chars() {
                assert!(CHARSET.contains(&(c as u8)));
            }
        }
    }

    #[test]
    fn recovery_key_uniqueness() {
        let rk1 = generate_recovery_key();
        let rk2 = generate_recovery_key();
        assert_ne!(rk1.phrase(), rk2.phrase());
    }

    #[test]
    fn recovery_wrap_unwrap_round_trip() {
        let cipher = XChaCha20Poly1305Cipher;
        let dek = [0xBB; 32];
        let rk = generate_recovery_key();

        let (wrapped, hash) = wrap_with_recovery(&cipher, &dek, rk.phrase()).unwrap();
        assert!(verify_recovery_hash(rk.phrase(), &hash));

        let unwrapped = unwrap_with_recovery(&cipher, &wrapped, rk.phrase()).unwrap();
        assert_eq!(*unwrapped, dek);
    }

    #[test]
    fn wrong_recovery_key_fails() {
        let cipher = XChaCha20Poly1305Cipher;
        let dek = [0xBB; 32];
        let rk = generate_recovery_key();
        let wrong_rk = generate_recovery_key();

        let (wrapped, _hash) = wrap_with_recovery(&cipher, &dek, rk.phrase()).unwrap();
        let result = unwrap_with_recovery(&cipher, &wrapped, wrong_rk.phrase());
        assert!(result.is_err());
    }

    #[test]
    fn recovery_key_case_insensitive() {
        let cipher = XChaCha20Poly1305Cipher;
        let dek = [0xBB; 32];
        let rk = generate_recovery_key();
        let phrase = rk.phrase();

        let (wrapped, _) = wrap_with_recovery(&cipher, &dek, phrase).unwrap();

        // Should work with lowercase input.
        let lower = phrase.to_lowercase();
        let unwrapped = unwrap_with_recovery(&cipher, &wrapped, &lower).unwrap();
        assert_eq!(*unwrapped, dek);
    }

    #[test]
    fn hash_verification() {
        let rk = generate_recovery_key();
        let hash = sha256_hex(rk.phrase());
        assert!(verify_recovery_hash(rk.phrase(), &hash));
        assert!(!verify_recovery_hash("wrong-phrase-AAAA-BBBB", &hash));
    }
}
