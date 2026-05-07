//! Matrix OIDC authentication (MSC3861).
//!
//! Implements the OAuth 2.0 / OIDC flow for Matrix homeservers that use
//! Matrix Authentication Service. Uses the matrix-sdk built-in OAuth API
//! (`Client::oauth()`), which handles PKCE, dynamic client registration,
//! token exchange, and automatic refresh.

use std::{error::Error as StdError, fmt, path::PathBuf};

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
    moltis_common::secret_serde,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tracing::{debug, info, instrument, warn},
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
#[derive(Clone, Serialize, Deserialize)]
struct PersistedOidcSession {
    client_id: String,
    user_id: String,
    device_id: String,
    #[serde(serialize_with = "secret_serde::serialize_secret")]
    access_token: Secret<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "secret_serde::serialize_option_secret"
    )]
    refresh_token: Option<Secret<String>>,
}

impl fmt::Debug for PersistedOidcSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedOidcSession")
            .field("client_id", &self.client_id)
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
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
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| ChannelError::external("matrix oidc create session dir", error))?;
    }
    let persisted = PersistedOidcSession {
        client_id: session.client_id.to_string(),
        user_id: session.user.meta.user_id.to_string(),
        device_id: session.user.meta.device_id.to_string(),
        access_token: Secret::new(session.user.tokens.access_token.clone()),
        refresh_token: session.user.tokens.refresh_token.clone().map(Secret::new),
    };
    let json = serde_json::to_string_pretty(&persisted)
        .map_err(|error| ChannelError::external("matrix oidc serialize session", error))?;
    write_session_file(&path, json.as_bytes()).await?;
    Ok(())
}

/// Write session data with restrictive file permissions (0o600 on Unix).
async fn write_session_file(path: &std::path::Path, data: &[u8]) -> ChannelResult<()> {
    // Write to a temporary location then set permissions before the final path,
    // or write directly with restricted perms on Unix.
    tokio::fs::write(path, data)
        .await
        .map_err(|error| ChannelError::external("matrix oidc write session", error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(path, perms)
            .await
            .map_err(|error| {
                ChannelError::external("matrix oidc set session file permissions", error)
            })?;
    }

    Ok(())
}

async fn load_oidc_session(account_id: &str) -> ChannelResult<Option<PersistedOidcSession>> {
    let path = oidc_session_path(account_id);
    match tokio::fs::read_to_string(&path).await {
        Ok(json) => {
            let session: PersistedOidcSession = serde_json::from_str(&json)
                .map_err(|error| ChannelError::external("matrix oidc parse session", error))?;
            Ok(Some(session))
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ChannelError::external("matrix oidc read session", error)),
    }
}

/// Project URL used as `client_uri` during OIDC dynamic client registration.
/// MAS validates this URL and rejects loopback addresses.
const MOLTIS_CLIENT_URI: &str = "https://moltis.org/";

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClientRegistrationDiagnostics {
    original_redirect_uri: String,
    registration_redirect_uri: String,
    is_loopback: bool,
    application_type: String,
    issuer: String,
    registration_endpoint: Option<String>,
    client_metadata_json: String,
}

impl ClientRegistrationDiagnostics {
    fn new(
        original_redirect_uri: &Url,
        registration_redirect_uri: &Url,
        metadata: &ClientMetadata,
        issuer: &Url,
        registration_endpoint: Option<&Url>,
        raw_metadata: &Raw<ClientMetadata>,
    ) -> Self {
        Self {
            original_redirect_uri: original_redirect_uri.to_string(),
            registration_redirect_uri: registration_redirect_uri.to_string(),
            is_loopback: is_loopback_uri(original_redirect_uri),
            application_type: metadata.application_type.to_string(),
            issuer: issuer.to_string(),
            registration_endpoint: registration_endpoint.map(ToString::to_string),
            client_metadata_json: raw_metadata.json().get().to_string(),
        }
    }
}

impl fmt::Display for ClientRegistrationDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "original_redirect_uri={}, registration_redirect_uri={}, is_loopback={}, \
             application_type={}, issuer={}, registration_endpoint={}, client_metadata_json={}",
            self.original_redirect_uri,
            self.registration_redirect_uri,
            self.is_loopback,
            self.application_type,
            self.issuer,
            self.registration_endpoint.as_deref().unwrap_or("none"),
            self.client_metadata_json
        )
    }
}

#[derive(Debug)]
struct ClientRegistrationFailure {
    diagnostics: ClientRegistrationDiagnostics,
    source: Box<dyn StdError + Send + Sync>,
}

impl ClientRegistrationFailure {
    fn new(
        diagnostics: ClientRegistrationDiagnostics,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            diagnostics,
            source: Box::new(source),
        }
    }
}

impl fmt::Display for ClientRegistrationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}; diagnostics: {}", self.source, self.diagnostics)
    }
}

impl StdError for ClientRegistrationFailure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

fn is_loopback_uri(uri: &Url) -> bool {
    let host = uri.host_str().unwrap_or_default();
    if host == "localhost" || host == "::1" || host.ends_with(".localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return ip.is_loopback();
    }
    false
}

/// Rewrite loopback redirect URIs from `https://` to `http://`.
///
/// MAS follows RFC 8252 §7.3 and requires native/loopback redirect URIs to
/// use `http://`. When Moltis serves over TLS on localhost, the browser's
/// HTTP-to-HTTPS redirect (or HSTS) will still deliver the callback to the
/// HTTPS server, so the OAuth code+state arrive correctly.
fn normalize_loopback_redirect(redirect_uri: &Url) -> Url {
    if is_loopback_uri(redirect_uri) && redirect_uri.scheme() == "https" {
        let mut normalized = redirect_uri.clone();
        let _ = normalized.set_scheme("http");
        normalized
    } else {
        redirect_uri.clone()
    }
}

/// Build OIDC client metadata for dynamic registration.
///
/// `redirect_uri` must already be normalized (loopback https -> http) via
/// [`normalize_loopback_redirect`] before calling this.
fn build_client_metadata(redirect_uri: &Url) -> ChannelResult<ClientMetadata> {
    let client_uri_url: Url = MOLTIS_CLIENT_URI
        .parse()
        .map_err(|error| ChannelError::external("matrix oidc parse client uri", error))?;
    let client_uri = Localized::new(client_uri_url, std::iter::empty());
    // MAS requires `Native` for loopback redirect URIs (RFC 8252) and `Web`
    // for non-loopback URIs (e.g. behind a reverse proxy).
    let app_type = if is_loopback_uri(redirect_uri) {
        ApplicationType::Native
    } else {
        ApplicationType::Web
    };
    Ok(ClientMetadata::new(
        app_type,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: vec![redirect_uri.clone()],
        }],
        client_uri,
    ))
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
    let server_metadata =
        client.oauth().server_metadata().await.map_err(|error| {
            ChannelError::external("matrix oidc server metadata discovery", error)
        })?;

    debug!(
        account_id,
        issuer = %server_metadata.issuer,
        registration_endpoint = ?server_metadata.registration_endpoint,
        "matrix OIDC server metadata discovered"
    );

    let registration_redirect = normalize_loopback_redirect(redirect_uri);
    let metadata = build_client_metadata(&registration_redirect)?;

    let is_loopback = is_loopback_uri(redirect_uri);
    debug!(
        account_id,
        original_redirect_uri = %redirect_uri,
        registration_redirect_uri = %registration_redirect,
        is_loopback,
        application_type = ?metadata.application_type,
        "matrix OIDC client registration parameters"
    );

    let raw_metadata: Raw<ClientMetadata> = Raw::new(&metadata)
        .map_err(|error| ChannelError::external("matrix oidc serialize client metadata", error))?;
    let diagnostics = ClientRegistrationDiagnostics::new(
        redirect_uri,
        &registration_redirect,
        &metadata,
        &server_metadata.issuer,
        server_metadata.registration_endpoint.as_ref(),
        &raw_metadata,
    );

    // Log the serialized metadata so operators can see exactly what is sent
    // to the MAS registration endpoint.
    debug!(
        account_id,
        client_metadata = %diagnostics.client_metadata_json,
        "matrix OIDC client metadata for dynamic registration"
    );

    let registration_data = ClientRegistrationData::new(raw_metadata);

    let device_id_owned = device_id
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
        .map(|device_id| device_id.into());

    // The redirect_uri in the authorization request must match the one
    // registered during client registration (the normalized loopback version).
    let OAuthAuthorizationData { url, state } = client
        .oauth()
        .login(
            registration_redirect,
            device_id_owned,
            Some(registration_data),
            None,
        )
        .build()
        .await
        .map_err(|error| {
            let failure = ClientRegistrationFailure::new(diagnostics.clone(), error);
            warn!(
                account_id,
                original_redirect_uri = %diagnostics.original_redirect_uri,
                registration_redirect_uri = %diagnostics.registration_redirect_uri,
                is_loopback = diagnostics.is_loopback,
                application_type = %diagnostics.application_type,
                issuer = %diagnostics.issuer,
                registration_endpoint = %diagnostics.registration_endpoint.as_deref().unwrap_or("none"),
                client_metadata = %diagnostics.client_metadata_json,
                error = %failure,
                error_debug = ?failure,
                "matrix OIDC client registration failed, \
                 check that redirect_uri is reachable and the homeserver's \
                 MAS allows dynamic client registration with this URI"
            );
            ChannelError::external("matrix oidc authorization code build", failure)
        })?;

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
                access_token: persisted.access_token.expose_secret().clone(),
                refresh_token: persisted
                    .refresh_token
                    .map(|secret| secret.expose_secret().clone()),
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
    use {
        crate::oidc::{
            ClientRegistrationDiagnostics, MOLTIS_CLIENT_URI, PersistedOidcSession,
            build_client_metadata, is_loopback_uri, normalize_loopback_redirect, oidc_session_path,
        },
        matrix_sdk::{
            authentication::oauth::registration::{ApplicationType, OAuthGrantType},
            ruma::serde::Raw,
        },
        secrecy::{ExposeSecret, Secret},
        url::Url,
    };

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
        let metadata =
            build_client_metadata(&redirect).unwrap_or_else(|error| panic!("should work: {error}"));
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

    #[test]
    fn build_client_metadata_uses_pre_normalized_loopback_and_project_client_uri() {
        // Caller must normalize before passing to build_client_metadata.
        let redirect: Url = "https://localhost:52979/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let normalized = normalize_loopback_redirect(&redirect);
        let metadata = build_client_metadata(&normalized).unwrap_or_else(|error| panic!("{error}"));
        match &metadata.grant_types[0] {
            OAuthGrantType::AuthorizationCode { redirect_uris } => {
                assert_eq!(
                    redirect_uris[0].as_str(),
                    "http://localhost:52979/auth/callback",
                    "loopback redirect_uri must be http:// for MAS RFC 8252 compliance"
                );
            },
            other => panic!("expected AuthorizationCode, got {other:?}"),
        }
        assert_eq!(
            metadata.client_uri.get(None).map(|u| u.as_str()),
            Some(MOLTIS_CLIENT_URI),
            "client_uri should be the project URL"
        );
    }

    #[test]
    fn normalize_loopback_redirect_rewrites_https_localhost() {
        let url: Url = "https://localhost:52979/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            normalize_loopback_redirect(&url).as_str(),
            "http://localhost:52979/auth/callback"
        );
    }

    #[test]
    fn normalize_loopback_redirect_preserves_non_loopback() {
        let url: Url = "https://moltis.example.com/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            normalize_loopback_redirect(&url).as_str(),
            "https://moltis.example.com/auth/callback"
        );
    }

    #[test]
    fn normalize_loopback_redirect_preserves_http_localhost() {
        let url: Url = "http://localhost:8080/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            normalize_loopback_redirect(&url).as_str(),
            "http://localhost:8080/auth/callback"
        );
    }

    #[test]
    fn build_client_metadata_uses_web_application_type_for_reverse_proxy() {
        let redirect: Url = "https://moltis.example.com/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let metadata = build_client_metadata(&redirect).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            metadata.application_type,
            ApplicationType::Web,
            "non-loopback redirect_uri must use ApplicationType::Web for MAS compatibility"
        );
        match &metadata.grant_types[0] {
            OAuthGrantType::AuthorizationCode { redirect_uris } => {
                assert_eq!(
                    redirect_uris[0].as_str(),
                    "https://moltis.example.com/auth/callback",
                    "non-loopback redirect_uri must be preserved as-is"
                );
            },
            other => panic!("expected AuthorizationCode, got {other:?}"),
        }
    }

    #[test]
    fn build_client_metadata_uses_native_application_type_for_loopback() {
        let redirect: Url = "http://localhost:8080/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let metadata = build_client_metadata(&redirect).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            metadata.application_type,
            ApplicationType::Native,
            "loopback redirect_uri must use ApplicationType::Native"
        );
    }

    #[test]
    fn registration_diagnostics_include_metadata_json_and_redirect_context() {
        let original_redirect: Url = "https://localhost:52979/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let registration_redirect = normalize_loopback_redirect(&original_redirect);
        let metadata =
            build_client_metadata(&registration_redirect).unwrap_or_else(|error| panic!("{error}"));
        let raw_metadata =
            Raw::new(&metadata).unwrap_or_else(|error| panic!("raw metadata: {error}"));
        let issuer: Url = "https://matrix.org/"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let registration_endpoint: Url =
            "https://matrix.org/_matrix/client/unstable/org.matrix.msc2965/auth_issuer/register"
                .parse()
                .unwrap_or_else(|error| panic!("{error}"));

        let diagnostics = ClientRegistrationDiagnostics::new(
            &original_redirect,
            &registration_redirect,
            &metadata,
            &issuer,
            Some(&registration_endpoint),
            &raw_metadata,
        );

        assert_eq!(
            diagnostics.original_redirect_uri,
            "https://localhost:52979/auth/callback"
        );
        assert_eq!(
            diagnostics.registration_redirect_uri,
            "http://localhost:52979/auth/callback"
        );
        assert!(diagnostics.is_loopback);
        assert_eq!(diagnostics.application_type, "native");
        assert_eq!(diagnostics.issuer, "https://matrix.org/");
        assert_eq!(
            diagnostics.registration_endpoint.as_deref(),
            Some(registration_endpoint.as_str())
        );
        assert!(diagnostics.client_metadata_json.contains("redirect_uris"));
        assert!(
            diagnostics
                .client_metadata_json
                .contains("http://localhost:52979/auth/callback")
        );

        let display = diagnostics.to_string();
        assert!(display.contains("original_redirect_uri=https://localhost:52979/auth/callback"));
        assert!(display.contains("registration_redirect_uri=http://localhost:52979/auth/callback"));
        assert!(display.contains("application_type=native"));
        assert!(display.contains("registration_endpoint=https://matrix.org/"));
        assert!(display.contains("client_metadata_json="));
    }

    #[test]
    fn registration_failure_display_includes_source_and_diagnostics() {
        let original_redirect: Url = "https://moltis.example.com/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let metadata =
            build_client_metadata(&original_redirect).unwrap_or_else(|error| panic!("{error}"));
        let raw_metadata =
            Raw::new(&metadata).unwrap_or_else(|error| panic!("raw metadata: {error}"));
        let issuer: Url = "https://matrix.org/"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        let diagnostics = ClientRegistrationDiagnostics::new(
            &original_redirect,
            &original_redirect,
            &metadata,
            &issuer,
            None,
            &raw_metadata,
        );
        let failure = crate::oidc::ClientRegistrationFailure::new(
            diagnostics,
            std::io::Error::other("invalid_redirect_uri: invalid redirect_uri"),
        );

        let display = failure.to_string();
        assert!(display.contains("invalid_redirect_uri: invalid redirect_uri"));
        assert!(display.contains("original_redirect_uri=https://moltis.example.com/auth/callback"));
        assert!(
            display.contains("registration_redirect_uri=https://moltis.example.com/auth/callback")
        );
        assert!(display.contains("is_loopback=false"));
        assert!(display.contains("application_type=web"));
        assert!(display.contains("registration_endpoint=none"));
    }

    #[test]
    fn is_loopback_uri_covers_full_127_range() {
        let url_127_0_0_2: Url = "http://127.0.0.2:8080/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(
            is_loopback_uri(&url_127_0_0_2),
            "127.0.0.2 is in 127.0.0.0/8 and must be treated as loopback"
        );

        let url_127_255: Url = "http://127.255.255.255:8080/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(
            is_loopback_uri(&url_127_255),
            "127.255.255.255 is in 127.0.0.0/8 and must be treated as loopback"
        );

        let url_external: Url = "https://10.0.0.1:8080/auth/callback"
            .parse()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!is_loopback_uri(&url_external), "10.0.0.1 is not loopback");
    }

    #[test]
    fn debug_impl_redacts_tokens() {
        let session = PersistedOidcSession {
            client_id: "test-client".into(),
            user_id: "@bot:example.com".into(),
            device_id: "TESTDEVICE".into(),
            access_token: Secret::new("super-secret-token".into()),
            refresh_token: Some(Secret::new("super-secret-refresh".into())),
        };
        let debug = format!("{session:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-token"));
        assert!(!debug.contains("super-secret-refresh"));
    }

    #[tokio::test]
    async fn save_and_load_oidc_session_round_trip() {
        let dir =
            tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir should work: {error}"));
        let account_id = "test-oidc-roundtrip";
        // Test the serialization logic directly.
        let persisted = PersistedOidcSession {
            client_id: "test-client-id".into(),
            user_id: "@bot:example.com".into(),
            device_id: "TESTDEVICE".into(),
            access_token: Secret::new("test-access-token".into()),
            refresh_token: Some(Secret::new("test-refresh-token".into())),
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
        assert_eq!(loaded.access_token.expose_secret(), "test-access-token");
        assert_eq!(
            loaded
                .refresh_token
                .as_ref()
                .map(|s| s.expose_secret().as_str()),
            Some("test-refresh-token")
        );
    }
}
