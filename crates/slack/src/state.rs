use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, RwLock},
};

use {
    moltis_channels::{ChannelEventSink, message_log::MessageLog, otp::OtpState},
    tokio_util::sync::CancellationToken,
};

use crate::config::SlackAccountConfig;

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Bounded set of recently seen Slack event ids.
///
/// Slack retries an envelope when it is not acknowledged in time, so the same
/// event can arrive more than once. Without this a retry would start a second
/// agent turn for one user message.
#[derive(Default)]
pub struct EventDedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl EventDedup {
    /// Maximum retained ids. Slack retries within minutes, so this is ample.
    const MAX: usize = 2048;

    /// Record an event id, returning `true` if it had not been seen before.
    pub fn insert_new(&mut self, event_id: &str) -> bool {
        if !self.seen.insert(event_id.to_string()) {
            return false;
        }
        self.order.push_back(event_id.to_string());
        while self.order.len() > Self::MAX {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

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
    pub otp: Mutex<OtpState>,
    /// Recently processed event ids, for retry idempotency.
    pub dedup: Mutex<EventDedup>,
}
