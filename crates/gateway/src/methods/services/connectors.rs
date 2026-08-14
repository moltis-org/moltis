use {
    moltis_protocol::{ErrorShape, error_codes},
    serde::{Deserialize, de::DeserializeOwned},
    serde_json::{Value, json},
};

use crate::connectors::{
    AccountUpdateRequest, ConnectorManager, ConnectorManagerError, DatasetCompileRequest,
    DatasetUpdateRequest, ItemQueryRequest,
};

use super::MethodRegistry;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdParams {
    #[serde(alias = "accountId", alias = "datasetId")]
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountUpdateParams {
    #[serde(alias = "accountId")]
    id: String,
    #[serde(flatten)]
    request: AccountUpdateRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DatasetUpdateParams {
    #[serde(alias = "datasetId")]
    id: String,
    #[serde(flatten)]
    request: DatasetUpdateRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunsListParams {
    dataset_id: String,
    #[serde(default = "default_limit")]
    limit: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemsQueryParams {
    dataset_id: String,
    #[serde(flatten)]
    query: ItemQueryRequest,
}

pub(super) fn register(registry: &mut MethodRegistry) {
    registry.register(
        "connectors.available",
        Box::new(|ctx| {
            Box::pin(async move {
                let manager = manager(&ctx.state)?;
                serialize(json!({ "connectors": manager.available() }))
            })
        }),
    );
    registry.register(
        "connectors.accounts.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let accounts = manager(&ctx.state)?
                    .list_accounts()
                    .await
                    .map_err(map_error)?;
                serialize(json!({ "accounts": accounts }))
            })
        }),
    );
    registry.register(
        "connectors.channel_sources.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let sources = manager(&ctx.state)?
                    .list_channel_sources()
                    .await
                    .map_err(map_error)?;
                serialize(json!({ "sources": sources }))
            })
        }),
    );
    registry.register(
        "connectors.accounts.add",
        Box::new(|ctx| {
            Box::pin(async move {
                let request = parse(ctx.params)?;
                let account = manager(&ctx.state)?
                    .add_account(request)
                    .await
                    .map_err(map_error)?;
                serialize(account)
            })
        }),
    );
    registry.register(
        "connectors.accounts.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: AccountUpdateParams = parse(ctx.params)?;
                let account = manager(&ctx.state)?
                    .update_account(&params.id, params.request)
                    .await
                    .map_err(map_error)?;
                serialize(account)
            })
        }),
    );
    registry.register(
        "connectors.accounts.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: IdParams = parse(ctx.params)?;
                manager(&ctx.state)?
                    .remove_account(&params.id)
                    .await
                    .map_err(map_error)?;
                Ok(json!({ "removed": true }))
            })
        }),
    );
    registry.register(
        "connectors.accounts.test",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: IdParams = parse(ctx.params)?;
                let result = manager(&ctx.state)?
                    .test_account(&params.id)
                    .await
                    .map_err(map_error)?;
                serialize(result)
            })
        }),
    );
    registry.register(
        "connectors.datasets.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let datasets = manager(&ctx.state)?
                    .list_datasets()
                    .await
                    .map_err(map_error)?;
                serialize(json!({ "datasets": datasets }))
            })
        }),
    );
    registry.register(
        "connectors.datasets.compile",
        Box::new(|ctx| {
            Box::pin(async move {
                let request: DatasetCompileRequest = parse(ctx.params)?;
                let response = manager(&ctx.state)?
                    .compile_dataset(request)
                    .await
                    .map_err(map_error)?;
                serialize(response)
            })
        }),
    );
    registry.register(
        "connectors.datasets.add",
        Box::new(|ctx| {
            Box::pin(async move {
                let request = parse(ctx.params)?;
                let dataset = manager(&ctx.state)?
                    .add_dataset(request)
                    .await
                    .map_err(map_error)?;
                serialize(dataset)
            })
        }),
    );
    registry.register(
        "connectors.datasets.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: DatasetUpdateParams = parse(ctx.params)?;
                let dataset = manager(&ctx.state)?
                    .update_dataset(&params.id, params.request)
                    .await
                    .map_err(map_error)?;
                serialize(dataset)
            })
        }),
    );
    registry.register(
        "connectors.datasets.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: IdParams = parse(ctx.params)?;
                manager(&ctx.state)?
                    .remove_dataset(&params.id)
                    .await
                    .map_err(map_error)?;
                Ok(json!({ "removed": true }))
            })
        }),
    );
    registry.register(
        "connectors.datasets.sync",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: IdParams = parse(ctx.params)?;
                let run = manager(&ctx.state)?
                    .sync_dataset(&params.id)
                    .await
                    .map_err(map_error)?;
                serialize(run)
            })
        }),
    );
    registry.register(
        "connectors.runs.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: RunsListParams = parse(ctx.params)?;
                let runs = manager(&ctx.state)?
                    .list_recent_runs(&params.dataset_id, params.limit)
                    .await
                    .map_err(map_error)?;
                serialize(json!({ "runs": runs }))
            })
        }),
    );
    registry.register(
        "connectors.items.query",
        Box::new(|ctx| {
            Box::pin(async move {
                let params: ItemsQueryParams = parse(ctx.params)?;
                let items = manager(&ctx.state)?
                    .query_items(&params.dataset_id, params.query)
                    .await
                    .map_err(map_error)?;
                serialize(json!({ "items": items }))
            })
        }),
    );
}

fn manager(
    state: &crate::state::GatewayState,
) -> Result<std::sync::Arc<ConnectorManager>, ErrorShape> {
    state.connector_manager().ok_or_else(|| {
        ErrorShape::new(
            error_codes::UNAVAILABLE,
            "connector manager is not initialized",
        )
    })
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, ErrorShape> {
    serde_json::from_value(params)
        .map_err(|error| ErrorShape::new(error_codes::INVALID_REQUEST, error.to_string()))
}

fn serialize(value: impl serde::Serialize) -> Result<Value, ErrorShape> {
    serde_json::to_value(value)
        .map_err(|_| ErrorShape::new(error_codes::INTERNAL, "failed to encode connector response"))
}

fn map_error(error: ConnectorManagerError) -> ErrorShape {
    match error {
        ConnectorManagerError::InvalidInput(message) => {
            ErrorShape::new(error_codes::INVALID_REQUEST, message)
        },
        ConnectorManagerError::NotFound { entity, id } => {
            ErrorShape::new(error_codes::NOT_FOUND, format!("{entity} not found: {id}"))
        },
        ConnectorManagerError::Conflict(id) => ErrorShape::new(
            error_codes::CONFLICT,
            format!("connector dataset is already synchronizing: {id}"),
        ),
        ConnectorManagerError::Unavailable(message) => {
            ErrorShape::new(error_codes::UNAVAILABLE, message)
        },
        ConnectorManagerError::Provider(error) => {
            tracing::warn!(error = ?error, "connector provider RPC operation failed");
            map_provider_error(error)
        },
        ConnectorManagerError::GmailProvider(error) => {
            tracing::warn!(error = ?error, "Gmail connector RPC operation failed");
            map_gmail_error(error)
        },
        ConnectorManagerError::HimalayaProvider(error) => {
            tracing::warn!(error = ?error, "Himalaya connector RPC operation failed");
            map_himalaya_error(error)
        },
        ConnectorManagerError::Channel(error) => {
            tracing::warn!(error = ?error, "channel history RPC operation failed");
            match error {
                moltis_channels::Error::InvalidInput { message } => {
                    ErrorShape::new(error_codes::INVALID_REQUEST, message)
                },
                moltis_channels::Error::UnknownAccount { .. } => {
                    ErrorShape::new(error_codes::NOT_FOUND, "channel account is not active")
                },
                _ => ErrorShape::new(
                    error_codes::UNAVAILABLE,
                    "channel message history request failed",
                ),
            }
        },
        error @ ConnectorManagerError::Internal(_) => {
            tracing::warn!(error = ?error, "connector RPC operation failed");
            ErrorShape::new(error_codes::INTERNAL, "connector operation failed")
        },
    }
}

fn map_gmail_error(error: moltis_connector_gmail::GmailConnectorError) -> ErrorShape {
    use moltis_connector_gmail::GmailConnectorError;

    match error {
        GmailConnectorError::AccountConfig(message)
        | GmailConnectorError::DatasetConfig(message)
        | GmailConnectorError::Credential(message) => {
            ErrorShape::new(error_codes::INVALID_REQUEST, message)
        },
        GmailConnectorError::ApiStatus(401 | 403) | GmailConnectorError::OAuth(_) => {
            ErrorShape::new(
                error_codes::INVALID_REQUEST,
                "Gmail authentication failed; reconfigure the Google Workspace account",
            )
        },
        GmailConnectorError::CredentialIo(_) | GmailConnectorError::CredentialJson(_) => {
            ErrorShape::new(
                error_codes::UNAVAILABLE,
                "Google Workspace credentials are not configured",
            )
        },
        GmailConnectorError::Timeout => {
            ErrorShape::new(error_codes::TIMEOUT, "Gmail request timed out")
        },
        GmailConnectorError::Http(error) if error.is_timeout() => {
            ErrorShape::new(error_codes::TIMEOUT, "Gmail request timed out")
        },
        GmailConnectorError::Http(_)
        | GmailConnectorError::ApiStatus(_)
        | GmailConnectorError::ServerResponse(_)
        | GmailConnectorError::MessageBody(_) => {
            ErrorShape::new(error_codes::UNAVAILABLE, "Gmail request failed")
        },
        GmailConnectorError::Serialization(_) => {
            ErrorShape::new(error_codes::INTERNAL, "failed to store Gmail data")
        },
    }
}

fn map_himalaya_error(error: moltis_connector_himalaya::HimalayaConnectorError) -> ErrorShape {
    use moltis_connector_himalaya::HimalayaConnectorError;

    match error {
        HimalayaConnectorError::AccountConfig(message)
        | HimalayaConnectorError::DatasetConfig(message) => {
            ErrorShape::new(error_codes::INVALID_REQUEST, message)
        },
        HimalayaConnectorError::ExecutableNotFound => {
            ErrorShape::new(error_codes::UNAVAILABLE, "Himalaya is not installed")
        },
        HimalayaConnectorError::UnsupportedVersion => {
            ErrorShape::new(error_codes::INVALID_REQUEST, "Himalaya v2 is required")
        },
        HimalayaConnectorError::Timeout => {
            ErrorShape::new(error_codes::TIMEOUT, "Himalaya command timed out")
        },
        HimalayaConnectorError::Spawn(_)
        | HimalayaConnectorError::CommandFailed
        | HimalayaConnectorError::OutputLimit
        | HimalayaConnectorError::Response(_) => {
            ErrorShape::new(error_codes::UNAVAILABLE, "Himalaya email request failed")
        },
        HimalayaConnectorError::Serialization(_) => {
            ErrorShape::new(error_codes::INTERNAL, "failed to store Himalaya data")
        },
    }
}

fn map_provider_error(error: moltis_connector_caldav::CalDavConnectorError) -> ErrorShape {
    use moltis_connector_caldav::CalDavConnectorError;

    match error {
        CalDavConnectorError::AccountConfig(message)
        | CalDavConnectorError::DatasetConfig(message) => {
            ErrorShape::new(error_codes::INVALID_REQUEST, message)
        },
        CalDavConnectorError::NetworkSafety(_) => ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "CalDAV server address was blocked by network safety policy",
        ),
        CalDavConnectorError::Client(moltis_caldav::Error::Timeout(_)) => {
            ErrorShape::new(error_codes::TIMEOUT, "CalDAV server timed out")
        },
        CalDavConnectorError::Client(moltis_caldav::Error::InvalidUrl(message))
        | CalDavConnectorError::Client(moltis_caldav::Error::Validation(message)) => {
            ErrorShape::new(error_codes::INVALID_REQUEST, message)
        },
        CalDavConnectorError::Client(moltis_caldav::Error::Protocol(message))
            if message.contains("401") || message.contains("Unauthorized") =>
        {
            ErrorShape::new(
                error_codes::INVALID_REQUEST,
                "CalDAV authentication failed; check the username and app password",
            )
        },
        CalDavConnectorError::Client(moltis_caldav::Error::Protocol(message))
            if message.contains("403") || message.contains("Forbidden") =>
        {
            ErrorShape::new(
                error_codes::INVALID_REQUEST,
                "CalDAV account does not have access to this server",
            )
        },
        CalDavConnectorError::Client(_) | CalDavConnectorError::ServerResponse(_) => {
            ErrorShape::new(error_codes::UNAVAILABLE, "CalDAV server request failed")
        },
        CalDavConnectorError::Serialization(_) => {
            ErrorShape::new(error_codes::INTERNAL, "failed to store CalDAV data")
        },
    }
}

fn default_limit() -> u64 {
    100
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn registers_all_connector_methods() {
        let registry = MethodRegistry::new();
        let names = registry.method_names();
        for method in [
            "connectors.available",
            "connectors.accounts.list",
            "connectors.channel_sources.list",
            "connectors.accounts.add",
            "connectors.accounts.update",
            "connectors.accounts.remove",
            "connectors.accounts.test",
            "connectors.datasets.list",
            "connectors.datasets.compile",
            "connectors.datasets.add",
            "connectors.datasets.update",
            "connectors.datasets.remove",
            "connectors.datasets.sync",
            "connectors.runs.list",
            "connectors.items.query",
        ] {
            assert!(names.iter().any(|name| name == method), "missing {method}");
        }
    }

    #[test]
    fn account_request_password_deserializes_as_a_secret() {
        let result = parse(json!({
            "kind": "caldav",
            "name": "Calendar",
            "serverUrl": "https://calendar.example.com",
            "username": "user@example.com",
            "password": "private-password"
        }));
        let request: crate::connectors::AccountCreateRequest = match result {
            Ok(request) => request,
            Err(_) => panic!("connector account request should deserialize"),
        };
        assert_eq!(request.password.expose_secret(), "private-password");
    }
}
