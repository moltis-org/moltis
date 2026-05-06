//! Configuration subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Get current configuration.
    Get {
        /// Optional key to retrieve a specific section.
        #[arg(long)]
        key: Option<String>,
    },
    /// Set a configuration value.
    Set {
        /// Configuration key (dot-separated path).
        #[arg(long)]
        key: String,
        /// Value to set (JSON).
        #[arg(long)]
        value: String,
    },
}

pub async fn run(client: &mut CtlClient, cmd: ConfigCommand) -> anyhow::Result<Value> {
    match cmd {
        ConfigCommand::Get { key } => {
            let params = match key {
                Some(k) => json!({ "key": k }),
                None => Value::Null,
            };
            client.call("config.get", params).await.map_err(Into::into)
        },
        ConfigCommand::Set { key, value } => {
            let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::String(value));
            client
                .call("config.set", json!({ "key": key, "value": parsed }))
                .await
                .map_err(Into::into)
        },
    }
}
