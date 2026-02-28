//! GraphQL API for Moltis.
//!
//! Provides a typed GraphQL schema that mirrors the RPC interface, exposing
//! queries, mutations, and subscriptions for all gateway services. The schema
//! is served at `/graphql` (GraphiQL on GET, queries on POST, subscriptions via
//! WebSocket upgrade on GET).
//!
//! Resolvers call domain services directly through the `Services` bundle from
//! `moltis-service-traits`, sharing the same code path as the RPC layer. No
//! string-based dispatch or RPC indirection is used.
//!
//! The gateway crate is responsible for building the HTTP handlers and wiring
//! them into the router. This crate only defines the schema, types, and resolvers.

pub mod context;
pub mod error;
pub mod mutations;
pub mod queries;
pub mod scalars;
pub mod schema;
pub mod subscriptions;
pub mod types;

pub use schema::{MoltisSchema, build_schema};

// ── Shared resolver helpers ─────────────────────────────────────────────────

/// Extract the `Services` bundle from a GraphQL resolver context.
///
/// Usage: `let s = services!(ctx);`
/// Then call methods directly: `s.session.list().await`
#[macro_export]
macro_rules! services {
    ($ctx:expr) => {{
        &$ctx
            .data::<std::sync::Arc<$crate::context::GqlContext>>()?
            .services
    }};
}
