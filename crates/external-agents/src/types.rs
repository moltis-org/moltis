use std::{collections::HashMap, path::PathBuf};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
};

pub use moltis_sessions::metadata::ExternalAgentKind as AgentTransportKind;

/// Events emitted by an external agent session.
///
/// Maps to existing `RunnerEvent` variants so the WebSocket broadcast path
/// is unchanged.
#[derive(Debug, Clone)]
pub enum ExternalAgentEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart {
        id: String,
        name: String,
        arguments: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        success: bool,
        result: Option<String>,
    },
    Done {
        usage: Option<TokenUsage>,
    },
    Error(String),
}

/// Token usage reported by an external agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Status of an external agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentStatus {
    Starting,
    Running,
    Idle,
    Stopped,
    Error,
}

/// Context snapshot passed to an external agent at session start or resync.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub working_dir: Option<PathBuf>,
    pub summary: Option<String>,
    pub recent_turns: Vec<ContextTurn>,
    pub system_instructions: Option<String>,
    pub memory_files: HashMap<String, String>,
    pub project_context: Option<String>,
}

/// A single conversation turn in the context snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextTurn {
    pub role: String,
    pub content: String,
}

/// Specification for starting an external agent session.
#[derive(Debug, Clone)]
pub struct ExternalAgentSpec {
    pub kind: AgentTransportKind,
    pub session_key: Option<String>,
    pub external_session_id: Option<String>,
    pub binary: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<PathBuf>,
    pub timeout_secs: Option<u64>,
    pub use_tmux: bool,
}

impl ExternalAgentSpec {
    /// Create a minimal spec for the given kind.
    #[must_use]
    pub fn new(kind: AgentTransportKind) -> Self {
        Self {
            kind,
            session_key: None,
            external_session_id: None,
            binary: None,
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
            timeout_secs: None,
            use_tmux: false,
        }
    }
}

/// ACP permission request normalized away from ACP wire types.
#[derive(Debug, Clone)]
pub struct AcpPermissionRequest {
    pub moltis_session_key: Option<String>,
    pub acp_session_id: String,
    pub tool_call: String,
    pub options: Vec<AcpPermissionOption>,
}

#[derive(Debug, Clone)]
pub struct AcpPermissionOption {
    pub id: String,
    pub name: String,
    pub kind: AcpPermissionOptionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpPermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// Host hook used by ACP runtimes to route permission prompts through Moltis approvals.
#[async_trait]
pub trait AcpPermissionHandler: Send + Sync {
    async fn select_option(&self, request: AcpPermissionRequest) -> anyhow::Result<Option<String>>;
}

/// Persisted bridge state for a session bound to an external agent.
///
/// Stored in SQLite KV so it survives process restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeState {
    pub agent_kind: AgentTransportKind,
    pub external_session_id: Option<String>,
    pub synced_message_count: u32,
    pub last_synced_tail_hash: Option<String>,
    pub initialized: bool,
    pub compaction_generation: u64,
}

impl BridgeState {
    /// Create initial bridge state for a new binding.
    #[must_use]
    pub fn new(agent_kind: AgentTransportKind) -> Self {
        Self {
            agent_kind,
            external_session_id: None,
            synced_message_count: 0,
            last_synced_tail_hash: None,
            initialized: false,
            compaction_generation: 0,
        }
    }
}

/// How prompts are delivered to the external agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDeliveryMode {
    Stdin,
    TmuxPaste,
    File,
    JsonRpc,
}

/// Info about an external agent for API/UI responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentInfo {
    pub kind: AgentTransportKind,
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
}
