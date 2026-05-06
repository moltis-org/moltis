#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use moltis_cron::{
    service::{AgentTurnFn, CronService, SystemEventFn},
    store_memory::InMemoryStore,
};

use super::*;

fn noop_sys() -> SystemEventFn {
    Arc::new(|_| {})
}

fn noop_agent() -> AgentTurnFn {
    Arc::new(|_| {
        Box::pin(async {
            Ok(moltis_cron::service::AgentTurnResult {
                output: "ok".into(),
                input_tokens: None,
                output_tokens: None,
                session_key: None,
            })
        })
    })
}

fn make_tool() -> CronTool {
    let store = Arc::new(InMemoryStore::new());
    let svc = CronService::new(store, noop_sys(), noop_agent());
    CronTool::new(svc)
}

#[tokio::test]
async fn test_status() {
    let tool = make_tool();
    let result = tool.execute(json!({ "action": "status" })).await.unwrap();
    assert_eq!(result["running"], false);
}

#[tokio::test]
async fn test_list_empty() {
    let tool = make_tool();
    let result = tool.execute(json!({ "action": "list" })).await.unwrap();
    assert!(result.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_add_and_list() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "test job",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "do stuff" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert!(add_result.get("id").is_some());

    let list = tool.execute(json!({ "action": "list" })).await.unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_remove() {
    let tool = make_tool();
    let add = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "to remove",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "x" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    let id = add["id"].as_str().unwrap();
    let result = tool
        .execute(json!({ "action": "remove", "id": id }))
        .await
        .unwrap();
    assert_eq!(result["removed"].as_str().unwrap(), id);
}

#[tokio::test]
async fn test_unknown_action() {
    let tool = make_tool();
    let result = tool.execute(json!({ "action": "nope" })).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_runs_empty() {
    let tool = make_tool();
    let result = tool
        .execute(json!({ "action": "runs", "id": "nonexistent" }))
        .await
        .unwrap();
    assert!(result.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_add_accepts_cron_expression_string_schedule() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "news update",
                "schedule": "5 11 * * *",
                "payload": { "kind": "agentTurn", "message": "fetch weather and summarize" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["schedule"]["kind"], "cron");
    assert_eq!(add_result["schedule"]["expr"], "5 11 * * *");
}

#[tokio::test]
async fn test_add_infers_schedule_kind_from_expr_without_kind() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "daily digest",
                "schedule": { "expr": "0 9 * * *" },
                "payload": { "kind": "agentTurn", "message": "send daily digest" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["schedule"]["kind"], "cron");
    assert_eq!(add_result["schedule"]["expr"], "0 9 * * *");
}

#[tokio::test]
async fn test_add_infers_payload_kind_for_main_session() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "morning reminder",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "text": "Tell me today's weather." },
                "sessionTarget": "main"
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["payload"]["kind"], "systemEvent");
    assert_eq!(add_result["payload"]["text"], "Tell me today's weather.");
}

#[tokio::test]
async fn test_update_accepts_schedule_string_patch() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "to patch",
                "schedule": { "kind": "every", "every_ms": 300000 },
                "payload": { "kind": "agentTurn", "message": "run task" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();
    let id = add_result["id"].as_str().unwrap();

    let updated = tool
        .execute(json!({
            "action": "update",
            "id": id,
            "patch": { "schedule": "*/15 * * * *" }
        }))
        .await
        .unwrap();

    assert_eq!(updated["schedule"]["kind"], "cron");
    assert_eq!(updated["schedule"]["expr"], "*/15 * * * *");
}

#[tokio::test]
async fn test_add_accepts_alias_fields_and_duration_strings() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "alias fields",
                "session_target": "isolated",
                "schedule": { "kind": "interval", "everyMs": "5m" },
                "payload": { "kind": "agent_turn", "text": "do work", "timeoutSecs": "30s" }
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["sessionTarget"], "isolated");
    assert_eq!(add_result["schedule"]["kind"], "every");
    assert_eq!(add_result["schedule"]["every_ms"], 300000);
    assert_eq!(add_result["payload"]["kind"], "agentTurn");
    assert_eq!(add_result["payload"]["message"], "do work");
    assert_eq!(add_result["payload"]["timeout_secs"], 30);
}

#[tokio::test]
async fn test_add_accepts_execution_target_and_image() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "sandboxed run",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": {
                    "kind": "agentTurn",
                    "message": "run diagnostics",
                    "model": "gpt-5.2"
                },
                "execution": {
                    "target": "sandbox",
                    "image": "ubuntu:25.10"
                }
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["payload"]["model"], "gpt-5.2");
    assert_eq!(add_result["sandbox"]["enabled"], true);
    assert_eq!(add_result["sandbox"]["image"], "ubuntu:25.10");
}

#[tokio::test]
async fn test_add_accepts_delivery_fields_for_agent_turn() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "delivered run",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": {
                    "kind": "agentTurn",
                    "message": "post an update",
                    "deliver": true,
                    "channel": "bot-main",
                    "to": "123456"
                },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["payload"]["kind"], "agentTurn");
    assert_eq!(add_result["payload"]["deliver"], true);
    assert_eq!(add_result["payload"]["channel"], "bot-main");
    assert_eq!(add_result["payload"]["to"], "123456");
}

#[tokio::test]
async fn test_update_accepts_host_execution_string() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "switch execution",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "run task" },
                "sandbox": { "enabled": true, "image": "ubuntu:25.10" }
            }
        }))
        .await
        .unwrap();
    let id = add_result["id"].as_str().unwrap();

    let updated = tool
        .execute(json!({
            "action": "update",
            "id": id,
            "patch": { "execution": "host" }
        }))
        .await
        .unwrap();

    assert_eq!(updated["sandbox"]["enabled"], false);
    assert!(updated["sandbox"]["image"].is_null());
}

#[tokio::test]
async fn test_update_accepts_delivery_fields_in_patch() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "toggle delivery",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "run task" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();
    let id = add_result["id"].as_str().unwrap();

    let updated = tool
        .execute(json!({
            "action": "update",
            "id": id,
            "patch": {
                "payload": {
                    "kind": "agentTurn",
                    "message": "run task",
                    "deliver": true,
                    "channel": "bot-main",
                    "to": "123456"
                }
            }
        }))
        .await
        .unwrap();

    assert_eq!(updated["payload"]["deliver"], true);
    assert_eq!(updated["payload"]["channel"], "bot-main");
    assert_eq!(updated["payload"]["to"], "123456");
}

#[test]
fn test_parameters_schema_has_no_one_of() {
    fn contains_one_of(value: &Value) -> bool {
        match value {
            Value::Object(obj) => {
                if obj.contains_key("oneOf") {
                    return true;
                }
                obj.values().any(contains_one_of)
            },
            Value::Array(items) => items.iter().any(contains_one_of),
            _ => false,
        }
    }

    let tool = make_tool();
    let schema = tool.parameters_schema();
    assert!(
        !contains_one_of(&schema),
        "cron tool schema must avoid oneOf for OpenAI Responses API compatibility"
    );
}

#[tokio::test]
async fn test_add_accepts_payload_string_shorthand() {
    let tool = make_tool();
    let add_result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "string payload",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": "Summarize headlines",
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert_eq!(add_result["payload"]["kind"], "agentTurn");
    assert_eq!(add_result["payload"]["message"], "Summarize headlines");
}

#[tokio::test]
async fn test_add_rejects_ambiguous_schedule_without_kind() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "ambiguous",
                "schedule": {
                    "expr": "*/5 * * * *",
                    "every_ms": 60000
                },
                "payload": { "kind": "agentTurn", "message": "x" },
                "sessionTarget": "isolated"
            }
        }))
        .await;

    let err = result.unwrap_err().to_string();
    assert!(err.contains("ambiguous fields"), "unexpected error: {err}");
}

#[test]
fn test_normalize_wake_mode_aliases() {
    assert_eq!(normalize_wake_mode("now"), Some("now"));
    assert_eq!(normalize_wake_mode("immediate"), Some("now"));
    assert_eq!(normalize_wake_mode("immediately"), Some("now"));
    assert_eq!(normalize_wake_mode("NOW"), Some("now"));
    assert_eq!(normalize_wake_mode("nextHeartbeat"), Some("nextHeartbeat"));
    assert_eq!(normalize_wake_mode("next_heartbeat"), Some("nextHeartbeat"));
    assert_eq!(normalize_wake_mode("next-heartbeat"), Some("nextHeartbeat"));
    assert_eq!(normalize_wake_mode("next"), Some("nextHeartbeat"));
    assert_eq!(normalize_wake_mode("default"), Some("nextHeartbeat"));
    assert_eq!(normalize_wake_mode("bogus"), None);
}

#[tokio::test]
async fn test_add_with_wake_mode() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "wake test",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "go" },
                "wakeMode": "now"
            }
        }))
        .await
        .unwrap();
    assert_eq!(result["wakeMode"], "now");
}

#[tokio::test]
async fn test_add_with_wake_mode_alias() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "alias wake",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "go" },
                "wake_mode": "immediate"
            }
        }))
        .await
        .unwrap();
    assert_eq!(result["wakeMode"], "now");
}

#[tokio::test]
async fn test_update_wake_mode() {
    let tool = make_tool();
    let add = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "update wake",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "go" }
            }
        }))
        .await
        .unwrap();
    let id = add["id"].as_str().unwrap();

    let updated = tool
        .execute(json!({
            "action": "update",
            "id": id,
            "patch": { "wakeMode": "now" }
        }))
        .await
        .unwrap();
    assert_eq!(updated["wakeMode"], "now");
}

// --- Stringified-JSON rescue tests (issue #430) ---

#[tokio::test]
async fn test_add_accepts_stringified_job() {
    let tool = make_tool();
    let job_json = serde_json::to_string(&json!({
        "name": "stringified job",
        "schedule": { "kind": "every", "every_ms": 60000 },
        "payload": { "kind": "agentTurn", "message": "do stuff" },
        "sessionTarget": "isolated"
    }))
    .unwrap();

    let result = tool
        .execute(json!({ "action": "add", "job": job_json }))
        .await
        .unwrap();

    assert!(result.get("id").is_some());
    assert_eq!(result["name"], "stringified job");
}

#[tokio::test]
async fn test_add_accepts_flat_params_without_job_wrapper() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "add",
            "name": "flat params",
            "schedule": { "kind": "every", "every_ms": 60000 },
            "payload": { "kind": "agentTurn", "message": "run" },
            "sessionTarget": "isolated"
        }))
        .await
        .unwrap();

    assert!(result.get("id").is_some());
    assert_eq!(result["name"], "flat params");
}

#[tokio::test]
async fn test_add_accepts_stringified_nested_fields() {
    let tool = make_tool();
    let result = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "stringified nested",
                "schedule": r#"{"kind":"cron","expr":"0 9 * * 1"}"#,
                "payload": r#"{"kind":"agentTurn","message":"hello"}"#,
                "sandbox": r#"{"enabled":false}"#,
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();

    assert!(result.get("id").is_some());
    assert_eq!(result["schedule"]["kind"], "cron");
    assert_eq!(result["schedule"]["expr"], "0 9 * * 1");
    assert_eq!(result["payload"]["kind"], "agentTurn");
    assert_eq!(result["payload"]["message"], "hello");
    assert_eq!(result["sandbox"]["enabled"], false);
}

#[tokio::test]
async fn test_update_accepts_stringified_patch() {
    let tool = make_tool();
    let add = tool
        .execute(json!({
            "action": "add",
            "job": {
                "name": "to patch",
                "schedule": { "kind": "every", "every_ms": 60000 },
                "payload": { "kind": "agentTurn", "message": "x" },
                "sessionTarget": "isolated"
            }
        }))
        .await
        .unwrap();
    let id = add["id"].as_str().unwrap();

    let patch_json = serde_json::to_string(&json!({ "name": "patched" })).unwrap();
    let updated = tool
        .execute(json!({ "action": "update", "id": id, "patch": patch_json }))
        .await
        .unwrap();

    assert_eq!(updated["name"], "patched");
}

#[tokio::test]
async fn test_stringified_job_with_invalid_json_is_rejected() {
    let tool = make_tool();
    let result = tool
        .execute(json!({ "action": "add", "job": "not valid json {" }))
        .await;
    assert!(result.is_err());
}
