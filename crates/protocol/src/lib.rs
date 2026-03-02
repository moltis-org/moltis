//! Gateway WebSocket/RPC protocol definitions.
//!
//! Protocol version 4 (backward-compatible with v3). All communication uses JSON frames over WebSocket.
//!
//! Frame types:
//! - `RequestFrame`  — client → gateway RPC call (also server → client in v4)
//! - `ResponseFrame` — gateway → client RPC result (also client → server in v4)
//! - `EventFrame`    — gateway → client server-push

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Constants ────────────────────────────────────────────────────────────────

pub const PROTOCOL_VERSION: u32 = 4;
pub const MAX_PAYLOAD_BYTES: usize = 524_288; // 512 KB
pub const MAX_BUFFERED_BYTES: usize = 1_572_864; // 1.5 MB
pub const TICK_INTERVAL_MS: u64 = 30_000; // 30s
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000; // 10s
pub const DEDUPE_TTL_MS: u64 = 300_000; // 5 min
pub const DEDUPE_MAX_ENTRIES: usize = 1_000;

// ── Subscriptions ────────────────────────────────────────────────────────────

pub mod subscriptions {
    /// Wildcard subscription: receive all events.
    pub const WILDCARD: &str = "*";
}

// ── Error codes ──────────────────────────────────────────────────────────────

pub mod error_codes {
    // v3 backward-compat codes
    pub const NOT_LINKED: &str = "NOT_LINKED";
    pub const NOT_PAIRED: &str = "NOT_PAIRED";
    pub const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
    pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
    pub const UNAVAILABLE: &str = "UNAVAILABLE";

    // v4 standardized codes
    pub const UNKNOWN_METHOD: &str = "UNKNOWN_METHOD";
    pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
    pub const FORBIDDEN: &str = "FORBIDDEN";
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const CONFLICT: &str = "CONFLICT";
    pub const RATE_LIMITED: &str = "RATE_LIMITED";
    pub const TIMEOUT: &str = "TIMEOUT";
    pub const INTERNAL: &str = "INTERNAL";
    pub const PROTOCOL_ERROR: &str = "PROTOCOL_ERROR";
    pub const PAYLOAD_TOO_LARGE: &str = "PAYLOAD_TOO_LARGE";
}

// ── Error shape ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(rename = "retryAfterMs", skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl ErrorShape {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            retryable: None,
            retry_after_ms: None,
        }
    }
}

// ── Frames ───────────────────────────────────────────────────────────────────

/// Client → gateway RPC request. Also used server → client in v4 bidirectional RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrame {
    pub r#type: String, // always "req"
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Gateway → client RPC response. Also client → server in v4 bidirectional RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub r#type: String, // always "res"
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

impl ResponseFrame {
    pub fn ok(id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            r#type: "res".into(),
            id: id.into(),
            ok: true,
            payload: Some(payload),
            error: None,
            channel: None,
        }
    }

    pub fn err(id: impl Into<String>, error: ErrorShape) -> Self {
        Self {
            r#type: "res".into(),
            id: id.into(),
            ok: false,
            payload: None,
            error: Some(error),
            channel: None,
        }
    }
}

/// Gateway → client server-push event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    pub r#type: String, // always "event"
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(rename = "stateVersion", skip_serializing_if = "Option::is_none")]
    pub state_version: Option<StateVersion>,
    /// Stream group ID for chunked delivery (v4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// End-of-stream marker (v4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    /// Logical channel for multiplexing (v4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

impl EventFrame {
    pub fn new(event: impl Into<String>, payload: serde_json::Value, seq: u64) -> Self {
        Self {
            r#type: "event".into(),
            event: event.into(),
            payload: Some(payload),
            seq: Some(seq),
            state_version: None,
            stream: None,
            done: None,
            channel: None,
        }
    }

    /// Create an event frame with stream metadata.
    pub fn streamed(
        event: impl Into<String>,
        payload: serde_json::Value,
        seq: u64,
        stream_id: String,
        done: bool,
    ) -> Self {
        Self {
            r#type: "event".into(),
            event: event.into(),
            payload: Some(payload),
            seq: Some(seq),
            state_version: None,
            stream: Some(stream_id),
            done: Some(done),
            channel: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<u64>,
}

/// Discriminated union of all frame types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GatewayFrame {
    #[serde(rename = "req")]
    Request(RequestFrameInner),
    #[serde(rename = "res")]
    Response(ResponseFrameInner),
    #[serde(rename = "event")]
    Event(EventFrameInner),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrameInner {
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrameInner {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrameInner {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(rename = "stateVersion", skip_serializing_if = "Option::is_none")]
    pub state_version: Option<StateVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

// ── Extensions ───────────────────────────────────────────────────────────────

/// Namespaced extension data for protocol-agnostic transport.
pub type Extensions = HashMap<String, serde_json::Value>;

// ── Connect handshake ────────────────────────────────────────────────────────

/// Protocol version range for v4 negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolRange {
    pub min: u32,
    pub max: u32,
}

/// v4 connect parameters: flat core + namespaced extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParamsV4 {
    pub protocol: ProtocolRange,
    pub client: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<ConnectAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: Extensions,
}

impl ConnectParamsV4 {
    /// Convert to v3-compatible `ConnectParams` for internal processing.
    pub fn into_connect_params(self) -> ConnectParams {
        // Extract Moltis-specific fields from extensions.
        let moltis = self.extensions.get("moltis");
        let caps = moltis
            .and_then(|v| v.get("caps"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let commands = moltis
            .and_then(|v| v.get("commands"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let permissions = moltis
            .and_then(|v| v.get("permissions"))
            .and_then(|v| v.as_object().cloned());
        let path_env = moltis
            .and_then(|v| v.get("pathEnv"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let device = moltis
            .and_then(|v| v.get("device"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let user_agent = moltis
            .and_then(|v| v.get("userAgent"))
            .and_then(|v| v.as_str())
            .map(String::from);

        ConnectParams {
            min_protocol: self.protocol.min,
            max_protocol: self.protocol.max,
            client: self.client,
            caps,
            commands,
            permissions,
            path_env,
            role: self.role,
            scopes: self.scopes,
            device,
            auth: self.auth,
            locale: self.locale,
            user_agent,
            timezone: self.timezone,
        }
    }
}

/// Parameters sent by the client in the initial `connect` request (v3 format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    #[serde(rename = "minProtocol")]
    pub min_protocol: u32,
    #[serde(rename = "maxProtocol")]
    pub max_protocol: u32,
    pub client: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(rename = "pathEnv", skip_serializing_if = "Option::is_none")]
    pub path_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<ConnectAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(rename = "userAgent", skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub id: String,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub version: String,
    pub platform: String,
    #[serde(rename = "deviceFamily", skip_serializing_if = "Option::is_none")]
    pub device_family: Option<String>,
    #[serde(rename = "modelIdentifier", skip_serializing_if = "Option::is_none")]
    pub model_identifier: Option<String>,
    pub mode: String,
    #[serde(rename = "instanceId", skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    pub signature: String,
    #[serde(rename = "signedAt")]
    pub signed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAuth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// Sent by the gateway after successful handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloOk {
    pub r#type: String, // always "hello-ok"
    pub protocol: u32,
    pub server: ServerInfo,
    pub features: Features,
    pub snapshot: serde_json::Value, // opaque for now
    #[serde(rename = "canvasHostUrl", skip_serializing_if = "Option::is_none")]
    pub canvas_host_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<HelloAuth>,
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(rename = "connId")]
    pub conn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Features {
    pub methods: Vec<String>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAuth {
    #[serde(rename = "deviceToken")]
    pub device_token: String,
    pub role: String,
    pub scopes: Vec<String>,
    #[serde(rename = "issuedAtMs", skip_serializing_if = "Option::is_none")]
    pub issued_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    #[serde(rename = "maxPayload")]
    pub max_payload: usize,
    #[serde(rename = "maxBufferedBytes")]
    pub max_buffered_bytes: usize,
    #[serde(rename = "tickIntervalMs")]
    pub tick_interval_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_payload: MAX_PAYLOAD_BYTES,
            max_buffered_bytes: MAX_BUFFERED_BYTES,
            tick_interval_ms: TICK_INTERVAL_MS,
        }
    }
}

// ── Known events ─────────────────────────────────────────────────────────────

pub const KNOWN_EVENTS: &[&str] = &[
    "tick",
    "shutdown",
    "agent",
    "chat",
    "presence",
    "health",
    "exec.approval.requested",
    "exec.approval.resolved",
    "device.pair.requested",
    "device.pair.resolved",
    "node.pair.requested",
    "node.pair.resolved",
    "node.invoke.request",
];

// ── Roles and scopes ─────────────────────────────────────────────────────────

pub mod roles {
    pub const OPERATOR: &str = "operator";
    pub const NODE: &str = "node";
}

pub mod scopes {
    pub const ADMIN: &str = "operator.admin";
    pub const READ: &str = "operator.read";
    pub const WRITE: &str = "operator.write";
    pub const APPROVALS: &str = "operator.approvals";
    pub const PAIRING: &str = "operator.pairing";
}

// ── Schema discovery (v4) ────────────────────────────────────────────────────

/// Describes a single RPC method for `system.describe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodDescriptor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "paramsSchema", skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<serde_json::Value>,
    #[serde(rename = "resultSchema", skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<serde_json::Value>,
    #[serde(rename = "requiredScope", skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<String>,
    #[serde(rename = "requiredRole", skip_serializing_if = "Option::is_none")]
    pub required_role: Option<String>,
}

/// Describes a server-push event for `system.describe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDescriptor {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "payloadSchema", skip_serializing_if = "Option::is_none")]
    pub payload_schema: Option<serde_json::Value>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── v3/v4 connect params round-trip ────────────────────────────────

    #[test]
    fn v3_connect_params_round_trip() {
        let json = serde_json::json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": { "id": "test", "version": "0.1.0", "platform": "browser", "mode": "operator" },
            "locale": "en",
        });
        let params: ConnectParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.min_protocol, 3);
        assert_eq!(params.max_protocol, 3);
        assert_eq!(params.client.id, "test");
        assert_eq!(params.locale.as_deref(), Some("en"));
    }

    #[test]
    fn v4_connect_params_parses_and_converts() {
        let json = serde_json::json!({
            "protocol": { "min": 3, "max": 4 },
            "client": { "id": "test-v4", "version": "0.2.0", "platform": "browser", "mode": "operator" },
            "locale": "fr",
            "extensions": {
                "moltis": {
                    "caps": ["audio"],
                    "pathEnv": "/usr/bin"
                }
            }
        });
        let v4: ConnectParamsV4 = serde_json::from_value(json).unwrap();
        assert_eq!(v4.protocol.min, 3);
        assert_eq!(v4.protocol.max, 4);

        let params = v4.into_connect_params();
        assert_eq!(params.min_protocol, 3);
        assert_eq!(params.max_protocol, 4);
        assert_eq!(params.client.id, "test-v4");
        assert_eq!(params.caps.as_ref().unwrap(), &["audio"]);
        assert_eq!(params.path_env.as_deref(), Some("/usr/bin"));
    }

    #[test]
    fn v4_connect_params_empty_extensions() {
        let json = serde_json::json!({
            "protocol": { "min": 4, "max": 4 },
            "client": { "id": "minimal", "version": "1.0", "platform": "cli", "mode": "operator" },
        });
        let v4: ConnectParamsV4 = serde_json::from_value(json).unwrap();
        let params = v4.into_connect_params();
        assert!(params.caps.is_none());
        assert!(params.device.is_none());
        assert!(params.path_env.is_none());
    }

    // ── EventFrame with stream/done/channel ────────────────────────────

    #[test]
    fn event_frame_new_omits_stream_fields() {
        let frame = EventFrame::new("chat", serde_json::json!({"text":"hi"}), 1);
        let json = serde_json::to_value(&frame).unwrap();
        assert!(!json.as_object().unwrap().contains_key("stream"));
        assert!(!json.as_object().unwrap().contains_key("done"));
        assert!(!json.as_object().unwrap().contains_key("channel"));
    }

    #[test]
    fn event_frame_streamed_includes_metadata() {
        let frame = EventFrame::streamed(
            "chat",
            serde_json::json!({"token":"hello"}),
            42,
            "run-abc".into(),
            false,
        );
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["stream"], "run-abc");
        assert_eq!(json["done"], false);
    }

    #[test]
    fn event_frame_stream_done_marker() {
        let frame = EventFrame::streamed(
            "chat",
            serde_json::json!({"final":true}),
            99,
            "run-abc".into(),
            true,
        );
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["done"], true);
    }

    #[test]
    fn event_frame_round_trip_with_channel() {
        let mut frame = EventFrame::new("chat", serde_json::json!({}), 1);
        frame.channel = Some("session:abc".into());
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: EventFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channel.as_deref(), Some("session:abc"));
    }

    // ── RequestFrame with channel ──────────────────────────────────────

    #[test]
    fn request_frame_channel_serialization() {
        let frame = RequestFrame {
            r#type: "req".into(),
            id: "1".into(),
            method: "chat.send".into(),
            params: None,
            channel: Some("session:xyz".into()),
        };
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["channel"], "session:xyz");
    }

    #[test]
    fn request_frame_omits_null_channel() {
        let frame = RequestFrame {
            r#type: "req".into(),
            id: "1".into(),
            method: "health".into(),
            params: None,
            channel: None,
        };
        let json = serde_json::to_value(&frame).unwrap();
        assert!(!json.as_object().unwrap().contains_key("channel"));
    }

    // ── HelloOk extensions ─────────────────────────────────────────────

    #[test]
    fn hello_ok_empty_extensions_not_serialized() {
        let hello = HelloOk {
            r#type: "hello-ok".into(),
            protocol: 4,
            server: ServerInfo {
                version: "0.1.0".into(),
                commit: None,
                host: None,
                conn_id: "test".into(),
            },
            features: Features {
                methods: vec![],
                events: vec![],
            },
            snapshot: serde_json::json!({}),
            canvas_host_url: None,
            auth: None,
            policy: Policy::default(),
            extensions: Extensions::new(),
        };
        let json = serde_json::to_value(&hello).unwrap();
        assert!(!json.as_object().unwrap().contains_key("extensions"));
    }

    #[test]
    fn hello_ok_with_extensions_serialized() {
        let mut extensions = Extensions::new();
        extensions.insert("moltis".into(), serde_json::json!({"extra": true}));
        let hello = HelloOk {
            r#type: "hello-ok".into(),
            protocol: 4,
            server: ServerInfo {
                version: "0.1.0".into(),
                commit: None,
                host: None,
                conn_id: "test".into(),
            },
            features: Features {
                methods: vec![],
                events: vec![],
            },
            snapshot: serde_json::json!({}),
            canvas_host_url: None,
            auth: None,
            policy: Policy::default(),
            extensions,
        };
        let json = serde_json::to_value(&hello).unwrap();
        assert_eq!(json["extensions"]["moltis"]["extra"], true);
    }

    // ── Error codes ────────────────────────────────────────────────────

    #[test]
    fn new_error_codes_exist() {
        assert_eq!(error_codes::UNKNOWN_METHOD, "UNKNOWN_METHOD");
        assert_eq!(error_codes::UNAUTHORIZED, "UNAUTHORIZED");
        assert_eq!(error_codes::FORBIDDEN, "FORBIDDEN");
        assert_eq!(error_codes::NOT_FOUND, "NOT_FOUND");
        assert_eq!(error_codes::CONFLICT, "CONFLICT");
        assert_eq!(error_codes::RATE_LIMITED, "RATE_LIMITED");
        assert_eq!(error_codes::TIMEOUT, "TIMEOUT");
        assert_eq!(error_codes::INTERNAL, "INTERNAL");
        assert_eq!(error_codes::PROTOCOL_ERROR, "PROTOCOL_ERROR");
        assert_eq!(error_codes::PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE");
    }

    // ── GatewayFrame round-trip with new fields ────────────────────────

    #[test]
    fn gateway_frame_response_round_trip() {
        let json = r#"{"type":"res","id":"1","ok":true,"payload":{"result":"ok"}}"#;
        let frame: GatewayFrame = serde_json::from_str(json).unwrap();
        match frame {
            GatewayFrame::Response(inner) => {
                assert!(inner.ok);
                assert!(inner.channel.is_none());
            },
            _ => panic!("expected Response frame"),
        }
    }

    #[test]
    fn gateway_frame_event_with_stream() {
        let json =
            r#"{"type":"event","event":"chat","payload":{},"seq":1,"stream":"run-1","done":false}"#;
        let frame: GatewayFrame = serde_json::from_str(json).unwrap();
        match frame {
            GatewayFrame::Event(inner) => {
                assert_eq!(inner.stream.as_deref(), Some("run-1"));
                assert_eq!(inner.done, Some(false));
            },
            _ => panic!("expected Event frame"),
        }
    }

    // ── Schema descriptors ─────────────────────────────────────────────

    #[test]
    fn method_descriptor_round_trip() {
        let desc = MethodDescriptor {
            name: "chat.send".into(),
            description: Some("Send a message".into()),
            params_schema: None,
            result_schema: None,
            required_scope: Some("operator.write".into()),
            required_role: None,
        };
        let json = serde_json::to_string(&desc).unwrap();
        let parsed: MethodDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "chat.send");
        assert_eq!(parsed.required_scope.as_deref(), Some("operator.write"));
    }
}
