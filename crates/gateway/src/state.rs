use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[cfg(feature = "metrics")]
use moltis_metrics::MetricsHandle;

// Re-export for use by other modules
#[cfg(feature = "metrics")]
pub use moltis_metrics::{MetricsHistoryPoint, MetricsStore, ProviderTokens, SqliteMetricsStore};

use tokio::sync::{RwLock, mpsc, oneshot};

// ── Metrics history ──────────────────────────────────────────────────────────

/// Ring buffer for storing metrics history.
#[cfg(feature = "metrics")]
pub struct MetricsHistory {
    points: VecDeque<MetricsHistoryPoint>,
    max_points: usize,
}

#[cfg(feature = "metrics")]
impl MetricsHistory {
    /// Create a new history buffer with the given capacity.
    /// Default: 360 points = 1 hour at 10-second intervals.
    pub fn new(max_points: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(max_points),
            max_points,
        }
    }

    /// Add a new data point, evicting the oldest if at capacity.
    pub fn push(&mut self, point: MetricsHistoryPoint) {
        if self.points.len() >= self.max_points {
            self.points.pop_front();
        }
        self.points.push_back(point);
    }

    /// Iterate over all stored points (oldest to newest).
    pub fn iter(&self) -> impl Iterator<Item = &MetricsHistoryPoint> {
        self.points.iter()
    }
}

#[cfg(feature = "metrics")]
impl Default for MetricsHistory {
    fn default() -> Self {
        Self::new(60480) // 7 days at 10-second intervals
    }
}

/// Broadcast payload for metrics updates via WebSocket.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsUpdatePayload {
    /// Current metrics snapshot.
    pub snapshot: moltis_metrics::MetricsSnapshot,
    /// Latest history point for charts.
    pub point: MetricsHistoryPoint,
}

use moltis_protocol::ConnectParams;

use moltis_tools::{approval::ApprovalManager, sandbox::SandboxRouter};

use moltis_channels::ChannelReplyTarget;

use crate::{
    auth::{CredentialStore, ResolvedAuth},
    nodes::NodeRegistry,
    pairing::PairingState,
    services::GatewayServices,
};

// ── Connected client ─────────────────────────────────────────────────────────

/// A WebSocket client currently connected to the gateway.
#[derive(Debug)]
pub struct ConnectedClient {
    pub conn_id: String,
    pub connect_params: ConnectParams,
    /// Channel for sending serialized frames to this client's write loop.
    pub sender: mpsc::UnboundedSender<String>,
    pub connected_at: Instant,
    pub last_activity: Instant,
    /// The `Accept-Language` header from the WebSocket upgrade request, forwarded
    /// to web tools so fetched pages and search results match the user's locale.
    pub accept_language: Option<String>,
}

impl ConnectedClient {
    pub fn role(&self) -> &str {
        self.connect_params.role.as_deref().unwrap_or("operator")
    }

    pub fn scopes(&self) -> Vec<&str> {
        self.connect_params
            .scopes
            .as_ref()
            .map(|s| s.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes()
            .iter()
            .any(|s| *s == moltis_protocol::scopes::ADMIN || *s == scope)
    }

    /// Send a serialized JSON frame to this client.
    pub fn send(&self, frame: &str) -> bool {
        self.sender.send(frame.to_string()).is_ok()
    }

    /// Touch the activity timestamp.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}

// ── Dedupe cache ─────────────────────────────────────────────────────────────

struct DedupeEntry {
    inserted_at: Instant,
}

/// Simple TTL-based idempotency cache.
pub struct DedupeCache {
    entries: HashMap<String, DedupeEntry>,
    ttl: std::time::Duration,
    max_entries: usize,
}

impl Default for DedupeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: std::time::Duration::from_millis(moltis_protocol::DEDUPE_TTL_MS),
            max_entries: moltis_protocol::DEDUPE_MAX_ENTRIES,
        }
    }

    /// Returns true if the key is a duplicate (already seen within TTL).
    pub fn check_and_insert(&mut self, key: &str) -> bool {
        self.evict_expired();
        if self.entries.contains_key(key) {
            return true;
        }
        if self.entries.len() >= self.max_entries
            && let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest_key);
        }
        self.entries.insert(key.to_string(), DedupeEntry {
            inserted_at: Instant::now(),
        });
        false
    }

    fn evict_expired(&mut self) {
        let cutoff = Instant::now() - self.ttl;
        self.entries.retain(|_, v| v.inserted_at > cutoff);
    }
}

// ── Pending node invoke ─────────────────────────────────────────────────────

/// A pending RPC invocation waiting for a node to respond.
pub struct PendingInvoke {
    pub request_id: String,
    pub sender: oneshot::Sender<serde_json::Value>,
    pub created_at: Instant,
}

// ── Discovered hook info ─────────────────────────────────────────────────────

/// Metadata about a discovered hook, exposed to the web UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredHookInfo {
    pub name: String,
    pub description: String,
    pub emoji: Option<String>,
    pub events: Vec<String>,
    pub command: Option<String>,
    pub timeout: u64,
    pub priority: i32,
    /// `"project"` or `"user"`.
    pub source: String,
    pub source_path: String,
    pub eligible: bool,
    pub missing_os: bool,
    pub missing_bins: Vec<String>,
    pub missing_env: Vec<String>,
    pub enabled: bool,
    /// Raw HOOK.md content (frontmatter + body).
    pub body: String,
    /// Server-rendered HTML of the markdown body (after frontmatter).
    pub body_html: String,
    pub call_count: u64,
    pub failure_count: u64,
    pub avg_latency_ms: u64,
}

// ── Gateway state ────────────────────────────────────────────────────────────

/// Shared gateway runtime state, wrapped in Arc for use across async tasks.
pub struct GatewayState {
    /// All connected WebSocket clients, keyed by conn_id.
    pub clients: RwLock<HashMap<String, ConnectedClient>>,
    /// Monotonically increasing sequence counter for broadcast events.
    pub seq: AtomicU64,
    /// Idempotency cache.
    pub dedupe: RwLock<DedupeCache>,
    /// Server version string.
    pub version: String,
    /// Hostname for HelloOk.
    pub hostname: String,
    /// Auth configuration.
    pub auth: ResolvedAuth,
    /// Connected device nodes.
    pub nodes: RwLock<NodeRegistry>,
    /// Device pairing state.
    pub pairing: RwLock<PairingState>,
    /// Pending node invoke requests awaiting results.
    pub pending_invokes: RwLock<HashMap<String, PendingInvoke>>,
    /// Domain services.
    pub services: GatewayServices,
    /// Approval manager for exec command gating.
    pub approval_manager: Arc<ApprovalManager>,
    /// Late-bound chat service override (for circular init).
    pub chat_override: RwLock<Option<Arc<dyn crate::services::ChatService>>>,
    /// Active session key per connection (conn_id → session key).
    pub active_sessions: RwLock<HashMap<String, String>>,
    /// Active project id per connection (conn_id → project id).
    pub active_projects: RwLock<HashMap<String, String>>,
    /// Credential store for authentication (password, passkeys, API keys).
    pub credential_store: Option<Arc<CredentialStore>>,
    /// WebAuthn state for passkey registration/authentication.
    pub webauthn_state: Option<Arc<crate::auth_webauthn::WebAuthnState>>,
    /// Per-session sandbox router (None if sandbox is not configured).
    pub sandbox_router: Option<Arc<SandboxRouter>>,
    /// Heartbeat configuration (for gon data and RPC methods).
    pub heartbeat_config: RwLock<moltis_config::schema::HeartbeatConfig>,
    /// Pending channel reply targets: when a channel message triggers a chat
    /// send, we queue the reply target so the "final" response can be routed
    /// back to the originating channel.
    pub channel_reply_queue: RwLock<HashMap<String, Vec<ChannelReplyTarget>>>,
    /// Hook registry for dispatching lifecycle events.
    pub hook_registry: RwLock<Option<Arc<moltis_common::hooks::HookRegistry>>>,
    /// Discovered hook metadata for the web UI.
    pub discovered_hooks: RwLock<Vec<DiscoveredHookInfo>>,
    /// Hook names that have been manually disabled via the UI.
    pub disabled_hooks: RwLock<HashSet<String>>,
    /// Memory manager for long-term memory search (None if no embedding provider).
    pub memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>>,
    /// One-time setup code displayed at startup, required during initial setup.
    /// Cleared after successful setup.
    pub setup_code: RwLock<Option<secrecy::Secret<String>>>,
    /// Whether the server is bound to a loopback address (localhost/127.0.0.1/::1).
    pub localhost_only: bool,
    /// Whether TLS is active on the gateway listener.
    pub tls_active: bool,
    /// Cloud deploy platform (e.g. "flyio", "digitalocean"), read from
    /// `MOLTIS_DEPLOY_PLATFORM`. `None` when running locally.
    pub deploy_platform: Option<String>,
    /// The port the gateway is bound to.
    pub port: u16,
    /// Auto-update availability state from GitHub releases.
    pub update: RwLock<crate::update_check::UpdateAvailability>,
    /// Last error per run_id (short-lived, for send_sync to retrieve).
    pub run_errors: RwLock<HashMap<String, String>>,
    /// Metrics handle for Prometheus export (None if metrics disabled).
    #[cfg(feature = "metrics")]
    pub metrics_handle: Option<MetricsHandle>,
    /// Historical metrics data for time-series charts (in-memory cache).
    #[cfg(feature = "metrics")]
    pub metrics_history: RwLock<MetricsHistory>,
    /// Persistent metrics store (SQLite or other backend).
    #[cfg(feature = "metrics")]
    pub metrics_store: Option<Arc<dyn MetricsStore>>,
    /// Push notification service for sending notifications to subscribed devices.
    #[cfg(feature = "push-notifications")]
    pub push_service: RwLock<Option<Arc<crate::push::PushService>>>,
}

impl GatewayState {
    pub fn new(
        auth: ResolvedAuth,
        services: GatewayServices,
        approval_manager: Arc<ApprovalManager>,
    ) -> Arc<Self> {
        Self::with_options(
            auth,
            services,
            approval_manager,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
            18789,
            None,
            #[cfg(feature = "metrics")]
            None,
            #[cfg(feature = "metrics")]
            None,
        )
    }

    pub fn with_sandbox_router(
        auth: ResolvedAuth,
        services: GatewayServices,
        approval_manager: Arc<ApprovalManager>,
        sandbox_router: Option<Arc<SandboxRouter>>,
    ) -> Arc<Self> {
        Self::with_options(
            auth,
            services,
            approval_manager,
            sandbox_router,
            None,
            None,
            false,
            false,
            None,
            None,
            18789,
            None,
            #[cfg(feature = "metrics")]
            None,
            #[cfg(feature = "metrics")]
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_options(
        auth: ResolvedAuth,
        services: GatewayServices,
        approval_manager: Arc<ApprovalManager>,
        sandbox_router: Option<Arc<SandboxRouter>>,
        credential_store: Option<Arc<CredentialStore>>,
        webauthn_state: Option<Arc<crate::auth_webauthn::WebAuthnState>>,
        localhost_only: bool,
        tls_active: bool,
        hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
        memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>>,
        port: u16,
        deploy_platform: Option<String>,
        #[cfg(feature = "metrics")] metrics_handle: Option<MetricsHandle>,
        #[cfg(feature = "metrics")] metrics_store: Option<Arc<dyn MetricsStore>>,
    ) -> Arc<Self> {
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".into());

        Arc::new(Self {
            clients: RwLock::new(HashMap::new()),
            seq: AtomicU64::new(0),
            dedupe: RwLock::new(DedupeCache::new()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            hostname,
            auth,
            nodes: RwLock::new(NodeRegistry::new()),
            pairing: RwLock::new(PairingState::new()),
            pending_invokes: RwLock::new(HashMap::new()),
            services,
            approval_manager,
            credential_store,
            webauthn_state,
            chat_override: RwLock::new(None),
            active_sessions: RwLock::new(HashMap::new()),
            active_projects: RwLock::new(HashMap::new()),
            sandbox_router,
            channel_reply_queue: RwLock::new(HashMap::new()),
            hook_registry: RwLock::new(hook_registry),
            discovered_hooks: RwLock::new(Vec::new()),
            disabled_hooks: RwLock::new(HashSet::new()),
            memory_manager,
            setup_code: RwLock::new(None),
            localhost_only,
            tls_active,
            deploy_platform,
            port,
            update: RwLock::new(crate::update_check::UpdateAvailability::default()),
            heartbeat_config: RwLock::new(moltis_config::schema::HeartbeatConfig::default()),
            run_errors: RwLock::new(HashMap::new()),
            #[cfg(feature = "metrics")]
            metrics_handle,
            #[cfg(feature = "metrics")]
            metrics_history: RwLock::new(MetricsHistory::default()),
            #[cfg(feature = "metrics")]
            metrics_store,
            #[cfg(feature = "push-notifications")]
            push_service: RwLock::new(None),
        })
    }

    /// Set a late-bound chat service (for circular init).
    pub async fn set_chat(&self, chat: Arc<dyn crate::services::ChatService>) {
        *self.chat_override.write().await = Some(chat);
    }

    /// Set the push notification service (late-bound initialization).
    #[cfg(feature = "push-notifications")]
    pub async fn set_push_service(&self, service: Arc<crate::push::PushService>) {
        *self.push_service.write().await = Some(service);
    }

    /// Get the push notification service if configured.
    #[cfg(feature = "push-notifications")]
    pub async fn get_push_service(&self) -> Option<Arc<crate::push::PushService>> {
        self.push_service.read().await.clone()
    }

    /// Get the active chat service (override or default).
    pub async fn chat(&self) -> Arc<dyn crate::services::ChatService> {
        if let Some(c) = self.chat_override.read().await.as_ref() {
            return Arc::clone(c);
        }
        Arc::clone(&self.services.chat)
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Register a new client connection.
    pub async fn register_client(&self, client: ConnectedClient) {
        let conn_id = client.conn_id.clone();
        self.clients.write().await.insert(conn_id, client);
    }

    /// Remove a client by conn_id. Returns the removed client if found.
    pub async fn remove_client(&self, conn_id: &str) -> Option<ConnectedClient> {
        self.clients.write().await.remove(conn_id)
    }

    /// Number of connected clients.
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Push a reply target for a session (used when a channel message triggers chat.send).
    pub async fn push_channel_reply(&self, session_key: &str, target: ChannelReplyTarget) {
        let mut queue = self.channel_reply_queue.write().await;
        queue
            .entry(session_key.to_string())
            .or_default()
            .push(target);
    }

    /// Drain all pending reply targets for a session.
    pub async fn drain_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget> {
        let mut queue = self.channel_reply_queue.write().await;
        queue.remove(session_key).unwrap_or_default()
    }

    /// Record a run error (for send_sync to retrieve).
    pub async fn set_run_error(&self, run_id: &str, error: String) {
        self.run_errors
            .write()
            .await
            .insert(run_id.to_string(), error);
    }

    /// Take (and remove) the last error for a run_id.
    pub async fn last_run_error(&self, run_id: &str) -> Option<String> {
        self.run_errors.write().await.remove(run_id)
    }

    /// Close a client: remove from registry, abort if needed.
    pub async fn close_client(&self, conn_id: &str) -> Option<ConnectedClient> {
        // Also unregister from node registry if it was a node.
        self.nodes.write().await.unregister_by_conn(conn_id);
        self.remove_client(conn_id).await
    }
}
