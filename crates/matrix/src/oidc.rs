//! Matrix OIDC authentication (MSC3861).
//!
//! Implements the OAuth 2.0 / OIDC flow for Matrix homeservers that use
//! Matrix Authentication Service. Uses the matrix-sdk built-in OAuth API
//! (`Client::oauth()`), which handles PKCE, dynamic client registration,
//! token exchange, and automatic refresh.

use std::path::PathBuf;

use {
    matrix_sdk::{
        Client,
        authentication::oauth::{
            ClientRegistrationData, OAuthAuthorizationData, OAuthSession,
            registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
        },
        ruma::serde::Raw,
        store::RoomLoadSettings,
    },
    serde::{Deserialize, Serialize},
    tracing::{info, instrument, warn},
    url::Url,
};

use moltis_channels::{Error as ChannelError, Result as ChannelResult};

use crate::client::AuthenticatedMatrixAccount;

/// Data returned when the first phase of OIDC login succeeds.
pub(crate) struct OidcLoginPending {
    /// URL the user must open in a browser to authenticate.
    pub auth_url: String,
    /// CSRF state token (used to match the callback).
    pub state: String,
}

/// Persisted OIDC session (client_id + user tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedOidcSession {
    client_id: String,
    user_id: String,
    device_id: String,
    access_token: String,
    refresh_token: Option<String>,
}

fn oidc_session_path(account_id: &str) -> PathBuf {
    moltis_config::data_dir().join("matrix").join(format!(
        "{}-oidc-session.json",
        sanitize_account_id(account_id)
    ))
}

fn sanitize_account_id(account_id: &str) -> String {
    let sanitized = account_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

async fn save_oidc_session(account_id: &str, session: &OAuthSession) -> ChannelResult<()> {
    let path = oidc_session_path(account_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ChannelError::external("matrix oidc create session dir", error))?;
    }
    let persisted = PersistedOidcSession {
        client_id: session.client_id.to_string(),
        user_id: session.user.meta.user_id.to_string(),
        device_id: session.user.meta.device_id.to_string(),
        access_token: session.user.tokens.access_token.clone(),
        refresh_token: session.user.tokens.refresh_token.clone(),
    };
    let json = serde_json::to_string_pretty(&persisted)
        .map_err(|error| ChannelError::external("matrix oidc serialize session", error))?;
    std::fs::write(&path, json)
        .map_err(|error| ChannelError::external("matrix oidc write session", error))?;
    Ok(())
}

async fn load_oidc_session(account_id: &str) -> ChannelResult<Option<PersistedOidcSession>> {
    let path = oidc_session_path(account_id);
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            let session: PersistedOidcSession = serde_json::from_str(&json)
                .map_err(|error| ChannelError::external("matrix oidc parse session", error))?;
            Ok(Some(session))
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ChannelError::external("matrix oidc read session", error)),
    }
}

fn build_client_metadata(redirect_uri: &Url) -> ClientMetadata {
    let origin_url: Url = redirect_uri
        .origin()
        .unicode_serialization()
        .parse()
        .unwrap_or_else(|_| {
            "http://localhost"
                .parse()
                .unwrap_or_else(|error| panic!("fallback URL should parse: {error}"))
        });
    let client_uri = Localized::new(origin_url, std::iter::empty());
    ClientMetadata::new(
        ApplicationType::Native,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: vec![redirect_uri.clone()],
        }],
        client_uri,
    )
}

/// Phase 1: Start the OIDC login flow.
///
/// Discovers OIDC metadata, registers the client dynamically, and returns
/// an authorization URL for the user to open in a browser.
#[instrument(skip(client, redirect_uri), fields(account_id))]
pub(crate) async fn start_oidc_login(
    client: &Client,
    account_id: &str,
    redirect_uri: &Url,
    device_id: Option<&str>,
) -> ChannelResult<OidcLoginPending> {
    // Verify the homeserver supports OIDC.
    client
        .oauth()
        .server_metadata()
        .await
        .map_err(|error| ChannelError::external("matrix oidc server metadata discovery", error))?;

    let metadata = build_client_metadata(redirect_uri);
    let raw_metadata: Raw<ClientMetadata> = Raw::new(&metadata)
        .map_err(|error| ChannelError::external("matrix oidc serialize client metadata", error))?;
    let registration_data = ClientRegistrationData::new(raw_metadata);

    let device_id_owned = device_id
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
        .map(|device_id| device_id.into());

    let OAuthAuthorizationData { url, state } = client
        .oauth()
        .login(
            redirect_uri.clone(),
            device_id_owned,
            Some(registration_data),
            None,
        )
        .build()
        .await
        .map_err(|error| ChannelError::external("matrix oidc authorization code build", error))?;

    info!(account_id, auth_url = %url, "matrix OIDC login started");

    Ok(OidcLoginPending {
        auth_url: url.to_string(),
        state: state.secret().to_string(),
    })
}

/// Phase 2: Complete the OIDC login after the user authenticated in the browser.
#[instrument(skip(client, callback_url), fields(account_id))]
pub(crate) async fn finish_oidc_login(
    client: &Client,
    account_id: &str,
    callback_url: &str,
) -> ChannelResult<AuthenticatedMatrixAccount> {
    let url: Url = callback_url
        .parse()
        .map_err(|error| ChannelError::external("matrix oidc parse callback url", error))?;

    client
        .oauth()
        .finish_login(url.into())
        .await
        .map_err(|error| ChannelError::external("matrix oidc finish login", error))?;

    let session = client.oauth().full_session().ok_or_else(|| {
        ChannelError::invalid_input("matrix OIDC login completed but no session was created")
    })?;

    save_oidc_session(account_id, &session).await?;
    spawn_session_persistence_task(client, account_id);

    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;

    // Bootstrap cross-signing without password (OIDC handles auth differently).
    if let Err(error) = client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
    {
        warn!(
            account_id,
            error = %error,
            "matrix OIDC cross-signing bootstrap skipped (may require browser approval)"
        );
    }

    let user_id = session.user.meta.user_id;
    info!(account_id, user_id = %user_id, "matrix OIDC login complete");

    Ok(AuthenticatedMatrixAccount {
        user_id,
        ownership_startup_error: None,
    })
}

/// Restore a previously saved OIDC session (used during `authenticate_client`).
#[instrument(skip(client), fields(account_id))]
pub(crate) async fn restore_oidc_session(
    client: &Client,
    account_id: &str,
) -> ChannelResult<AuthenticatedMatrixAccount> {
    let persisted = load_oidc_session(account_id).await?.ok_or_else(|| {
        ChannelError::invalid_input(
            "no saved OIDC session found; complete the OIDC login flow first via channels.oauth_start",
        )
    })?;

    let user_id: matrix_sdk::ruma::OwnedUserId = persisted
        .user_id
        .parse()
        .map_err(|error| ChannelError::external("matrix oidc parse user_id", error))?;
    let device_id: matrix_sdk::ruma::OwnedDeviceId = persisted.device_id.into();
    let client_id = matrix_sdk::authentication::oauth::ClientId::new(persisted.client_id);

    let session = OAuthSession {
        client_id,
        user: matrix_sdk::authentication::oauth::UserSession {
            meta: matrix_sdk::SessionMeta {
                user_id: user_id.clone(),
                device_id,
            },
            tokens: matrix_sdk::authentication::SessionTokens {
                access_token: persisted.access_token,
                refresh_token: persisted.refresh_token,
            },
        },
    };

    client
        .oauth()
        .restore_session(session, RoomLoadSettings::default())
        .await
        .map_err(|error| ChannelError::external("matrix oidc restore session", error))?;

    spawn_session_persistence_task(client, account_id);

    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;

    info!(account_id, user_id = %user_id, "matrix OIDC session restored");

    Ok(AuthenticatedMatrixAccount {
        user_id,
        ownership_startup_error: None,
    })
}

/// Spawn a background task that persists refreshed tokens to disk.
fn spawn_session_persistence_task(client: &Client, account_id: &str) {
    let mut rx = client.subscribe_to_session_changes();
    let account_id = account_id.to_string();
    let client = client.clone();

    tokio::spawn(async move {
        while let Ok(change) = rx.recv().await {
            match change {
                matrix_sdk::SessionChange::TokensRefreshed => {
                    if let Some(session) = client.oauth().full_session()
                        && let Err(error) = save_oidc_session(&account_id, &session).await
                    {
                        warn!(
                            account_id = %account_id,
                            error = %error,
                            "failed to persist refreshed OIDC tokens"
                        );
                    }
                },
                matrix_sdk::SessionChange::UnknownToken { soft_logout } => {
                    warn!(
                        account_id = %account_id,
                        soft_logout,
                        "matrix OIDC session token invalidated"
                    );
                },
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_session_path_returns_expected_path() {
        let path = oidc_session_path("matrix-org-bot");
        let file_name = path.file_name().and_then(|name| name.to_str());
        assert_eq!(file_name, Some("matrix-org-bot-oidc-session.json"));
        assert!(path.to_string_lossy().contains("matrix"));
    }

    #[test]
    fn oidc_session_path_sanitizes_special_chars() {
        let path = oidc_session_path("matrix:org/test bot");
        let file_name = path.file_name().and_then(|name| name.to_str());
        assert_eq!(file_name, Some("matrix-org-test-bot-oidc-session.json"));
    }

    #[test]
    fn build_client_metadata_produces_valid_structure() {
        let redirect = "http://localhost:8080/api/oauth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("redirect url should parse: {error}"));
        let metadata = build_client_metadata(&redirect);
        assert_eq!(metadata.application_type, ApplicationType::Native);
        assert_eq!(metadata.grant_types.len(), 1);
        match &metadata.grant_types[0] {
            OAuthGrantType::AuthorizationCode { redirect_uris } => {
                assert_eq!(redirect_uris.len(), 1);
                assert_eq!(
                    redirect_uris[0].as_str(),
                    "http://localhost:8080/api/oauth/callback"
                );
            },
            other => panic!("expected AuthorizationCode grant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn save_and_load_oidc_session_round_trip() {
        let dir =
            tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir should work: {error}"));
        let account_id = "test-oidc-roundtrip";
        // Override path by testing the serialization logic directly.
        let persisted = PersistedOidcSession {
            client_id: "test-client-id".into(),
            user_id: "@bot:example.com".into(),
            device_id: "TESTDEVICE".into(),
            access_token: "test-access-token".into(),
            refresh_token: Some("test-refresh-token".into()),
        };
        let path = dir.path().join(format!("{account_id}-oidc-session.json"));
        let json = serde_json::to_string_pretty(&persisted)
            .unwrap_or_else(|error| panic!("serialize should work: {error}"));
        std::fs::write(&path, &json).unwrap_or_else(|error| panic!("write should work: {error}"));

        let loaded: PersistedOidcSession = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read should work: {error}")),
        )
        .unwrap_or_else(|error| panic!("parse should work: {error}"));

        assert_eq!(loaded.client_id, "test-client-id");
        assert_eq!(loaded.user_id, "@bot:example.com");
        assert_eq!(loaded.device_id, "TESTDEVICE");
        assert_eq!(loaded.access_token, "test-access-token");
        assert_eq!(loaded.refresh_token.as_deref(), Some("test-refresh-token"));
    }
}
