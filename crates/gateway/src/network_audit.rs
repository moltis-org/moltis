//! Network audit service layer.
//!
//! The buffer, filter, and stats types live in `moltis-network-filter`;
//! this module provides the `NetworkAuditService` trait (tied to gateway's
//! `ServiceResult`) and the live/noop implementations.

use std::path::PathBuf;

use {async_trait::async_trait, serde_json::Value, tracing::debug};

// Re-export types from the trusted-network crate so existing gateway code
// doesn't need to add a direct dependency when the feature is on.
#[cfg(feature = "trusted-network")]
pub use moltis_network_filter::{
    FilterOutcome, NetworkAuditEntry, NetworkProtocol,
    buffer::{NetworkAuditBuffer, NetworkAuditFilter, NetworkAuditStats},
};

use crate::services::ServiceResult;

// ── Service trait ───────────────────────────────────────────────────────────

#[async_trait]
pub trait NetworkAuditService: Send + Sync {
    async fn list(&self, params: Value) -> ServiceResult;
    async fn tail(&self, params: Value) -> ServiceResult;
    async fn stats(&self) -> ServiceResult;
    fn log_file_path(&self) -> Option<PathBuf>;
}

// ── Noop impl ───────────────────────────────────────────────────────────────

pub struct NoopNetworkAuditService;

#[async_trait]
impl NetworkAuditService for NoopNetworkAuditService {
    async fn list(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "entries": [] }))
    }

    async fn tail(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "entries": [] }))
    }

    async fn stats(&self) -> ServiceResult {
        Ok(serde_json::json!({ "total": 0, "allowed": 0, "denied": 0, "by_domain": [] }))
    }

    fn log_file_path(&self) -> Option<PathBuf> {
        None
    }
}

// ── Live impl (requires trusted-network feature) ───────────────────────────

#[cfg(feature = "trusted-network")]
pub struct LiveNetworkAuditService {
    buffer: NetworkAuditBuffer,
}

#[cfg(feature = "trusted-network")]
impl LiveNetworkAuditService {
    /// Create a new service, spawning a background task that reads from the
    /// `mpsc::Receiver` and pushes entries into the buffer.
    pub fn new(
        mut rx: tokio::sync::mpsc::Receiver<NetworkAuditEntry>,
        file_path: PathBuf,
        capacity: usize,
    ) -> Self {
        let buffer = NetworkAuditBuffer::new(capacity);
        buffer.enable_persistence(file_path);

        let buf_clone = buffer.clone();
        tokio::spawn(async move {
            while let Some(entry) = rx.recv().await {
                debug!(domain = %entry.domain, action = %entry.action, "network audit entry");
                buf_clone.push(entry);
            }
        });

        Self { buffer }
    }

    /// Get a reference to the underlying buffer (for broadcasting).
    pub fn buffer(&self) -> &NetworkAuditBuffer {
        &self.buffer
    }
}

#[cfg(feature = "trusted-network")]
#[async_trait]
impl NetworkAuditService for LiveNetworkAuditService {
    async fn list(&self, params: Value) -> ServiceResult {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
        let filter = NetworkAuditFilter {
            domain: params
                .get("domain")
                .and_then(|v| v.as_str())
                .map(String::from),
            protocol: params
                .get("protocol")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_value(Value::String(s.into())).ok()),
            action: params
                .get("action")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_value(Value::String(s.into())).ok()),
            search: params
                .get("search")
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        // Try in-memory first.
        let entries = self.buffer.list(&filter, limit);
        if !entries.is_empty() {
            return Ok(serde_json::json!({ "entries": entries }));
        }
        // Fall back to file.
        let buffer = self.buffer.clone();
        let entries = tokio::task::spawn_blocking(move || buffer.list_from_file(&filter, limit))
            .await
            .unwrap_or_default();
        Ok(serde_json::json!({ "entries": entries }))
    }

    async fn tail(&self, params: Value) -> ServiceResult {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
        let filter = NetworkAuditFilter {
            domain: None,
            protocol: None,
            action: None,
            search: None,
        };
        let entries = self.buffer.list(&filter, limit);
        Ok(serde_json::json!({ "entries": entries }))
    }

    async fn stats(&self) -> ServiceResult {
        let stats = self.buffer.stats();
        Ok(serde_json::to_value(stats).unwrap_or_default())
    }

    fn log_file_path(&self) -> Option<PathBuf> {
        self.buffer.file_path()
    }
}
