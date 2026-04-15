use {
    serde::{Deserialize, Serialize},
    serde_json::{Map, Value},
};

/// Tagged persisted representation for secrets stored inside JSON configs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoredSecret {
    VaultEncrypted { ciphertext: String },
}

impl StoredSecret {
    fn into_value(self) -> Result<Value, Error> {
        serde_json::to_value(self).map_err(Error::from)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("secret-bearing config must be a JSON object")]
    ConfigMustBeObject,

    #[error("stored secret field '{field}' has an unsupported JSON type")]
    InvalidSecretFieldType { field: String },

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[cfg(feature = "vault")]
    #[error(transparent)]
    Vault(#[from] moltis_vault::VaultError),
}

fn object(config: &Value) -> Result<&Map<String, Value>, Error> {
    config.as_object().ok_or(Error::ConfigMustBeObject)
}

fn object_mut(config: &mut Value) -> Result<&mut Map<String, Value>, Error> {
    config.as_object_mut().ok_or(Error::ConfigMustBeObject)
}

fn parse_stored_secret(value: &Value) -> Result<Option<StoredSecret>, Error> {
    let Value::Object(map) = value else {
        return Ok(None);
    };

    if !map.contains_key("kind") {
        return Ok(None);
    }

    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(Error::from)
}

fn field_aad(scope: &str, field: &str) -> String {
    format!("{scope}:{field}")
}

/// Whether any declared secret field is still stored as plaintext.
pub fn has_plaintext_secret_fields(config: &Value, secret_fields: &[&str]) -> Result<bool, Error> {
    let map = object(config)?;
    for field in secret_fields {
        let Some(value) = map.get(*field) else {
            continue;
        };

        if value.is_string() {
            return Ok(true);
        }

        if parse_stored_secret(value)?.is_some() {
            continue;
        }

        if value.is_null() {
            continue;
        }

        return Err(Error::InvalidSecretFieldType {
            field: (*field).to_string(),
        });
    }

    Ok(false)
}

/// Whether any declared secret field is stored as an encrypted tagged object.
pub fn has_encrypted_secret_fields(config: &Value, secret_fields: &[&str]) -> Result<bool, Error> {
    let map = object(config)?;
    for field in secret_fields {
        let Some(value) = map.get(*field) else {
            continue;
        };

        if parse_stored_secret(value)?.is_some() {
            return Ok(true);
        }

        if value.is_string() || value.is_null() {
            continue;
        }

        return Err(Error::InvalidSecretFieldType {
            field: (*field).to_string(),
        });
    }

    Ok(false)
}

#[cfg(feature = "vault")]
pub async fn encrypt_secret_fields<C: moltis_vault::Cipher>(
    config: &mut Value,
    secret_fields: &[&str],
    aad_scope: &str,
    vault: &moltis_vault::Vault<C>,
) -> Result<bool, Error> {
    let map = object_mut(config)?;
    let mut changed = false;

    for field in secret_fields {
        let Some(value) = map.get_mut(*field) else {
            continue;
        };

        match value {
            Value::String(plaintext) => {
                let ciphertext = vault
                    .encrypt_string(plaintext, &field_aad(aad_scope, field))
                    .await?;
                *value = StoredSecret::VaultEncrypted { ciphertext }.into_value()?;
                changed = true;
            },
            Value::Null => {},
            _ if parse_stored_secret(value)?.is_some() => {},
            _ => {
                return Err(Error::InvalidSecretFieldType {
                    field: (*field).to_string(),
                });
            },
        }
    }

    Ok(changed)
}

#[cfg(feature = "vault")]
pub async fn decrypt_secret_fields<C: moltis_vault::Cipher>(
    config: &mut Value,
    secret_fields: &[&str],
    aad_scope: &str,
    vault: &moltis_vault::Vault<C>,
) -> Result<bool, Error> {
    let map = object_mut(config)?;
    let mut changed = false;

    for field in secret_fields {
        let Some(value) = map.get_mut(*field) else {
            continue;
        };

        match parse_stored_secret(value)? {
            Some(StoredSecret::VaultEncrypted { ciphertext }) => {
                let plaintext = vault
                    .decrypt_string(&ciphertext, &field_aad(aad_scope, field))
                    .await?;
                *value = Value::String(plaintext);
                changed = true;
            },
            None if value.is_string() || value.is_null() => {},
            None => {
                return Err(Error::InvalidSecretFieldType {
                    field: (*field).to_string(),
                });
            },
        }
    }

    Ok(changed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[cfg(feature = "vault")]
    async fn test_vault() -> moltis_vault::Vault<moltis_vault::XChaCha20Poly1305Cipher> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS vault_metadata (
                id                   INTEGER PRIMARY KEY CHECK (id = 1),
                version              INTEGER NOT NULL DEFAULT 1,
                kdf_salt             TEXT NOT NULL,
                kdf_params           TEXT NOT NULL,
                wrapped_dek          TEXT NOT NULL,
                recovery_wrapped_dek TEXT,
                recovery_key_hash    TEXT,
                created_at           TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let vault = moltis_vault::Vault::with_cipher(pool, moltis_vault::XChaCha20Poly1305Cipher)
            .await
            .unwrap();
        vault.initialize("test-password").await.unwrap();
        vault
    }

    #[test]
    fn plaintext_detection_accepts_legacy_strings() {
        let config = serde_json::json!({
            "token": "secret-token",
            "room_policy": "allowlist"
        });

        assert!(has_plaintext_secret_fields(&config, &["token"]).unwrap());
        assert!(!has_encrypted_secret_fields(&config, &["token"]).unwrap());
    }

    #[cfg(feature = "vault")]
    #[tokio::test]
    async fn encrypt_and_decrypt_round_trip_secret_fields() {
        let vault = test_vault().await;
        let mut config = serde_json::json!({
            "token": "secret-token",
            "password": "hunter2",
            "room_policy": "allowlist"
        });

        let encrypted = encrypt_secret_fields(
            &mut config,
            &["token", "password"],
            "channel:matrix:test",
            &vault,
        )
        .await
        .unwrap();
        assert!(encrypted);
        assert!(has_encrypted_secret_fields(&config, &["token", "password"]).unwrap());
        assert_eq!(config["room_policy"], "allowlist");

        let decrypted = decrypt_secret_fields(
            &mut config,
            &["token", "password"],
            "channel:matrix:test",
            &vault,
        )
        .await
        .unwrap();
        assert!(decrypted);
        assert_eq!(config["token"], "secret-token");
        assert_eq!(config["password"], "hunter2");
    }

    #[cfg(feature = "vault")]
    #[tokio::test]
    async fn encrypt_skips_already_tagged_fields() {
        let vault = test_vault().await;
        let ciphertext = vault
            .encrypt_string("secret-token", "channel:telegram:bot1:token")
            .await
            .unwrap();
        let mut config = serde_json::json!({
            "token": {
                "kind": "vault_encrypted",
                "ciphertext": ciphertext
            }
        });

        let changed =
            encrypt_secret_fields(&mut config, &["token"], "channel:telegram:bot1", &vault)
                .await
                .unwrap();
        assert!(!changed);
        assert!(has_encrypted_secret_fields(&config, &["token"]).unwrap());
    }
}
