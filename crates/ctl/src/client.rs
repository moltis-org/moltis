//! WebSocket RPC client for communicating with the Moltis gateway.
//!
//! Implements the v4 protocol handshake and request/response framing.

use std::time::Duration;

use {
    futures::{SinkExt, StreamExt},
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio_tungstenite::{connect_async, tungstenite::Message},
};

use crate::error::CtlError;

// ── Protocol types (minimal subset of moltis-protocol) ──────────────────────

#[derive(Serialize)]
struct RequestFrame {
    r#type: &'static str,
    id: String,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize)]
struct ResponseFrame {
    #[allow(dead_code)]
    r#type: String,
    #[allow(dead_code)]
    id: String,
    ok: bool,
    payload: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct ConnectFrame {
    r#type: &'static str,
    id: String,
    method: &'static str,
    params: ConnectParams,
}

#[derive(Serialize)]
struct ConnectParams {
    protocol: ProtocolRange,
    client: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<ConnectAuth>,
}

#[derive(Serialize)]
struct ProtocolRange {
    min: u32,
    max: u32,
}

#[derive(Serialize)]
struct ClientInfo {
    id: String,
    version: String,
    platform: String,
    mode: String,
}

#[derive(Serialize)]
struct ConnectAuth {
    api_key: String,
}

// ── Client ──────────────────────────────────────────────────────────────────

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct CtlClient {
    ws: WsStream,
    req_counter: u64,
}

impl CtlClient {
    /// Connect to the Moltis gateway and perform the v4 handshake.
    pub async fn connect(gateway_url: &str, api_key: &str) -> Result<Self, CtlError> {
        let ws_url = to_ws_url(gateway_url);

        let (mut ws, _) = connect_async(&ws_url)
            .await
            .map_err(|e| CtlError::Connect(format!("{ws_url}: {e}")))?;

        // Send v4 connect frame.
        let connect_id = new_id();
        let connect = ConnectFrame {
            r#type: "req",
            id: connect_id.clone(),
            method: "connect",
            params: ConnectParams {
                protocol: ProtocolRange { min: 4, max: 4 },
                client: ClientInfo {
                    id: "moltis-ctl".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    platform: "cli".into(),
                    mode: "operator".into(),
                },
                auth: if api_key.is_empty() {
                    None
                } else {
                    Some(ConnectAuth {
                        api_key: api_key.into(),
                    })
                },
            },
        };

        let msg = serde_json::to_string(&connect)
            .map_err(|e| CtlError::Handshake(format!("serialize connect: {e}")))?;
        ws.send(Message::Text(msg.into()))
            .await
            .map_err(|e| CtlError::Handshake(format!("send connect: {e}")))?;

        // Wait for hello-ok (with timeout).
        let hello = tokio::time::timeout(Duration::from_secs(10), read_hello(&mut ws))
            .await
            .map_err(|_| CtlError::Timeout)?
            .map_err(|e| CtlError::Handshake(format!("waiting for hello-ok: {e}")))?;

        if !hello {
            return Err(CtlError::Handshake("gateway rejected connection".into()));
        }

        Ok(Self { ws, req_counter: 0 })
    }

    /// Send an RPC request and wait for the response.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, CtlError> {
        self.req_counter += 1;
        let id = format!("ctl-{}", self.req_counter);

        let frame = RequestFrame {
            r#type: "req",
            id: id.clone(),
            method: method.into(),
            params: if params.is_null() {
                None
            } else {
                Some(params)
            },
        };

        let msg = serde_json::to_string(&frame)
            .map_err(|e| CtlError::Other(format!("serialize request: {e}")))?;
        self.ws
            .send(Message::Text(msg.into()))
            .await
            .map_err(|e| CtlError::Ws(format!("send: {e}")))?;

        // Read frames until we get our response (skip events/pings).
        let timeout = Duration::from_secs(120);
        let resp = tokio::time::timeout(timeout, self.read_response(&id))
            .await
            .map_err(|_| CtlError::Timeout)??;

        if resp.ok {
            Ok(resp.payload.unwrap_or(Value::Null))
        } else if let Some(err) = resp.error {
            Err(CtlError::Rpc {
                code: err.code,
                message: err.message,
            })
        } else {
            Err(CtlError::Rpc {
                code: "UNKNOWN".into(),
                message: "RPC returned ok=false with no error".into(),
            })
        }
    }

    async fn read_response(&mut self, expected_id: &str) -> Result<ResponseFrame, CtlError> {
        loop {
            let Some(msg) = self.ws.next().await else {
                return Err(CtlError::Ws("connection closed".into()));
            };
            let msg = msg.map_err(|e| CtlError::Ws(e.to_string()))?;

            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Ping(_) => continue,
                Message::Pong(_) => continue,
                Message::Close(_) => {
                    return Err(CtlError::Ws("connection closed by server".into()));
                },
                _ => continue,
            };

            // Try to parse as response frame. Skip events and other frames.
            let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if parsed.get("type").and_then(Value::as_str) != Some("res") {
                continue;
            }
            if parsed.get("id").and_then(Value::as_str) != Some(expected_id) {
                continue;
            }

            let resp: ResponseFrame = serde_json::from_value(parsed)
                .map_err(|e| CtlError::Other(format!("parse response: {e}")))?;
            return Ok(resp);
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert an HTTP gateway URL to a WebSocket URL.
fn to_ws_url(gateway_url: &str) -> String {
    let base = gateway_url.trim_end_matches('/');
    if base.starts_with("https://") {
        format!("{}/ws", base.replacen("https://", "wss://", 1))
    } else if base.starts_with("http://") {
        format!("{}/ws", base.replacen("http://", "ws://", 1))
    } else {
        format!("ws://{base}/ws")
    }
}

/// Read frames until we get a hello-ok or an error response.
async fn read_hello(ws: &mut WsStream) -> Result<bool, String> {
    loop {
        let Some(msg) = ws.next().await else {
            return Err("connection closed during handshake".into());
        };
        let msg = msg.map_err(|e| e.to_string())?;
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(reason) => {
                return Err(format!(
                    "closed during handshake: {}",
                    reason.map(|r| r.reason.to_string()).unwrap_or_default()
                ));
            },
            _ => continue,
        };

        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("hello-ok") => return Ok(true),
            Some("res") if v.get("ok") == Some(&Value::Bool(false)) => {
                let msg = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(msg.into());
            },
            _ => continue,
        }
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_to_ws_conversion() {
        assert_eq!(to_ws_url("http://localhost:8080"), "ws://localhost:8080/ws");
        assert_eq!(
            to_ws_url("https://moltis.example.com"),
            "wss://moltis.example.com/ws"
        );
        assert_eq!(
            to_ws_url("http://localhost:8080/"),
            "ws://localhost:8080/ws"
        );
        assert_eq!(to_ws_url("192.168.1.1:8080"), "ws://192.168.1.1:8080/ws");
    }
}
