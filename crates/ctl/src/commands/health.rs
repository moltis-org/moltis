//! Health and status subcommands.

use crate::client::CtlClient;

pub async fn health(client: &mut CtlClient) -> anyhow::Result<serde_json::Value> {
    client
        .call("health", serde_json::Value::Null)
        .await
        .map_err(Into::into)
}

pub async fn status(client: &mut CtlClient) -> anyhow::Result<serde_json::Value> {
    client
        .call("status", serde_json::Value::Null)
        .await
        .map_err(Into::into)
}
