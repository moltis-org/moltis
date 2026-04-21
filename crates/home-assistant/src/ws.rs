//! Home Assistant WebSocket API client.
//!
//! Provides real-time event streaming and request-response commands
//! over the HA WebSocket protocol at `/api/websocket`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Error, Result};
use crate::types::HaEvent;

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// A pending WebSocket request awaiting a server response.
type Pending = oneshot::Sender<Result<Value>>;

/// WebSocket client for Home Assistant real-time events and commands.
pub struct HaWebSocket {
    url: String,
    token: String,
    next_id: AtomicU64,
    /// Pending request-response map: message id → sender.
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
}

impl HaWebSocket {
    /// Create a new WebSocket client (does not connect yet).
    #[must_use]
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.to_owned(),
            token: token.to_owned(),
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Connect, authenticate, and spawn the background reader.
    ///
    /// Returns a receiver that yields [`HaEvent`] until disconnect.
    /// Use [`Self::call_command`] for request-response commands.
    pub async fn subscribe(&self) -> Result<mpsc::Receiver<HaEvent>> {
        let (tx, rx) = mpsc::channel(256);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.url)
            .await
            .map_err(|e| Error::WebSocket(e.to_string()))?;

        let (mut write, mut read) = ws_stream.split();

        Self::authenticate(&mut write, &mut read, &self.token).await?;
        Self::spawn_reader(tx, read, Arc::clone(&self.pending));

        Ok(rx)
    }

    /// Send a command and wait for the response.
    ///
    /// Must be called after [`Self::subscribe`] to ensure the reader
    /// task is running and can dispatch responses.
    ///
    /// **Not yet implemented** — use the REST client for commands.
    /// This will be completed in Phase 2 when the write handle is
    /// moved into the shared state.
    pub async fn call_command(&self, _command: Value) -> Result<Value> {
        Err(Error::WebSocket(
            "WS commands not yet implemented — use REST client".to_owned(),
        ))
    }

    /// Perform the HA WebSocket auth handshake.
    async fn authenticate(
        write: &mut futures_util::stream::SplitSink<WsStream, Message>,
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
    /// Then echoes back any text messages as events.
    async fn start_mock_ha_ws() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, mut read) = ws.split();

                    // Send auth_required
                    let auth_required = json!({"type": "auth_required", "ha_version": "2025.1"});
                    write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await
                        .unwrap();

                    // Wait for auth
                    match read.next().await {
                        Some(Ok(Message::Text(text))) => {
                            let val: Value = serde_json::from_str(&text).unwrap();
                            if val.get("type").and_then(|t| t.as_str()) == Some("auth") {
                                let auth_ok = json!({"type": "auth_ok", "ha_version": "2025.1"});
                                write
                                    .send(Message::Text(auth_ok.to_string().into()))
                                    .await
                                    .unwrap();
                            } else {
                                let auth_invalid = json!({"type": "auth_invalid", "message": "bad token"});
                                write
                                    .send(Message::Text(auth_invalid.to_string().into()))
                                    .await
                                    .unwrap();
                                return;
                            }
                        }
                        _ => return,
                    }

                    // Echo incoming messages back as-is (simulates event bus)
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                let _ = write.send(Message::Text(text)).await;
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

    /// Start a mock HA WS server that closes before auth_ok.
    async fn start_mock_ha_ws_auth_reject() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let (mut write, _read) = ws.split();

                    let auth_required = json!({"type": "auth_required"});
                    write
                        .send(Message::Text(auth_required.to_string().into()))
                        .await
                        .unwrap();
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

        // If subscribe returns Ok, auth handshake succeeded
        let rx = ws.subscribe().await;
        assert!(rx.is_ok());
    }

    #[tokio::test]
    async fn call_command_returns_not_implemented() {
        let ws = HaWebSocket::new("ws://127.0.0.1:1", "token");
        let err = ws.call_command(json!({"type": "call_service"})).await;
        assert!(err.is_err());
        assert!(matches!(err.unwrap_err(), Error::WebSocket(_)));
    }

    #[tokio::test]
    async fn new_generates_unique_ids() {
        let ws = HaWebSocket::new("ws://localhost:1", "token");
        let id1 = ws.next_id();
        let id2 = ws.next_id();
        let id3 = ws.next_id();
        assert!(id1 < id2);
        assert!(id2 < id3);
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
