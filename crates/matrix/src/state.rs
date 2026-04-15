use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use {
    matrix_sdk::encryption::recovery::IdentityResetHandle,
    moltis_channels::{ChannelEventSink, message_log::MessageLog, otp::OtpState},
    tokio_util::sync::CancellationToken,
};

use crate::config::MatrixAccountConfig;

/// Pending OIDC login state held between the start and complete phases.
pub(crate) struct OidcPendingState {
    pub client: matrix_sdk::Client,
    pub csrf_state: String,
}

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub account_id: String,
    pub config: MatrixAccountConfig,
    pub client: matrix_sdk::Client,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    pub cancel: CancellationToken,
    pub bot_user_id: String,
    pub ownership_startup_error: Option<String>,
    pub initial_sync_complete: AtomicBool,
    pub pending_identity_reset: Mutex<Option<IdentityResetHandle>>,
    /// In-memory OTP challenges (std::sync::Mutex — never held across .await).
    pub otp: Mutex<OtpState>,
    /// In-memory Matrix verification flow state.
    pub verification: Mutex<VerificationRuntimeState>,
    /// Pending OIDC login state (between start and complete phases).
    pub(crate) oidc_pending: Mutex<Option<OidcPendingState>>,
}

impl AccountState {
    pub fn initial_sync_complete(&self) -> bool {
        self.initial_sync_complete.load(Ordering::Relaxed)
    }

    pub fn mark_initial_sync_complete(&self) {
        self.initial_sync_complete.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationPrompt {
    pub flow_id: String,
    pub other_user_id: String,
    pub room_id: Option<String>,
    pub emoji_lines: Vec<String>,
}

#[derive(Debug, Default)]
pub struct VerificationRuntimeState {
    pub watched_flows: HashSet<String>,
    pub prompts: HashMap<String, VerificationPrompt>,
    pub recent_utd_notice_by_room: HashMap<String, Instant>,
}
