//! Gateway WebSocket/RPC protocol definitions.
//!
//! Protocol version 3. All communication uses JSON frames over WebSocket.
//!
//! Frame types:
//! - `RequestFrame`  — client → gateway RPC call
//! - `ResponseFrame` — gateway → client RPC result
//! - `EventFrame`    — gateway → client server-push

use serde::{Deserialize, Serialize};

// ── Constants ────────────────────────────────────────────────────────────────

pub const PROTOCOL_VERSION: u32 = 3;
pub const MAX_PAYLOAD_BYTES: usize = 524_288; // 512 KB
pub const MAX_BUFFERED_BYTES: usize = 1_572_864; // 1.5 MB
pub const TICK_INTERVAL_MS: u64 = 30_000; // 30s
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000; // 10s
pub const DEDUPE_TTL_MS: u64 = 300_000; // 5 min
pub const DEDUPE_MAX_ENTRIES: usize = 1_000;

// ── Error codes ──────────────────────────────────────────────────────────────

pub mod error_codes {
    pub const NOT_LINKED: &str = "NOT_LINKED";
    pub const NOT_PAIRED: &str = "NOT_PAIRED";
    pub const AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
    pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
    pub const UNAVAILABLE: &str = "UNAVAILABLE";
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

/// Client → gateway RPC request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrame {
    pub r#type: String, // always "req"
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Gateway → client RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub r#type: String, // always "res"
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
}

impl ResponseFrame {
    pub fn ok(id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            r#type: "res".into(),
            id: id.into(),
            ok: true,
            payload: Some(payload),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, error: ErrorShape) -> Self {
        Self {
            r#type: "res".into(),
            id: id.into(),
            ok: false,
            payload: None,
            error: Some(error),
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
}

impl EventFrame {
    pub fn new(event: impl Into<String>, payload: serde_json::Value, seq: u64) -> Self {
        Self {
            r#type: "event".into(),
            event: event.into(),
            payload: Some(payload),
            seq: Some(seq),
            state_version: None,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrameInner {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
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
}

// ── Connect handshake ────────────────────────────────────────────────────────

/// Parameters sent by the client in the initial `connect` request.
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

impl Policy {
    pub fn default_policy() -> Self {
        Self {
            max_payload: MAX_PAYLOAD_BYTES,
            max_buffered_bytes: MAX_BUFFERED_BYTES,
            tick_interval_ms: TICK_INTERVAL_MS,
        }
    }
}

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
