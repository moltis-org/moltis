//! Schema construction and type alias.

use std::sync::Arc;

use {
    async_graphql::Schema, moltis_service_traits::Services, serde_json::Value,
    tokio::sync::broadcast,
};

use crate::{
    context::GqlContext, mutations::MutationRoot, queries::QueryRoot,
    subscriptions::SubscriptionRoot,
};

/// The full Moltis GraphQL schema type.
pub type MoltisSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

/// Build the GraphQL schema with a services bundle and broadcast channel.
///
/// The `services` bundle holds all domain service trait objects. Both the RPC
/// layer and GraphQL resolvers share these same instances — no indirection.
///
/// The `broadcast_tx` is a `tokio::sync::broadcast::Sender` that carries
/// `(event_name, payload)` tuples for subscriptions.
pub fn build_schema(
    services: Arc<Services>,
    broadcast_tx: broadcast::Sender<(String, Value)>,
) -> MoltisSchema {
    let ctx = Arc::new(GqlContext {
        broadcast_tx,
        services,
    });

    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(ctx)
        .finish()
}
