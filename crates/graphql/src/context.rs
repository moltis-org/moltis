//! GraphQL request context bridging to gateway state and services.

use std::sync::Arc;

use {serde_json::Value, tokio::sync::broadcast};

/// Context injected into every GraphQL resolver via `Context::data()`.
///
/// Holds an `Arc` to the broadcast sender so resolvers can subscribe to
/// real-time events, plus a service caller for dispatching RPC methods.
pub struct GqlContext {
    /// Broadcast channel for subscription events.
    /// Each event is `(event_name, payload)`.
    pub broadcast_tx: broadcast::Sender<(String, Value)>,

    /// Service caller: takes a method name + JSON params and returns
    /// a JSON result. This is wired to the gateway's method dispatch
    /// so every RPC method is accessible.
    pub call: Arc<dyn ServiceCaller>,
}

impl GqlContext {
    /// Invoke an RPC method by name.
    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        self.call.call(method, params).await
    }

    /// Subscribe to broadcast events.
    pub fn subscribe(&self) -> broadcast::Receiver<(String, Value)> {
        self.broadcast_tx.subscribe()
    }
}

/// Trait abstraction over the gateway's method dispatch.
///
/// This lets the graphql crate call any RPC method without depending on
/// `moltis-gateway` directly (the gateway crate provides the implementation).
#[async_trait::async_trait]
pub trait ServiceCaller: Send + Sync {
    /// Invoke an RPC method by name with JSON params.
    async fn call(&self, method: &str, params: Value) -> Result<Value, String>;
}
