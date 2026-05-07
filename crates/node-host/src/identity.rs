//! Ed25519 node identity: keypair generation, persistence, and fingerprinting.
//!
//! Each node generates a unique Ed25519 keypair on first run. The private key
//! is stored at `~/.moltis/node_key` (mode 0600 on Unix), the public key at
//! `~/.moltis/node_key.pub`. The gateway pins the public key on first approval
//! (TOFU model) and verifies subsequent connections via challenge-response.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use {
    base64::{Engine, engine::general_purpose::STANDARD as BASE64},
    ed25519_dalek::{SigningKey, VerifyingKey},
    rand::Rng,
    sha2::{Digest, Sha256},
};

use crate::error::{Error, Result};

// ── NodeIdentity (newtype for SigningKey) ────────────────────────────────────

/// Wraps an Ed25519 `SigningKey` with a safe `Debug` impl.
pub struct NodeIdentity {
    signing_key: SigningKey,
}

impl fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeIdentity")
            .field("fingerprint", &self.fingerprint())
            .finish()
    }
}

impl NodeIdentity {
    /// Create from an existing `SigningKey`.
    pub fn from_signing_key(signing_key: SigningKey) -> Self {
        Self { signing_key }
    }

    /// The Ed25519 signing key (private).
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// The Ed25519 verifying key (public).
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Base64-encoded public key bytes (32 bytes → standard base64).
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.verifying_key().as_bytes())
    }

    /// Human-readable fingerprint: `SHA256:<base64 of SHA-256 of public key>`.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.verifying_key())
    }

    /// Sign arbitrary bytes and return the 64-byte signature.
    pub fn sign(&self, message: &[u8]) -> ed25519_dalek::Signature {
        use ed25519_dalek::Signer;
        self.signing_key.sign(message)
    }
}

// ── Fingerprint helper ──────────────────────────────────────────────────────

/// Compute `SHA256:<base64>` fingerprint for a verifying (public) key.
pub fn fingerprint(verifying_key: &VerifyingKey) -> String {
    let hash = Sha256::digest(verifying_key.as_bytes());
    format!("SHA256:{}", BASE64.encode(hash))
}

// ── Key persistence ─────────────────────────────────────────────────────────

const PRIVATE_KEY_FILENAME: &str = "node_key";
const PUBLIC_KEY_FILENAME: &str = "node_key.pub";

/// Resolve the path to the private key file inside `dir`.
fn private_key_path(dir: &Path) -> PathBuf {
    dir.join(PRIVATE_KEY_FILENAME)
}

/// Resolve the path to the public key file inside `dir`.
fn public_key_path(dir: &Path) -> PathBuf {
    dir.join(PUBLIC_KEY_FILENAME)
}

/// Load an existing identity from `dir`, or generate and persist a new one.
///
/// `dir` is typically `~/.moltis/` (from `moltis_config::data_dir()`).
///
/// On Unix the private key file is created with mode 0600. If the file exists
/// with wrong permissions, this function returns an error rather than silently
/// proceeding.
pub fn load_or_create(dir: &Path) -> Result<NodeIdentity> {
    let priv_path = private_key_path(dir);
    if priv_path.exists() {
        load(dir)
    } else {
        generate_and_save(dir)
    }
}

/// Load an existing keypair from `dir`. Returns an error if the files are
/// missing or have wrong permissions.
pub fn load(dir: &Path) -> Result<NodeIdentity> {
    let priv_path = private_key_path(dir);

    // Check permissions before reading (Unix only).
    #[cfg(unix)]
    check_permissions(&priv_path)?;

    let seed_bytes = fs::read(&priv_path).map_err(|e| {
        Error::Config(format!(
            "failed to read node private key at {}: {e}",
            priv_path.display()
        ))
    })?;

    if seed_bytes.len() != 32 {
        return Err(Error::Config(format!(
            "node private key at {} has invalid length {} (expected 32 bytes)",
            priv_path.display(),
            seed_bytes.len()
        )));
    }

    let seed: [u8; 32] = seed_bytes.try_into().map_err(|_| {
        Error::Config(format!(
            "node private key at {} has unexpected length",
            priv_path.display()
        ))
    })?;

    let signing_key = SigningKey::from_bytes(&seed);
    Ok(NodeIdentity::from_signing_key(signing_key))
}

/// Generate a fresh Ed25519 keypair and write it to `dir`.
fn generate_and_save(dir: &Path) -> Result<NodeIdentity> {
    fs::create_dir_all(dir).map_err(|e| {
        Error::Config(format!(
            "failed to create config directory {}: {e}",
            dir.display()
        ))
    })?;

    // Generate 32 random bytes from the OS CSPRNG via rand 0.10, then
    // construct the key from those bytes. We avoid `SigningKey::generate`
    // directly because ed25519-dalek 2.x depends on rand_core 0.6 while the
    // workspace uses rand 0.10 (rand_core 0.9).
    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    // Write private key (raw 32-byte seed).
    // On Unix, create the file with mode 0600 atomically to avoid a window
    // where the key is world-readable between write and chmod.
    let priv_path = private_key_path(dir);
    write_private_file(&priv_path, &signing_key.to_bytes())?;

    // Write public key (raw 32-byte public key).
    let pub_path = public_key_path(dir);
    fs::write(&pub_path, verifying_key.as_bytes()).map_err(|e| {
        Error::Config(format!(
            "failed to write node public key to {}: {e}",
            pub_path.display()
        ))
    })?;

    Ok(NodeIdentity::from_signing_key(signing_key))
}

/// Write a file with owner-only permissions atomically.
///
/// On Unix, the file is opened with mode 0600 at creation time so the key
/// is never world-readable, even briefly. On other platforms, falls back to
/// a normal write (inheriting parent directory ACLs).
#[cfg(unix)]
fn write_private_file(path: &Path, data: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| {
            Error::Config(format!(
                "failed to create node private key at {}: {e}",
                path.display()
            ))
        })?;
    file.write_all(data).map_err(|e| {
        Error::Config(format!(
            "failed to write node private key to {}: {e}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).map_err(|e| {
        Error::Config(format!(
            "failed to write node private key to {}: {e}",
            path.display()
        ))
    })
}

/// Verify the private key file has mode 0600 (owner-only read/write).
#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).map_err(|e| {
        Error::Config(format!(
            "failed to read metadata for {}: {e}",
            path.display()
        ))
    })?;

    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(Error::Config(format!(
            "node private key {} has permissions {mode:04o}, expected 0600 — \
             fix with: chmod 600 {}",
            path.display(),
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn generate_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let identity = load_or_create(dir.path()).unwrap();
        let fp1 = identity.fingerprint();

        // Loading the same dir should produce the same identity.
        let identity2 = load_or_create(dir.path()).unwrap();
        assert_eq!(fp1, identity2.fingerprint());
        assert_eq!(identity.public_key_base64(), identity2.public_key_base64());
    }

    #[test]
    fn fingerprint_format() {
        let dir = tempfile::tempdir().unwrap();
        let identity = load_or_create(dir.path()).unwrap();
        let fp = identity.fingerprint();
        assert!(
            fp.starts_with("SHA256:"),
            "fingerprint should start with SHA256:"
        );
        // SHA-256 base64 is 44 chars.
        assert_eq!(fp.len(), 7 + 44, "SHA256: prefix + 44 base64 chars");
    }

    #[test]
    fn public_key_base64_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let identity = load_or_create(dir.path()).unwrap();
        let pk_b64 = identity.public_key_base64();
        let decoded = BASE64.decode(&pk_b64).unwrap();
        assert_eq!(decoded.len(), 32, "Ed25519 public key is 32 bytes");
    }

    #[test]
    fn sign_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let identity = load_or_create(dir.path()).unwrap();
        let message = b"challenge-nonce-12345";
        let signature = identity.sign(message);

        use ed25519_dalek::Verifier;
        identity
            .verifying_key()
            .verify(message, &signature)
            .expect("signature should be valid");
    }

    #[cfg(unix)]
    #[test]
    fn wrong_permissions_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let _identity = load_or_create(dir.path()).unwrap();

        // Widen permissions.
        let priv_path = dir.path().join("node_key");
        fs::set_permissions(&priv_path, fs::Permissions::from_mode(0o644)).unwrap();

        let err = load(dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("0644") && msg.contains("0600"),
            "error should mention both actual and expected mode: {msg}"
        );
    }

    #[test]
    fn invalid_key_length_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let priv_path = dir.path().join("node_key");
        fs::write(&priv_path, b"too-short").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&priv_path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let err = load(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("invalid length"),
            "should report invalid length: {err}"
        );
    }

    #[test]
    fn debug_impl_redacts_key() {
        let dir = tempfile::tempdir().unwrap();
        let identity = load_or_create(dir.path()).unwrap();
        let debug = format!("{identity:?}");
        assert!(debug.contains("SHA256:"), "debug should show fingerprint");
        // Must not contain raw key material.
        assert!(
            !debug.contains(&identity.public_key_base64()),
            "debug should not expose raw public key"
        );
    }
}
