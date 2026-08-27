use serde_json::{Value, json};

fn time_field(description: &str) -> Value {
    json!({
        "type": ["integer", "string"],
        "description": description
    })
}

fn job_schema() -> Value {
    json!({
        "type": "object",
        "description": "Job specification for the add action",
        "properties": {
            "name": { "type": "string", "description": "Human-readable job name" },
            "schedule": {
                "type": ["object", "string", "integer"],
                "description": "For one-off jobs use {kind:'at', delay_ms}; never compute at_ms yourself. For recurring jobs use {kind:'every', every_ms} or {kind:'cron', expr, tz?}. Prefer weekday names such as MON-FRI because numeric mappings differ across cron implementations.",
                "properties": {
                    "kind": { "type": "string", "enum": ["at", "every", "cron"] },
                    "delay_ms": time_field("Milliseconds from now. Accepts an integer or duration such as '10m'. Preferred over at_ms."),
                    "at_ms": time_field("Absolute epoch milliseconds or ISO-8601 timestamp. Prefer delay_ms unless the timestamp is exact."),
                    "every_ms": time_field("Recurring interval. Accepts an integer or duration such as '15m'."),
                    "anchor_ms": time_field("Optional interval anchor as epoch milliseconds or ISO-8601 timestamp."),
                    "expr": { "type": "string", "description": "Cron expression. Prefer weekday names such as MON-FRI over numbers." },
                    "tz": { "type": "string", "description": "Optional timezone for a cron expression." }
                },
                "required": ["kind"]
            },
            "payload": {
                "type": ["object", "string"],
                "description": "Use {kind:'systemEvent', text} for main-session reminders or {kind:'agentTurn', message}. In a messaging conversation, deliver_to_current_chat:true routes the agentTurn back to that exact conversation without copying identifiers.",
                "properties": {
                    "kind": { "type": "string", "enum": ["systemEvent", "agentTurn"] },
                    "text": { "type": "string" },
                    "message": { "type": "string" },
                    "model": { "type": "string" },
                    "timeout_secs": {
                        "type": ["integer", "string"],
                        "description": "Optional timeout in seconds or duration such as '2m'."
                    },
                    "active_tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tool whitelist for the scheduled agent."
                    },
                    "tool_choice": {
                        "type": "object",
                        "description": "Optional provider tool choice.",
                        "properties": {
                            "type": { "type": "string", "enum": ["auto", "any", "none", "tool"] },
                            "name": { "type": "string" }
                        },
                        "required": ["type"]
                    },
                    "deliver_to_current_chat": {
                        "type": "boolean",
                        "description": "Deliver to the current messaging conversation. The host resolves the trusted destination and forces isolated agent execution."
                    },
                    "deliver": {
                        "type": "boolean",
                        "description": "Deliver after the run. Requires channel and to unless deliver_to_current_chat is true."
                    },
                    "channel": {
                        "type": "string",
                        "description": "Channel account for explicit delivery."
                    },
                    "to": {
                        "type": "string",
                        "description": "Recipient address for explicit delivery."
                    }
                },
                "required": ["kind"]
            },
            "sessionTarget": {
                "type": "string",
                "enum": ["main", "isolated"],
                "default": "isolated"
            },
            "sandbox": {
                "type": "object",
                "description": "Execution environment. Use {enabled:false} for host or {enabled:true, image?} for sandbox.",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "image": { "type": "string" }
                }
            },
            "execution": {
                "type": ["object", "string"],
                "description": "Alias for sandbox settings. Accepts 'host', 'sandbox', or {target, image?}.",
                "properties": {
                    "target": { "type": "string", "enum": ["host", "sandbox"] },
                    "image": { "type": "string" }
                }
            },
            "deleteAfterRun": { "type": "boolean", "default": false },
            "enabled": { "type": "boolean", "default": true },
            "wakeMode": {
                "type": "string",
                "enum": ["now", "nextHeartbeat"],
                "default": "nextHeartbeat"
            }
        },
        "required": ["name", "schedule", "payload"]
    })
}

pub(super) fn parameters() -> Value {
    let job = job_schema();
    let mut patch = job.clone();
    if let Some(schema) = patch.as_object_mut() {
        schema.insert(
            "description".to_string(),
            json!("Fields to update for the update action"),
        );
        schema.remove("required");
    }

    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["status", "list", "add", "update", "remove", "run", "runs"],
                "description": "The action to perform"
            },
            "job": job,
            "patch": patch,
            "id": {
                "type": "string",
                "description": "Job ID for update, remove, run, or runs"
            },
            "force": {
                "type": "boolean",
                "description": "Force-run a disabled job"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum run records to return; defaults to 20"
            }
        },
        "required": ["action"]
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn update_patch_exposes_mutable_job_fields() {
        let schema = parameters();
        let patch = &schema["properties"]["patch"];

        assert!(patch["required"].is_null());
        for field in ["name", "schedule", "payload", "sessionTarget", "enabled"] {
            assert!(
                patch["properties"].get(field).is_some(),
                "missing patch field {field}"
            );
        }
    }

    #[test]
    fn strict_mode_keeps_patch_fields_usable() {
        let mut schema = parameters();
        moltis_providers::openai_compat::patch_schema_for_strict_mode(&mut schema);
        let patch = &schema["properties"]["patch"];

        assert_eq!(patch["additionalProperties"], false);
        assert!(
            patch["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == "payload")
        );
        assert!(
            patch["properties"]["payload"]["properties"]
                .get("deliver_to_current_chat")
                .is_some()
        );
    }
}
