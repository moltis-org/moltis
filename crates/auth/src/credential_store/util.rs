use {
    argon2::{
        Argon2,
        password_hash::{
            PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
        },
    },
    sha2::{Digest, Sha256},
};

/// Pre-computed Argon2 hash used for constant-time dummy verification when no password is set.
pub(crate) const DUMMY_ARGON2_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub fn is_loopback(ip: &str) -> bool {
    ip == "127.0.0.1" || ip.starts_with("127.") || ip == "::1" || ip.starts_with("::ffff:127.")
}

pub(crate) fn hash_password(password: &str) -> crate::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| crate::Error::Crypto(format!("failed to hash password: {e}")))?;
    Ok(hash.to_string())
}

pub(crate) fn verify_password(password: &str, hash_str: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash_str) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub(crate) fn generate_token() -> String {
    use {base64::Engine, rand::Rng};

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn hmac_sha256_hex(input: &str, salt: &str) -> String {
    use hmac::{Hmac, Mac};

    type HmacSha256 = Hmac<Sha256>;
    let Ok(mut mac) = HmacSha256::new_from_slice(salt.as_bytes()) else {
        return sha256_hex(input);
    };
    mac.update(input.as_bytes());
    format!("{:x}", mac.finalize().into_bytes())
}

pub(crate) fn safe_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a
        .as_bytes()
        .iter()
        .zip(b.as_bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}
