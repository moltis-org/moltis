//! Nostr key parsing and validation.
//!
//! Accepts `nsec1...` (bech32) or 64-character hex secret keys.
//! Derives public keys and normalizes `npub1...` or hex pubkeys.

use {
    nostr_sdk::prelude::{Keys, PublicKey, SecretKey},
    secrecy::{ExposeSecret, Secret},
};

use crate::error::Error;

/// Parse a secret key from either `nsec1...` bech32 or 64-char hex.
pub fn parse_secret_key(input: &Secret<String>) -> Result<SecretKey, Error> {
    let raw = input.expose_secret();
    SecretKey::parse(raw).map_err(|e| Error::Config(format!("invalid secret key: {e}")))
}

/// Derive the full `Keys` (secret + public) from a secret key string.
pub fn derive_keys(secret: &Secret<String>) -> Result<Keys, Error> {
    let sk = parse_secret_key(secret)?;
    Ok(Keys::new(sk))
}

/// Parse a public key from `npub1...` bech32 or 64-char hex.
pub fn parse_pubkey(input: &str) -> Result<PublicKey, Error> {
    PublicKey::parse(input).map_err(|e| Error::Config(format!("invalid pubkey '{input}': {e}")))
}

/// Normalize a list of pubkey strings (npub1/hex) into parsed `PublicKey`s.
/// Invalid entries are logged and skipped.
pub fn normalize_pubkeys(raw: &[String]) -> Vec<PublicKey> {
    raw.iter()
        .filter_map(|s| match parse_pubkey(s) {
            Ok(pk) => Some(pk),
            Err(e) => {
                tracing::warn!("skipping invalid pubkey in allowlist: {e}");
                None
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use {nostr_sdk::prelude::ToBech32, secrecy::Secret};

    use super::*;

    #[test]
    fn parse_hex_secret_key() {
        let keys = Keys::generate();
        let hex = keys.secret_key().to_secret_hex();
        let secret = Secret::new(hex);
        let parsed = parse_secret_key(&secret);
        assert!(parsed.is_ok());
        assert_eq!(
            parsed.ok().map(|sk| sk.to_secret_hex()),
            Some(keys.secret_key().to_secret_hex())
        );
    }

    #[test]
    fn parse_bech32_secret_key() {
        let keys = Keys::generate();
        // to_bech32() returns Result<String, Infallible>
        let nsec = keys.secret_key().to_bech32().unwrap_or_default();
        let secret = Secret::new(nsec);
        let parsed = parse_secret_key(&secret);
        assert!(parsed.is_ok());
        assert_eq!(
            parsed.ok().map(|sk| sk.to_secret_hex()),
            Some(keys.secret_key().to_secret_hex())
        );
    }

    #[test]
    fn derive_keys_from_secret() {
        let original = Keys::generate();
        let secret = Secret::new(original.secret_key().to_secret_hex());
        let derived = derive_keys(&secret);
        assert!(derived.is_ok());
        let derived = derived.unwrap_or_else(|e| panic!("derive_keys failed: {e}"));
        assert_eq!(derived.public_key(), original.public_key());
    }

    #[test]
    fn parse_pubkey_hex() {
        let keys = Keys::generate();
        let hex = keys.public_key().to_hex();
        let parsed = parse_pubkey(&hex);
        assert!(parsed.is_ok());
        assert_eq!(parsed.ok(), Some(keys.public_key()));
    }

    #[test]
    fn parse_pubkey_bech32() {
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().unwrap_or_default();
        let parsed = parse_pubkey(&npub);
        assert!(parsed.is_ok());
        assert_eq!(parsed.ok(), Some(keys.public_key()));
    }

    #[test]
    fn invalid_secret_key_rejected() {
        let secret = Secret::new("not-a-key".to_string());
        assert!(parse_secret_key(&secret).is_err());
    }

    #[test]
    fn invalid_pubkey_rejected() {
        assert!(parse_pubkey("not-a-pubkey").is_err());
    }

    #[test]
    fn normalize_pubkeys_skips_invalid() {
        let keys = Keys::generate();
        let valid = keys.public_key().to_hex();
        let list = vec![valid.clone(), "invalid".to_string()];
        let result = normalize_pubkeys(&list);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].to_hex(), valid);
    }
}
