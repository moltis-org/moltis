//! Agent-callable webhook tool for managing webhook subscriptions.

use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_service_traits::WebhooksService,
    serde_json::{Value, json},
};

/// The webhook management tool exposed to LLM agents.
pub struct WebhookTool {
    service: Arc<dyn WebhooksService>,
}

impl WebhookTool {
    pub fn new(service: Arc<dyn WebhooksService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for WebhookTool {
    fn name(&self) -> &str {
        "webhook"
    }

    fn description(&self) -> &str {
        "Manage webhook subscriptions. External services (GitHub, GitLab, Stripe, etc.) \
         POST events to Moltis, which either runs an agent in response or forwards the \
         event directly to a channel (deliver_only mode, zero LLM tokens).\n\n\
         Actions:\n\
         - list: List all webhooks\n\
         - create: Create a new webhook endpoint\n\
         - get: Get webhook details by ID\n\
         - update: Update webhook settings\n\
         - delete: Delete a webhook\n\
         - profiles: List available source profiles\n\
         - deliveries: View delivery history for a webhook"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "get", "update", "delete", "profiles", "deliveries"],
                    "description": "The operation to perform."
                },
                "id": {
                    "type": "integer",
                    "description": "Webhook ID (for get, update, delete)."
                },
                "name": {
                    "type": "string",
                    "description": "Webhook name (for create)."
                },
                "source_profile": {
                    "type": "string",
                    "enum": ["generic", "github", "gitlab", "stripe", "linear", "pagerduty", "sentry"],
                    "description": "Source profile (for create). Default: generic."
                },
                "auth_mode": {
                    "type": "string",
                    "enum": ["none", "static_header", "bearer", "github_hmac_sha256", "gitlab_token",
                             "stripe_webhook_signature", "linear_webhook_signature",
                             "pagerduty_v2_signature", "sentry_webhook_signature"],
                    "description": "Authentication mode (for create)."
                },
                "events": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Event types to accept (for create). Empty means accept all."
                },
                "system_prompt_suffix": {
                    "type": "string",
                    "description": "Extra text appended to the agent system prompt."
                },
                "deliver_only": {
                    "type": "boolean",
                    "description": "When true, skip the agent and forward the rendered template directly \
                                    to a channel. Zero LLM tokens, sub-second delivery."
                },
                "prompt_template": {
                    "type": "string",
                    "description": "Template with {dot.notation} variables from the payload. \
                                    Example: 'Issue #{issue.number}: {issue.title}'"
                },
                "deliver_to": {
                    "type": "string",
                    "description": "Target channel for deliver_only mode (telegram, discord, slack, etc.)."
                },
                "webhook_id": {
                    "type": "integer",
                    "description": "Webhook ID (for deliveries)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (for deliveries). Default: 20."
                },
                "patch": {
                    "type": "object",
                    "description": "Fields to update (for update). Any subset of webhook fields."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        match action {
            "list" => self
                .service
                .list()
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),

            "profiles" => self
                .service
                .profiles()
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),

            "get" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for get"))?;
                self.service
                    .get(json!({ "id": id }))
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },

            "create" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'name' for create"))?;

                let mut create_params = json!({
                    "name": name,
                    "source_profile": params.get("source_profile").and_then(|v| v.as_str()).unwrap_or("generic"),
                    "auth_mode": params.get("auth_mode").and_then(|v| v.as_str()).unwrap_or("none"),
                    "session_mode": params.get("session_mode").and_then(|v| v.as_str()).unwrap_or("per_delivery"),
                });

                let obj = create_params
                    .as_object_mut()
                    .unwrap_or_else(|| unreachable!());

                if let Some(events) = params.get("events").and_then(|v| v.as_array()) {
                    let allow: Vec<&str> = events.iter().filter_map(|v| v.as_str()).collect();
                    obj.insert("event_filter".into(), json!({ "allow": allow }));
                }
                if let Some(suffix) = params.get("system_prompt_suffix").and_then(|v| v.as_str()) {
                    obj.insert("system_prompt_suffix".into(), json!(suffix));
                }
                if let Some(true) = params.get("deliver_only").and_then(|v| v.as_bool()) {
                    obj.insert("deliver_only".into(), json!(true));
                }
                if let Some(pt) = params.get("prompt_template").and_then(|v| v.as_str()) {
                    obj.insert("prompt_template".into(), json!(pt));
                }
                if let Some(dt) = params.get("deliver_to").and_then(|v| v.as_str()) {
                    obj.insert("deliver_to".into(), json!(dt));
                }

                self.service
                    .create(create_params)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },

            "update" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for update"))?;
                let patch = params.get("patch").cloned().unwrap_or(json!({}));
                self.service
                    .update(json!({ "id": id, "patch": patch }))
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },

            "delete" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for delete"))?;
                self.service
                    .delete(json!({ "id": id }))
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },

            "deliveries" => {
                let webhook_id = params
                    .get("webhook_id")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| anyhow::anyhow!("missing 'webhook_id' for deliveries"))?;
                let limit = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
                self.service
                    .deliveries(json!({ "webhookId": webhook_id, "limit": limit }))
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },

            other => Err(anyhow::anyhow!(
                "unknown action '{other}'. Use: list, create, get, update, delete, profiles, deliveries"
            )),
        }
    }
}
