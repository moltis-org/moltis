//! Provider management subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum ProvidersCommand {
    /// List available providers and their configuration status.
    List,
    /// Save an API key for a provider.
    SaveKey {
        /// Provider name (e.g. openai, anthropic, google).
        #[arg(long)]
        provider: String,
        /// API key value.
        #[arg(long)]
        key: String,
    },
    /// Validate a provider's API key.
    Validate {
        /// Provider name.
        #[arg(long)]
        provider: String,
    },
    /// Remove a provider's API key.
    RemoveKey {
        /// Provider name.
        #[arg(long)]
        provider: String,
    },
}

pub async fn run(client: &mut CtlClient, cmd: ProvidersCommand) -> anyhow::Result<Value> {
    match cmd {
        ProvidersCommand::List => client
            .call("providers.available", Value::Null)
            .await
            .map_err(Into::into),
        ProvidersCommand::SaveKey { provider, key } => client
            .call(
                "providers.save_key",
                json!({ "provider": provider, "key": key }),
            )
            .await
            .map_err(Into::into),
        ProvidersCommand::Validate { provider } => client
            .call("providers.validate_key", json!({ "provider": provider }))
            .await
            .map_err(Into::into),
        ProvidersCommand::RemoveKey { provider } => client
            .call("providers.remove_key", json!({ "provider": provider }))
            .await
            .map_err(Into::into),
    }
}
