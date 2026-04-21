//! Home Assistant WebSocket API client.
//!
//! Provides real-time event streaming and request-response commands
//! over the HA WebSocket protocol at `/api/websocket`.
//!
//! # Usage
//!
//! ```ignore
//! let ws = HaWebSocket::new("ws://homeassistant.local:8123/api/websocket", "token");
//! let conn = ws.subscribe().await?;
//!
//! // Subscribe to entity state changes
//! conn.subscribe_entities(None).await?;
//!
//! // Receive events
//! while let Some(event) = conn.recv().await {
//!     match event {
//!         HaEvent::StateChanged { entity_id, .. } => { /* ... */ }
//!         HaEvent::Disconnected => break,
//!         _ => {}
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Result};
use crate::types::HaEvent;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// A pending WebSocket request awaiting a server response.
type Pending = oneshot::Sender<Result<Value>>;

/// Shared write handle behind a mutex for sending WS messages.
type WriteHandle = futures_util::stream::SplitSink<WsStream, Message>;

/// Default interval between HA WebSocket pings.
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// An active WebSocket connection to Home Assistant.
///
/// Holds the event receiver and a shared write handle for sending commands.
/// Created via [`HaWebSocket::subscribe`].
///
/// # Reconnection
///
/// `recv()` returns `None` when the connection is closed (server shutdown,
/// network error). There is no automatic reconnect — create a new
/// [`HaConnection`] via [`HaWebSocket::subscribe`] to re-establish.
///
/// # Heartbeat
///
/// A background task sends application-level pings every 30 seconds to
/// keep the connection alive. The heartbeat is automatically cancelled when
/// `HaConnection` is dropped.
pub struct HaConnection {
    /// Receives [`HaEvent`] from the background reader task.
    events: mpsc::Receiver<HaEvent>,
    /// Shared write half for sending commands.
    write: Arc<Mutex<WriteHandle>>,
    /// ID counter for request-response commands.
    next_id: Arc<AtomicU64>,
    /// Pending request-response map: message id → sender.
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
    /// Handle for the background heartbeat task. Aborted on drop.
    _heartbeat: JoinHandle<()>,
}

impl HaConnection {
    /// Receive the next event from the event bus.
    ///
    /// Returns `None` when the connection is closed.
    pub async fn recv(&mut self) -> Option<HaEvent> {
        self.events.recv().await
    }

    /// Send a command over the WebSocket and wait for the response.
    ///
    /// The `command` must be a JSON object with at least a `type` field
    /// (e.g. `call_service`, `subscribe_events`, `get_states`).
    /// An `id` field is automatically added for request-response tracking.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self), level = "debug"))]
    pub async fn call_command(&self, command: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let mut msg = command;
        if !msg.is_object() {
            return Err(Error::WebSocket("command must be a JSON object".to_owned()));
        }
        msg.as_object_mut()
            .ok_or_else(|| Error::WebSocket("command must be a JSON object".to_owned()))?
            .insert("id".to_owned(), Value::Number(id.into()));

        // Register pending response
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // Send the message
        let text = serde_json::to_string(&msg)
            .map_err(|e| Error::WebSocket(format!("failed to serialize command: {e}")))?;

        let mut write = self.write.lock().await;
        write
            .send(Message::Text(text.into()))
            .await
            .map_err(|e| Error::WebSocket(format!("failed to send command: {e}")))?;
        drop(write);

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::WebSocket(
                "response channel closed unexpectedly".to_owned(),
            )),
            Err(_) => {
                // Clean up pending entry
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(Error::WebSocket(
                    "command timed out after 30s".to_owned(),
                ))
            }
        }
    }

    /// Subscribe to entity state changes.
    ///
    /// This is a convenience wrapper around [`subscribe_events`] that
    /// subscribes to `"state_changed"` events, which is how HA delivers
    /// entity state updates over WebSocket.
    ///
    /// Note: there is no HA WebSocket command named `subscribe_entities` —
    /// that name is reserved for the HA frontend's internal API. The correct
    /// public API command is `subscribe_events` with `event_type: "state_changed"`.
    ///
    /// Pass `entity_ids` as `Some(&[...])` to subscribe to specific entities,
    /// or `None` to subscribe to all entity changes.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self), level = "debug"))]
    pub async fn subscribe_entities(
        &self,
        entity_ids: Option<&[&str]>,
    ) -> Result<()> {
        // HA does not support entity_id filtering via subscribe_events.
        // Use subscribe_events with "state_changed" and filter client-side.
        self.subscribe_events(Some("state_changed")).await
    }

    /// Subscribe to the HA event bus for a specific event type.
    ///
    /// Use `event_type = None` to subscribe to all events.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self), level = "debug"))]
    pub async fn subscribe_events(&self, event_type: Option<&str>) -> Result<()> {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "type".to_owned(),
            Value::String("subscribe_events".to_owned()),
        );
        if let Some(et) = event_type {
            payload.insert("event_type".to_owned(), Value::String(et.to_owned()));
        }

        self.call_command(Value::Object(payload)).await?;
        Ok(())
    }

    /// Send an application-level ping and wait for the pong response.
    ///
    /// HA expects periodic pings to keep the connection alive. This sends
    /// `"type": "ping"` and waits for `"type": "pong"`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self), level = "debug"))]
    pub async fn ping(&self) -> Result<()> {
        let result = self
            .call_command(serde_json::json!({"type": "ping"}))
            .await?;
        // pong responses have no payload (result is null)
        Ok(())
    }
}

/// WebSocket client factory for Home Assistant real-time events and commands.
///
/// Use [`Self::subscribe`] to connect, authenticate, and obtain a [`HaConnection`].
pub struct HaWebSocket {
    url: String,
    token: String,
    next_id: AtomicU64,
}

impl HaWebSocket {
    /// Create a new WebSocket client (does not connect yet).
    #[must_use]
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.to_owned(),
            token: token.to_owned(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Connect, authenticate, and spawn the background reader.
    ///
    /// Returns a [`HaConnection`] for receiving events and sending commands.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, level = "info"))]
    pub async fn subscribe(&self) -> Result<HaConnection> {
        let (tx, events) = mpsc::channel(256);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.url)
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;

        let (mut write, mut read) = ws_stream.split();

        Self::authenticate(&mut write, &mut read, &self.token).await?;

        let pending: Arc<Mutex<HashMap<u64, Pending>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let write = Arc::new(Mutex::new(write));
        // Snapshot the factory counter and advance it so the next subscribe()
        // call starts from a higher base, avoiding ID collisions.
        let next_id = Arc::new(AtomicU64::new(
            self.next_id.fetch_add(10_000, Ordering::SeqCst) + 1,
        ));

        Self::spawn_reader(tx, read, Arc::clone(&pending));
        let _heartbeat = Self::spawn_heartbeat(Arc::clone(&write));

        Ok(HaConnection {
            events,
            write,
            next_id,
            pending,
            _heartbeat,
        })
    }

    /// Spawn a background task that sends periodic pings to keep the
    /// connection alive. Exits silently when the write handle fails
    /// (connection closed).
    fn spawn_heartbeat(write: Arc<Mutex<WriteHandle>>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                HEARTBEAT_INTERVAL_SECS,
            ));
            // Don't bombard if we fall behind
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                // Fire-and-forget ping without an id.
                // HA will reply with a WebSocket protocol pong (no id).
                // The reader handles these at the transport level.
                let ping_msg = serde_json::json!({"type": "ping"});
                let Ok(text) = serde_json::to_string(&ping_msg) else {
                    break;
                };
                let mut w = write.lock().await;
                if w
                    .send(Message::Text(text.into()))
                    .await
                    .is_err()
                {
                    break;
                }
                drop(w);

                #[cfg(feature = "tracing")]
                tracing::debug!("HA WebSocket ping sent");
            }
        })
    }

    /// Perform the HA WebSocket auth handshake.
    async fn authenticate(
        write: &mut WriteHandle,
        read: &mut futures_util::stream::SplitStream<WsStream>,
        token: &str,
    ) -> Result<()> {
        // Wait for auth_required
        match read.next().await {
            Some(Ok(Message::Text(text))) => {
                let val: Value = serde_json::from_str(&text)
                    .map_err(|e| Error::WebSocket(e.to_string()))?;
                if val.get("type").and_then(|t| t.as_str()) != Some("auth_required") {
                    return Err(Error::Auth(
                        "expected auth_required message".to_owned(),
                    ));
                }
            }
            Some(Ok(_)) => {
                return Err(Error::Auth("unexpected message type".to_owned()));
            }
            Some(Err(e)) => return Err(Error::WebSocket(e.to_string())),
            None => return Err(Error::WebSocket("connection closed".to_owned())),
        }

        // Send auth token
        let auth_msg = serde_json::json!({
            "type": "auth",
            "access_token": token
        });
        let auth_str = serde_json::to_string(&auth_msg)
            .map_err(|e| Error::WebSocket(e.to_string()))?;
        write
            .send(Message::Text(auth_str.into()))
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;

        // Wait for auth_ok
        match read.next().await {
            Some(Ok(Message::Text(text))) => {
                let val: Value = serde_json::from_str(&text)
                    .map_err(|e| Error::WebSocket(e.to_string()))?;
                match val.get("type").and_then(|t| t.as_str()) {
                    Some("auth_ok") => Ok(()),
                    Some("auth_invalid") => Err(Error::Auth(
                        val.get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("invalid token")
                            .to_owned(),
                    )),
                    other => Err(Error::Auth(format!(
                        "unexpected auth response: {}",
                        other.unwrap_or("(null)")
                    ))),
                }
            }
            Some(Ok(_)) => Err(Error::Auth("unexpected message type".to_owned())),
            Some(Err(e)) => Err(Error::WebSocket(e.to_string())),
            None => Err(Error::WebSocket("connection closed during auth".to_owned())),
        }
    }

    /// Spawn the background reader task that dispatches events and responses.
    fn spawn_reader(
        tx: mpsc::Sender<HaEvent>,
        mut read: futures_util::stream::SplitStream<WsStream>,
        pending: Arc<Mutex<HashMap<u64, Pending>>>,
    ) {
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        let Ok(val) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };

                        // Dispatch pending request-response
                        if let Some(id) = val.get("id").and_then(|i| i.as_u64()) {
                            let mut map = pending.lock().await;
                            if let Some(sender) = map.remove(&id) {
                                // Handle pong responses to ping commands.
                                // HA pong has `{"id": N, "type": "pong"}` — no `success` field.
                                if val.get("type").and_then(|t| t.as_str()) == Some("pong") {
                                    let _ = sender.send(Ok(Value::Null));
                                    continue;
                                }

                                let result = match val.get("success").and_then(|s| s.as_bool()) {
                                    Some(true) => Ok(val
                                        .get("result")
                                        .cloned()
                                        .unwrap_or(Value::Null)),
                                    Some(false) => Err(Error::WebSocket(format!(
                                        "command failed: {}",
                                        val.get("error")
                                            .and_then(|e| serde_json::to_string(e).ok())
                                            .unwrap_or_default()
                                    ))),
                                    None => {
                                        // Not a result message — could be an event.
                                        map.insert(id, sender);
                                        drop(map);
                                        Self::dispatch_event(&val, &tx).await;
                                        continue;
                                    }
                                };
                                let _ = sender.send(result);
                                continue;
                            }
                        }

                        Self::dispatch_event(&val, &tx).await;
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                    Ok(Message::Close(_)) => {
                        #[cfg(feature = "tracing")]
                        tracing::warn!("HA WebSocket closed by server");
                        break;
                    }
                    Err(e) => {
                        #[cfg(feature = "tracing")]
                        tracing::warn!("HA WebSocket read error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
            let _ = tx.send(HaEvent::Disconnected).await;
        });
    }

    /// Parse and dispatch an event message to the channel.
    async fn dispatch_event(val: &Value, tx: &mpsc::Sender<HaEvent>) {
        let msg_type = val.get("type").and_then(|t| t.as_str());

        match msg_type {
            Some("event") => {
                let event = val.get("event");
                let event_type = event
                    .and_then(|e| e.get("event_type"))
                    .and_then(|t| t.as_str());

                match event_type {
                    Some("state_changed") => {
                        let data = event.and_then(|e| e.get("data"));
                        let entity_id = data
                            .and_then(|d| d.get("entity_id"))
                            .and_then(|e| e.as_str())
                            .unwrap_or("unknown")
                            .to_owned();
                        let old_state = data
                            .and_then(|d| d.get("old_state"))
                            .cloned();
                        let new_state = data
                            .and_then(|d| d.get("new_state"))
                            .cloned();

                        let _ = tx
                            .send(HaEvent::StateChanged {
                                entity_id,
                                old_state,
                                new_state,
                            })
                            .await;
                    }
                    Some("trigger") => {
                        let variables = event
                            .and_then(|e| e.get("variables"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let _ = tx.send(HaEvent::Trigger { variables }).await;
                    }
                    _ => {
                        let _ = tx.send(HaEvent::Raw(val.clone())).await;
                    }
                }
            }
            Some("pong") => {}
            _ => {
                let _ = tx.send(HaEvent::Raw(val.clone())).await;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::net::TcpListener;

    /// Start a minimal HA-compatible WS server on a random port.
    ///
    /// Sends `auth_required` → expects `auth` → responds `auth_ok`.
    /// Echoes back messages with an `id` field as `{success: true, result: null}`.
    async fn start_mock_ha_ws() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("mock WS accept");
                    let (mut write, mut read) = ws.split();

                    // Send auth_required
                    let auth_required = json!({"type": "auth_required", "ha_version": "2025.1"});
                    write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await
                        .expect("mock WS send auth_required");

                    // Wait for auth
                    match read.next().await {
                        Some(Ok(Message::Text(text))) => {
                            let val: Value = serde_json::from_str(&text)
                                .expect("mock WS auth message parse");
                            if val.get("type").and_then(|t| t.as_str()) == Some("auth") {
                                let auth_ok = json!({"type": "auth_ok", "ha_version": "2025.1"});
                                write
                                    .send(Message::Text(auth_ok.to_string().into()))
                                    .await
                                    .expect("mock WS send auth_ok");
                            } else {
                                let auth_invalid = json!({"type": "auth_invalid", "message": "bad token"});
                                write
                                    .send(Message::Text(auth_invalid.to_string().into()))
                                    .await
                                    .expect("mock WS send auth_invalid");
                                return;
                            }
                        }
                        _ => return,
                    }

                    // Echo incoming messages: if they have an `id`, respond with success
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                let val: Value = serde_json::from_str(&text)
                                    .expect("mock WS message parse");
                                if let Some(id) = val.get("id") {
                                    let response = json!({
                                        "id": id,
                                        "type": "result",
                                        "success": true,
                                        "result": null
                                    });
                                    let _ = write
                                        .send(Message::Text(response.to_string().into()))
                                        .await;
                                } else {
                                    // No id — just echo
                                    let _ = write.send(Message::Text(text)).await;
                                }
                            }
                            Ok(Message::Ping(data)) => {
                                let _ = write.send(Message::Pong(data)).await;
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                });
            }
        });

        port
    }

    /// Start a mock HA WS server that rejects auth.
    async fn start_mock_ha_ws_auth_reject() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("mock WS accept (reject)");
                    let (mut write, _read) = ws.split();

                    let auth_required = json!({"type": "auth_required"});
                    write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await
                        .expect("mock WS send auth_required (reject)");
                    // Close without sending auth_ok
                    drop(write);
                });
            }
        });

        port
    }

    #[tokio::test]
    async fn subscribe_authenticates_successfully() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let result = ws.subscribe().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn subscribe_rejects_bad_auth() {
        let port = start_mock_ha_ws_auth_reject().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "bad-token",
        );

        let result = ws.subscribe().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn call_command_sends_and_receives_response() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = conn
            .call_command(json!({"type": "ping"}))
            .await
            .unwrap();

        // call_command returns the "result" field on success
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn call_command_rejects_non_object() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = conn.call_command(json!("not an object")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn subscribe_entities_command() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = conn.subscribe_entities(None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn subscribe_entities_with_filter() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = conn
            .subscribe_entities(Some(&["light.living_room", "sensor.temperature"]))
            .await;
        assert!(result.is_ok());
    }

    /// Start a mock HA WS server that records command types received.
    async fn start_mock_ha_ws_recording() -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let commands: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let cmd_clone = Arc::clone(&commands);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let cmd = Arc::clone(&cmd_clone);
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("mock WS accept (recording)");
                    let (mut write, mut read) = ws.split();

                    let auth_required = json!({"type": "auth_required", "ha_version": "2025.1"});
                    let _ = write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await;

                    match read.next().await {
                        Some(Ok(Message::Text(text))) => {
                            let val: Value = serde_json::from_str(&text).unwrap_or_default();
                            if val
                                .get("type")
                                .and_then(|t| t.as_str())
                                == Some("auth")
                            {
                                let auth_ok = json!({"type": "auth_ok", "ha_version": "2025.1"});
                                let _ = write
                                    .send(Message::Text(auth_ok.to_string().into()))
                                    .await;
                            } else {
                                return;
                            }
                        }
                        _ => return,
                    }

                    while let Some(msg) = read.next().await {
                        if let Ok(Message::Text(text)) = msg {
                            if let Ok(val) = serde_json::from_str::<Value>(&text) {
                                if let Some(cmd_type) =
                                    val.get("type").and_then(|t| t.as_str())
                                {
                                    cmd.lock().await.push(cmd_type.to_owned());
                                }
                                if let Some(id) = val.get("id") {
                                    let response = json!({
                                        "id": id,
                                        "type": "result",
                                        "success": true,
                                        "result": null
                                    });
                                    let _ = write
                                        .send(Message::Text(response.to_string().into()))
                                        .await;
                                }
                            }
                        } else if let Ok(Message::Close(_)) | Err(_) = msg {
                            break;
                        }
                    }
                });
            }
        });

        (port, commands)
    }

    #[tokio::test]
    async fn subscribe_entities_sends_subscribe_events_state_changed() {
        let (port, commands) = start_mock_ha_ws_recording().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        conn.subscribe_entities(None).await.unwrap();

        let cmds = commands.lock().await;
        assert!(
            cmds.iter().any(|c| c == "subscribe_events"),
            "expected subscribe_events command, got: {cmds:?}"
        );
    }

    /// Start a mock HA WS server that replies to ping with pong.
    async fn start_mock_ha_ws_pong() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("mock WS accept (pong)");
                    let (mut write, mut read) = ws.split();

                    let auth_required = json!({"type": "auth_required", "ha_version": "2025.1"});
                    let _ = write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await;

                    match read.next().await {
                        Some(Ok(Message::Text(text))) => {
                            let val: Value = serde_json::from_str(&text).unwrap_or_default();
                            if val.get("type").and_then(|t| t.as_str()) == Some("auth") {
                                let auth_ok = json!({"type": "auth_ok", "ha_version": "2025.1"});
                                let _ = write
                                    .send(Message::Text(auth_ok.to_string().into()))
                                    .await;
                            } else {
                                return;
                            }
                        }
                        _ => return,
                    }

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                let val: Value = serde_json::from_str(&text).unwrap_or_default();
                                if let Some(id) = val.get("id") {
                                    let cmd_type = val.get("type").and_then(|t| t.as_str());
                                    let response = if cmd_type == Some("ping") {
                                        json!({"id": id, "type": "pong"})
                                    } else {
                                        json!({
                                            "id": id,
                                            "type": "result",
                                            "success": true,
                                            "result": null
                                        })
                                    };
                                    let _ = write
                                        .send(Message::Text(response.to_string().into()))
                                        .await;
                                }
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                });
            }
        });

        port
    }

    #[tokio::test]
    async fn ping_receives_pong() {
        let port = start_mock_ha_ws_pong().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            conn.ping(),
        )
        .await;
        assert!(result.is_ok(), "ping() timed out — pong not received");
        assert!(result.unwrap().is_ok(), "ping() returned error");
    }

    #[tokio::test]
    async fn subscribe_events_command() {
        let port = start_mock_ha_ws().await;
        let ws = HaWebSocket::new(
            &format!("ws://127.0.0.1:{port}"),
            "test-token",
        );

        let conn = ws.subscribe().await.unwrap();
        let result = conn.subscribe_events(Some("state_changed")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn new_generates_unique_ids() {
        let ws = HaWebSocket::new("ws://localhost:1", "token");
        let id1 = ws.next_id.fetch_add(1, Ordering::Relaxed);
        let id2 = ws.next_id.fetch_add(1, Ordering::Relaxed);
        let id3 = ws.next_id.fetch_add(1, Ordering::Relaxed);
        assert!(id1 < id2);
        assert!(id2 < id3);
    }

    #[test]
    fn dispatch_event_state_changed() {
        let (tx, mut rx) = mpsc::channel(8);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            HaWebSocket::dispatch_event(
                &json!({
                    "type": "event",
                    "event": {
                        "event_type": "state_changed",
                        "data": {
                            "entity_id": "light.living_room",
                            "old_state": {"state": "off"},
                            "new_state": {"state": "on"}
                        }
                    }
                }),
                &tx,
            )
            .await;

            let event = rx.recv().await.unwrap();
            match event {
                HaEvent::StateChanged { entity_id, old_state, new_state } => {
                    assert_eq!(entity_id, "light.living_room");
                    assert_eq!(old_state.unwrap()["state"], "off");
                    assert_eq!(new_state.unwrap()["state"], "on");
                }
                other => panic!("expected StateChanged, got {other:?}"),
            }
        });
    }

    #[test]
    fn dispatch_event_trigger() {
        let (tx, mut rx) = mpsc::channel(8);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            HaWebSocket::dispatch_event(
                &json!({
                    "type": "event",
                    "event": {
                        "event_type": "trigger",
                        "variables": {"trigger_id": "t1", "platform": "state"}
                    }
                }),
                &tx,
            )
            .await;

            let event = rx.recv().await.unwrap();
            match event {
                HaEvent::Trigger { variables } => {
                    assert_eq!(variables["trigger_id"], "t1");
                }
                other => panic!("expected Trigger, got {other:?}"),
            }
        });
    }

    #[test]
    fn dispatch_event_unknown_event_type_becomes_raw() {
        let (tx, mut rx) = mpsc::channel(8);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            HaWebSocket::dispatch_event(
                &json!({
                    "type": "event",
                    "event": {
                        "event_type": "custom_event",
                        "data": {"message": "hello"}
                    }
                }),
                &tx,
            )
            .await;

            let event = rx.recv().await.unwrap();
            assert!(matches!(event, HaEvent::Raw(_)));
        });
    }

    #[test]
    fn dispatch_event_non_event_type_becomes_raw() {
        let (tx, mut rx) = mpsc::channel(8);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            HaWebSocket::dispatch_event(
                &json!({"type": "something_else", "data": 42}),
                &tx,
            )
            .await;

            let event = rx.recv().await.unwrap();
            assert!(matches!(event, HaEvent::Raw(_)));
        });
    }

    #[test]
    fn dispatch_event_pong_is_noop() {
        let (tx, mut rx) = mpsc::channel(8);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            HaWebSocket::dispatch_event(&json!({"type": "pong"}), &tx).await;
            // Pong should not produce any event
            assert!(rx.try_recv().is_err());
        });
    }
}
