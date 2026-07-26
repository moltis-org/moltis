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
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use {
    agent_client_protocol as acp,
    async_trait::async_trait,
    moltis_acp::{AcpBackend, BackendCapabilities, SessionKey, TurnUpdates},
    moltis_gateway::state::GatewayState,
    moltis_protocol::{ClientInfo, ConnectParams, PROTOCOL_VERSION},
    serde_json::{Value, json},
    tokio::sync::mpsc,
    tracing::{debug, warn},
};

use self::updates::{FrameAction, FrameMapper};

/// Frames buffered for one turn before the gateway starts dropping them.
///
/// Generous because a fast provider can outrun the forwarder briefly; the
/// gateway drops rather than blocks when a client is slow, so an undersized
/// buffer costs tokens rather than backpressure.
const FRAME_BUFFER: usize = 1024;

/// Serves ACP prompts by running real Moltis turns.
pub struct MoltisBackend {
    state: Arc<GatewayState>,
    /// Distinguishes the synthetic clients this backend registers, so two
    /// concurrent turns cannot collide on a connection id.
    next_conn: AtomicU64,
}

impl MoltisBackend {
    #[must_use]
    pub fn new(state: Arc<GatewayState>) -> Self {
        Self {
            state,
            next_conn: AtomicU64::new(0),
        }
    }

    /// Registers a broadcast client and returns its id plus the frame stream.
    ///
    /// The caller must pair this with [`Self::unregister`]; [`TurnClient`] does
    /// that on drop so an early return cannot leak a registration.
    async fn register(&self) -> (String, mpsc::Receiver<String>) {
        let seq = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let conn_id = format!("acp-{seq}");
        let (tx, rx) = mpsc::channel::<String>(FRAME_BUFFER);
        let client = moltis_gateway::state::ConnectedClient {
            conn_id: conn_id.clone(),
            connect_params: acp_connect_params(),
            sender: tx,
            connected_at: std::time::Instant::now(),
            last_activity_ms: AtomicU64::new(0),
            accept_language: None,
            remote_ip: None,
            timezone: None,
            // Wildcard: the mapper filters by session, and subscribing narrowly
            // here would silently drop any chat state added later.
            subscriptions: None,
            joined_channels: std::collections::HashSet::new(),
            negotiated_protocol: PROTOCOL_VERSION,
        };
        self.state.register_client(client).await;
        (conn_id, rx)
    }
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
            version: env!("CARGO_PKG_VERSION").to_string(),
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
        let state = Arc::clone(&self.state);
        let conn_id = std::mem::take(&mut self.conn_id);
        // Drop runs outside async context, and removal takes a write lock.
        tokio::spawn(async move {
            state.remove_client(&conn_id).await;
        });
    }
}

#[async_trait]
impl AcpBackend for MoltisBackend {
    async fn create_session(&self, cwd: &Path) -> anyhow::Result<SessionKey> {
        // Moltis materializes a session on first write, so there is nothing to
        // create here beyond choosing the key. The `acp:` namespace is what
        // keeps these from colliding with Web UI and channel sessions, and the
        // protocol layer rejects anything outside it.
        let key = SessionKey::namespaced(uuid::Uuid::new_v4().to_string());
        debug!(session = %key, cwd = %cwd.display(), "ACP session created");
        Ok(key)
    }

    async fn load_session(&self, key: &SessionKey) -> anyhow::Result<Vec<acp::SessionUpdate>> {
        let history = self
            .state
            .chat()
            .history(json!({ "_session_key": key.as_str() }))
            .await
            .map_err(|error| anyhow::anyhow!("failed to read session history: {error}"))?;
        Ok(history_to_updates(&history))
    }

    async fn prompt(
        &self,
        key: &SessionKey,
        prompt: String,
        updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        let (conn_id, mut frames) = self.register().await;
        let _client = TurnClient {
            state: Arc::clone(&self.state),
            conn_id,
        };

        let chat = self.state.chat();
        let turn = chat.send_sync(json!({
            "text": prompt,
            "_session_key": key.as_str(),
        }));
        let mut turn = std::pin::pin!(turn);

        let mut mapper = FrameMapper::new();
        let mut reported_error: Option<String> = None;

        // Forward frames while the turn runs. `send_sync` resolving is what ends
        // the turn — the broadcast has no terminal frame to wait for, and
        // waiting for the channel to close would hang, since the gateway holds
        // the sender until the client is unregistered.
        let outcome = loop {
            tokio::select! {
                result = &mut turn => break result,
                frame = frames.recv() => match frame {
                    Some(frame) => match mapper.map(&frame, key.as_str()) {
                        FrameAction::Emit(batch) => {
                            for update in batch {
                                if !updates.send(update) {
                                    // The client hung up. Let the turn finish so
                                    // the reply is still persisted, but stop
                                    // formatting updates nobody will read.
                                    debug!("ACP client stopped reading updates mid-turn");
                                    break;
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
        while let Ok(frame) = frames.try_recv() {
            match mapper.map(&frame, key.as_str()) {
                FrameAction::Emit(batch) => {
                    for update in batch {
                        if !updates.send(update) {
                            break;
                        }
                    }
                },
                FrameAction::Failed(message) => reported_error = Some(message),
                FrameAction::Ignore => {},
            }
        }

        match outcome {
            Ok(_) => Ok(acp::StopReason::EndTurn),
            Err(error) => {
                // Prefer the broadcast's message: `send_sync` reports a generic
                // failure while the frame carries the provider's own words.
                let detail = reported_error.unwrap_or_else(|| error.to_string());
                warn!(session = %key, "ACP turn failed: {detail}");
                Err(anyhow::anyhow!(detail))
            },
        }
    }

    async fn cancel(&self, key: &SessionKey) -> anyhow::Result<()> {
        self.state
            .chat()
            .abort(json!({ "sessionKey": key.as_str() }))
            .await
            .map_err(|error| anyhow::anyhow!("failed to abort turn: {error}"))?;
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
fn history_to_updates(history: &Value) -> Vec<acp::SessionUpdate> {
    let Some(messages) = history.as_array() else {
        return Vec::new();
    };
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
        let history = json!([
            { "role": "user", "content": "hello" },
            { "role": "system", "content": "[error] boom" },
            { "role": "assistant", "content": "hi there" },
        ]);
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
        let history = json!([{
            "role": "assistant",
            "content": [
                { "type": "text", "text": "part one " },
                { "type": "text", "text": "part two" },
            ],
        }]);
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
        assert!(history_to_updates(&json!([])).is_empty());
        assert!(history_to_updates(&json!({})).is_empty());
        // An assistant turn persisted with no text would otherwise replay as an
        // empty message.
        assert!(history_to_updates(&json!([{ "role": "assistant", "content": "  " }])).is_empty());
        assert!(history_to_updates(&json!([{ "role": "assistant" }])).is_empty());
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
}
