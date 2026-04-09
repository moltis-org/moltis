use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
};

use {
    async_trait::async_trait,
    matrix_sdk::encryption::{VerificationState, recovery::RecoveryState},
    tracing::{info, instrument, warn},
};

use moltis_channels::{
    ChannelConfigView, Error as ChannelError, Result as ChannelResult,
    message_log::MessageLog,
    otp::{OtpChallengeInfo, OtpState},
    plugin::{
        ChannelEventSink, ChannelHealthSnapshot, ChannelOtpProvider, ChannelOutbound,
        ChannelPlugin, ChannelStatus, ChannelStreamOutbound, ChannelThreadContext,
    },
};

use crate::{
    client,
    config::{MatrixAccountConfig, MatrixOwnershipMode, RedactedConfig},
    outbound::MatrixOutbound,
    state::{AccountState, AccountStateMap, VerificationPrompt},
};

fn verification_state_label(state: VerificationState) -> &'static str {
    match state {
        VerificationState::Unknown => "unknown",
        VerificationState::Verified => "verified",
        VerificationState::Unverified => "unverified",
    }
}

fn recovery_state_label(state: RecoveryState) -> &'static str {
    match state {
        RecoveryState::Unknown => "unknown",
        RecoveryState::Enabled => "enabled",
        RecoveryState::Disabled => "disabled",
        RecoveryState::Incomplete => "incomplete",
    }
}

fn ownership_mode_label(mode: MatrixOwnershipMode) -> &'static str {
    match mode {
        MatrixOwnershipMode::UserManaged => "user_managed",
        MatrixOwnershipMode::MoltisOwned => "moltis_owned",
    }
}

fn effective_recovery_state(
    cached_state: RecoveryState,
    secret_storage_enabled: bool,
    cross_signing_complete: bool,
    backups_enabled: bool,
) -> RecoveryState {
    if !secret_storage_enabled {
        return if matches!(cached_state, RecoveryState::Unknown) {
            RecoveryState::Unknown
        } else {
            RecoveryState::Disabled
        };
    }

    if cross_signing_complete && (backups_enabled || matches!(cached_state, RecoveryState::Enabled))
    {
        RecoveryState::Enabled
    } else {
        RecoveryState::Incomplete
    }
}

struct MatrixStatusSnapshot {
    client: matrix_sdk::Client,
    config: MatrixAccountConfig,
    prompts: Vec<VerificationPrompt>,
    startup_error: Option<String>,
}

async fn matrix_status_extra(snapshot: MatrixStatusSnapshot) -> serde_json::Value {
    let verification_state = snapshot.client.encryption().verification_state().get();
    let cached_recovery_state = snapshot.client.encryption().recovery().state();
    let cross_signing_complete = snapshot
        .client
        .encryption()
        .cross_signing_status()
        .await
        .is_some_and(|status| status.is_complete());
    let secret_storage_enabled = snapshot
        .client
        .encryption()
        .secret_storage()
        .is_enabled()
        .await
        .unwrap_or(false);
    let backups_enabled = snapshot.client.encryption().backups().are_enabled().await;
    let device_verified_by_owner = snapshot
        .client
        .encryption()
        .get_own_device()
        .await
        .ok()
        .flatten()
        .is_some_and(|device| device.is_cross_signed_by_owner());
    let recovery_state = effective_recovery_state(
        cached_recovery_state,
        secret_storage_enabled,
        cross_signing_complete,
        backups_enabled,
    );
    serde_json::json!({
        "matrix": {
            "verification_state": verification_state_label(verification_state),
            "pending_verifications": snapshot.prompts.iter().map(|prompt| serde_json::json!({
                "flow_id": prompt.flow_id,
                "other_user_id": prompt.other_user_id,
                "room_id": prompt.room_id,
                "emoji_lines": prompt.emoji_lines,
            })).collect::<Vec<_>>(),
            "ownership_mode": ownership_mode_label(snapshot.config.ownership_mode.clone()),
            "auth_mode": match client::auth_mode(&snapshot.config) {
                Ok(client::AuthMode::Password) => "password",
                _ => "access_token",
            },
            "user_id": snapshot.client.user_id().map(|user_id| user_id.to_string()).or_else(|| snapshot.config.user_id.clone()),
            "device_id": snapshot.client.device_id().map(|device_id| device_id.to_string()).or_else(|| snapshot.config.device_id.clone()),
            "device_display_name": snapshot.config.device_display_name,
            "cross_signing_complete": cross_signing_complete,
            "device_verified_by_owner": device_verified_by_owner,
            "recovery_state": recovery_state_label(recovery_state),
            "ownership_error": snapshot.startup_error,
        }
    })
}

/// Matrix/Element channel plugin for Moltis.
pub struct MatrixPlugin {
    accounts: AccountStateMap,
    outbound: MatrixOutbound,
    message_log: Option<Arc<dyn MessageLog>>,
    event_sink: Option<Arc<dyn ChannelEventSink>>,
}

impl MatrixPlugin {
    pub fn new() -> Self {
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        let outbound = MatrixOutbound {
            accounts: Arc::clone(&accounts),
        };
        Self {
            accounts,
            outbound,
            message_log: None,
            event_sink: None,
        }
    }

    pub fn with_message_log(mut self, log: Arc<dyn MessageLog>) -> Self {
        self.message_log = Some(log);
        self
    }

    pub fn with_event_sink(mut self, sink: Arc<dyn ChannelEventSink>) -> Self {
        self.event_sink = Some(sink);
        self
    }

    /// List pending OTP challenges for a specific account.
    pub fn pending_otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.list_pending()
            })
            .unwrap_or_default()
    }
}

impl Default for MatrixPlugin {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_account_config(config: serde_json::Value) -> ChannelResult<MatrixAccountConfig> {
    if config.get("e2ee").is_some() {
        return Err(ChannelError::invalid_input(
            "matrix `e2ee` config has been removed because encrypted rooms are not configurable per account",
        ));
    }

    Ok(serde_json::from_value(config)?)
}

fn merge_json_value(base: &mut serde_json::Value, update: serde_json::Value) {
    match (base, update) {
        (serde_json::Value::Object(base_obj), serde_json::Value::Object(update_obj)) => {
            for (key, value) in update_obj {
                merge_json_value(
                    base_obj.entry(key).or_insert(serde_json::Value::Null),
                    value,
                );
            }
        },
        (base, update) => *base = update,
    }
}

fn has_explicit_secret(update: &serde_json::Map<String, serde_json::Value>, field: &str) -> bool {
    update
        .get(field)
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            !value.trim().is_empty() && value != moltis_common::secret_serde::REDACTED
        })
}

fn sanitize_secret_update_fields(update: &mut serde_json::Map<String, serde_json::Value>) {
    let switching_to_password = has_explicit_secret(update, "password");
    let switching_to_access_token = has_explicit_secret(update, "access_token");

    if update
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            value.trim().is_empty() && !switching_to_password
                || value == moltis_common::secret_serde::REDACTED
        })
    {
        update.remove("access_token");
    }

    if update
        .get("password")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            value.trim().is_empty() && !switching_to_access_token
                || value == moltis_common::secret_serde::REDACTED
        })
    {
        update.remove("password");
    }
}

fn merge_update_with_existing(
    update: serde_json::Value,
    existing: &MatrixAccountConfig,
) -> ChannelResult<MatrixAccountConfig> {
    let mut merged = serde_json::to_value(existing)
        .map_err(|error| ChannelError::external("matrix config serialize", error))?;
    let mut update = update;
    let serde_json::Value::Object(update_obj) = &mut update else {
        return Err(ChannelError::invalid_input(
            "matrix config update payload must be a JSON object",
        ));
    };
    sanitize_secret_update_fields(update_obj);
    merge_json_value(&mut merged, update);
    parse_account_config(merged)
}

#[async_trait]
impl ChannelPlugin for MatrixPlugin {
    fn id(&self) -> &str {
        "matrix"
    }

    fn name(&self) -> &str {
        "Matrix"
    }

    #[instrument(skip(self, config), fields(account_id))]
    async fn start_account(
        &mut self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let cfg = parse_account_config(config)?;
        if cfg.homeserver.is_empty() {
            return Err(ChannelError::invalid_input("homeserver URL is required"));
        }
        client::auth_mode(&cfg)?;

        info!(account_id, homeserver = %cfg.homeserver, "starting matrix account");

        let (client, authenticated) =
            client::build_and_authenticate_client(account_id, &cfg).await?;
        let bot_user_id = authenticated.user_id;

        let cancel = tokio_util::sync::CancellationToken::new();

        {
            let otp_cooldown = cfg.otp_cooldown_secs;
            let mut cfg = cfg;
            cfg.user_id = Some(bot_user_id.to_string());
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.insert(account_id.to_string(), AccountState {
                account_id: account_id.to_string(),
                config: cfg,
                client: client.clone(),
                message_log: self.message_log.clone(),
                event_sink: self.event_sink.clone(),
                cancel: cancel.clone(),
                bot_user_id: bot_user_id.to_string(),
                ownership_startup_error: authenticated.ownership_startup_error,
                initial_sync_complete: AtomicBool::new(false),
                pending_identity_reset: Mutex::new(None),
                otp: Mutex::new(OtpState::new(otp_cooldown)),
                verification: Mutex::new(Default::default()),
            });
        }

        client::register_event_handlers(&client, account_id, &self.accounts, &bot_user_id);
        client::sync_once_and_spawn_loop(&client, account_id, &self.accounts, cancel.clone())
            .await?;

        Ok(())
    }

    async fn stop_account(&mut self, account_id: &str) -> ChannelResult<()> {
        let cancel = {
            let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
            accounts.remove(account_id).map(|s| s.cancel)
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
            info!(account_id, "matrix account stopped");
        } else {
            warn!(account_id, "matrix account not found");
        }
        Ok(())
    }

    async fn retry_account_setup(&mut self, account_id: &str) -> ChannelResult<()> {
        let (client, config, pending_handle) = {
            let mut accounts = self
                .accounts
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let state = accounts
                .get_mut(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;
            let pending_handle = state
                .pending_identity_reset
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            (state.client.clone(), state.config.clone(), pending_handle)
        };

        let pending_handle = pending_handle.ok_or_else(|| {
            ChannelError::invalid_input("matrix account has no pending ownership approval to retry")
        })?;

        pending_handle.reset(None).await.map_err(|error| {
            ChannelError::external("matrix recovery reset identity approval", error)
        })?;
        client
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;

        let ownership_attempt =
            client::maybe_take_matrix_account_ownership(&client, account_id, &config).await;

        {
            let mut accounts = self
                .accounts
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let state = accounts
                .get_mut(account_id)
                .ok_or_else(|| ChannelError::unknown_account(account_id))?;
            state.ownership_startup_error = ownership_attempt.startup_error.clone();
            let mut pending_identity_reset = state
                .pending_identity_reset
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *pending_identity_reset = ownership_attempt.pending_identity_reset;
        }

        if let Some(error) = ownership_attempt.startup_error {
            return Err(ChannelError::invalid_input(error));
        }

        Ok(())
    }

    fn outbound(&self) -> Option<&dyn ChannelOutbound> {
        Some(&self.outbound)
    }

    fn status(&self) -> Option<&dyn ChannelStatus> {
        Some(self)
    }

    fn has_account(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.contains_key(account_id)
    }

    fn account_ids(&self) -> Vec<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts.keys().cloned().collect()
    }

    fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| Box::new(s.config.clone()) as Box<dyn ChannelConfigView>)
    }

    fn account_config_json(&self, account_id: &str) -> Option<serde_json::Value> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| serde_json::to_value(RedactedConfig(&s.config)).ok())
    }

    fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> ChannelResult<()> {
        let mut accounts = self.accounts.write().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get_mut(account_id) {
            let parsed = merge_update_with_existing(config, &state.config)?;
            {
                let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.set_cooldown(parsed.otp_cooldown_secs);
            }
            state.config = parsed;
            Ok(())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
        Arc::new(MatrixOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
        Arc::new(MatrixOutbound {
            accounts: Arc::clone(&self.accounts),
        })
    }

    fn thread_context(&self) -> Option<&dyn ChannelThreadContext> {
        Some(&self.outbound)
    }

    fn as_otp_provider(&self) -> Option<&dyn ChannelOtpProvider> {
        Some(self)
    }
}

impl ChannelOtpProvider for MatrixPlugin {
    fn pending_otp_challenges(&self, account_id: &str) -> Vec<OtpChallengeInfo> {
        MatrixPlugin::pending_otp_challenges(self, account_id)
    }
}

#[async_trait]
impl ChannelStatus for MatrixPlugin {
    async fn probe(&self, account_id: &str) -> ChannelResult<ChannelHealthSnapshot> {
        let snapshot = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map(|state| {
                let prompts = {
                    let verification = state
                        .verification
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let mut prompts = verification.prompts.values().cloned().collect::<Vec<_>>();
                    prompts.sort_by(|left, right| left.other_user_id.cmp(&right.other_user_id));
                    prompts
                };
                (
                    state.client.matrix_auth().logged_in(),
                    state.bot_user_id.clone(),
                    MatrixStatusSnapshot {
                        client: state.client.clone(),
                        config: state.config.clone(),
                        prompts,
                        startup_error: state.ownership_startup_error.clone(),
                    },
                )
            })
        };

        if let Some((connected, bot_user_id, snapshot)) = snapshot {
            let details = if connected {
                format!("syncing as {bot_user_id}")
            } else {
                "not logged in".to_string()
            };
            Ok(ChannelHealthSnapshot {
                connected,
                account_id: account_id.to_string(),
                details: Some(details),
                extra: Some(matrix_status_extra(snapshot).await),
            })
        } else {
            Ok(ChannelHealthSnapshot {
                connected: false,
                account_id: account_id.to_string(),
                details: Some("account not started".into()),
                extra: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        crate::{
            client,
            client::AuthMode,
            config::{AutoJoinPolicy, MatrixAccountConfig},
            plugin::effective_recovery_state,
            state::{AccountState, VerificationPrompt},
        },
        matrix_sdk::encryption::recovery::RecoveryState,
        moltis_channels::{ChannelPlugin, ChannelStatus, ChannelType, InboundMode, otp::OtpState},
        secrecy::{ExposeSecret, Secret},
        std::sync::{Mutex, atomic::AtomicBool},
        tokio_util::sync::CancellationToken,
    };

    use crate::plugin::MatrixPlugin;

    fn test_account_state(cancel: CancellationToken) -> AccountState {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("matrix test runtime should build: {error}"));

        AccountState {
            account_id: "test".into(),
            config: MatrixAccountConfig {
                homeserver: "https://matrix.example.com".into(),
                access_token: Secret::new("test_token".into()),
                user_id: Some("@moltis:example.com".into()),
                ..Default::default()
            },
            client: runtime
                .block_on(
                    matrix_sdk::Client::builder()
                        .homeserver_url("https://matrix.example.com")
                        .build(),
                )
                .unwrap_or_else(|error| panic!("matrix test client should build: {error}")),
            message_log: None,
            event_sink: None,
            cancel,
            bot_user_id: "@moltis:example.com".into(),
            ownership_startup_error: None,
            initial_sync_complete: AtomicBool::new(true),
            pending_identity_reset: Mutex::new(None),
            otp: Mutex::new(OtpState::new(300)),
            verification: Mutex::new(Default::default()),
        }
    }

    #[test]
    fn descriptor_coherence() {
        let plugin = MatrixPlugin::new();
        let desc = ChannelType::Matrix.descriptor();

        assert_eq!(desc.channel_type, ChannelType::Matrix);
        assert_eq!(desc.display_name, "Matrix");
        assert_eq!(desc.capabilities.inbound_mode, InboundMode::GatewayLoop);
        assert!(desc.capabilities.supports_streaming);
        assert!(desc.capabilities.supports_threads);
        assert!(desc.capabilities.supports_reactions);
        assert!(desc.capabilities.supports_otp);
        assert!(desc.capabilities.supports_interactive);
        assert!(desc.capabilities.supports_voice_ingest);
        assert!(!desc.capabilities.supports_pairing);

        assert!(plugin.thread_context().is_some());
        assert!(plugin.as_otp_provider().is_some());
    }

    #[test]
    fn plugin_id_and_name() {
        let plugin = MatrixPlugin::new();
        assert_eq!(plugin.id(), "matrix");
        assert_eq!(plugin.name(), "Matrix");
    }

    #[test]
    fn no_accounts_initially() {
        let plugin = MatrixPlugin::new();
        assert!(!plugin.has_account("test"));
        assert!(plugin.account_ids().is_empty());
    }

    #[test]
    fn pending_otp_challenges_are_exposed_via_provider_trait() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            map.insert("test".into(), test_account_state(cancel));
        }

        {
            let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = map.get("test") else {
                panic!("test account inserted");
            };
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            otp.initiate("alice", Some("alice".into()), Some("Alice".into()));
        }

        let Some(provider) = plugin.as_otp_provider() else {
            panic!("matrix should expose otp provider");
        };
        let pending = provider.pending_otp_challenges("test");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].peer_id, "alice");
        assert_eq!(pending[0].username.as_deref(), Some("alice"));
        assert_eq!(pending[0].sender_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn probe_exposes_matrix_ownership_details() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            let mut state = test_account_state(cancel);
            state.config.ownership_mode = crate::config::MatrixOwnershipMode::MoltisOwned;
            state.config.password = Some(Secret::new("wordpass".into()));
            state.config.access_token = Secret::new(String::new());
            state.config.device_id = Some("MOLTISBOT".into());
            state.config.device_display_name = Some("Moltis Matrix Bot".into());
            state.ownership_startup_error = Some("ownership setup failed".into());
            map.insert("test".into(), state);
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("matrix test runtime should build: {error}"));

        let snapshot = runtime
            .block_on(plugin.probe("test"))
            .unwrap_or_else(|error| panic!("probe should succeed: {error}"));

        assert!(!snapshot.connected);
        let extra = snapshot
            .extra
            .unwrap_or_else(|| panic!("matrix probe should include extra status"));
        assert_eq!(extra["matrix"]["ownership_mode"], "moltis_owned");
        assert_eq!(extra["matrix"]["auth_mode"], "password");
        assert_eq!(extra["matrix"]["user_id"], "@moltis:example.com");
        assert_eq!(extra["matrix"]["device_id"], "MOLTISBOT");
        assert_eq!(extra["matrix"]["device_display_name"], "Moltis Matrix Bot");
        assert_eq!(extra["matrix"]["ownership_error"], "ownership setup failed");
    }

    #[test]
    fn effective_recovery_state_reports_enabled_once_live_crypto_is_ready() {
        assert_eq!(
            effective_recovery_state(RecoveryState::Incomplete, true, true, true),
            RecoveryState::Enabled
        );
        assert_eq!(
            effective_recovery_state(RecoveryState::Enabled, true, true, false),
            RecoveryState::Enabled
        );
    }

    #[test]
    fn effective_recovery_state_keeps_incomplete_when_crypto_is_only_half_alive() {
        assert_eq!(
            effective_recovery_state(RecoveryState::Incomplete, true, false, true),
            RecoveryState::Incomplete
        );
        assert_eq!(
            effective_recovery_state(RecoveryState::Incomplete, true, true, false),
            RecoveryState::Incomplete
        );
        assert_eq!(
            effective_recovery_state(RecoveryState::Disabled, false, true, true),
            RecoveryState::Disabled
        );
    }

    #[test]
    fn update_account_config_preserves_otp_state_and_updates_cooldown() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            map.insert("test".into(), test_account_state(cancel));
        }

        {
            let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = map.get("test") else {
                panic!("test account inserted");
            };
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            otp.initiate("alice", Some("alice".into()), None);
        }

        if let Err(error) = plugin.update_account_config(
            "test",
            serde_json::json!({
                "homeserver": "https://matrix.example.com",
                "access_token": "test_token",
                "user_id": "@moltis:example.com",
                "allowlist": ["alice"],
                "otp_cooldown_secs": 1,
            }),
        ) {
            panic!("config update should succeed: {error}");
        }

        {
            let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = map.get("test") else {
                panic!("test account inserted");
            };
            let otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            assert!(otp.has_pending("alice"));
        }

        std::thread::sleep(std::time::Duration::from_millis(1100));

        {
            let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = map.get("test") else {
                panic!("test account inserted");
            };
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(
                otp.verify("alice", "000000"),
                moltis_channels::otp::OtpVerifyResult::WrongCode { attempts_left: 2 }
            );
            assert_eq!(
                otp.verify("alice", "000001"),
                moltis_channels::otp::OtpVerifyResult::WrongCode { attempts_left: 1 }
            );
            assert_eq!(
                otp.verify("alice", "000002"),
                moltis_channels::otp::OtpVerifyResult::LockedOut
            );
            assert_eq!(
                otp.initiate("alice", Some("alice".into()), None),
                moltis_channels::otp::OtpInitResult::LockedOut
            );
        }

        std::thread::sleep(std::time::Duration::from_millis(1100));

        {
            let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
            let Some(state) = map.get("test") else {
                panic!("test account inserted");
            };
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            assert!(matches!(
                otp.initiate("alice", Some("alice".into()), None),
                moltis_channels::otp::OtpInitResult::Created(_)
            ));
        }
    }

    #[test]
    fn update_account_config_preserves_redacted_access_token_and_identity_fields() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            map.insert("test".into(), test_account_state(cancel));
        }

        if let Err(error) = plugin.update_account_config(
            "test",
            serde_json::json!({
                "access_token": "[REDACTED]",
                "dm_policy": "open",
            }),
        ) {
            panic!("config update should succeed: {error}");
        }

        let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = map.get("test") else {
            panic!("test account inserted");
        };
        assert_eq!(state.config.homeserver, "https://matrix.example.com");
        assert_eq!(state.config.user_id.as_deref(), Some("@moltis:example.com"));
        assert_eq!(state.config.access_token.expose_secret(), "test_token");
        assert_eq!(
            state.config.dm_policy,
            moltis_channels::gating::DmPolicy::Open
        );
    }

    #[test]
    fn partial_update_preserves_omitted_fields_instead_of_resetting_defaults() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            let mut state = test_account_state(cancel);
            state.config.mention_mode = moltis_channels::gating::MentionMode::Always;
            state.config.reply_to_message = false;
            state.config.auto_join = AutoJoinPolicy::Allowlist;
            map.insert("test".into(), state);
        }

        if let Err(error) = plugin.update_account_config(
            "test",
            serde_json::json!({
                "dm_policy": "open",
            }),
        ) {
            panic!("partial update should succeed: {error}");
        }

        let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = map.get("test") else {
            panic!("test account inserted");
        };
        assert_eq!(
            state.config.mention_mode,
            moltis_channels::gating::MentionMode::Always
        );
        assert!(!state.config.reply_to_message);
        assert_eq!(state.config.auto_join, AutoJoinPolicy::Allowlist);
        assert_eq!(
            state.config.dm_policy,
            moltis_channels::gating::DmPolicy::Open
        );
    }

    #[test]
    fn config_update_can_switch_from_access_token_to_password_auth() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            map.insert("test".into(), test_account_state(cancel));
        }

        if let Err(error) = plugin.update_account_config(
            "test",
            serde_json::json!({
                "access_token": "",
                "password": "wordpass",
                "user_id": "@moltis:example.com",
            }),
        ) {
            panic!("auth mode switch should succeed: {error}");
        }

        let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = map.get("test") else {
            panic!("test account inserted");
        };
        assert!(state.config.access_token.expose_secret().is_empty());
        assert_eq!(
            state
                .config
                .password
                .as_ref()
                .map(|secret| secret.expose_secret().as_str()),
            Some("wordpass")
        );
        assert!(matches!(
            client::auth_mode(&state.config),
            Ok(AuthMode::Password)
        ));
    }

    #[test]
    fn redacted_password_update_preserves_existing_password() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            let mut state = test_account_state(cancel);
            state.config.access_token = Secret::new(String::new());
            state.config.password = Some(Secret::new("wordpass".into()));
            map.insert("test".into(), state);
        }

        if let Err(error) = plugin.update_account_config(
            "test",
            serde_json::json!({
                "password": "[REDACTED]",
                "room_policy": "open",
            }),
        ) {
            panic!("password-preserving update should succeed: {error}");
        }

        let map = plugin.accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = map.get("test") else {
            panic!("test account inserted");
        };
        assert_eq!(
            state
                .config
                .password
                .as_ref()
                .map(|secret| secret.expose_secret().as_str()),
            Some("wordpass")
        );
        assert_eq!(
            state.config.room_policy,
            moltis_channels::gating::GroupPolicy::Open
        );
    }

    #[test]
    fn probe_exposes_matrix_verification_status_details() {
        let plugin = MatrixPlugin::new();
        let cancel = CancellationToken::new();
        {
            let mut map = plugin.accounts.write().unwrap_or_else(|e| e.into_inner());
            let state = test_account_state(cancel);
            {
                let mut verification = state
                    .verification
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                verification
                    .prompts
                    .insert("flow-1".into(), VerificationPrompt {
                        flow_id: "flow-1".into(),
                        other_user_id: "@alice:example.com".into(),
                        room_id: Some("!room:example.com".into()),
                        emoji_lines: vec!["🐶 Dog".into(), "🔥 Fire".into()],
                    });
            }
            map.insert("test".into(), state);
        }

        let snapshot = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("matrix test runtime should build: {error}"))
            .block_on(plugin.probe("test"))
            .unwrap_or_else(|error| panic!("matrix probe should succeed: {error}"));

        let extra = snapshot
            .extra
            .unwrap_or_else(|| panic!("matrix probe should expose extra status"));
        assert_eq!(
            extra["matrix"]["verification_state"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            extra["matrix"]["pending_verifications"][0]["other_user_id"].as_str(),
            Some("@alice:example.com")
        );
        assert_eq!(
            extra["matrix"]["pending_verifications"][0]["emoji_lines"][0].as_str(),
            Some("🐶 Dog")
        );
    }
}
