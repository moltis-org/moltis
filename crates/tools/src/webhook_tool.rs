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

fn object_or_json_string_schema(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [
            { "type": "object" },
            {
                "type": "string",
                "description": "JSON-encoded object; use this form with strict tool-calling providers."
            },
            { "type": "null" }
        ]
    })
}

fn webhook_patch_parameter_schema() -> Value {
    json!({
        "type": "object",
        "description": "Fields to update (for update). Any subset of webhook fields.",
        "properties": {
            "name": { "type": "string" },
            "description": { "type": ["string", "null"] },
            "enabled": { "type": "boolean" },
            "agentId": { "type": ["string", "null"] },
            "model": { "type": ["string", "null"] },
            "systemPromptSuffix": { "type": ["string", "null"] },
            "toolPolicy": {
                "type": ["object", "null"],
                "properties": {
                    "allow": { "type": "array", "items": { "type": "string" } },
                    "deny": { "type": "array", "items": { "type": "string" } }
                }
            },
            "authMode": {
                "type": "string",
                "enum": ["none", "static_header", "bearer", "github_hmac_sha256", "gitlab_token", "stripe_webhook_signature", "linear_webhook_signature", "pagerduty_v2_signature", "sentry_webhook_signature"]
            },
            "authConfig": object_or_json_string_schema("Authentication configuration object."),
            "sourceConfig": object_or_json_string_schema("Source-profile configuration object."),
            "eventFilter": {
                "type": "object",
                "properties": {
                    "allow": { "type": "array", "items": { "type": "string" } },
                    "deny": { "type": "array", "items": { "type": "string" } }
                }
            },
            "sessionMode": { "type": "string", "enum": ["per_delivery", "per_entity", "named_session"] },
            "namedSessionKey": { "type": ["string", "null"] },
            "allowedCidrs": { "type": "array", "items": { "type": "string" } },
            "maxBodyBytes": { "type": "integer" },
            "rateLimitPerMinute": { "type": "integer" },
            "deliverOnly": { "type": "boolean" },
            "promptTemplate": { "type": ["string", "null"] },
            "deliverTo": { "type": ["string", "null"] },
            "deliverExtra": object_or_json_string_schema("Extra channel delivery configuration object.")
        }
    })
}

fn normalize_stringified_object_fields(value: &mut Value, fields: &[&str]) -> anyhow::Result<()> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    for field in fields {
        let Some(raw) = object.get_mut(*field) else {
            continue;
        };
        let Value::String(encoded) = raw else {
            continue;
        };
        let decoded: Value = serde_json::from_str(encoded)
            .map_err(|error| anyhow::anyhow!("invalid JSON object in '{field}': {error}"))?;
        if !decoded.is_object() {
            return Err(anyhow::anyhow!("'{field}' must encode a JSON object"));
        }
        *raw = decoded;
    }
    Ok(())
}

fn normalize_nullable_filter_arrays(value: &mut Value) {
    let Some(patch) = value.as_object_mut() else {
        return;
    };
    for field in ["toolPolicy", "eventFilter"] {
        let Some(filter) = patch.get_mut(field).and_then(Value::as_object_mut) else {
            continue;
        };
        for array_field in ["allow", "deny"] {
            if filter.get(array_field).is_some_and(Value::is_null) {
                filter.remove(array_field);
            }
        }
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
                "patch": webhook_patch_parameter_schema()
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
                let mut patch = params.get("patch").cloned().unwrap_or(json!({}));
                normalize_stringified_object_fields(&mut patch, &[
                    "authConfig",
                    "sourceConfig",
                    "deliverExtra",
                ])?;
                normalize_nullable_filter_arrays(&mut patch);
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

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_patch_schema_preserves_supported_shapes() {
        let patch = webhook_patch_parameter_schema();
        let properties = &patch["properties"];
        assert!(
            properties.is_object(),
            "webhook patch schema must declare properties"
        );

        assert_eq!(properties["name"]["type"], "string");
        assert_eq!(properties["eventFilter"]["type"], "object");
        assert!(
            properties["eventFilter"]["properties"]
                .get("allow")
                .is_some()
        );

        for field in ["authConfig", "sourceConfig", "deliverExtra"] {
            let supports_json_string = properties[field]["anyOf"]
                .as_array()
                .is_some_and(|variants| variants.iter().any(|schema| schema["type"] == "string"));
            assert!(
                supports_json_string,
                "webhook patch field '{field}' must accept a JSON string"
            );
        }
    }

    #[test]
    fn normalizes_json_encoded_webhook_config_objects() {
        let mut patch = json!({
            "authConfig": "{\"header\":\"x-secret\"}",
            "deliverExtra": {"chat_id": "123"}
        });

        normalize_stringified_object_fields(&mut patch, &[
            "authConfig",
            "sourceConfig",
            "deliverExtra",
        ])
        .unwrap();

        assert_eq!(patch["authConfig"]["header"], "x-secret");
        assert_eq!(patch["deliverExtra"]["chat_id"], "123");
    }

    #[test]
    fn removes_strict_mode_nulls_from_webhook_filter_arrays() {
        let mut patch = json!({
            "description": null,
            "toolPolicy": { "allow": null, "deny": ["shell"] },
            "eventFilter": { "allow": ["push"], "deny": null }
        });

        normalize_nullable_filter_arrays(&mut patch);

        assert!(patch["description"].is_null());
        assert!(patch["toolPolicy"].get("allow").is_none());
        assert_eq!(patch["toolPolicy"]["deny"], json!(["shell"]));
        assert_eq!(patch["eventFilter"]["allow"], json!(["push"]));
        assert!(patch["eventFilter"].get("deny").is_none());
    }
}
