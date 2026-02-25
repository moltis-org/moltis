//! Argon2id key derivation for password → KEK.

use {argon2::Argon2, zeroize::Zeroizing};

use crate::error::VaultError;

/// Argon2id parameters stored alongside the wrapped DEK.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KdfParams {
    /// Memory cost in KiB (default: 64 MiB = 65536).
    pub m_cost: u32,
    /// Number of iterations (default: 3).
    pub t_cost: u32,
    /// Degree of parallelism (default: 1).
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost: 65536, // 64 MiB
            t_cost: 3,
            p_cost: 1,
        }
    }
}

/// Derive a 256-bit key from a password and salt using Argon2id.
pub fn derive_key(
    password: &[u8],
    salt: &[u8],
    params: &KdfParams,
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let argon2_params = argon2::Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
        .map_err(|e| VaultError::CipherError(format!("invalid KDF params: {e}")))?;

    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2_params,
    );

    let mut output = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(password, salt, output.as_mut())
        .map_err(|e| VaultError::CipherError(format!("KDF failed: {e}")))?;

    Ok(output)
}

/// Generate a random 16-byte salt and return it as base64.
pub fn generate_salt() -> String {
    use {base64::Engine, rand::RngCore};

    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    base64::engine::general_purpose::STANDARD.encode(salt)
}

/// Decode a base64-encoded salt.
pub fn decode_salt(b64: &str) -> Result<Vec<u8>, VaultError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(VaultError::Base64)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_deterministic() {
        let params = KdfParams {
            m_cost: 256, // Low cost for tests
            t_cost: 1,
            p_cost: 1,
        };
        let salt = b"test-salt-16byte";

        let key1 = derive_key(b"password", salt, &params).unwrap();
        let key2 = derive_key(b"password", salt, &params).unwrap();
        assert_eq!(*key1, *key2);
    }

    #[test]
    fn different_passwords_different_keys() {
        let params = KdfParams {
            m_cost: 256,
            t_cost: 1,
            p_cost: 1,
        };
        let salt = b"test-salt-16byte";

        let key1 = derive_key(b"password1", salt, &params).unwrap();
        let key2 = derive_key(b"password2", salt, &params).unwrap();
        assert_ne!(*key1, *key2);
    }

    #[test]
    fn different_salts_different_keys() {
        let params = KdfParams {
            m_cost: 256,
            t_cost: 1,
            p_cost: 1,
        };

        let key1 = derive_key(b"password", b"salt-aaaaaaaaaaaa", &params).unwrap();
        let key2 = derive_key(b"password", b"salt-bbbbbbbbbbbb", &params).unwrap();
        assert_ne!(*key1, *key2);
    }

    #[test]
    fn generate_and_decode_salt() {
        let b64 = generate_salt();
        let decoded = decode_salt(&b64).unwrap();
        assert_eq!(decoded.len(), 16);
    }

    #[test]
    fn kdf_params_serialization() {
        let params = KdfParams::default();
        let json = serde_json::to_string(&params).unwrap();
        let parsed: KdfParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.m_cost, params.m_cost);
        assert_eq!(parsed.t_cost, params.t_cost);
        assert_eq!(parsed.p_cost, params.p_cost);
    }
}
