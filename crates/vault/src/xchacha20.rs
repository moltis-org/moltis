//! XChaCha20-Poly1305 implementation of the [`Cipher`] trait.

#[allow(deprecated)] // upstream generic-array 0.x deprecation
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::RngCore;

use crate::{error::VaultError, traits::Cipher};

/// Version tag for the XChaCha20-Poly1305 cipher.
pub const VERSION_TAG: u8 = 0x01;

/// Nonce size for XChaCha20-Poly1305 (24 bytes).
const NONCE_LEN: usize = 24;

/// XChaCha20-Poly1305 AEAD cipher.
///
/// Encrypted blob layout: `[nonce: 24 bytes][ciphertext + Poly1305 tag: N + 16 bytes]`.
pub struct XChaCha20Poly1305Cipher;

impl Cipher for XChaCha20Poly1305Cipher {
    fn version_tag(&self) -> u8 {
        VERSION_TAG
    }

    #[allow(deprecated)]
    fn encrypt(&self, key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, VaultError> {
        let cipher = XChaCha20Poly1305::new(key.into());

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, Payload {
                msg: plaintext,
                aad,
            })
            .map_err(|e| VaultError::CipherError(e.to_string()))?;

        let mut result = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    #[allow(deprecated)]
    fn decrypt(
        &self,
        key: &[u8; 32],
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, VaultError> {
        if ciphertext.len() < NONCE_LEN + 16 {
            return Err(VaultError::CipherError("ciphertext too short".to_string()));
        }

        let (nonce_bytes, ct) = ciphertext.split_at(NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);
        let cipher = XChaCha20Poly1305::new(key.into());

        cipher
            .decrypt(nonce, Payload { msg: ct, aad })
            .map_err(|e| VaultError::CipherError(e.to_string()))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_no_aad() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = b"hello vault";

        let encrypted = cipher.encrypt(&key, plaintext, b"").unwrap();
        let decrypted = cipher.decrypt(&key, &encrypted, b"").unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn round_trip_with_aad() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = b"secret data";
        let aad = b"env:MY_KEY";

        let encrypted = cipher.encrypt(&key, plaintext, aad).unwrap();
        let decrypted = cipher.decrypt(&key, &encrypted, aad).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let cipher = XChaCha20Poly1305Cipher;
        let key1 = [0x42u8; 32];
        let key2 = [0x43u8; 32];
        let plaintext = b"secret";

        let encrypted = cipher.encrypt(&key1, plaintext, b"").unwrap();
        let result = cipher.decrypt(&key2, &encrypted, b"");
        assert!(result.is_err());
    }

    #[test]
    fn wrong_aad_fails() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = b"secret";

        let encrypted = cipher.encrypt(&key, plaintext, b"correct").unwrap();
        let result = cipher.decrypt(&key, &encrypted, b"wrong");
        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = b"secret";

        let mut encrypted = cipher.encrypt(&key, plaintext, b"").unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0x01;
        let result = cipher.decrypt(&key, &encrypted, b"");
        assert!(result.is_err());
    }

    #[test]
    fn too_short_ciphertext_fails() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];

        let result = cipher.decrypt(&key, &[0u8; 30], b"");
        assert!(result.is_err());
    }

    #[test]
    fn different_nonces_produce_different_ciphertexts() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = b"same input";

        let enc1 = cipher.encrypt(&key, plaintext, b"").unwrap();
        let enc2 = cipher.encrypt(&key, plaintext, b"").unwrap();
        assert_ne!(enc1, enc2);
    }

    #[test]
    fn version_tag_is_0x01() {
        let cipher = XChaCha20Poly1305Cipher;
        assert_eq!(cipher.version_tag(), 0x01);
    }

    #[test]
    fn empty_plaintext_round_trip() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];

        let encrypted = cipher.encrypt(&key, b"", b"").unwrap();
        let decrypted = cipher.decrypt(&key, &encrypted, b"").unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn large_plaintext_round_trip() {
        let cipher = XChaCha20Poly1305Cipher;
        let key = [0x42u8; 32];
        let plaintext = vec![0xAB; 100_000];

        let encrypted = cipher.encrypt(&key, &plaintext, b"").unwrap();
        let decrypted = cipher.decrypt(&key, &encrypted, b"").unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
