use {
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    rand::Rng,
    sha2::{Digest, Sha256},
};

use crate::types::PkceChallenge;

/// Generate a PKCE S256 challenge pair.
pub fn generate_pkce() -> PkceChallenge {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    PkceChallenge {
        verifier,
        challenge,
    }
}

/// Generate a random state parameter.
pub fn generate_state() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
