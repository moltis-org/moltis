use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {
    moltis_channels::{ChannelEventSink, message_log::MessageLog},
    tokio_util::sync::CancellationToken,
};

use crate::config::SlackAccountConfig;

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub account_id: String,
    pub config: SlackAccountConfig,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    pub cancel: CancellationToken,
    /// Bot user ID obtained from `auth.test` — signals the connection is ready.
    pub bot_user_id: Option<String>,
    /// Pending thread timestamps keyed by `channel_id:user_id`.
    /// Used to route replies into the correct thread.
    pub pending_threads: HashMap<String, String>,
}
