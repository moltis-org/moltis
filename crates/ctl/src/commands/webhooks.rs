//! Webhook management subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
pub enum WebhooksCommand {
    /// List all webhooks.
    List,
    /// Get webhook details.
    Get {
        /// Webhook ID.
        #[arg(long)]
        id: i64,
    },
    /// Create a new webhook.
    Create {
        /// Webhook name.
        #[arg(long)]
        name: String,
        /// Source profile (github, gitlab, stripe, linear, pagerduty, sentry, generic).
        #[arg(long, default_value = "generic")]
        source_profile: String,
        /// Auth mode (none, bearer, github_hmac_sha256, etc.).
        #[arg(long, default_value = "none")]
        auth_mode: String,
        /// Session mode (per_delivery, per_entity, named_session).
        #[arg(long, default_value = "per_delivery")]
        session_mode: String,
        /// System prompt suffix for agent runs.
        #[arg(long)]
        system_prompt: Option<String>,
        /// Full JSON params (overrides individual flags).
        #[arg(long)]
        json: Option<String>,
    },
    /// Delete a webhook.
    Delete {
        /// Webhook ID.
        #[arg(long)]
        id: i64,
    },
    /// List available source profiles.
    Profiles,
    /// View delivery history for a webhook.
    Deliveries {
        /// Webhook ID.
        #[arg(long, rename_all = "camelCase")]
        webhook_id: i64,
        /// Max results.
        #[arg(long, default_value = "50")]
        limit: i64,
    },
}

pub async fn run(client: &mut CtlClient, cmd: WebhooksCommand) -> anyhow::Result<Value> {
    match cmd {
        WebhooksCommand::List => client
            .call("webhooks.list", Value::Null)
            .await
            .map_err(Into::into),
        WebhooksCommand::Get { id } => client
            .call("webhooks.get", json!({ "id": id }))
            .await
            .map_err(Into::into),
        WebhooksCommand::Create {
            name,
            source_profile,
            auth_mode,
            session_mode,
            system_prompt,
            json: json_override,
        } => {
            let params = if let Some(raw) = json_override {
                serde_json::from_str(&raw)?
            } else {
                let mut p = json!({
                    "name": name,
                    "source_profile": source_profile,
                    "auth_mode": auth_mode,
                    "session_mode": session_mode,
                });
                if let Some(sp) = system_prompt {
                    p.as_object_mut()
                        .unwrap_or_else(|| unreachable!())
                        .insert("system_prompt_suffix".into(), json!(sp));
                }
                p
            };
            client
                .call("webhooks.create", params)
                .await
                .map_err(Into::into)
        },
        WebhooksCommand::Delete { id } => client
            .call("webhooks.delete", json!({ "id": id }))
            .await
            .map_err(Into::into),
        WebhooksCommand::Profiles => client
            .call("webhooks.profiles", Value::Null)
            .await
            .map_err(Into::into),
        WebhooksCommand::Deliveries { webhook_id, limit } => client
            .call(
                "webhooks.deliveries",
                json!({ "webhookId": webhook_id, "limit": limit }),
            )
            .await
            .map_err(Into::into),
    }
}
