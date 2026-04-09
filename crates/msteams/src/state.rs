use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {
    moltis_channels::{ChannelEventSink, message_log::MessageLog},
    reqwest::Client,
    tokio::sync::Mutex,
};

use crate::{auth::CachedAccessToken, config::MsTeamsAccountConfig, jwt::BotFrameworkJwtValidator};

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Per-account runtime state.
pub struct AccountState {
    pub account_id: String,
    pub config: MsTeamsAccountConfig,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    pub http: Client,
    /// Bot Framework token cache (audience: api.botframework.com).
    pub token_cache: Arc<Mutex<Option<CachedAccessToken>>>,
    /// Microsoft Graph token cache (audience: graph.microsoft.com).
    pub graph_token_cache: Arc<Mutex<Option<CachedAccessToken>>>,
    pub service_urls: Arc<RwLock<HashMap<String, String>>>,
    /// JWT validator for Bot Framework token verification.
    pub jwt_validator: Option<Arc<BotFrameworkJwtValidator>>,
    /// Tracks which conversations have received a welcome card/message.
    pub welcomed_conversations: Arc<RwLock<std::collections::HashSet<String>>>,
}
