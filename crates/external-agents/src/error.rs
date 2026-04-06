/// Errors specific to external agent operations.
///
/// Each variant represents a concrete failure mode. New failure modes in
/// later phases should get their own named variant rather than a catch-all.
#[derive(Debug, thiserror::Error)]
pub enum ExternalAgentError {
    #[error("external agent binary not found: {binary}")]
    BinaryNotFound { binary: String },

    #[error("external agent session not alive")]
    SessionNotAlive,

    #[error("external agent session start failed: {reason}")]
    SessionStartFailed { reason: String },

    #[error("prompt delivery failed: {reason}")]
    PromptDeliveryFailed { reason: String },

    #[error("transport not supported for agent kind: {kind}")]
    UnsupportedTransport { kind: String },

    #[error("bridge state mismatch: {reason}")]
    BridgeStateMismatch { reason: String },

    #[error("compaction detected: {reason}")]
    CompactionDetected { reason: String },

    #[error("agent registry: no runtime registered for kind {kind}")]
    NoRuntimeForKind { kind: String },

    #[error("child process failed to spawn: {0}")]
    ProcessSpawn(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(#[from] serde_json::Error),
}
