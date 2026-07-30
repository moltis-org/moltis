//! Real Moltis turns behind the ACP backend seam.
//!
//! This is the same path the Web UI takes. A prompt becomes `ChatService::send_sync`,
//! which runs the agent loop — providers, tools, memory, session history — and
//! resolves with the final assistant message. The tokens the Web UI renders as
//! they arrive are broadcast as event frames while that call is pending, so to
//! stream them we register a client on the gateway's broadcast registry and
//! forward what it receives.
//!
//! # Why a registered client rather than a bespoke hook
//!
//! `ConnectedClient` is just a bounded `mpsc::Sender<String>` plus subscription
//! metadata, and `broadcast()` fans frames out to every registered client. Using
//! that seam means ACP sees exactly what the Web UI sees, including frames added
//! later, with no parallel notification path in the chat crate to keep in sync.
//!
//! # No server required
//!
//! `prepare_gateway_core` is the transport-agnostic half of startup and binds no
//! socket, so `moltis acp` boots the stack in-process. It does open the databases
//! under `data_dir()`, so it shares session state with a running gateway rather
//! than talking to it.

mod updates;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use {
    agent_client_protocol as acp,
    async_trait::async_trait,
    moltis_acp::{
        AcpBackend, BackendCapabilities, SessionKey, SessionNotFound, SessionSetup, TurnUpdates,
        backend::{MAX_HISTORY_BYTES, MAX_HISTORY_UPDATES},
    },
    moltis_chat::LiveChatService,
    moltis_gateway::state::GatewayState,
    moltis_protocol::{ClientInfo, ConnectParams, PROTOCOL_VERSION},
    moltis_service_traits::ChatService,
    serde_json::{Value, json},
    tokio::sync::{Notify, RwLock, mpsc},
    tracing::{debug, warn},
};

use {
    self::updates::{FrameAction, FrameMapper},
    crate::acp_mcp::SessionMcpRuntime,
};

/// Frames buffered for one turn before the gateway starts dropping them.
///
/// Generous because a fast provider can outrun the forwarder briefly; the
/// gateway drops rather than blocks when a client is slow, so an undersized
/// buffer costs tokens rather than backpressure.
const FRAME_BUFFER: usize = 1024;

/// Serves ACP prompts by running real Moltis turns.
pub struct MoltisBackend {
    core: moltis_gateway::server::PreparedGatewayCore,
    state: Arc<GatewayState>,
    chat: Arc<LiveChatService>,
    sessions: RwLock<HashMap<SessionKey, SessionRuntime>>,
    active_prompts: RwLock<HashMap<SessionKey, Arc<PromptSignal>>>,
    lifecycle: Arc<BackendLifecycle>,
    /// Distinguishes the synthetic clients this backend registers, so two
    /// concurrent turns cannot collide on a connection id.
    next_conn: AtomicU64,
}

struct SessionRuntime {
    mcp: Option<SessionMcpRuntime>,
}

#[derive(Default)]
struct PromptSignal {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Default)]
struct BackendLifecycle {
    closing: AtomicBool,
    active: AtomicUsize,
    idle: Notify,
}

impl BackendLifecycle {
    fn begin(self: &Arc<Self>) -> anyhow::Result<OperationGuard> {
        if self.closing.load(Ordering::Acquire) {
            anyhow::bail!("ACP backend is shutting down");
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.closing.load(Ordering::Acquire) {
            self.finish();
            anyhow::bail!("ACP backend is shutting down");
        }
        Ok(OperationGuard(Arc::clone(self)))
    }

    fn finish(&self) {
        if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_waiters();
        }
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                break;
            }
            notified.await;
        }
    }
}

struct OperationGuard(Arc<BackendLifecycle>);

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.finish();
    }
}

impl MoltisBackend {
    #[must_use]
    pub fn new(core: moltis_gateway::server::PreparedGatewayCore) -> Self {
        let state = Arc::clone(&core.state);
        let chat = Arc::clone(&core.live_chat);
        Self {
            core,
            state,
            chat,
            sessions: RwLock::new(HashMap::new()),
            active_prompts: RwLock::new(HashMap::new()),
            lifecycle: Arc::new(BackendLifecycle::default()),
            next_conn: AtomicU64::new(0),
        }
    }

    async fn canonical_cwd(setup: &SessionSetup) -> anyhow::Result<PathBuf> {
        let cwd = tokio::fs::canonicalize(setup.cwd())
            .await
            .map_err(|error| anyhow::anyhow!("invalid session cwd: {error}"))?;
        if !cwd.is_dir() {
            anyhow::bail!("session cwd is not a directory");
        }
        Ok(cwd)
    }

    async fn bind_project(&self, key: &SessionKey, cwd: &Path) -> anyhow::Result<()> {
        let listed = self.state.services.project.list().await?;
        let projects = serde_json::from_value::<Vec<moltis_projects::Project>>(listed)?;
        let existing = projects.iter().find(|project| {
            std::fs::canonicalize(&project.directory).is_ok_and(|directory| directory == cwd)
        });
        let project_id = if let Some(project) = existing {
            project.id.clone()
        } else {
            let mut project = moltis_projects::detect::detect_project(cwd)
                .ok_or_else(|| anyhow::anyhow!("failed to detect project for {}", cwd.display()))?;
            if projects.iter().any(|existing| existing.id == project.id) {
                project.id = format!("{}-{}", project.id, uuid::Uuid::new_v4().simple());
            }
            self.state
                .services
                .project
                .upsert(serde_json::to_value(&project)?)
                .await?;
            project.id
        };
        self.chat
            .bind_session_project(key.as_str(), &project_id)
            .await;
        Ok(())
    }

    async fn install_session(&self, key: &SessionKey, setup: &SessionSetup) -> anyhow::Result<()> {
        if self.lifecycle.closing.load(Ordering::Acquire) {
            anyhow::bail!("ACP backend is shutting down");
        }
        let cwd = Self::canonical_cwd(setup).await?;
        let mcp = SessionMcpRuntime::start(key, setup).await?;
        if self.lifecycle.closing.load(Ordering::Acquire) {
            if let Some(runtime) = mcp {
                runtime.shutdown().await;
            }
            anyhow::bail!("ACP backend is shutting down");
        }
        if let Err(error) = self.bind_project(key, &cwd).await {
            if let Some(runtime) = mcp {
                runtime.shutdown().await;
            }
            return Err(error);
        }
        if self.lifecycle.closing.load(Ordering::Acquire) {
            if let Some(runtime) = mcp {
                runtime.shutdown().await;
            }
            anyhow::bail!("ACP backend is shutting down");
        }
        if let Some(runtime) = mcp.as_ref() {
            self.chat
                .set_session_tool_overlay(key.as_str(), runtime.tools())
                .await;
        } else {
            self.chat.remove_session_tool_overlay(key.as_str()).await;
        }
        let previous = self
            .sessions
            .write()
            .await
            .insert(key.clone(), SessionRuntime { mcp });
        if let Some(previous) = previous
            && let Some(mcp) = previous.mcp
        {
            mcp.shutdown().await;
        }
        Ok(())
    }

    /// Registers a broadcast client and returns its id plus the frame stream.
    ///
    /// The caller must pair this with [`Self::unregister`]; [`TurnClient`] does
    /// that on drop so an early return cannot leak a registration.
    async fn register(&self, key: &SessionKey) -> (String, mpsc::Receiver<String>, Arc<AtomicU64>) {
        let seq = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let conn_id = format!("acp-{seq}");
        let (tx, rx) = mpsc::channel::<String>(FRAME_BUFFER);
        let delivery_failures = Arc::new(AtomicU64::new(0));
        let client = moltis_gateway::state::ConnectedClient {
            conn_id: conn_id.clone(),
            connect_params: acp_connect_params(),
            sender: tx,
            delivery_failures: Arc::clone(&delivery_failures),
            connected_at: std::time::Instant::now(),
            last_activity_ms: AtomicU64::new(0),
            accept_language: None,
            remote_ip: None,
            timezone: None,
            subscriptions: Some(std::collections::HashSet::from(["chat".to_string()])),
            session_filter: Some(key.as_str().to_string()),
            payload_state_filter: Some(std::collections::HashSet::from([
                "delta".to_string(),
                "error".to_string(),
                "iteration".to_string(),
                "thinking_text".to_string(),
                "tool_call_end".to_string(),
                "tool_call_start".to_string(),
            ])),
            joined_channels: std::collections::HashSet::new(),
            negotiated_protocol: PROTOCOL_VERSION,
        };
        self.state.register_client(client).await;
        (conn_id, rx, delivery_failures)
    }
}

fn send_mapped_update(
    updates: &TurnUpdates,
    mapper: &mut FrameMapper,
    update: acp::SessionUpdate,
) -> bool {
    if !updates.send(update.clone()) {
        return false;
    }
    mapper.record_sent(&update);
    true
}

fn reconcile_final_text(
    updates: &TurnUpdates,
    mapper: &mut FrameMapper,
    final_text: &str,
) -> anyhow::Result<()> {
    let Some(update) = mapper.finish_text(final_text).map_err(anyhow::Error::msg)? else {
        return Ok(());
    };
    if !send_mapped_update(updates, mapper, update) {
        anyhow::bail!("ACP client stopped reading final turn output");
    }
    Ok(())
}

/// Connection metadata for the synthetic client backing one ACP turn.
///
/// The ACP client is the local parent process, already trusted, so this claims
/// the operator role rather than inventing a narrower one it would then have to
/// widen every time a chat frame gained a scope guard.
fn acp_connect_params() -> ConnectParams {
    ConnectParams {
        min_protocol: PROTOCOL_VERSION,
        max_protocol: PROTOCOL_VERSION,
        client: ClientInfo {
            id: "moltis-acp".to_string(),
            display_name: Some("ACP client".to_string()),
            version: moltis_config::VERSION.to_string(),
            platform: std::env::consts::OS.to_string(),
            device_family: None,
            model_identifier: None,
            mode: "agent".to_string(),
            instance_id: None,
        },
        caps: None,
        commands: None,
        permissions: None,
        path_env: None,
        role: Some("operator".to_string()),
        scopes: None,
        device: None,
        auth: None,
        locale: None,
        user_agent: None,
        timezone: None,
    }
}

/// Keeps a registered broadcast client alive for the duration of a turn.
///
/// Unregistering matters: a leaked client keeps a channel in the gateway's
/// registry forever, and `broadcast()` walks every registered client on every
/// frame.
struct TurnClient {
    state: Arc<GatewayState>,
    conn_id: String,
}

impl Drop for TurnClient {
    fn drop(&mut self) {
        if self.conn_id.is_empty() {
            return;
        }
        let state = Arc::clone(&self.state);
        let conn_id = std::mem::take(&mut self.conn_id);
        // Drop runs outside async context, and removal takes a write lock.
        tokio::spawn(async move {
            state.remove_client(&conn_id).await;
        });
    }
}

impl TurnClient {
    async fn close(mut self) {
        let conn_id = std::mem::take(&mut self.conn_id);
        self.state.remove_client(&conn_id).await;
    }
}

#[async_trait]
impl AcpBackend for MoltisBackend {
    async fn create_session(&self, setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        let _operation = self.lifecycle.begin()?;
        // Moltis materializes a session on first write, so there is nothing to
        // create here beyond choosing the key. The `acp:` namespace is what
        // keeps these from colliding with Web UI and channel sessions, and the
        // protocol layer rejects anything outside it.
        let key = SessionKey::namespaced(uuid::Uuid::new_v4().to_string());
        self.install_session(&key, setup).await?;
        debug!(session = %key, "ACP session created");
        Ok(key)
    }

    async fn load_session(
        &self,
        key: &SessionKey,
        setup: &SessionSetup,
    ) -> anyhow::Result<Vec<acp::SessionUpdate>> {
        let _operation = self.lifecycle.begin()?;
        if !self.chat.session_exists(key.as_str()).await {
            return Err(SessionNotFound.into());
        }
        let history = self
            .chat
            .read_session_history_bounded(key.as_str(), MAX_HISTORY_UPDATES, MAX_HISTORY_BYTES)
            .await
            .map_err(|error| anyhow::anyhow!("failed to read session history: {error}"))?;
        let updates = history_to_updates(&history);
        moltis_acp::validate_history(&updates)?;
        self.install_session(key, setup).await?;
        Ok(updates)
    }

    async fn discard_session(&self, key: &SessionKey) -> anyhow::Result<()> {
        let runtime = self.sessions.write().await.remove(key);
        self.chat.remove_session_tool_overlay(key.as_str()).await;
        if let Some(SessionRuntime { mcp: Some(mcp), .. }) = runtime {
            mcp.shutdown().await;
        }
        Ok(())
    }

    async fn prompt(
        &self,
        key: &SessionKey,
        prompt: String,
        updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        let _operation = self.lifecycle.begin()?;
        let signal = Arc::new(PromptSignal::default());
        let mut active_prompts = self.active_prompts.write().await;
        if active_prompts.contains_key(key) {
            anyhow::bail!("session {key} already has an active prompt");
        }
        active_prompts.insert(key.clone(), Arc::clone(&signal));
        drop(active_prompts);
        if self.lifecycle.closing.load(Ordering::Acquire) {
            signal.cancelled.store(true, Ordering::Release);
            signal.notify.notify_one();
        }

        let result = async {
            if signal.cancelled.load(Ordering::Acquire) {
                return Ok(acp::StopReason::Cancelled);
            }
            let (conn_id, mut frames, delivery_failures) = self.register(key).await;
            let client = TurnClient {
                state: Arc::clone(&self.state),
                conn_id,
            };

            let turn = self.chat.send_sync(json!({
                "text": prompt,
                "_session_key": key.as_str(),
                "_history_limits": {
                    "max_messages": MAX_HISTORY_UPDATES,
                    "max_bytes": MAX_HISTORY_BYTES,
                },
            }));
            let mut turn = std::pin::pin!(turn);

            let mut mapper = FrameMapper::new();
            let mut reported_error: Option<String> = None;
            let mut cancelled = false;
            let mut delivery_failed = false;

            // Forward frames while the turn runs. `send_sync` resolving is what ends
            // the turn — the broadcast has no terminal frame to wait for, and
            // waiting for the channel to close would hang, since the gateway holds
            // the sender until the client is unregistered.
            let outcome = 'turn: loop {
                tokio::select! {
                    biased;
                    _ = signal.notify.notified() => {
                        let _ = self
                            .chat
                            .abort(json!({ "sessionKey": key.as_str() }))
                            .await;
                        cancelled = true;
                        break Ok(json!({}));
                    },
                    result = &mut turn => break result,
                    frame = frames.recv() => match frame {
                        Some(frame) => match mapper.map(&frame, key.as_str()) {
                            FrameAction::Emit(batch) => {
                                for update in batch {
                                    if !send_mapped_update(&updates, &mut mapper, update) {
                                        debug!("ACP client stopped reading updates mid-turn");
                                        let _ = self
                                            .chat
                                            .abort(json!({ "sessionKey": key.as_str() }))
                                            .await;
                                        delivery_failed = true;
                                        break 'turn Ok(json!({}));
                                    }
                                }
                            },
                            FrameAction::Failed(message) => reported_error = Some(message),
                            FrameAction::Ignore => {},
                        },
                        // The gateway dropped our registration; the turn still owns
                        // the outcome, so wait for it rather than guessing.
                        None => break (&mut turn).await,
                    },
                }
            };

            // Frames already queued when the turn resolved are still this turn's
            // output; dropping them would truncate the visible reply.
            'flush: while !cancelled
                && !delivery_failed
                && let Ok(frame) = frames.try_recv()
            {
                match mapper.map(&frame, key.as_str()) {
                    FrameAction::Emit(batch) => {
                        for update in batch {
                            if !send_mapped_update(&updates, &mut mapper, update) {
                                delivery_failed = true;
                                break 'flush;
                            }
                        }
                    },
                    FrameAction::Failed(message) => reported_error = Some(message),
                    FrameAction::Ignore => {},
                }
            }

            client.close().await;

            if delivery_failures.load(Ordering::Relaxed) > 0 {
                delivery_failed = true;
            }

            if cancelled {
                Ok(acp::StopReason::Cancelled)
            } else if delivery_failed {
                Err(anyhow::anyhow!(
                    "ACP client could not accept all final turn updates"
                ))
            } else {
                match outcome {
                    Ok(result) if result.get("rejected").and_then(Value::as_bool) == Some(true) => {
                        let reason = result
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("prompt rejected");
                        Err(anyhow::anyhow!(reason.to_string()))
                    },
                    Ok(result) => {
                        if let Some(final_text) = result.get("text").and_then(Value::as_str) {
                            reconcile_final_text(&updates, &mut mapper, final_text)?;
                        }
                        Ok(acp::StopReason::EndTurn)
                    },
                    Err(error) => {
                        // Prefer the broadcast's message: `send_sync` reports a generic
                        // failure while the frame carries the provider's own words.
                        let detail = reported_error.unwrap_or_else(|| error.to_string());
                        warn!(session = %key, "ACP turn failed");
                        Err(anyhow::anyhow!(detail))
                    },
                }
            }
        }
        .await;

        let mut active_prompts = self.active_prompts.write().await;
        if active_prompts
            .get(key)
            .is_some_and(|active| Arc::ptr_eq(active, &signal))
        {
            active_prompts.remove(key);
        }
        result
    }

    async fn cancel(&self, key: &SessionKey) -> anyhow::Result<()> {
        let signal = self.active_prompts.read().await.get(key).cloned();
        if let Some(signal) = signal.as_ref() {
            signal.cancelled.store(true, Ordering::Release);
            signal.notify.notify_one();
        }
        let result = self
            .chat
            .abort(json!({ "sessionKey": key.as_str() }))
            .await
            .map_err(|error| anyhow::anyhow!("failed to abort turn: {error}"))?;
        if result.get("aborted").and_then(Value::as_bool) != Some(true) && signal.is_none() {
            anyhow::bail!("no active turn found for session {key}");
        }
        Ok(())
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        debug!("shutting down ACP backend");
        self.lifecycle.closing.store(true, Ordering::Release);
        for signal in self.active_prompts.read().await.values() {
            signal.cancelled.store(true, Ordering::Release);
            signal.notify.notify_one();
        }
        if tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.lifecycle.wait_idle(),
        )
        .await
        .is_err()
        {
            warn!("timed out waiting for ACP prompts to stop during shutdown");
        }
        let sessions = {
            let mut sessions = self.sessions.write().await;
            std::mem::take(&mut *sessions)
        };
        for (key, runtime) in sessions {
            let _ = self.chat.abort(json!({ "sessionKey": key.as_str() })).await;
            self.chat.remove_session_tool_overlay(key.as_str()).await;
            if let Some(mcp) = runtime.mcp {
                mcp.shutdown().await;
            }
            debug!(session = %key, "ACP session shut down");
        }
        self.core.mcp_manager.shutdown_all().await;
        debug!("ACP backend shut down");
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities { load_session: true }
    }
}

/// Converts persisted session history into replayable ACP updates.
///
/// `chat.history` returns the same message shapes the Web UI renders. Only user
/// and assistant text carries over: system entries and tool bookkeeping have no
/// ACP representation, and replaying them as messages would put words in the
/// agent's mouth.
fn history_to_updates(messages: &[Value]) -> Vec<acp::SessionUpdate> {
    messages
        .iter()
        .filter_map(|message| {
            let text = message_text(message)?;
            if text.trim().is_empty() {
                return None;
            }
            match message.get("role").and_then(Value::as_str)? {
                "user" => Some(acp::SessionUpdate::UserMessageChunk(
                    acp::ContentChunk::new(acp::ContentBlock::from(text)),
                )),
                "assistant" => Some(acp::SessionUpdate::AgentMessageChunk(
                    acp::ContentChunk::new(acp::ContentBlock::from(text)),
                )),
                _ => None,
            }
        })
        .collect()
}

/// Pulls displayable text out of a persisted message.
///
/// Content is either a bare string or an array of typed blocks, depending on
/// whether the message carried attachments.
fn message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let blocks = content.as_array()?;
    let text = blocks
        .iter()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn history_replays_only_user_and_assistant_text() {
        let history = vec![
            json!({ "role": "user", "content": "hello" }),
            json!({ "role": "system", "content": "[error] boom" }),
            json!({ "role": "assistant", "content": "hi there" }),
        ];
        let updates = history_to_updates(&history);
        assert_eq!(updates.len(), 2, "the system entry must not be replayed");
        assert!(matches!(
            updates[0],
            acp::SessionUpdate::UserMessageChunk(_)
        ));
        assert!(matches!(
            updates[1],
            acp::SessionUpdate::AgentMessageChunk(_)
        ));
    }

    #[test]
    fn history_reads_block_structured_content() {
        let history = vec![json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "part one " },
                { "type": "text", "text": "part two" },
            ],
        })];
        let updates = history_to_updates(&history);
        assert_eq!(updates.len(), 1);
        let acp::SessionUpdate::AgentMessageChunk(chunk) = &updates[0] else {
            panic!("expected an agent message");
        };
        let acp::ContentBlock::Text(text) = &chunk.content else {
            panic!("expected text content");
        };
        assert_eq!(text.text, "part one part two");
    }

    #[test]
    fn empty_and_malformed_history_is_not_replayed() {
        assert!(history_to_updates(&[]).is_empty());
        // An assistant turn persisted with no text would otherwise replay as an
        // empty message.
        assert!(history_to_updates(&[json!({ "role": "assistant", "content": "  " })]).is_empty());
        assert!(history_to_updates(&[json!({ "role": "assistant" })]).is_empty());
    }

    #[test]
    fn created_sessions_are_inside_the_acp_namespace() {
        // The protocol layer rejects out-of-namespace keys, so a backend that
        // minted one would fail every `session/new`.
        let key = SessionKey::namespaced(uuid::Uuid::new_v4().to_string());
        assert!(key.is_namespaced());
    }

    #[test]
    fn created_sessions_are_unique() {
        let first = SessionKey::namespaced(uuid::Uuid::new_v4().to_string());
        let second = SessionKey::namespaced(uuid::Uuid::new_v4().to_string());
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn lifecycle_rejects_new_work_and_waits_for_active_work() {
        let lifecycle = Arc::new(BackendLifecycle::default());
        let operation = lifecycle.begin().expect("operation before shutdown");
        lifecycle.closing.store(true, Ordering::Release);
        assert!(lifecycle.begin().is_err());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), lifecycle.wait_idle())
                .await
                .is_err()
        );
        drop(operation);
        tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle.wait_idle())
            .await
            .expect("active operation should release lifecycle");
    }

    #[test]
    fn closed_update_channels_are_not_counted_as_delivered() {
        let (tx, rx) = mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        let mut mapper = FrameMapper::new();
        let first = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::from("first"),
        ));
        let second = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::from("second"),
        ));
        drop(rx);
        assert!(!send_mapped_update(&updates, &mut mapper, first));
        assert!(!send_mapped_update(&updates, &mut mapper, second));
        assert!(
            mapper
                .finish_text("firstsecond")
                .is_ok_and(|value| value.is_some())
        );
    }

    #[test]
    fn divergent_final_text_fails_the_turn() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        let mut mapper = FrameMapper::new();
        let streamed = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::from("draft"),
        ));
        assert!(send_mapped_update(&updates, &mut mapper, streamed));
        let error = reconcile_final_text(&updates, &mut mapper, "corrected").unwrap_err();
        assert!(error.to_string().contains("diverged"));
    }

    #[test]
    fn missing_final_suffix_is_delivered() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        let mut mapper = FrameMapper::new();
        let streamed = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp::ContentBlock::from("partial"),
        ));
        assert!(send_mapped_update(&updates, &mut mapper, streamed));
        assert!(reconcile_final_text(&updates, &mut mapper, "partial response").is_ok());
        assert!(rx.try_recv().is_ok());
        let update = rx.try_recv().expect("missing final suffix update");
        let acp::SessionUpdate::AgentMessageChunk(chunk) = update else {
            panic!("expected agent message");
        };
        let acp::ContentBlock::Text(text) = chunk.content else {
            panic!("expected text content");
        };
        assert_eq!(text.text, " response");
    }
}
