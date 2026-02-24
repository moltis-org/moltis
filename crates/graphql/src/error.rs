//! Error mapping from service errors to GraphQL errors.

use moltis_service_traits::ServiceResult;

use crate::scalars::Json;

/// Convert a service error string into an `async_graphql::Error`.
pub fn gql_err(msg: String) -> async_graphql::Error {
    async_graphql::Error::new(msg)
}

/// Convert a serde_json parse error into an `async_graphql::Error`.
pub fn parse_err(e: serde_json::Error) -> async_graphql::Error {
    async_graphql::Error::new(format!("failed to parse response: {e}"))
}

/// Convert a `ServiceResult` into a typed GraphQL result.
///
/// Deserializes the JSON value from the service into the expected type `T`.
pub fn from_service<T: serde::de::DeserializeOwned>(
    result: ServiceResult,
) -> async_graphql::Result<T> {
    let value = result.map_err(gql_err)?;
    serde_json::from_value(value).map_err(parse_err)
}

/// Convert a `ServiceResult` into a raw JSON GraphQL result.
///
/// Returns the JSON value as-is, wrapped in the `Json` scalar. Use this for
/// truly dynamic/untyped data where deserialization into a concrete type is
/// not practical.
pub fn from_service_json(result: ServiceResult) -> async_graphql::Result<Json> {
    let value = result.map_err(gql_err)?;
    Ok(Json(value))
}
