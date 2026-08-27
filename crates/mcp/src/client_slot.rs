//! Stable handles for replaceable MCP client connections.

use std::sync::Arc;

use tokio::sync::{OwnedRwLockReadGuard, RwLock};

use crate::traits::McpClientTrait;

pub(crate) type SharedMcpClient = Arc<RwLock<dyn McpClientTrait>>;

/// A stable per-server handle whose client can be replaced after a restart.
#[derive(Default)]
pub(crate) struct ClientSlot {
    current: RwLock<Option<SharedMcpClient>>,
}

impl ClientSlot {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn with_client(client: SharedMcpClient) -> Self {
        Self {
            current: RwLock::new(Some(client)),
        }
    }

    pub(crate) async fn current(&self) -> Option<SharedMcpClient> {
        self.current.read().await.clone()
    }

    pub(crate) async fn read_client(&self) -> Option<OwnedRwLockReadGuard<dyn McpClientTrait>> {
        let current = self.current.read().await;
        let client = current.as_ref()?.clone();

        // Keep the slot read-locked until the client is read-locked. A restart
        // therefore cannot remove and close this client between the two steps.
        Some(client.read_owned().await)
    }

    pub(crate) async fn replace(&self, client: SharedMcpClient) -> Option<SharedMcpClient> {
        self.current.write().await.replace(client)
    }

    pub(crate) async fn take(&self) -> Option<SharedMcpClient> {
        self.current.write().await.take()
    }
}
