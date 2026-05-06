//! Shared serde helpers for `Secret<String>` fields.
//!
//! Two serialization paths:
//! - **Storage** (`serialize_secret`, `serialize_option_secret`): exposes raw values for persistence.
//! - **Redacted** (`REDACTED` constant): used by per-channel `RedactedConfig` wrapper types to
//!   write `"[REDACTED]"` for API responses where secrets must not leak.
//!
//! The per-channel `RedactedConfig` wrapper types let channel config structs opt into the
//! redacted path without cloning or runtime string matching.

use secrecy::{ExposeSecret, Secret};

/// Sentinel value used for redacted secret fields in API responses.
pub const REDACTED: &str = "[REDACTED]";

// ---------------------------------------------------------------------------
// Storage path (exposes raw value)
// ---------------------------------------------------------------------------

/// Serialize a `Secret<String>` by exposing the raw value. Use for storage/persistence only.
pub fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

/// Serialize an `Option<Secret<String>>` by exposing the raw value (or `null`).
pub fn serialize_option_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_some(s.expose_secret()),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {super::*, serde::Serialize};

    #[derive(Serialize)]
    struct StorageExample {
        #[serde(serialize_with = "serialize_secret")]
        token: Secret<String>,
        #[serde(serialize_with = "serialize_option_secret")]
        optional: Option<Secret<String>>,
    }

    #[test]
    fn storage_path_exposes_values() {
        let ex = StorageExample {
            token: Secret::new("my-secret".into()),
            optional: Some(Secret::new("opt-secret".into())),
        };
        let v = serde_json::to_value(&ex).unwrap();
        assert_eq!(v["token"], "my-secret");
        assert_eq!(v["optional"], "opt-secret");
    }

    #[test]
    fn storage_path_option_none_is_null() {
        let ex = StorageExample {
            token: Secret::new("tok".into()),
            optional: None,
        };
        let v = serde_json::to_value(&ex).unwrap();
        assert!(v["optional"].is_null());
    }
}
