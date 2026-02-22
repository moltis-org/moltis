//! Error mapping from service errors to GraphQL errors.

/// Convert a service error string into an `async_graphql::Error`.
pub fn gql_err(msg: String) -> async_graphql::Error {
    async_graphql::Error::new(msg)
}

/// Convert a serde_json parse error into an `async_graphql::Error`.
pub fn parse_err(e: serde_json::Error) -> async_graphql::Error {
    async_graphql::Error::new(format!("failed to parse response: {e}"))
}
