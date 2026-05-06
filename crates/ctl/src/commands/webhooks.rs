//! Webhook management subcommands.

use {
    clap::Subcommand,
    serde_json::{Value, json},
};

use crate::client::CtlClient;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
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
        /// Event types to accept (comma-separated, e.g. "pull_request,issues").
        #[arg(long)]
        events: Option<String>,
        /// Skip agent, deliver rendered template directly to a channel.
        #[arg(long)]
        deliver_only: bool,
        /// Template with {dot.notation} variables from the payload.
        #[arg(long)]
        prompt_template: Option<String>,
        /// Target channel for deliver_only mode (telegram, discord, slack, etc.).
        #[arg(long)]
        deliver_to: Option<String>,
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
            events,
            deliver_only,
            prompt_template,
            deliver_to,
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
                let obj = p.as_object_mut().unwrap_or_else(|| unreachable!());
                if let Some(sp) = system_prompt {
                    obj.insert("system_prompt_suffix".into(), json!(sp));
                }
                if let Some(ev) = events {
                    let allow: Vec<&str> = ev.split(',').map(str::trim).collect();
                    obj.insert("event_filter".into(), json!({ "allow": allow }));
                }
                if deliver_only {
                    obj.insert("deliver_only".into(), json!(true));
                }
                if let Some(pt) = prompt_template {
                    obj.insert("prompt_template".into(), json!(pt));
                }
                if let Some(dt) = deliver_to {
                    obj.insert("deliver_to".into(), json!(dt));
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
