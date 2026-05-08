use {
    matrix_sdk::{
        Client,
        encryption::{
            CrossSigningResetAuthType,
            recovery::{IdentityResetHandle, RecoveryState},
        },
        ruma::api::client::uiaa::{AuthData, Password, UserIdentifier},
    },
    secrecy::ExposeSecret,
    tracing::{info, instrument, warn},
};

use moltis_channels::{Error as ChannelError, Result as ChannelResult};

use crate::{
    client::{AuthMode, auth_mode},
    config::{MatrixAccountConfig, MatrixOwnershipMode},
};

#[derive(Default)]
pub(crate) struct OwnershipAttemptResult {
    pub startup_error: Option<String>,
    pub pending_identity_reset: Option<IdentityResetHandle>,
}

pub(crate) async fn maybe_take_matrix_account_ownership(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> OwnershipAttemptResult {
    if config.ownership_mode != MatrixOwnershipMode::MoltisOwned {
        return OwnershipAttemptResult::default();
    }

    if matches!(auth_mode(config), Ok(AuthMode::Oidc)) {
        return ensure_oidc_owned_encryption_state(client, account_id).await;
    }

    match ensure_moltis_owned_encryption_state(client, account_id, config).await {
        Ok(Some(handle)) => {
            let startup_error = ownership_approval_message(&handle);
            warn!(
                account_id,
                error = startup_error,
                "matrix ownership setup failed"
            );
            OwnershipAttemptResult {
                startup_error: Some(startup_error),
                pending_identity_reset: Some(handle),
            }
        },
        Ok(None) => OwnershipAttemptResult::default(),
        Err(error) => {
            warn!(account_id, error = %error, "matrix ownership setup failed");
            OwnershipAttemptResult {
                startup_error: Some(error.to_string()),
                pending_identity_reset: None,
            }
        },
    }
}

pub(crate) async fn wait_for_e2ee_state_to_settle(client: &Client) {
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
}

async fn ownership_is_ready(client: &Client) -> ChannelResult<bool> {
    Ok(ownership_is_effectively_ready(
        cross_signing_is_complete(client).await,
        own_device_is_cross_signed_by_owner(client).await?,
    ))
}

async fn try_recover_secret_storage_with_password(
    client: &Client,
    account_id: &str,
    password: &str,
) -> bool {
    match client.encryption().recovery().recover(password).await {
        Ok(()) => {
            wait_for_e2ee_state_to_settle(client).await;
            info!(
                account_id,
                "matrix ownership recovered existing secret storage with account password"
            );
            true
        },
        Err(error) => {
            warn!(
                account_id,
                error = %error,
                "matrix ownership could not recover existing secret storage with account password"
            );
            false
        },
    }
}

#[instrument(skip(client, config), fields(account_id))]
async fn ensure_moltis_owned_encryption_state(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<Option<IdentityResetHandle>> {
    let Some(user_id) = config
        .user_id
        .as_deref()
        .filter(|user_id| !user_id.is_empty())
    else {
        return Err(ChannelError::invalid_input(
            "user_id is required when Moltis owns a Matrix account",
        ));
    };
    let Some(password) = config.password.as_ref() else {
        return Err(ChannelError::invalid_input(
            "password is required when Moltis owns a Matrix account",
        ));
    };

    bootstrap_cross_signing_with_password(client, user_id, password.expose_secret()).await?;

    if !ownership_is_ready(client).await? {
        wait_for_e2ee_state_to_settle(client).await;
    }

    let initial_recovery_state = client.encryption().recovery().state();
    if should_try_recover_existing_secret_storage(
        ownership_is_ready(client).await?,
        initial_recovery_state,
    ) {
        let _ =
            try_recover_secret_storage_with_password(client, account_id, password.expose_secret())
                .await;
    }

    if !ownership_is_ready(client).await?
        && let Some(handle) =
            force_take_over_existing_identity(client, account_id, user_id, password.expose_secret())
                .await?
    {
        return Ok(Some(handle));
    }

    match client.encryption().recovery().state() {
        RecoveryState::Disabled => {
            enable_password_backed_recovery(client, password.expose_secret()).await?;
            info!(
                account_id,
                "matrix ownership recovery enabled with password-backed secret storage"
            );
        },
        RecoveryState::Enabled => {
            info!(account_id, "matrix ownership recovery already enabled");
        },
        RecoveryState::Incomplete => {
            if !try_recover_secret_storage_with_password(
                client,
                account_id,
                password.expose_secret(),
            )
            .await
            {
                force_take_over_existing_identity(
                    client,
                    account_id,
                    user_id,
                    password.expose_secret(),
                )
                .await?;
            }
        },
        RecoveryState::Unknown => {
            warn!(
                account_id,
                "matrix recovery state is still unknown after login, skipping automatic ownership bootstrap"
            );
        },
    }

    ensure_own_device_is_cross_signed(client).await?;

    if !ownership_is_ready(client).await? {
        return Err(ChannelError::invalid_input(
            "matrix ownership bootstrap completed but cross-signing is still incomplete",
        ));
    }

    Ok(None)
}

#[instrument(skip(client), fields(account_id))]
async fn ensure_oidc_owned_encryption_state(
    client: &Client,
    account_id: &str,
) -> OwnershipAttemptResult {
    if let Err(error) = client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
    {
        if let Some(handle) = client
            .encryption()
            .recovery()
            .reset_identity()
            .await
            .ok()
            .flatten()
        {
            match handle.auth_type() {
                CrossSigningResetAuthType::OAuth(_) => {
                    let startup_error = ownership_approval_message(&handle);
                    warn!(
                        account_id,
                        error = startup_error,
                        "matrix OIDC ownership needs browser approval"
                    );
                    return OwnershipAttemptResult {
                        startup_error: Some(startup_error),
                        pending_identity_reset: Some(handle),
                    };
                },
                CrossSigningResetAuthType::Uiaa(_) => {
                    warn!(
                        account_id,
                        error = %error,
                        "matrix OIDC ownership bootstrap failed (UIAA required but no password available)"
                    );
                    return OwnershipAttemptResult {
                        startup_error: Some(format!(
                            "cross-signing bootstrap needs password auth: {error}"
                        )),
                        pending_identity_reset: None,
                    };
                },
            }
        }

        warn!(
            account_id,
            error = %error,
            "matrix OIDC cross-signing bootstrap failed"
        );
        return OwnershipAttemptResult {
            startup_error: Some(error.to_string()),
            pending_identity_reset: None,
        };
    }

    wait_for_e2ee_state_to_settle(client).await;

    if !ownership_is_ready(client).await.unwrap_or(false) {
        info!(
            account_id,
            "matrix OIDC cross-signing exists but this device lacks signing keys, resetting identity"
        );

        match client.encryption().recovery().reset_identity().await {
            Ok(Some(handle)) => match handle.auth_type() {
                CrossSigningResetAuthType::OAuth(_) => {
                    let startup_error = ownership_approval_message(&handle);
                    warn!(
                        account_id,
                        error = startup_error,
                        "matrix OIDC ownership needs browser approval"
                    );
                    return OwnershipAttemptResult {
                        startup_error: Some(startup_error),
                        pending_identity_reset: Some(handle),
                    };
                },
                CrossSigningResetAuthType::Uiaa(_) => {
                    if let Err(error) = handle.reset(None).await {
                        warn!(
                            account_id,
                            error = %error,
                            "matrix OIDC identity reset auth failed"
                        );
                        return OwnershipAttemptResult {
                            startup_error: Some(error.to_string()),
                            pending_identity_reset: None,
                        };
                    }
                    let _ = client
                        .encryption()
                        .bootstrap_cross_signing_if_needed(None)
                        .await;
                    wait_for_e2ee_state_to_settle(client).await;
                },
            },
            Ok(None) => {
                let _ = client
                    .encryption()
                    .bootstrap_cross_signing_if_needed(None)
                    .await;
                wait_for_e2ee_state_to_settle(client).await;
            },
            Err(error) => {
                warn!(
                    account_id,
                    error = %error,
                    "matrix OIDC identity reset failed"
                );
                return OwnershipAttemptResult {
                    startup_error: Some(error.to_string()),
                    pending_identity_reset: None,
                };
            },
        }
    }

    let _ = client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await;
    wait_for_e2ee_state_to_settle(client).await;

    if let Err(error) = ensure_own_device_is_cross_signed(client).await {
        warn!(
            account_id,
            error = %error,
            "matrix OIDC device self-signing failed"
        );
    }

    if ownership_is_ready(client).await.unwrap_or(false) {
        info!(account_id, "matrix OIDC ownership bootstrap complete");
    } else {
        info!(
            account_id,
            "matrix OIDC ownership bootstrap partial, device may need verification in Element"
        );
    }

    OwnershipAttemptResult::default()
}

async fn enable_password_backed_recovery(client: &Client, password: &str) -> ChannelResult<String> {
    let recovery_key = client
        .encryption()
        .recovery()
        .enable()
        .wait_for_backups_to_upload()
        .with_passphrase(password)
        .await
        .map_err(|error| ChannelError::external("matrix recovery enable", error))?;
    wait_for_e2ee_state_to_settle(client).await;
    Ok(recovery_key)
}

async fn ensure_own_device_is_cross_signed(client: &Client) -> ChannelResult<()> {
    if own_device_is_cross_signed_by_owner(client).await? {
        return Ok(());
    }

    let Some(own_device) = client
        .encryption()
        .get_own_device()
        .await
        .map_err(|error| ChannelError::external("matrix own device lookup", error))?
    else {
        return Ok(());
    };

    own_device
        .verify()
        .await
        .map_err(|error| ChannelError::external("matrix own device self-sign", error))
}

async fn own_device_is_cross_signed_by_owner(client: &Client) -> ChannelResult<bool> {
    Ok(client
        .encryption()
        .get_own_device()
        .await
        .map_err(|error| ChannelError::external("matrix own device lookup", error))?
        .is_some_and(|device| device.is_cross_signed_by_owner()))
}

fn ownership_is_effectively_ready(
    cross_signing_complete: bool,
    own_device_cross_signed_by_owner: bool,
) -> bool {
    cross_signing_complete || own_device_cross_signed_by_owner
}

fn should_try_recover_existing_secret_storage(
    ownership_ready: bool,
    recovery_state: RecoveryState,
) -> bool {
    !ownership_ready
        && matches!(
            recovery_state,
            RecoveryState::Enabled | RecoveryState::Incomplete
        )
}

async fn cross_signing_is_complete(client: &Client) -> bool {
    client
        .encryption()
        .cross_signing_status()
        .await
        .is_some_and(|status| status.is_complete())
}

fn ownership_approval_message(handle: &IdentityResetHandle) -> String {
    match handle.auth_type() {
        CrossSigningResetAuthType::OAuth(info) => format!(
            "matrix account requires browser approval to reset cross-signing at {}; complete that in Element or switch to user-managed mode",
            info.approval_url
        ),
        CrossSigningResetAuthType::Uiaa(_) => {
            "matrix account requires additional authentication to reset cross-signing".to_string()
        },
    }
}

#[instrument(skip(client, password), fields(account_id))]
async fn force_take_over_existing_identity(
    client: &Client,
    account_id: &str,
    user_id: &str,
    password: &str,
) -> ChannelResult<Option<IdentityResetHandle>> {
    let maybe_handle = client
        .encryption()
        .recovery()
        .reset_identity()
        .await
        .map_err(|error| ChannelError::external("matrix recovery reset identity", error))?;

    if let Some(handle) = maybe_handle {
        match handle.auth_type() {
            CrossSigningResetAuthType::Uiaa(uiaa) => {
                let mut auth = Password::new(
                    UserIdentifier::UserIdOrLocalpart(user_id.to_owned()),
                    password.to_owned(),
                );
                auth.session = uiaa.session.clone();
                handle
                    .reset(Some(AuthData::Password(auth)))
                    .await
                    .map_err(|error| {
                        ChannelError::external("matrix recovery reset identity auth", error)
                    })?;
                wait_for_e2ee_state_to_settle(client).await;
            },
            CrossSigningResetAuthType::OAuth(_) => {
                return Ok(Some(handle));
            },
        }
    }

    let _recovery_key = enable_password_backed_recovery(client, password).await?;

    info!(
        account_id,
        "matrix ownership forcibly reset existing recovery state and bootstrapped fresh Moltis-managed recovery"
    );

    Ok(None)
}

async fn bootstrap_cross_signing_with_password(
    client: &Client,
    user_id: &str,
    password: &str,
) -> ChannelResult<()> {
    match client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) => {
            let Some(response) = error.as_uiaa_response() else {
                return Err(ChannelError::external(
                    "matrix cross-signing bootstrap",
                    error,
                ));
            };

            let mut auth = Password::new(
                UserIdentifier::UserIdOrLocalpart(user_id.to_owned()),
                password.to_owned(),
            );
            auth.session = response.session.clone();

            client
                .encryption()
                .bootstrap_cross_signing(Some(AuthData::Password(auth)))
                .await
                .map_err(|error| ChannelError::external("matrix cross-signing bootstrap", error))
        },
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::encryption::recovery::RecoveryState;

    use crate::client::ownership::{
        ownership_is_effectively_ready, should_try_recover_existing_secret_storage,
    };

    #[test]
    fn ownership_is_effectively_ready_when_cross_signing_is_complete() {
        assert!(ownership_is_effectively_ready(true, false));
    }

    #[test]
    fn ownership_is_effectively_ready_when_own_device_is_signed() {
        assert!(ownership_is_effectively_ready(false, true));
    }

    #[test]
    fn ownership_is_not_effectively_ready_when_neither_signal_is_present() {
        assert!(!ownership_is_effectively_ready(false, false));
    }

    #[test]
    fn restart_prefers_secret_storage_recovery_when_account_is_not_ready() {
        assert!(should_try_recover_existing_secret_storage(
            false,
            RecoveryState::Enabled
        ));
        assert!(should_try_recover_existing_secret_storage(
            false,
            RecoveryState::Incomplete
        ));
    }

    #[test]
    fn restart_skips_secret_storage_recovery_when_it_cannot_help() {
        assert!(!should_try_recover_existing_secret_storage(
            true,
            RecoveryState::Enabled
        ));
        assert!(!should_try_recover_existing_secret_storage(
            false,
            RecoveryState::Disabled
        ));
        assert!(!should_try_recover_existing_secret_storage(
            false,
            RecoveryState::Unknown
        ));
    }
}
