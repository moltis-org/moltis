use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};

#[cfg(feature = "graphql")]
use std::sync::atomic::AtomicBool;

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

    /// Return the configured maximum capacity.
    pub fn capacity(&self) -> usize {
        self.max_points
    }
}

#[cfg(feature = "metrics")]
impl Default for MetricsHistory {
    fn default() -> Self {
        Self::new(360) // 1 hour at 10-second intervals
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

use moltis_protocol::{ConnectParams, EventFrame};

use moltis_tools::sandbox::SandboxRouter;

use {moltis_channels::ChannelReplyTarget, moltis_sessions::session_events::SessionEventBus};

use crate::{
    auth::{CredentialStore, ResolvedAuth},
    nodes::NodeRegistry,
    pairing::PairingState,
    services::GatewayServices,
};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsRuntimeOverride {
    pub provider: Option<String>,
    pub voice_id: Option<String>,
    pub model: Option<String>,
}

// ── Connected client ─────────────────────────────────────────────────────────

/// A WebSocket client currently connected to the gateway.
#[derive(Debug)]
pub struct ConnectedClient {
    pub conn_id: String,
    pub connect_params: ConnectParams,
    /// Bounded channel for sending serialized frames to this client's write loop.
    pub sender: mpsc::Sender<String>,
    pub connected_at: Instant,
    pub last_activity: Instant,
    /// The `Accept-Language` header from the WebSocket upgrade request, forwarded
    /// to web tools so fetched pages and search results match the user's locale.
    pub accept_language: Option<String>,
    /// The client's public IP address (extracted from proxy headers or direct
    /// connection). `None` when the client connects from a private/loopback address.
    pub remote_ip: Option<String>,
    /// The client's IANA timezone (e.g. `Europe/Lisbon`), sent by the browser
    /// via `Intl.DateTimeFormat().resolvedOptions().timeZone`.
    pub timezone: Option<String>,
    /// Event subscriptions (v4).
    /// `None` = wildcard (receive everything, v3 compat).
    /// `Some(set)` = only events in the set (or `"*"` = wildcard).
    pub subscriptions: Option<HashSet<String>>,
    /// Channels this client has joined (v4 multiplexing).
    pub joined_channels: HashSet<String>,
    /// Negotiated protocol version for this connection.
    pub negotiated_protocol: u32,
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

    /// Check whether this client is subscribed to the given event.
    /// `None` subscriptions = wildcard (receive everything).
    pub fn is_subscribed_to(&self, event: &str) -> bool {
        match &self.subscriptions {
            None => true,
            Some(set) => {
                set.contains(moltis_protocol::subscriptions::WILDCARD) || set.contains(event)
            },
        }
    }

    /// Check whether this client has joined the given channel.
    pub fn is_in_channel(&self, channel: &str) -> bool {
        self.joined_channels.contains(channel)
    }

    /// Send a serialized JSON frame to this client.
    ///
    /// Uses `try_send` to avoid blocking; drops the frame if the client's
    /// outbound buffer is full (slow consumer protection).
    pub fn send(&self, frame: &str) -> bool {
        self.sender.try_send(frame.to_string()).is_ok()
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

// ── Pending client request (v4 bidir RPC) ───────────────────────────────────

/// A server-initiated RPC request waiting for a client response.
pub struct PendingClientRequest {
    pub method: String,
    pub sender: oneshot::Sender<Result<serde_json::Value, moltis_protocol::ErrorShape>>,
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

// ── Mutable runtime state ────────────────────────────────────────────────────

/// All mutable runtime state, protected by the single `RwLock` on `GatewayState`.
pub struct GatewayInner {
    /// All connected WebSocket clients, keyed by conn_id.
    pub clients: HashMap<String, ConnectedClient>,
    /// Idempotency cache.
    pub dedupe: DedupeCache,
    /// Connected device nodes.
    pub nodes: NodeRegistry,
    /// Device pairing state.
    pub pairing: PairingState,
    /// Pending node invoke requests awaiting results.
    pub pending_invokes: HashMap<String, PendingInvoke>,
    /// Pending server → client RPC requests awaiting client responses (v4).
    pub pending_client_requests: HashMap<String, PendingClientRequest>,
    /// Late-bound chat service override (for circular init).
    pub chat_override: Option<Arc<dyn crate::services::ChatService>>,
    /// Active session key per connection (conn_id → session key).
    pub active_sessions: HashMap<String, String>,
    /// Active project id per connection (conn_id → project id).
    pub active_projects: HashMap<String, String>,
    /// Heartbeat configuration (for gon data and RPC methods).
    pub heartbeat_config: moltis_config::schema::HeartbeatConfig,
    /// Pending channel reply targets: when a channel message triggers a chat
    /// send, we queue the reply target so the "final" response can be routed
    /// back to the originating channel.
    pub channel_reply_queue: HashMap<String, Vec<ChannelReplyTarget>>,
    /// Per-session TTS runtime overrides (session_key -> override).
    pub tts_session_overrides: HashMap<String, TtsRuntimeOverride>,
    /// Per-channel-account TTS runtime overrides ((channel, account) -> override).
    pub tts_channel_overrides: HashMap<String, TtsRuntimeOverride>,
    /// Hook registry for dispatching lifecycle events.
    pub hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    /// Discovered hook metadata for the web UI.
    pub discovered_hooks: Vec<DiscoveredHookInfo>,
    /// Hook names that have been manually disabled via the UI.
    pub disabled_hooks: HashSet<String>,
    /// One-time setup code displayed at startup, required during initial setup.
    /// Cleared after successful setup.
    pub setup_code: Option<secrecy::Secret<String>>,
    /// Auto-update availability state from GitHub releases.
    pub update: crate::update_check::UpdateAvailability,
    /// Last error per run_id (short-lived, for send_sync to retrieve).
    pub run_errors: HashMap<String, String>,
    /// Historical metrics data for time-series charts (in-memory cache).
    #[cfg(feature = "metrics")]
    pub metrics_history: MetricsHistory,
    /// Push notification service for sending notifications to subscribed devices.
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<crate::push::PushService>>,
    /// LLM provider registry for lightweight generation (e.g. TTS phrases).
    pub llm_providers: Option<Arc<RwLock<moltis_providers::ProviderRegistry>>>,
    /// Cached user geolocation from browser Geolocation API, persisted to `USER.md`.
    pub cached_location: Option<moltis_config::GeoLocation>,
    /// Per-session buffer for channel status messages (tool use, model selection).
    /// Drained when the final response is delivered to the channel.
    pub channel_status_log: HashMap<String, Vec<String>>,
    /// Sessions currently in channel command mode (/sh passthrough).
    pub channel_command_mode_sessions: HashSet<String>,
    /// Which channel types are offered in the web UI (from config).
    pub channels_offered: Vec<String>,
    /// Hostnames that were discovered after passkeys already existed.
    /// Users should sign in with password and register a fresh passkey on these hosts.
    pub passkey_host_update_pending: HashSet<String>,
}

impl GatewayInner {
    fn new(hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>) -> Self {
        Self {
            clients: HashMap::new(),
            dedupe: DedupeCache::new(),
            nodes: NodeRegistry::new(),
            pairing: PairingState::new(),
            pending_invokes: HashMap::new(),
            pending_client_requests: HashMap::new(),
            chat_override: None,
            active_sessions: HashMap::new(),
            active_projects: HashMap::new(),
            heartbeat_config: moltis_config::schema::HeartbeatConfig::default(),
            channel_reply_queue: HashMap::new(),
            tts_session_overrides: HashMap::new(),
            tts_channel_overrides: HashMap::new(),
            hook_registry,
            discovered_hooks: Vec::new(),
            disabled_hooks: HashSet::new(),
            setup_code: None,
            update: crate::update_check::UpdateAvailability::default(),
            run_errors: HashMap::new(),
            #[cfg(feature = "metrics")]
            metrics_history: MetricsHistory::default(),
            #[cfg(feature = "push-notifications")]
            push_service: None,
            llm_providers: None,
            cached_location: moltis_config::load_user().and_then(|u| u.location),
            channel_status_log: HashMap::new(),
            channel_command_mode_sessions: HashSet::new(),
            channels_offered: vec!["telegram".into()],
            passkey_host_update_pending: HashSet::new(),
        }
    }

    /// Insert a client, returning the new client count.
    pub fn register_client(&mut self, client: ConnectedClient) -> usize {
        let conn_id = client.conn_id.clone();
        self.clients.insert(conn_id, client);
        self.clients.len()
    }

    /// Remove a client by conn_id. Returns the removed client and the new count.
    pub fn remove_client(&mut self, conn_id: &str) -> (Option<ConnectedClient>, usize) {
        let removed = self.clients.remove(conn_id);
        (removed, self.clients.len())
    }
}

// ── Gateway state ────────────────────────────────────────────────────────────

/// Shared gateway runtime state, wrapped in `Arc` for use across async tasks.
///
/// Immutable fields and atomics live directly on this struct (no lock needed).
/// All mutable runtime state is consolidated in [`GatewayInner`] behind a
/// single `RwLock`.
pub struct GatewayState {
    // ── Immutable (set at construction, never changes) ──────────────────────
    /// Server version string.
    pub version: String,
    /// Hostname for HelloOk.
    pub hostname: String,
    /// Auth configuration.
    pub auth: ResolvedAuth,
    /// Domain services.
    pub services: GatewayServices,
    /// Credential store for authentication (password, passkeys, API keys).
    /// `Arc` because it is shared cross-crate (e.g. `ExecTool` as `dyn EnvVarProvider`).
    pub credential_store: Option<Arc<CredentialStore>>,
    /// Per-session sandbox router (None if sandbox is not configured).
    /// `Arc` because it is shared with `ExecTool`/`ProcessTool` in `moltis-tools`.
    pub sandbox_router: Option<Arc<SandboxRouter>>,
    /// Memory manager for long-term memory search (None if no embedding provider).
    /// `Arc` because it is cloned into background tokio tasks.
    pub memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>>,
    /// Whether the server is bound to a loopback address (localhost/127.0.0.1/::1).
    pub localhost_only: bool,
    /// Whether the server is known to be behind a reverse proxy.
    /// Set via `MOLTIS_BEHIND_PROXY=true`.  When true, loopback source IPs are
    /// never treated as proof of a direct local connection.
    pub behind_proxy: bool,
    /// Whether TLS is active on the gateway listener.
    pub tls_active: bool,
    /// Whether WebSocket request/response logging is enabled.
    pub ws_request_logs: bool,
    /// Runtime GraphQL availability toggle.
    #[cfg(feature = "graphql")]
    pub graphql_enabled: AtomicBool,
    /// Broadcast channel for GraphQL subscriptions. Events are `(event_name, payload)`.
    #[cfg(feature = "graphql")]
    pub graphql_broadcast: tokio::sync::broadcast::Sender<(String, serde_json::Value)>,
    /// Session event bus for cross-UI synchronisation (macOS ↔ web).
    pub session_event_bus: SessionEventBus,
    /// Cloud deploy platform (e.g. "flyio", "digitalocean"), read from
    /// `MOLTIS_DEPLOY_PLATFORM`. `None` when running locally.
    pub deploy_platform: Option<String>,
    /// The port the gateway is bound to.
    pub port: u16,
    /// Monotonic process start timestamp used for uptime calculations.
    pub started_at: Instant,
    /// Metrics handle for Prometheus export (None if metrics disabled).
    #[cfg(feature = "metrics")]
    pub metrics_handle: Option<MetricsHandle>,
    /// Persistent metrics store (SQLite or other backend).
    #[cfg(feature = "metrics")]
    pub metrics_store: Option<Arc<dyn MetricsStore>>,
    /// Encryption-at-rest vault for environment variables.
    #[cfg(feature = "vault")]
    pub vault: Option<Arc<moltis_vault::Vault>>,

    // ── Atomics (lock-free) ─────────────────────────────────────────────────
    /// Monotonically increasing sequence counter for broadcast events.
    pub seq: AtomicU64,
    /// Sequential counter for TTS test phrase round-robin picking.
    pub tts_phrase_counter: AtomicUsize,

    // ── Mutable runtime state (single lock) ─────────────────────────────────
    /// All mutable runtime state, behind a single lock.
    pub inner: RwLock<GatewayInner>,
}

impl GatewayState {
    pub fn new(auth: ResolvedAuth, services: GatewayServices) -> Arc<Self> {
        Self::with_options(
            auth,
            services,
            None,
            None,
            false,
            false,
            false,
            None,
            None,
            18789,
            false,
            None,
            None,
            #[cfg(feature = "metrics")]
            None,
            #[cfg(feature = "metrics")]
            None,
            #[cfg(feature = "vault")]
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_options(
        auth: ResolvedAuth,
        services: GatewayServices,
        sandbox_router: Option<Arc<SandboxRouter>>,
        credential_store: Option<Arc<CredentialStore>>,
        localhost_only: bool,
        behind_proxy: bool,
        tls_active: bool,
        hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
        memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>>,
        port: u16,
        ws_request_logs: bool,
        deploy_platform: Option<String>,
        session_event_bus: Option<SessionEventBus>,
        #[cfg(feature = "metrics")] metrics_handle: Option<MetricsHandle>,
        #[cfg(feature = "metrics")] metrics_store: Option<Arc<dyn MetricsStore>>,
        #[cfg(feature = "vault")] vault: Option<Arc<moltis_vault::Vault>>,
    ) -> Arc<Self> {
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".into());

        Arc::new(Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            hostname,
            auth,
            services,
            credential_store,
            sandbox_router,
            memory_manager,
            localhost_only,
            behind_proxy,
            tls_active,
            ws_request_logs,
            session_event_bus: session_event_bus.unwrap_or_default(),
            deploy_platform,
            port,
            started_at: Instant::now(),
            #[cfg(feature = "graphql")]
            graphql_enabled: AtomicBool::new(true),
            #[cfg(feature = "metrics")]
            metrics_handle,
            #[cfg(feature = "metrics")]
            metrics_store,
            #[cfg(feature = "vault")]
            vault,
            seq: AtomicU64::new(0),
            tts_phrase_counter: AtomicUsize::new(0),
            #[cfg(feature = "graphql")]
            graphql_broadcast: {
                let (tx, _) = tokio::sync::broadcast::channel(256);
                tx
            },
            inner: RwLock::new(GatewayInner::new(hook_registry)),
        })
    }

    /// Process uptime in milliseconds since this gateway state was created.
    pub fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Set a late-bound chat service (for circular init).
    pub async fn set_chat(&self, chat: Arc<dyn crate::services::ChatService>) {
        self.inner.write().await.chat_override = Some(chat);
    }

    /// Set the push notification service (late-bound initialization).
    #[cfg(feature = "push-notifications")]
    pub async fn set_push_service(&self, service: Arc<crate::push::PushService>) {
        self.inner.write().await.push_service = Some(service);
    }

    /// Get the push notification service if configured.
    #[cfg(feature = "push-notifications")]
    pub async fn get_push_service(&self) -> Option<Arc<crate::push::PushService>> {
        self.inner.read().await.push_service.clone()
    }

    /// Return the next sequential index for TTS phrase round-robin picking.
    pub fn next_tts_phrase_index(&self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        self.tts_phrase_counter.fetch_add(1, Ordering::Relaxed) % len
    }

    /// Get the active chat service (override or default).
    pub async fn chat(&self) -> Arc<dyn crate::services::ChatService> {
        if let Some(c) = self.inner.read().await.chat_override.as_ref() {
            return Arc::clone(c);
        }
        Arc::clone(&self.services.chat)
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    #[cfg(feature = "graphql")]
    pub fn is_graphql_enabled(&self) -> bool {
        self.graphql_enabled.load(Ordering::Relaxed)
    }

    #[cfg(feature = "graphql")]
    pub fn set_graphql_enabled(&self, enabled: bool) {
        self.graphql_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Register a new client connection.
    pub async fn register_client(&self, client: ConnectedClient) {
        let count = self.inner.write().await.register_client(client);

        #[cfg(feature = "metrics")]
        moltis_metrics::gauge!(moltis_metrics::system::CONNECTED_CLIENTS).set(count as f64);
    }

    /// Remove a client by conn_id. Returns the removed client if found.
    pub async fn remove_client(&self, conn_id: &str) -> Option<ConnectedClient> {
        let (removed, count) = self.inner.write().await.remove_client(conn_id);

        #[cfg(feature = "metrics")]
        {
            let _ = count;
            moltis_metrics::gauge!(moltis_metrics::system::CONNECTED_CLIENTS).set(count as f64);
        }
        #[cfg(not(feature = "metrics"))]
        let _ = count;

        removed
    }

    /// Number of connected clients.
    pub async fn client_count(&self) -> usize {
        self.inner.read().await.clients.len()
    }

    /// Push a reply target for a session (used when a channel message triggers chat.send).
    pub async fn push_channel_reply(&self, session_key: &str, target: ChannelReplyTarget) {
        self.inner
            .write()
            .await
            .channel_reply_queue
            .entry(session_key.to_string())
            .or_default()
            .push(target);
    }

    /// Drain all pending reply targets for a session.
    pub async fn drain_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget> {
        self.inner
            .write()
            .await
            .channel_reply_queue
            .remove(session_key)
            .unwrap_or_default()
    }

    /// Get a copy of pending reply targets without removing them.
    pub async fn peek_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget> {
        self.inner
            .read()
            .await
            .channel_reply_queue
            .get(session_key)
            .cloned()
            .unwrap_or_default()
    }

    /// Record a run error (for send_sync to retrieve).
    pub async fn set_run_error(&self, run_id: &str, error: String) {
        self.inner
            .write()
            .await
            .run_errors
            .insert(run_id.to_string(), error);
    }

    /// Take (and remove) the last error for a run_id.
    pub async fn last_run_error(&self, run_id: &str) -> Option<String> {
        self.inner.write().await.run_errors.remove(run_id)
    }

    /// Append a status line (e.g. tool use, model selection) to the channel
    /// status log for a session. These are drained and appended as a logbook
    /// when the final response is delivered.
    pub async fn push_channel_status_log(&self, session_key: &str, message: String) {
        self.inner
            .write()
            .await
            .channel_status_log
            .entry(session_key.to_string())
            .or_default()
            .push(message);
    }

    /// Drain all buffered status log entries for a session.
    pub async fn drain_channel_status_log(&self, session_key: &str) -> Vec<String> {
        self.inner
            .write()
            .await
            .channel_status_log
            .remove(session_key)
            .unwrap_or_default()
    }

    /// Enable or disable /sh command mode for a channel session.
    pub async fn set_channel_command_mode(&self, session_key: &str, enabled: bool) {
        let mut inner = self.inner.write().await;
        if enabled {
            inner
                .channel_command_mode_sessions
                .insert(session_key.to_string());
        } else {
            inner.channel_command_mode_sessions.remove(session_key);
        }
    }

    /// Check whether /sh command mode is enabled for a channel session.
    pub async fn is_channel_command_mode_enabled(&self, session_key: &str) -> bool {
        self.inner
            .read()
            .await
            .channel_command_mode_sessions
            .contains(session_key)
    }

    /// Mark a hostname as needing passkey refresh.
    pub async fn add_passkey_host_update_pending(&self, host: &str) {
        let normalized = crate::auth_webauthn::normalize_host(host);
        if normalized.is_empty() {
            return;
        }
        self.inner
            .write()
            .await
            .passkey_host_update_pending
            .insert(normalized);
    }

    /// Clear the passkey-refresh marker for a hostname.
    pub async fn clear_passkey_host_update_pending(&self, host: &str) {
        let normalized = crate::auth_webauthn::normalize_host(host);
        if normalized.is_empty() {
            return;
        }
        self.inner
            .write()
            .await
            .passkey_host_update_pending
            .remove(&normalized);
    }

    /// Clear all pending passkey host update markers.
    pub async fn clear_all_passkey_host_update_pending(&self) {
        self.inner.write().await.passkey_host_update_pending.clear();
    }

    /// Return sorted hostnames that currently require a passkey refresh.
    pub async fn passkey_host_update_pending(&self) -> Vec<String> {
        let mut hosts = self
            .inner
            .read()
            .await
            .passkey_host_update_pending
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        hosts.sort();
        hosts
    }

    /// Send an RPC request to a connected client and await its response (v4 bidirectional RPC).
    pub async fn send_client_request(
        &self,
        conn_id: &str,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, moltis_protocol::ErrorShape> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let req_frame = moltis_protocol::RequestFrame {
            r#type: "req".into(),
            id: request_id.clone(),
            method: method.into(),
            params: Some(params),
            channel: None,
        };
        let json = serde_json::to_string(&req_frame).map_err(|e| {
            moltis_protocol::ErrorShape::new(moltis_protocol::error_codes::INTERNAL, e.to_string())
        })?;

        let (tx, rx) = oneshot::channel();

        // Register the pending request BEFORE sending to avoid a race where
        // the client responds before the entry exists (response would be dropped).
        {
            let mut inner = self.inner.write().await;
            if !inner.clients.contains_key(conn_id) {
                return Err(moltis_protocol::ErrorShape::new(
                    moltis_protocol::error_codes::UNAVAILABLE,
                    "client not connected",
                ));
            }
            inner
                .pending_client_requests
                .insert(request_id.clone(), PendingClientRequest {
                    method: method.into(),
                    sender: tx,
                    created_at: Instant::now(),
                });
            let sent = inner
                .clients
                .get(conn_id)
                .map(|c| c.send(&json))
                .unwrap_or(false);
            if !sent {
                inner.pending_client_requests.remove(&request_id);
                return Err(moltis_protocol::ErrorShape::new(
                    moltis_protocol::error_codes::UNAVAILABLE,
                    "client send failed",
                ));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(moltis_protocol::ErrorShape::new(
                moltis_protocol::error_codes::UNAVAILABLE,
                "client request cancelled",
            )),
            Err(_) => {
                self.inner
                    .write()
                    .await
                    .pending_client_requests
                    .remove(&request_id);
                Err(moltis_protocol::ErrorShape::new(
                    moltis_protocol::error_codes::TIMEOUT,
                    "client request timeout",
                ))
            },
        }
    }

    /// Close a client: remove from registry and unregister from nodes.
    pub async fn close_client(&self, conn_id: &str) -> Option<ConnectedClient> {
        let mut inner = self.inner.write().await;
        inner.nodes.unregister_by_conn(conn_id);
        let (removed, count) = inner.remove_client(conn_id);
        drop(inner);

        #[cfg(feature = "metrics")]
        moltis_metrics::gauge!(moltis_metrics::system::CONNECTED_CLIENTS).set(count as f64);
        #[cfg(not(feature = "metrics"))]
        let _ = count;

        removed
    }

    /// Disconnect all WebSocket clients: send an `auth.credentials_changed`
    /// event so browsers can redirect to login, then drain every connection.
    pub async fn disconnect_all_clients(&self, reason: &str) {
        let mut inner = self.inner.write().await;

        // Build and serialize the notification frame.
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = EventFrame::new(
            "auth.credentials_changed",
            serde_json::json!({ "reason": reason }),
            seq,
        );
        if let Ok(json) = serde_json::to_string(&frame) {
            for client in inner.clients.values() {
                let _ = client.send(&json);
            }
        }

        // Drain all state keyed by connection.
        inner.nodes.clear();
        inner.clients.clear();
        inner.active_sessions.clear();
        inner.active_projects.clear();

        drop(inner);

        #[cfg(feature = "metrics")]
        moltis_metrics::gauge!(moltis_metrics::system::CONNECTED_CLIENTS).set(0.0);

        tracing::info!(
            reason,
            "disconnected all WebSocket clients (credentials changed)"
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use {
        super::*,
        crate::{
            auth::{AuthMode, ResolvedAuth},
            services::GatewayServices,
        },
    };

    fn test_state() -> Arc<GatewayState> {
        GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            GatewayServices::noop(),
        )
    }

    fn mock_client(conn_id: &str) -> (ConnectedClient, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(512);
        let client = ConnectedClient {
            conn_id: conn_id.to_string(),
            connect_params: ConnectParams {
                min_protocol: 1,
                max_protocol: 1,
                client: moltis_protocol::ClientInfo {
                    id: "test".into(),
                    display_name: None,
                    version: "0.0.0".into(),
                    platform: "test".into(),
                    device_family: None,
                    model_identifier: None,
                    mode: "operator".into(),
                    instance_id: None,
                },
                caps: None,
                commands: None,
                permissions: None,
                path_env: None,
                role: None,
                scopes: None,
                device: None,
                auth: None,
                locale: None,
                user_agent: None,
                timezone: None,
            },
            sender: tx,
            connected_at: Instant::now(),
            last_activity: Instant::now(),
            accept_language: None,
            remote_ip: None,
            timezone: None,
            subscriptions: None,
            joined_channels: HashSet::new(),
            negotiated_protocol: moltis_protocol::PROTOCOL_VERSION,
        };
        (client, rx)
    }

    #[tokio::test]
    async fn disconnect_all_clients_drains_state_and_notifies() {
        let state = test_state();

        let (c1, mut rx1) = mock_client("conn-1");
        let (c2, mut rx2) = mock_client("conn-2");
        state.register_client(c1).await;
        state.register_client(c2).await;

        // Set up some active_sessions / active_projects entries.
        {
            let mut inner = state.inner.write().await;
            inner
                .active_sessions
                .insert("conn-1".into(), "session-a".into());
            inner
                .active_projects
                .insert("conn-2".into(), "project-b".into());
        }

        assert_eq!(state.client_count().await, 2);

        state.disconnect_all_clients("test_reason").await;

        // All clients are removed.
        assert_eq!(state.client_count().await, 0);

        // active_sessions and active_projects are cleared.
        {
            let inner = state.inner.read().await;
            assert!(inner.active_sessions.is_empty());
            assert!(inner.active_projects.is_empty());
        }

        // Both receivers got the event frame before the channel closed.
        let msg1 = rx1.recv().await.expect("should receive event");
        let msg2 = rx2.recv().await.expect("should receive event");

        let frame1: serde_json::Value = serde_json::from_str(&msg1).unwrap();
        assert_eq!(frame1["event"], "auth.credentials_changed");
        assert_eq!(frame1["payload"]["reason"], "test_reason");

        let frame2: serde_json::Value = serde_json::from_str(&msg2).unwrap();
        assert_eq!(frame2["event"], "auth.credentials_changed");

        // Channels are closed (all senders dropped).
        assert!(rx1.recv().await.is_none());
        assert!(rx2.recv().await.is_none());
    }

    #[tokio::test]
    async fn disconnect_all_clients_is_noop_when_empty() {
        let state = test_state();
        assert_eq!(state.client_count().await, 0);
        // Should not panic.
        state.disconnect_all_clients("noop").await;
        assert_eq!(state.client_count().await, 0);
    }

    // ── Subscription tests ──────────────────────────────────────────────

    #[test]
    fn is_subscribed_to_none_is_wildcard() {
        let (mut client, _rx) = mock_client("c1");
        client.subscriptions = None;
        assert!(client.is_subscribed_to("chat"));
        assert!(client.is_subscribed_to("presence"));
        assert!(client.is_subscribed_to("anything"));
    }

    #[test]
    fn is_subscribed_to_empty_set_blocks_all() {
        let (mut client, _rx) = mock_client("c1");
        client.subscriptions = Some(HashSet::new());
        assert!(!client.is_subscribed_to("chat"));
        assert!(!client.is_subscribed_to("presence"));
    }

    #[test]
    fn is_subscribed_to_specific_events() {
        let (mut client, _rx) = mock_client("c1");
        let mut subs = HashSet::new();
        subs.insert("chat".to_string());
        subs.insert("presence".to_string());
        client.subscriptions = Some(subs);
        assert!(client.is_subscribed_to("chat"));
        assert!(client.is_subscribed_to("presence"));
        assert!(!client.is_subscribed_to("tick"));
    }

    #[test]
    fn is_subscribed_to_wildcard_in_set() {
        let (mut client, _rx) = mock_client("c1");
        let mut subs = HashSet::new();
        subs.insert("*".to_string());
        client.subscriptions = Some(subs);
        assert!(client.is_subscribed_to("chat"));
        assert!(client.is_subscribed_to("anything"));
    }

    // ── Channel tests ───────────────────────────────────────────────────

    #[test]
    fn is_in_channel_empty() {
        let (client, _rx) = mock_client("c1");
        assert!(!client.is_in_channel("session:abc"));
    }

    #[test]
    fn is_in_channel_after_join() {
        let (mut client, _rx) = mock_client("c1");
        client.joined_channels.insert("session:abc".to_string());
        assert!(client.is_in_channel("session:abc"));
        assert!(!client.is_in_channel("session:xyz"));
    }

    // ── Broadcast subscription filtering ────────────────────────────────

    #[tokio::test]
    async fn broadcast_skips_unsubscribed_clients() {
        let state = test_state();

        // Client 1: subscribed to "chat" only
        let (mut c1, mut rx1) = mock_client("conn-sub");
        c1.subscriptions = Some(["chat".to_string()].into());
        state.register_client(c1).await;

        // Client 2: wildcard (None)
        let (c2, mut rx2) = mock_client("conn-wild");
        state.register_client(c2).await;

        // Broadcast a "presence" event
        crate::broadcast::broadcast(
            &state,
            "presence",
            serde_json::json!({"type": "test"}),
            crate::broadcast::BroadcastOpts::default(),
        )
        .await;

        // Client 1 should NOT receive it (not subscribed to presence)
        assert!(rx1.try_recv().is_err());

        // Client 2 should receive it (wildcard)
        let msg = rx2.try_recv().expect("wildcard should receive");
        let frame: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(frame["event"], "presence");
    }

    #[tokio::test]
    async fn broadcast_channel_filter_skips_non_members() {
        let state = test_state();

        // Client 1: in channel "session:abc"
        let (mut c1, mut rx1) = mock_client("conn-in");
        c1.joined_channels.insert("session:abc".to_string());
        state.register_client(c1).await;

        // Client 2: not in channel
        let (c2, mut rx2) = mock_client("conn-out");
        state.register_client(c2).await;

        // Broadcast scoped to channel
        crate::broadcast::broadcast(
            &state,
            "chat",
            serde_json::json!({"text": "hello"}),
            crate::broadcast::BroadcastOpts {
                channel: Some("session:abc".into()),
                ..Default::default()
            },
        )
        .await;

        // Client 1 should receive it
        let msg = rx1.try_recv().expect("channel member should receive");
        let frame: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(frame["event"], "chat");
        assert_eq!(frame["channel"], "session:abc");

        // Client 2 should NOT receive it
        assert!(rx2.try_recv().is_err());
    }
}
