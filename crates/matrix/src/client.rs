use std::{fs, future::Future, path::PathBuf, sync::Arc};

use {
    matrix_sdk::{
        Client, Room,
        config::SyncSettings,
        encryption::{
            BackupDownloadStrategy, CrossSigningResetAuthType, EncryptionSettings,
            recovery::{IdentityResetHandle, RecoveryState},
        },
        ruma::{
            OwnedUserId,
            api::client::uiaa::{AuthData, Password, UserIdentifier},
            events::room::encrypted::OriginalSyncRoomEncryptedEvent,
        },
    },
    reqwest::StatusCode,
    secrecy::ExposeSecret,
    serde::Deserialize,
    tokio_util::sync::CancellationToken,
    tracing::{info, instrument, warn},
};

use moltis_channels::{Error as ChannelError, Result as ChannelResult};

use crate::{
    config::{MatrixAccountConfig, MatrixAuthMode, MatrixOwnershipMode},
    handler, oidc,
    state::AccountStateMap,
    verification,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthMode {
    AccessToken,
    Password,
    Oidc,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedMatrixAccount {
    pub user_id: OwnedUserId,
    pub ownership_startup_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct OwnershipAttemptResult {
    pub startup_error: Option<String>,
    pub pending_identity_reset: Option<IdentityResetHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccessTokenIdentity {
    user_id: OwnedUserId,
    device_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccessTokenWhoAmIResponse {
    user_id: OwnedUserId,
    #[serde(default)]
    device_id: Option<String>,
}

#[instrument(skip(config), fields(account_id, homeserver = %config.homeserver))]
pub(crate) async fn build_client(
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<Client> {
    let store_path = ensure_store_path(account_id)?;
    let mut builder = Client::builder()
        .homeserver_url(&config.homeserver)
        .with_encryption_settings(encryption_settings())
        .sqlite_store(&store_path, None);

    if matches!(auth_mode(config), Ok(AuthMode::Oidc)) {
        builder = builder.handle_refresh_tokens();
    }

    builder
        .build()
        .await
        .map_err(|error| ChannelError::external("matrix client build", error))
}

#[instrument(skip(config), fields(account_id))]
pub(crate) async fn build_and_authenticate_client(
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<(Client, AuthenticatedMatrixAccount)> {
    let client = build_client(account_id, config).await?;
    match authenticate_client(&client, account_id, config).await {
        Ok(authenticated) => Ok((client, authenticated)),
        Err(error) if should_rebuild_store_after_auth_error(config, &error) => {
            warn!(
                account_id,
                "matrix crypto store is pinned to an old device, resetting local store and retrying login once"
            );
            reset_store_path(account_id)?;
            let client = build_client(account_id, config).await?;
            let authenticated = authenticate_client(&client, account_id, config).await?;
            Ok((client, authenticated))
        },
        Err(error) => Err(error),
    }
}

fn encryption_settings() -> EncryptionSettings {
    EncryptionSettings {
        auto_enable_cross_signing: true,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
        ..Default::default()
    }
}

fn should_rebuild_store_after_auth_error(
    config: &MatrixAccountConfig,
    error: &ChannelError,
) -> bool {
    matches!(auth_mode(config), Ok(AuthMode::Password))
        && matches!(
            error,
            ChannelError::External { context, source }
                if context == "matrix password login"
                    && source
                        .to_string()
                        .contains("the account in the store doesn't match the account in the constructor")
        )
}

pub(crate) fn auth_mode(config: &MatrixAccountConfig) -> ChannelResult<AuthMode> {
    // Explicit auth_mode takes precedence when present.
    if let Some(ref explicit) = config.auth_mode {
        return match explicit {
            MatrixAuthMode::Oidc => {
                if config.homeserver.trim().is_empty() {
                    return Err(ChannelError::invalid_input(
                        "homeserver is required when using OIDC authentication",
                    ));
                }
                Ok(AuthMode::Oidc)
            },
            MatrixAuthMode::Password => {
                if config.user_id.as_deref().is_none_or(str::is_empty) {
                    return Err(ChannelError::invalid_input(
                        "user_id is required when using password authentication",
                    ));
                }
                let password = config
                    .password
                    .as_ref()
                    .map(|secret| secret.expose_secret().trim())
                    .unwrap_or_default();
                if password.is_empty() || password == moltis_common::secret_serde::REDACTED {
                    return Err(ChannelError::invalid_input(
                        "password is required when auth_mode is \"password\"",
                    ));
                }
                Ok(AuthMode::Password)
            },
            MatrixAuthMode::AccessToken => {
                let access_token = config.access_token.expose_secret().trim();
                if access_token.is_empty() || access_token == moltis_common::secret_serde::REDACTED
                {
                    return Err(ChannelError::invalid_input(
                        "access_token is required when auth_mode is \"access_token\"",
                    ));
                }
                Ok(AuthMode::AccessToken)
            },
        };
    }

    // Backward-compatible auto-detection from credentials.
    let access_token = config.access_token.expose_secret().trim();
    if !access_token.is_empty() && access_token != moltis_common::secret_serde::REDACTED {
        return Ok(AuthMode::AccessToken);
    }

    let password = config
        .password
        .as_ref()
        .map(|secret| secret.expose_secret().trim())
        .unwrap_or_default();
    if password.is_empty() || password == moltis_common::secret_serde::REDACTED {
        return Err(ChannelError::invalid_input(
            "either access_token or password is required",
        ));
    }

    if config.user_id.as_deref().is_none_or(str::is_empty) {
        return Err(ChannelError::invalid_input(
            "user_id is required when using password authentication",
        ));
    }

    Ok(AuthMode::Password)
}

#[instrument(skip(client, config), fields(account_id))]
pub(crate) async fn authenticate_client(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<AuthenticatedMatrixAccount> {
    match auth_mode(config)? {
        AuthMode::AccessToken => {
            let identity = restore_access_token_session(client, account_id, config).await?;
            client
                .encryption()
                .wait_for_e2ee_initialization_tasks()
                .await;
            info!(
                account_id,
                user_id = %identity.user_id,
                device_id = identity.device_id.as_deref().unwrap_or("<unknown>"),
                "matrix session restored"
            );
            Ok(AuthenticatedMatrixAccount {
                user_id: identity.user_id,
                ownership_startup_error: None,
            })
        },
        AuthMode::Password => {
            login_with_password(client, account_id, config).await?;
            client
                .encryption()
                .wait_for_e2ee_initialization_tasks()
                .await;
            let bot_user_id = client
                .whoami()
                .await
                .map_err(|error| ChannelError::external("matrix whoami", error))?
                .user_id;
            info!(account_id, user_id = %bot_user_id, "matrix password login complete");
            Ok(AuthenticatedMatrixAccount {
                user_id: bot_user_id,
                ownership_startup_error: None,
            })
        },
        AuthMode::Oidc => oidc::restore_oidc_session(client, account_id).await,
    }
}

pub(crate) async fn maybe_take_matrix_account_ownership(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> OwnershipAttemptResult {
    if config.ownership_mode != MatrixOwnershipMode::MoltisOwned {
        return OwnershipAttemptResult::default();
    }

    // OIDC sessions don't have a password — bootstrap cross-signing
    // without UIAA password auth (MAS handles auth via OAuth).
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

async fn wait_for_e2ee_state_to_settle(client: &Client) {
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

/// OIDC ownership: bootstrap cross-signing without a password.
///
/// MAS handles authentication via OAuth, so `bootstrap_cross_signing_if_needed(None)`
/// works without UIAA password auth. If MAS requires browser approval for a
/// cross-signing reset, we return the handle for the user to approve.
#[instrument(skip(client), fields(account_id))]
async fn ensure_oidc_owned_encryption_state(
    client: &Client,
    account_id: &str,
) -> OwnershipAttemptResult {
    // Bootstrap cross-signing without password auth.
    if let Err(error) = client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
    {
        // If UIAA is needed, try reset_identity which may return an OAuth approval handle.
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
                    // OIDC sessions can't provide UIAA password auth.
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

    // After bootstrap, check if this device can actually self-sign.
    // If cross-signing was already set up by a previous session, this device
    // won't have the private signing keys. In that case, reset the identity
    // to create fresh cross-signing keys owned by this device.
    if !ownership_is_ready(client).await.unwrap_or(false) {
        info!(
            account_id,
            "matrix OIDC cross-signing exists but this device lacks signing keys, resetting identity"
        );

        match client.encryption().recovery().reset_identity().await {
            Ok(Some(handle)) => {
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
                        // Try reset without auth — MAS may allow it.
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
                }
            },
            Ok(None) => {
                // Reset completed — re-bootstrap to load private signing
                // keys into the local crypto store, then settle.
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

    // Re-bootstrap to ensure private signing keys are in the local store.
    // After identity reset or on restart, the keys may exist on the server
    // but not be loaded locally.
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
            "matrix OIDC ownership bootstrap partial — device may need verification in Element"
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

#[instrument(skip(client, accounts), fields(account_id, user_id = %bot_user_id))]
pub(crate) fn register_event_handlers(
    client: &Client,
    account_id: &str,
    accounts: &AccountStateMap,
    bot_user_id: &OwnedUserId,
) {
    let accounts_for_msg = Arc::clone(accounts);
    let account_id_msg = account_id.to_string();
    let bot_uid_msg = bot_user_id.clone();
    client.add_event_handler(
        move |ev: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent,
              room: Room| {
            let accounts = Arc::clone(&accounts_for_msg);
            let aid = account_id_msg.clone();
            let buid = bot_uid_msg.clone();
            async move {
                handler::handle_room_message(ev, room, aid, accounts, buid).await;
            }
        },
    );

    let accounts_for_encrypted = Arc::clone(accounts);
    let account_id_encrypted = account_id.to_string();
    let bot_uid_encrypted = bot_user_id.clone();
    client.add_event_handler(move |ev: OriginalSyncRoomEncryptedEvent, room: Room| {
        let accounts = Arc::clone(&accounts_for_encrypted);
        let aid = account_id_encrypted.clone();
        let buid = bot_uid_encrypted.clone();
        async move {
            handler::handle_room_encrypted_event(ev, room, aid, accounts, buid).await;
        }
    });

    let accounts_for_to_device = Arc::clone(accounts);
    let account_id_to_device = account_id.to_string();
    client.add_event_handler(
        move |ev: matrix_sdk::ruma::events::ToDeviceEvent<
            matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEventContent,
        >| {
            let accounts = Arc::clone(&accounts_for_to_device);
            let aid = account_id_to_device.clone();
            async move {
                verification::handle_to_device_verification_request(ev, aid, accounts).await;
            }
        },
    );

    let accounts_for_poll = Arc::clone(accounts);
    let account_id_poll = account_id.to_string();
    client.add_event_handler(
        move |ev: matrix_sdk::ruma::events::poll::response::OriginalSyncPollResponseEvent,
              room: Room| {
            let accounts = Arc::clone(&accounts_for_poll);
            let aid = account_id_poll.clone();
            let sender_id = ev.sender.to_string();
            let callback_data = handler::first_selection(&ev.content.selections);
            async move {
                handler::handle_poll_response(room, aid, accounts, sender_id, callback_data).await;
            }
        },
    );

    let accounts_for_unstable_poll = Arc::clone(accounts);
    let account_id_unstable_poll = account_id.to_string();
    client.add_event_handler(
        move |ev: matrix_sdk::ruma::events::poll::unstable_response::OriginalSyncUnstablePollResponseEvent,
              room: Room| {
            let accounts = Arc::clone(&accounts_for_unstable_poll);
            let aid = account_id_unstable_poll.clone();
            let sender_id = ev.sender.to_string();
            let callback_data = handler::first_selection(&ev.content.poll_response.answers);
            async move {
                handler::handle_poll_response(room, aid, accounts, sender_id, callback_data).await;
            }
        },
    );

    let accounts_for_invite = Arc::clone(accounts);
    let account_id_invite = account_id.to_string();
    let bot_uid_invite = bot_user_id.clone();
    client.add_event_handler(
        move |ev: matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent, room: Room| {
            let accounts = Arc::clone(&accounts_for_invite);
            let aid = account_id_invite.clone();
            let buid = bot_uid_invite.clone();
            async move {
                handler::handle_invite(ev, room, aid, accounts, buid).await;
            }
        },
    );
}

const INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(300);
const HEALTHY_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(60);

async fn retry_loop<F, Fut, T>(mut sync_fn: F, account_id: &str, cancel: CancellationToken)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
{
    let mut backoff = INITIAL_BACKOFF;
    loop {
        let start = tokio::time::Instant::now();
        tokio::select! {
            _ = sync_fn() => {
                if start.elapsed() >= HEALTHY_THRESHOLD {
                    backoff = INITIAL_BACKOFF;
                }
                warn!(
                    account_id = %account_id,
                    "matrix sync loop ended unexpectedly, retrying in {:?}",
                    backoff,
                );
                tokio::select! {
                    () = tokio::time::sleep(backoff) => {}
                    () = cancel.cancelled() => {
                        info!(account_id = %account_id, "matrix sync loop cancelled during backoff");
                        break;
                    }
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
            () = cancel.cancelled() => {
                info!(account_id = %account_id, "matrix sync loop cancelled");
                break;
            }
        }
    }
}

#[instrument(skip(client, accounts, cancel), fields(account_id))]
pub(crate) async fn sync_once_and_spawn_loop(
    client: &Client,
    account_id: &str,
    accounts: &AccountStateMap,
    cancel: CancellationToken,
) -> ChannelResult<()> {
    info!(account_id, "performing initial sync...");
    client
        .sync_once(SyncSettings::default())
        .await
        .map_err(|error| ChannelError::external("matrix initial sync", error))?;
    {
        let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
        if let Some(state) = guard.get(account_id) {
            state.mark_initial_sync_complete();
        }
    }
    wait_for_e2ee_state_to_settle(client).await;
    let ownership_startup_error = {
        let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
        guard
            .get(account_id)
            .map(|state| state.config.clone())
            .filter(|config| config.ownership_mode == MatrixOwnershipMode::MoltisOwned)
            .map(|config| async move {
                maybe_take_matrix_account_ownership(client, account_id, &config).await
            })
    };
    if let Some(ownership_attempt) = ownership_startup_error {
        let ownership_attempt = ownership_attempt.await;
        let event_sink = {
            let mut guard = accounts.write().unwrap_or_else(|error| error.into_inner());
            let sink = guard.get(account_id).and_then(|s| s.event_sink.clone());
            if let Some(state) = guard.get_mut(account_id) {
                state.ownership_startup_error = ownership_attempt.startup_error;
                let mut pending_identity_reset = state
                    .pending_identity_reset
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                *pending_identity_reset = ownership_attempt.pending_identity_reset;
            }
            sink
        };
        // Notify the UI that the channel status changed (ownership result).
        if let Some(sink) = event_sink {
            sink.emit(moltis_channels::ChannelEvent::StatusChanged {
                channel_type: moltis_channels::ChannelType::Matrix,
                account_id: account_id.to_string(),
            })
            .await;
        }
    }
    info!(
        account_id,
        "initial sync complete, starting continuous sync"
    );

    let account_id_for_sync = account_id.to_string();
    let client_for_sync = client.clone();
    tokio::spawn(async move {
        retry_loop(
            || client_for_sync.sync(SyncSettings::default()),
            &account_id_for_sync,
            cancel,
        )
        .await;
    });

    Ok(())
}

fn ensure_store_path(account_id: &str) -> ChannelResult<PathBuf> {
    let path = store_path(account_id);
    fs::create_dir_all(&path)
        .map_err(|error| ChannelError::external("matrix create store directory", error))?;
    Ok(path)
}

fn store_path(account_id: &str) -> PathBuf {
    moltis_config::data_dir()
        .join("matrix")
        .join(account_store_component(account_id))
}

fn reset_store_path(account_id: &str) -> ChannelResult<()> {
    let path = store_path(account_id);
    match fs::remove_dir_all(&path) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => {
            return Err(ChannelError::external(
                "matrix remove stale store directory",
                error,
            ));
        },
    }
    fs::create_dir_all(&path)
        .map_err(|error| ChannelError::external("matrix recreate store directory", error))?;
    Ok(())
}

fn account_store_component(account_id: &str) -> String {
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

fn resolved_device_id(account_id: &str, configured_device_id: Option<&str>) -> String {
    configured_device_id
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("moltis_{}", account_store_component(account_id)))
}

fn configured_device_id(configured_device_id: Option<&str>) -> Option<String> {
    configured_device_id
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
        .map(str::to_string)
}

#[instrument(skip(client, config), fields(account_id))]
async fn restore_access_token_session(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<AccessTokenIdentity> {
    let identity = resolve_access_token_identity(config).await?;
    let session = access_token_session(account_id, config, &identity);

    client
        .restore_session(session)
        .await
        .map_err(|error| ChannelError::external("matrix session restore", error))?;

    Ok(identity)
}

fn access_token_session(
    account_id: &str,
    config: &MatrixAccountConfig,
    identity: &AccessTokenIdentity,
) -> matrix_sdk::authentication::matrix::MatrixSession {
    if config.user_id.as_deref().is_some_and(|user_id| {
        let trimmed = user_id.trim();
        !trimmed.is_empty() && trimmed != identity.user_id.as_str()
    }) {
        warn!(
            account_id,
            configured_user_id = config.user_id.as_deref().unwrap_or_default(),
            authenticated_user_id = %identity.user_id,
            "matrix configured user_id does not match token owner, using authenticated user"
        );
    }

    if config.device_id.as_deref().is_some_and(|device_id| {
        let trimmed = device_id.trim();
        identity
            .device_id
            .as_deref()
            .is_some_and(|actual_device_id| !trimmed.is_empty() && trimmed != actual_device_id)
    }) {
        warn!(
            account_id,
            configured_device_id = config.device_id.as_deref().unwrap_or_default(),
            authenticated_device_id = identity.device_id.as_deref().unwrap_or_default(),
            "matrix configured device_id does not match token device, using authenticated device"
        );
    }

    let device_id = identity
        .device_id
        .clone()
        .unwrap_or_else(|| resolved_device_id(account_id, config.device_id.as_deref()));

    matrix_sdk::authentication::matrix::MatrixSession {
        meta: matrix_sdk::SessionMeta {
            user_id: identity.user_id.clone(),
            device_id: device_id.into(),
        },
        tokens: matrix_sdk::SessionTokens {
            access_token: config.access_token.expose_secret().clone(),
            refresh_token: None,
        },
    }
}

#[instrument(skip(config))]
async fn resolve_access_token_identity(
    config: &MatrixAccountConfig,
) -> ChannelResult<AccessTokenIdentity> {
    let homeserver = config.homeserver.trim_end_matches('/');
    let whoami_url = format!("{homeserver}/_matrix/client/v3/account/whoami");
    let response = reqwest::Client::new()
        .get(&whoami_url)
        .bearer_auth(config.access_token.expose_secret())
        .send()
        .await
        .map_err(|error| ChannelError::external("matrix access token whoami", error))?;

    let response = response
        .error_for_status()
        .map_err(|error| match error.status() {
            Some(StatusCode::UNAUTHORIZED) => {
                ChannelError::external("matrix access token whoami", error)
            },
            _ => ChannelError::external("matrix access token whoami", error),
        })?;

    let whoami = response
        .json::<AccessTokenWhoAmIResponse>()
        .await
        .map_err(|error| ChannelError::external("matrix access token whoami decode", error))?;

    Ok(AccessTokenIdentity {
        user_id: whoami.user_id,
        device_id: whoami
            .device_id
            .map(|device_id| device_id.trim().to_string())
            .filter(|device_id| !device_id.is_empty()),
    })
}

#[instrument(skip(client, config), fields(account_id))]
async fn login_with_password(
    client: &Client,
    account_id: &str,
    config: &MatrixAccountConfig,
) -> ChannelResult<()> {
    let user_id = config
        .user_id
        .as_deref()
        .filter(|user_id| !user_id.is_empty())
        .ok_or_else(|| {
            ChannelError::invalid_input("user_id is required when using password authentication")
        })?;
    let password = config
        .password
        .as_ref()
        .map(|secret| secret.expose_secret())
        .ok_or_else(|| ChannelError::invalid_input("password is required"))?;

    let mut login = client.matrix_auth().login_username(user_id, password);
    if let Some(device_id) = configured_device_id(config.device_id.as_deref()) {
        login = login.device_id(&device_id);
    }
    if let Some(display_name) = config
        .device_display_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        login = login.initial_device_display_name(display_name);
    }

    login
        .send()
        .await
        .map_err(|error| ChannelError::external("matrix password login", error))?;

    info!(account_id, "matrix password login restored session");
    Ok(())
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::Secret};

    fn config() -> MatrixAccountConfig {
        MatrixAccountConfig {
            homeserver: "https://matrix.example.com".into(),
            ..Default::default()
        }
    }

    #[test]
    fn access_token_auth_is_preferred_when_both_credentials_exist() {
        let cfg = MatrixAccountConfig {
            access_token: Secret::new("syt_test".into()),
            password: Some(Secret::new("wordpass".into())),
            user_id: Some("@bot:example.com".into()),
            ..config()
        };

        assert!(matches!(auth_mode(&cfg), Ok(AuthMode::AccessToken)));
    }

    #[test]
    fn password_auth_is_used_when_token_is_missing() {
        let cfg = MatrixAccountConfig {
            password: Some(Secret::new("wordpass".into())),
            user_id: Some("@bot:example.com".into()),
            ..config()
        };

        assert!(matches!(auth_mode(&cfg), Ok(AuthMode::Password)));
    }

    #[test]
    fn password_auth_requires_user_id() {
        let cfg = MatrixAccountConfig {
            password: Some(Secret::new("wordpass".into())),
            ..config()
        };

        let error = match auth_mode(&cfg) {
            Ok(mode) => panic!("password auth without user_id should fail, got {mode:?}"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("user_id is required"));
    }

    #[test]
    fn authentication_requires_token_or_password() {
        let error = match auth_mode(&config()) {
            Ok(mode) => panic!("missing auth should fail, got {mode:?}"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("either access_token or password is required"));
    }

    #[test]
    fn access_token_session_uses_authenticated_user_and_device_identity() {
        let cfg = MatrixAccountConfig {
            access_token: Secret::new("syt_test".into()),
            user_id: Some("@wrong:example.com".into()),
            device_id: Some("WRONG".into()),
            ..config()
        };
        let actual_user_id = "@bot:example.com"
            .parse()
            .unwrap_or_else(|error| panic!("actual user id should parse: {error}"));
        let identity = AccessTokenIdentity {
            user_id: actual_user_id,
            device_id: Some("ABC123".into()),
        };

        let session = access_token_session("matrix-org", &cfg, &identity);

        assert_eq!(session.meta.user_id.as_str(), "@bot:example.com");
        assert_eq!(session.meta.device_id.as_str(), "ABC123");
    }

    #[test]
    fn access_token_session_falls_back_to_stable_device_id_when_whoami_omits_it() {
        let cfg = MatrixAccountConfig {
            access_token: Secret::new("syt_test".into()),
            ..config()
        };
        let actual_user_id = "@bot:example.com"
            .parse()
            .unwrap_or_else(|error| panic!("actual user id should parse: {error}"));
        let identity = AccessTokenIdentity {
            user_id: actual_user_id,
            device_id: None,
        };

        let session = access_token_session("matrix:org/test bot", &cfg, &identity);

        assert_eq!(session.meta.user_id.as_str(), "@bot:example.com");
        assert_eq!(
            session.meta.device_id.as_str(),
            "moltis_matrix-org-test-bot"
        );
    }

    #[test]
    fn account_store_component_sanitizes_path_segment() {
        assert_eq!(
            account_store_component("matrix-org-lq7m2z"),
            "matrix-org-lq7m2z"
        );
        assert_eq!(
            account_store_component("matrix:org/test bot"),
            "matrix-org-test-bot"
        );
        assert_eq!(account_store_component(":::"), "default");
    }

    #[test]
    fn resolved_device_id_prefers_configured_value() {
        assert_eq!(
            resolved_device_id("matrix-org", Some("MOLTISBOT")),
            "MOLTISBOT"
        );
        assert_eq!(
            resolved_device_id("matrix-org", Some("   ")),
            "moltis_matrix-org"
        );
    }

    #[test]
    fn resolved_device_id_is_stable_without_config() {
        assert_eq!(
            resolved_device_id("matrix:org/test bot", None),
            "moltis_matrix-org-test-bot"
        );
    }

    #[test]
    fn configured_device_id_ignores_blank_values() {
        assert_eq!(
            configured_device_id(Some("MOLTISBOT")),
            Some("MOLTISBOT".into())
        );
        assert_eq!(configured_device_id(Some("   ")), None);
        assert_eq!(configured_device_id(None), None);
    }

    #[test]
    fn stale_store_mismatch_triggers_rebuild_for_password_auth() {
        let cfg = MatrixAccountConfig {
            password: Some(Secret::new("wordpass".into())),
            user_id: Some("@bot:example.com".into()),
            ..config()
        };
        let error = ChannelError::external(
            "matrix password login",
            std::io::Error::other(
                "failed to read or write to the crypto store the account in the store doesn't match the account in the constructor: expected @bot:example.com:OLD, got @bot:example.com:NEW",
            ),
        );

        assert!(should_rebuild_store_after_auth_error(&cfg, &error));
    }

    #[test]
    fn unrelated_auth_failures_do_not_reset_the_store() {
        let password_cfg = MatrixAccountConfig {
            password: Some(Secret::new("wordpass".into())),
            user_id: Some("@bot:example.com".into()),
            ..config()
        };
        let access_token_cfg = MatrixAccountConfig {
            access_token: Secret::new("token".into()),
            ..config()
        };
        let wrong_error = ChannelError::external(
            "matrix password login",
            std::io::Error::other("some other login failure"),
        );
        let access_token_error = ChannelError::external(
            "matrix password login",
            std::io::Error::other(
                "the account in the store doesn't match the account in the constructor",
            ),
        );

        assert!(!should_rebuild_store_after_auth_error(
            &password_cfg,
            &wrong_error
        ));
        assert!(!should_rebuild_store_after_auth_error(
            &access_token_cfg,
            &access_token_error
        ));
    }

    #[test]
    fn encryption_settings_enable_cross_signing_and_key_backfill() {
        let settings = encryption_settings();

        assert!(settings.auto_enable_cross_signing);
        assert_eq!(
            settings.backup_download_strategy,
            BackupDownloadStrategy::AfterDecryptionFailure
        );
    }

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

    #[test]
    fn explicit_oidc_auth_mode_returns_oidc() {
        let cfg = MatrixAccountConfig {
            auth_mode: Some(MatrixAuthMode::Oidc),
            ..config()
        };
        assert!(matches!(auth_mode(&cfg), Ok(AuthMode::Oidc)));
    }

    #[test]
    fn explicit_oidc_auth_mode_requires_homeserver() {
        let cfg = MatrixAccountConfig {
            auth_mode: Some(MatrixAuthMode::Oidc),
            homeserver: String::new(),
            ..Default::default()
        };
        let error = match auth_mode(&cfg) {
            Ok(mode) => panic!("OIDC with empty homeserver should fail, got {mode:?}"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("homeserver is required"));
    }

    #[test]
    fn backward_compat_auto_detection_ignores_absent_auth_mode() {
        // No auth_mode field — should auto-detect from credentials.
        let token_cfg = MatrixAccountConfig {
            access_token: Secret::new("syt_test".into()),
            ..config()
        };
        assert!(matches!(auth_mode(&token_cfg), Ok(AuthMode::AccessToken)));

        let password_cfg = MatrixAccountConfig {
            password: Some(Secret::new("wordpass".into())),
            user_id: Some("@bot:example.com".into()),
            ..config()
        };
        assert!(matches!(auth_mode(&password_cfg), Ok(AuthMode::Password)));
    }

    mod retry_loop_tests {
        use std::sync::atomic::{AtomicU32, Ordering};

        use tokio_util::sync::CancellationToken;

        use super::super::{HEALTHY_THRESHOLD, INITIAL_BACKOFF, MAX_BACKOFF, retry_loop};

        #[tokio::test(start_paused = true)]
        async fn cancellation_during_sync_exits_immediately() {
            let cancel = CancellationToken::new();
            let cancel_inner = cancel.clone();

            let call_count = std::sync::Arc::new(AtomicU32::new(0));
            let count = call_count.clone();

            cancel_inner.cancel();

            retry_loop(
                || {
                    count.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<()>()
                },
                "test-account",
                cancel,
            )
            .await;

            assert_eq!(call_count.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn cancellation_during_backoff_exits_without_waiting() {
            let cancel = CancellationToken::new();
            let cancel_inner = cancel.clone();

            let call_count = std::sync::Arc::new(AtomicU32::new(0));
            let count = call_count.clone();

            tokio::spawn(async move {
                // Let the first sync fail and enter backoff, then cancel.
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                cancel_inner.cancel();
            });

            retry_loop(
                || {
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if n == 0 {
                            // First call: fail immediately.
                            return;
                        }
                        // Should not be called — cancel fires during backoff.
                        std::future::pending::<()>().await;
                    }
                },
                "test-account",
                cancel,
            )
            .await;

            // Only 1 sync call — loop exited during backoff before retrying.
            assert_eq!(call_count.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn backoff_doubles_on_consecutive_failures() {
            let cancel = CancellationToken::new();
            let cancel_inner = cancel.clone();

            let call_count = std::sync::Arc::new(AtomicU32::new(0));
            let count = call_count.clone();

            let timestamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ts = timestamps.clone();

            tokio::spawn(async move {
                // Let 3 sync failures + backoffs complete, then cancel.
                // Backoff sequence: 5s, 10s, 20s = 35s total + tiny sync time.
                tokio::time::sleep(std::time::Duration::from_secs(36)).await;
                cancel_inner.cancel();
            });

            retry_loop(
                || {
                    count.fetch_add(1, Ordering::SeqCst);
                    let ts = ts.clone();
                    async move {
                        ts.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(tokio::time::Instant::now());
                    }
                },
                "test-account",
                cancel,
            )
            .await;

            let times = timestamps.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                times.len() >= 3,
                "expected at least 3 sync calls, got {}",
                times.len()
            );
            // Between call 1 and 2: ~5s backoff
            let gap1 = times[1] - times[0];
            assert!(
                gap1 >= INITIAL_BACKOFF,
                "first gap {gap1:?} < {INITIAL_BACKOFF:?}"
            );
            // Between call 2 and 3: ~10s backoff
            let gap2 = times[2] - times[1];
            assert!(
                gap2 >= INITIAL_BACKOFF * 2,
                "second gap {gap2:?} < {:?}",
                INITIAL_BACKOFF * 2
            );
        }

        #[tokio::test(start_paused = true)]
        async fn backoff_resets_after_healthy_connection() {
            let cancel = CancellationToken::new();
            let cancel_inner = cancel.clone();

            let call_count = std::sync::Arc::new(AtomicU32::new(0));
            let count = call_count.clone();

            let timestamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ts = timestamps.clone();

            tokio::spawn(async move {
                // Call 1: fails immediately → 5s backoff
                // Call 2: runs for HEALTHY_THRESHOLD (60s), then fails → backoff resets to 5s
                // Call 3: recorded, then we cancel during backoff
                // Total: ~0 + 5 + 60 + 5 + ~0 = ~70s, cancel at 71s.
                tokio::time::sleep(std::time::Duration::from_secs(71)).await;
                cancel_inner.cancel();
            });

            retry_loop(
                || {
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    let ts = ts.clone();
                    async move {
                        ts.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(tokio::time::Instant::now());
                        if n == 1 {
                            // Simulate healthy connection that eventually dies.
                            tokio::time::sleep(HEALTHY_THRESHOLD).await;
                        }
                        // All others fail immediately.
                    }
                },
                "test-account",
                cancel,
            )
            .await;

            let times = timestamps.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                times.len() >= 3,
                "expected at least 3 sync calls, got {}",
                times.len()
            );
            // Gap between call 2 (after healthy run) and call 3 should be
            // INITIAL_BACKOFF (reset), not doubled.
            let gap = times[2] - times[1];
            let expected = HEALTHY_THRESHOLD + INITIAL_BACKOFF;
            let tolerance = std::time::Duration::from_secs(1);
            assert!(
                gap <= expected + tolerance,
                "gap after healthy connection {gap:?} should be ~{expected:?} (reset backoff), not doubled"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn backoff_caps_at_max() {
            let cancel = CancellationToken::new();
            let cancel_inner = cancel.clone();

            let call_count = std::sync::Arc::new(AtomicU32::new(0));
            let count = call_count.clone();

            let timestamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let ts = timestamps.clone();

            // Backoff sequence: 5, 10, 20, 40, 80, 160, 300, 300, ...
            // After 7 calls: 5+10+20+40+80+160+300 = 615s total.
            // Cancel at 620s to let 7th call happen.
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(620)).await;
                cancel_inner.cancel();
            });

            retry_loop(
                || {
                    count.fetch_add(1, Ordering::SeqCst);
                    let ts = ts.clone();
                    async move {
                        ts.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(tokio::time::Instant::now());
                    }
                },
                "test-account",
                cancel,
            )
            .await;

            let times = timestamps.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                times.len() >= 8,
                "expected at least 8 sync calls, got {}",
                times.len()
            );
            // Gap between call 7 and 8 should be capped at MAX_BACKOFF.
            let gap = times[7] - times[6];
            assert!(
                gap >= MAX_BACKOFF,
                "gap {gap:?} should be at least {MAX_BACKOFF:?}"
            );
            assert!(
                gap <= MAX_BACKOFF + std::time::Duration::from_secs(1),
                "gap {gap:?} should be at most {MAX_BACKOFF:?} + 1s"
            );
        }
    }
}
