//! Schema construction and type alias.

use std::sync::Arc;

use {async_graphql::Schema, serde_json::Value, tokio::sync::broadcast};

use crate::{
    context::{GqlContext, ServiceCaller},
    mutations::MutationRoot,
    queries::QueryRoot,
    subscriptions::SubscriptionRoot,
};

/// The full Moltis GraphQL schema type.
pub type MoltisSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

/// Build the GraphQL schema with a service caller and broadcast channel.
///
/// The `caller` implements `ServiceCaller` — the gateway provides an
/// implementation that delegates to its `MethodRegistry`.
///
/// The `broadcast_tx` is a `tokio::sync::broadcast::Sender` that carries
/// `(event_name, payload)` tuples for subscriptions.
pub fn build_schema(
    caller: Arc<dyn ServiceCaller>,
    broadcast_tx: broadcast::Sender<(String, Value)>,
) -> MoltisSchema {
    let ctx = Arc::new(GqlContext {
        broadcast_tx,
        call: caller,
    });

    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(ctx)
        .finish()
}
