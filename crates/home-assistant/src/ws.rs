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
