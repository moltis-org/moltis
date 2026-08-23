use {
    moltis_common::hooks::ChannelBinding,
    serde_json::{Map, Value, json},
};

use crate::{Result, error::Error};

struct DeliveryTarget {
    account_id: String,
    to: String,
}

fn shortcut_enabled(payload: &mut Map<String, Value>) -> Result<bool> {
    if !payload.contains_key("deliver_to_current_chat")
        && let Some(value) = payload.remove("deliverToCurrentChat")
    {
        payload.insert("deliver_to_current_chat".to_string(), value);
    }

    let Some(value) = payload.remove("deliver_to_current_chat") else {
        return Ok(false);
    };
    value
        .as_bool()
        .ok_or_else(|| Error::message("payload.deliver_to_current_chat must be a boolean"))
}

fn delivery_target(channel_context: Option<&Value>) -> Result<DeliveryTarget> {
    let value = channel_context.ok_or_else(|| {
        Error::message(
            "payload.deliver_to_current_chat requires a current messaging channel; use explicit payload.channel and payload.to outside a channel conversation",
        )
    })?;
    let binding: ChannelBinding = serde_json::from_value(value.clone())
        .map_err(|error| Error::message(format!("invalid current channel context: {error}")))?;
    let account_id = required_text(
        binding.account_id,
        "current channel has no delivery account",
    )?;
    let to = required_text(
        binding.outbound_to.or(binding.chat_id),
        "current channel has no delivery destination",
    )?;
    Ok(DeliveryTarget { account_id, to })
}

fn required_text(value: Option<String>, error: &str) -> Result<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::message(error))
}

fn reject_conflicting_destination(
    payload: &Map<String, Value>,
    target: &DeliveryTarget,
) -> Result<()> {
    for (field, expected) in [
        ("channel", target.account_id.as_str()),
        ("to", target.to.as_str()),
    ] {
        if let Some(actual) = payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && actual != expected
        {
            return Err(Error::message(format!(
                "payload.{field} conflicts with the current channel destination"
            )));
        }
    }
    Ok(())
}

pub(super) fn apply(job: &mut Map<String, Value>, channel_context: Option<&Value>) -> Result<()> {
    let Some(Value::Object(payload)) = job.get_mut("payload") else {
        return Ok(());
    };
    if !shortcut_enabled(payload)? {
        return Ok(());
    }

    let target = delivery_target(channel_context)?;
    reject_conflicting_destination(payload, &target)?;
    if !payload.contains_key("message")
        && let Some(text) = payload.get("text").cloned()
    {
        payload.insert("message".to_string(), text);
    }
    payload.insert("kind".to_string(), json!("agentTurn"));
    payload.insert("deliver".to_string(), json!(true));
    payload.insert("channel".to_string(), json!(target.account_id));
    payload.insert("to".to_string(), json!(target.to));
    job.insert("sessionTarget".to_string(), json!("isolated"));
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

    use {
        moltis_agents::tool_registry::AgentTool,
        moltis_cron::{
            service::{AgentTurnFn, CronService, SystemEventFn},
            store_memory::InMemoryStore,
        },
    };

    use super::*;

    fn cron_tool() -> super::super::CronTool {
        let system_event: SystemEventFn = Arc::new(|_| {});
        let agent_turn: AgentTurnFn = Arc::new(|_| {
            Box::pin(async {
                Ok(moltis_cron::service::AgentTurnResult {
                    output: "ok".into(),
                    input_tokens: None,
                    output_tokens: None,
                    session_key: None,
                    delivery_error: None,
                })
            })
        });
        let service = CronService::new(Arc::new(InMemoryStore::new()), system_event, agent_turn);
        super::super::CronTool::new(service)
    }

    fn channel(outbound_to: Option<&str>) -> Value {
        json!({
            "channel_type": "whatsapp",
            "account_id": "main",
            "chat_id": "5511999999999",
            "outbound_to": outbound_to
        })
    }

    #[test]
    fn resolves_current_chat_and_forces_agent_turn() {
        let mut job = json!({
            "sessionTarget": "main",
            "payload": {
                "kind": "systemEvent",
                "text": "Send the report",
                "deliver_to_current_chat": true
            }
        });

        apply(job.as_object_mut().unwrap(), Some(&channel(None))).unwrap();

        assert_eq!(job["sessionTarget"], "isolated");
        assert_eq!(job["payload"]["kind"], "agentTurn");
        assert_eq!(job["payload"]["message"], "Send the report");
        assert_eq!(job["payload"]["deliver"], true);
        assert_eq!(job["payload"]["channel"], "main");
        assert_eq!(job["payload"]["to"], "5511999999999");
        assert!(job["payload"].get("deliver_to_current_chat").is_none());
    }

    #[test]
    fn preserves_threaded_outbound_destination() {
        let mut job = json!({
            "payload": {
                "message": "Send the report",
                "deliverToCurrentChat": true
            }
        });

        apply(
            job.as_object_mut().unwrap(),
            Some(&channel(Some("-100123:42"))),
        )
        .unwrap();

        assert_eq!(job["payload"]["to"], "-100123:42");
    }

    #[test]
    fn rejects_missing_or_conflicting_destination() {
        let original = json!({
            "payload": {
                "message": "Send the report",
                "deliver_to_current_chat": true,
                "to": "someone-else"
            }
        });
        let mut no_context = original.clone();
        let error = apply(no_context.as_object_mut().unwrap(), None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires a current messaging channel")
        );

        let mut conflict = original;
        let error = apply(conflict.as_object_mut().unwrap(), Some(&channel(None))).unwrap_err();
        assert!(error.to_string().contains("conflicts"));
    }

    #[test]
    fn false_shortcut_is_removed_without_changing_routing() {
        let mut job = json!({
            "sessionTarget": "main",
            "payload": {
                "kind": "systemEvent",
                "text": "Keep local",
                "deliver_to_current_chat": false
            }
        });

        apply(job.as_object_mut().unwrap(), None).unwrap();

        assert_eq!(job["sessionTarget"], "main");
        assert!(job["payload"].get("deliver_to_current_chat").is_none());
        assert!(job["payload"].get("deliver").is_none());
    }

    #[tokio::test]
    async fn tool_run_and_strict_update_return_expected_results() {
        let tool = cron_tool();
        let added = tool
            .execute(json!({
                "action": "add",
                "_channel": channel(Some("5511999999999")),
                "job": {
                    "name": "daily report",
                    "schedule": { "kind": "every", "every_ms": 60000 },
                    "payload": {
                        "kind": "systemEvent",
                        "text": "Old report",
                        "deliver_to_current_chat": true
                    },
                    "sessionTarget": "main"
                }
            }))
            .await
            .unwrap();

        assert_eq!(added["sessionTarget"], "isolated");
        assert_eq!(added["payload"]["kind"], "agentTurn");
        assert_eq!(added["payload"]["message"], "Old report");
        assert_eq!(added["payload"]["deliver"], true);
        assert_eq!(added["payload"]["channel"], "main");
        assert_eq!(added["payload"]["to"], "5511999999999");

        let run = tool
            .execute(json!({ "action": "run", "id": added["id"] }))
            .await
            .unwrap();
        assert_eq!(run["run"]["status"], "ok");
        assert_eq!(run["run"]["output"], "ok");

        let updated = tool
            .execute(json!({
                "action": "update",
                "id": added["id"],
                "_channel": channel(Some("5511999999999")),
                "patch": {
                    "name": null,
                    "schedule": null,
                    "payload": {
                        "kind": "systemEvent",
                        "text": "Send the new report",
                        "message": null,
                        "model": null,
                        "timeout_secs": null,
                        "active_tools": null,
                        "tool_choice": null,
                        "deliver_to_current_chat": true,
                        "deliver": null,
                        "channel": null,
                        "to": null
                    },
                    "sessionTarget": "main",
                    "sandbox": null,
                    "execution": null,
                    "deleteAfterRun": null,
                    "enabled": null,
                    "wakeMode": null
                }
            }))
            .await
            .unwrap();

        assert_eq!(updated["sessionTarget"], "isolated");
        assert_eq!(updated["payload"]["kind"], "agentTurn");
        assert_eq!(updated["payload"]["message"], "Send the new report");
        assert_eq!(updated["payload"]["deliver"], true);
        assert_eq!(updated["payload"]["channel"], "main");
        assert_eq!(updated["payload"]["to"], "5511999999999");
    }
}
