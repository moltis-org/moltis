use {
    matrix_sdk::{Client, config::SyncSettings},
    secrecy::ExposeSecret,
    tokio_util::sync::CancellationToken,
    tracing::{error, info},
};

use crate::{config::MatrixAccountConfig, error::Result, state::AccountStateMap};

#[tracing::instrument(skip(config))]
pub async fn build_client(config: &MatrixAccountConfig, store_path: &std::path::Path) -> Result<Client> {
    let homeserver = config.homeserver_url.as_str();

    let builder = Client::builder().homeserver_url(homeserver);

    let _ = store_path;

    let client = builder.build().await.map_err(|e| crate::Error::message(e.to_string()))?;

    Ok(client)
}

#[tracing::instrument(skip(client, config))]
pub async fn authenticate(client: &Client, config: &MatrixAccountConfig) -> Result<()> {
    let token = config.access_token.expose_secret();
    if !token.is_empty() {
        let device_id = config
            .device_id
            .as_deref()
            .unwrap_or("MOLTIS");

        let session = matrix_sdk::authentication::matrix::MatrixSession {
            meta: matrix_sdk::SessionMeta {
                user_id: config.user_id.as_str().try_into().map_err(|e| {
                    crate::Error::message(format!("invalid user_id '{}': {e}", config.user_id))
                })?,
                device_id: device_id.into(),
            },
            tokens: matrix_sdk::authentication::SessionTokens {
                access_token: token.to_string(),
                refresh_token: None,
            },
        };
        client.matrix_auth().restore_session(session, matrix_sdk::store::RoomLoadSettings::default()).await?;
    } else if let Some(password) = &config.password {
        let pw = password.expose_secret();
        let mut login = client
            .matrix_auth()
            .login_username(&config.user_id, pw);

        if let Some(device_name) = &config.device_display_name {
            login = login.initial_device_display_name(device_name);
        }

        login.send().await?;
    } else {
        return Err(crate::Error::message(
            "either access_token or password must be provided",
        ));
    }

    Ok(())
}

pub fn start_sync(
    client: Client,
    cancel: CancellationToken,
    _accounts: AccountStateMap,
    account_id: String,
) {
    tokio::spawn(async move {
        info!(account_id, "matrix sync loop started");

        let settings = SyncSettings::default();

        let sync_cancel = cancel.clone();
        let sync_client = client.clone();

        tokio::select! {
            result = sync_client.sync(settings) => {
                match result {
                    Ok(()) => info!(account_id, "matrix sync loop ended normally"),
                    Err(e) => error!(account_id, error = %e, "matrix sync loop error"),
                }
            }
            () = sync_cancel.cancelled() => {
                info!(account_id, "matrix sync loop cancelled");
            }
        }
    });
}

pub fn validate_homeserver_url(url: &str, allow_private: bool) -> std::result::Result<(), String> {
    let parsed: url::Url = url.parse().map_err(|e| format!("invalid homeserver URL: {e}"))?;

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(format!("unsupported scheme '{}' — use https or http", parsed.scheme()));
    }

    if !allow_private {
        if let Some(host) = parsed.host_str() {
            if host == "localhost"
                || host == "127.0.0.1"
                || host == "::1"
                || host.starts_with("10.")
                || host.starts_with("172.")
                || host.starts_with("192.168.")
            {
                return Err(format!(
                    "private network homeserver '{host}' blocked — set allow_private_network = true to override"
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_private_ips() {
        assert!(validate_homeserver_url("https://127.0.0.1", false).is_err());
        assert!(validate_homeserver_url("https://localhost", false).is_err());
        assert!(validate_homeserver_url("https://192.168.1.1", false).is_err());
        assert!(validate_homeserver_url("https://10.0.0.1", false).is_err());
    }

    #[test]
    fn validate_allows_private_when_enabled() {
        assert!(validate_homeserver_url("https://localhost", true).is_ok());
        assert!(validate_homeserver_url("https://192.168.1.1", true).is_ok());
    }

    #[test]
    fn validate_allows_public_urls() {
        assert!(validate_homeserver_url("https://matrix.org", false).is_ok());
        assert!(validate_homeserver_url("https://example.com:8448", false).is_ok());
    }

    #[test]
    fn validate_rejects_bad_scheme() {
        assert!(validate_homeserver_url("ftp://matrix.org", false).is_err());
    }

    #[test]
    fn validate_rejects_invalid_url() {
        assert!(validate_homeserver_url("not a url", false).is_err());
    }
}
