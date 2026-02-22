//! GraphQL API for Moltis.
//!
//! Provides a typed GraphQL schema that mirrors the RPC interface, exposing
//! queries, mutations, and subscriptions for all gateway services. The schema
//! is served at `/graphql` (GraphiQL on GET, queries on POST, subscriptions via
//! WebSocket upgrade on GET).
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

// ── Shared resolver macros ──────────────────────────────────────────────────

/// Invoke an RPC method and deserialize the result into a typed response.
#[macro_export]
macro_rules! rpc_call {
    ($method:expr, $ctx:expr) => {{
        let c = $ctx.data::<std::sync::Arc<$crate::context::GqlContext>>()?;
        let r = c
            .rpc($method, serde_json::json!({}))
            .await
            .map_err($crate::error::gql_err)?;
        serde_json::from_value(r).map_err($crate::error::parse_err)
    }};
    ($method:expr, $ctx:expr, $params:expr) => {{
        let c = $ctx.data::<std::sync::Arc<$crate::context::GqlContext>>()?;
        let r = c
            .rpc($method, $params)
            .await
            .map_err($crate::error::gql_err)?;
        serde_json::from_value(r).map_err($crate::error::parse_err)
    }};
}

/// Invoke an RPC method and return the raw JSON value wrapped in a `Json` scalar.
/// Only for truly dynamic/untyped data.
#[macro_export]
macro_rules! rpc_json_call {
    ($method:expr, $ctx:expr) => {{
        let c = $ctx.data::<std::sync::Arc<$crate::context::GqlContext>>()?;
        let r = c
            .rpc($method, serde_json::json!({}))
            .await
            .map_err($crate::error::gql_err)?;
        Ok($crate::scalars::Json(r))
    }};
    ($method:expr, $ctx:expr, $params:expr) => {{
        let c = $ctx.data::<std::sync::Arc<$crate::context::GqlContext>>()?;
        let r = c
            .rpc($method, $params)
            .await
            .map_err($crate::error::gql_err)?;
        Ok($crate::scalars::Json(r))
    }};
}
