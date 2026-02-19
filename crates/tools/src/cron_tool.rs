//! Agent-callable cron tool for managing scheduled jobs.

use std::sync::Arc;

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    serde_json::{Map, Value, json},
};

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_cron::{
        parse::{parse_absolute_time_ms, parse_duration_ms},
        service::CronService,
        types::{CronJobCreate, CronJobPatch},
    },
};

/// The cron tool exposed to LLM agents.
pub struct CronTool {
    service: Arc<CronService>,
}

impl CronTool {
    pub fn new(service: Arc<CronService>) -> Self {
        Self { service }
    }
}

fn take_alias(obj: &mut Map<String, Value>, canonical: &str, aliases: &[&str]) {
    if obj.contains_key(canonical) {
        return;
    }
    for alias in aliases {
        if let Some(value) = obj.remove(*alias) {
            obj.insert(canonical.to_string(), value);
            return;
        }
    }
}

fn parse_epoch_millis(value: &Value, field: &str) -> Result<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("{field} must be a non-negative integer")),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                bail!("{field} cannot be empty");
            }
            if let Ok(v) = trimmed.parse::<u64>() {
                return Ok(v);
            }
            parse_absolute_time_ms(trimmed).map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))
        },
        _ => bail!("{field} must be an integer milliseconds value or ISO-8601 timestamp"),
    }
}

fn parse_interval_millis(value: &Value, field: &str) -> Result<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("{field} must be a non-negative integer")),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                bail!("{field} cannot be empty");
            }
            if let Ok(v) = trimmed.parse::<u64>() {
                return Ok(v);
            }
            parse_duration_ms(trimmed).map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))
        },
        _ => bail!("{field} must be an integer milliseconds value or duration string"),
    }
}

fn parse_timeout_seconds(value: &Value, field: &str) -> Result<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("{field} must be a non-negative integer")),
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                bail!("{field} cannot be empty");
            }
            if let Ok(v) = trimmed.parse::<u64>() {
                return Ok(v);
            }
            let ms =
                parse_duration_ms(trimmed).map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))?;
            Ok(ms.saturating_div(1_000))
        },
        _ => bail!("{field} must be a number of seconds or duration string"),
    }
}

fn normalize_schedule_kind(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "at" | "once" | "oneshot" | "one-shot" => Some("at"),
        "every" | "interval" | "recurring" => Some("every"),
        "cron" => Some("cron"),
        _ => None,
    }
}

fn normalize_schedule_value(schedule: &mut Value) -> Result<()> {
    match schedule {
        Value::String(expr) => {
            let expr = expr.trim();
            if expr.is_empty() {
                bail!("schedule cron expression cannot be empty");
            }
            *schedule = json!({ "kind": "cron", "expr": expr });
            Ok(())
        },
        Value::Number(_) => {
            let at_ms = parse_epoch_millis(schedule, "schedule")?;
            *schedule = json!({ "kind": "at", "at_ms": at_ms });
            Ok(())
        },
        Value::Object(obj) => {
            take_alias(obj, "kind", &["type", "scheduleKind"]);
            take_alias(obj, "at_ms", &[
                "atMs",
                "at",
                "timestamp",
                "time",
                "timeMs",
                "time_ms",
            ]);
            take_alias(obj, "every_ms", &[
                "everyMs",
                "every",
                "interval",
                "intervalMs",
                "interval_ms",
            ]);
            take_alias(obj, "anchor_ms", &[
                "anchorMs",
                "anchor",
                "anchorTime",
                "anchor_time",
            ]);
            take_alias(obj, "expr", &[
                "cronExpr",
                "cron_expr",
                "expression",
                "cron",
            ]);

            if let Some(kind_val) = obj.get_mut("kind") {
                let kind_raw = kind_val
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("schedule.kind must be a string"))?;
                let kind_norm = normalize_schedule_kind(kind_raw).ok_or_else(|| {
                    anyhow::anyhow!(
                        "invalid schedule kind `{kind_raw}` (expected `at`, `every`, or `cron`)"
                    )
                })?;
                *kind_val = Value::String(kind_norm.to_string());
            } else {
                let has_at = obj.contains_key("at_ms");
                let has_every = obj.contains_key("every_ms");
                let has_expr = obj.contains_key("expr");
                let count = [has_at, has_every, has_expr]
                    .into_iter()
                    .filter(|has| *has)
                    .count();

                let inferred = match count {
                    1 if has_at => "at",
                    1 if has_every => "every",
                    1 if has_expr => "cron",
                    0 => bail!(
                        "invalid schedule: missing `kind` and no recognizable fields (expected one of `at_ms`, `every_ms`, `expr`)"
                    ),
                    _ => bail!(
                        "invalid schedule: ambiguous fields, specify `kind` explicitly (`at`, `every`, or `cron`)"
                    ),
                };
                obj.insert("kind".to_string(), Value::String(inferred.to_string()));
            }

            let kind = obj
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("schedule.kind must be a string"))?;
            match kind {
                "at" => {
                    let at_raw = obj
                        .get("at_ms")
                        .ok_or_else(|| anyhow::anyhow!("schedule kind `at` requires `at_ms`"))?;
                    let at_ms = parse_epoch_millis(at_raw, "schedule.at_ms")?;
                    obj.insert("at_ms".to_string(), json!(at_ms));
                },
                "every" => {
                    let every_raw = obj.get("every_ms").ok_or_else(|| {
                        anyhow::anyhow!("schedule kind `every` requires `every_ms`")
                    })?;
                    let every_ms = parse_interval_millis(every_raw, "schedule.every_ms")?;
                    obj.insert("every_ms".to_string(), json!(every_ms));
                    if let Some(anchor_raw) = obj.get("anchor_ms") {
                        let anchor_ms = parse_epoch_millis(anchor_raw, "schedule.anchor_ms")?;
                        obj.insert("anchor_ms".to_string(), json!(anchor_ms));
                    }
                },
                "cron" => {
                    let expr = obj
                        .get("expr")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|expr| !expr.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("schedule kind `cron` requires `expr`"))?;
                    obj.insert("expr".to_string(), Value::String(expr.to_string()));
                },
                _ => unreachable!("schedule kind normalized above"),
            }
            Ok(())
        },
        _ => bail!("schedule must be an object, cron expression string, or epoch milliseconds"),
    }
}

fn normalize_payload_kind(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "systemevent" | "system_event" | "system-event" | "event" => Some("systemEvent"),
        "agentturn" | "agent_turn" | "agent-turn" | "agent" => Some("agentTurn"),
        _ => None,
    }
}

fn prefers_system_event(session_target_hint: Option<&str>) -> bool {
    session_target_hint
        .map(str::trim)
        .is_some_and(|target| target.eq_ignore_ascii_case("main"))
}

fn normalize_payload_value(payload: &mut Value, session_target_hint: Option<&str>) -> Result<()> {
    match payload {
        Value::String(message) => {
            let message = message.trim();
            if message.is_empty() {
                bail!("payload message cannot be empty");
            }
            if prefers_system_event(session_target_hint) {
                *payload = json!({ "kind": "systemEvent", "text": message });
            } else {
                *payload = json!({ "kind": "agentTurn", "message": message });
            }
            Ok(())
        },
        Value::Object(obj) => {
            take_alias(obj, "kind", &["payloadKind", "type"]);
            take_alias(obj, "text", &["event", "instruction"]);
            take_alias(obj, "message", &["prompt", "content"]);
            take_alias(obj, "timeout_secs", &[
                "timeoutSecs",
                "timeout",
                "timeoutSeconds",
                "timeout_seconds",
            ]);

            if let Some(timeout_raw) = obj.get("timeout_secs") {
                let timeout = parse_timeout_seconds(timeout_raw, "payload.timeout_secs")?;
                obj.insert("timeout_secs".to_string(), json!(timeout));
            }

            if let Some(kind_val) = obj.get_mut("kind") {
                let kind_raw = kind_val
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("payload.kind must be a string"))?;
                let kind_norm = normalize_payload_kind(kind_raw).ok_or_else(|| {
                    anyhow::anyhow!(
                        "invalid payload kind `{kind_raw}` (expected `systemEvent` or `agentTurn`)"
                    )
                })?;
                *kind_val = Value::String(kind_norm.to_string());
            } else {
                let has_text = obj.contains_key("text");
                let has_message = obj.contains_key("message");
                let inferred = match (has_text, has_message) {
                    (true, false) => "systemEvent",
                    (false, true) => "agentTurn",
                    (true, true) if prefers_system_event(session_target_hint) => "systemEvent",
                    (true, true) => "agentTurn",
                    (false, false) => {
                        bail!(
                            "invalid payload: missing `kind` and no recognizable fields (expected one of `text` or `message`)"
                        )
                    },
                };
                obj.insert("kind".to_string(), Value::String(inferred.to_string()));
            }

            let kind = obj
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("payload.kind must be a string"))?;
            match kind {
                "systemEvent" => {
                    if !obj.contains_key("text")
                        && let Some(message) = obj.get("message").cloned()
                    {
                        obj.insert("text".to_string(), message);
                    }
                    let text = obj
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!("payload kind `systemEvent` requires `text`")
                        })?;
                    obj.insert("text".to_string(), Value::String(text.to_string()));
                },
                "agentTurn" => {
                    if !obj.contains_key("message")
                        && let Some(text) = obj.get("text").cloned()
                    {
                        obj.insert("message".to_string(), text);
                    }
                    let message = obj
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!("payload kind `agentTurn` requires `message`")
                        })?;
                    obj.insert("message".to_string(), Value::String(message.to_string()));
                },
                _ => unreachable!("payload kind normalized above"),
            }
            Ok(())
        },
        _ => bail!("payload must be an object or message string"),
    }
}

fn normalize_wake_mode(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "now" | "immediate" | "immediately" => Some("now"),
        "nextheartbeat" | "next_heartbeat" | "next-heartbeat" | "next" | "default" => {
            Some("nextHeartbeat")
        },
        _ => None,
    }
}

fn normalize_wake_mode_field(obj: &mut Map<String, Value>) -> Result<()> {
    take_alias(obj, "wakeMode", &["wake_mode"]);
    if let Some(val) = obj.get_mut("wakeMode") {
        let raw = val
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("wakeMode must be a string"))?;
        let norm = normalize_wake_mode(raw).ok_or_else(|| {
            anyhow::anyhow!("invalid wakeMode `{raw}` (expected `now` or `nextHeartbeat`)")
        })?;
        *val = Value::String(norm.to_string());
    }
    Ok(())
}

fn normalize_session_target_field(obj: &mut Map<String, Value>) {
    take_alias(obj, "sessionTarget", &["session_target", "target"]);
}

fn normalize_execution_target(raw: &str) -> Option<bool> {
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "sandbox" | "container" | "isolated" | "enabled" | "on" | "true" => Some(true),
        "host" | "local" | "disabled" | "off" | "none" | "false" => Some(false),
        _ => None,
    }
}

fn parse_sandbox_enabled(value: &Value, field: &str) -> Result<bool> {
    match value {
        Value::Bool(enabled) => Ok(*enabled),
        Value::String(raw) => normalize_execution_target(raw).ok_or_else(|| {
            anyhow::anyhow!("{field} string must be one of `host`, `local`, or `sandbox`")
        }),
        _ => bail!("{field} must be a boolean or execution target string"),
    }
}

fn normalize_sandbox_value(sandbox: &mut Value, field: &str) -> Result<()> {
    match sandbox {
        Value::Bool(enabled) => {
            *sandbox = json!({ "enabled": enabled });
            Ok(())
        },
        Value::String(raw) => {
            let enabled = normalize_execution_target(raw).ok_or_else(|| {
                anyhow::anyhow!("{field} string must be one of `host`, `local`, or `sandbox`")
            })?;
            *sandbox = json!({ "enabled": enabled });
            Ok(())
        },
        Value::Object(obj) => {
            take_alias(obj, "enabled", &[
                "sandboxEnabled",
                "sandbox_enabled",
                "sandboxed",
                "useSandbox",
            ]);
            take_alias(obj, "image", &[
                "sandboxImage",
                "sandbox_image",
                "containerImage",
                "imageName",
            ]);
            take_alias(obj, "target", &[
                "mode",
                "runtime",
                "executionTarget",
                "execution_target",
                "where",
            ]);

            if let Some(target_raw) = obj.get("target") {
                let enabled = parse_sandbox_enabled(target_raw, &format!("{field}.target"))?;
                obj.insert("enabled".to_string(), json!(enabled));
                obj.remove("target");
            }

            if let Some(enabled_raw) = obj.get("enabled") {
                let enabled = parse_sandbox_enabled(enabled_raw, &format!("{field}.enabled"))?;
                obj.insert("enabled".to_string(), json!(enabled));
            } else if obj.get("image").is_some() {
                obj.insert("enabled".to_string(), json!(true));
            }

            if let Some(image_raw) = obj.get("image") {
                match image_raw {
                    Value::Null => {},
                    Value::String(image) => {
                        let image = image.trim();
                        if image.is_empty() {
                            obj.insert("image".to_string(), Value::Null);
                        } else {
                            obj.insert("image".to_string(), Value::String(image.to_string()));
                        }
                    },
                    _ => bail!("{field}.image must be a string when provided"),
                }
            }
            Ok(())
        },
        _ => bail!("{field} must be an object, boolean, or execution target string"),
    }
}

fn normalize_sandbox_field(obj: &mut Map<String, Value>) -> Result<()> {
    take_alias(obj, "sandbox", &["sandboxConfig", "sandbox_config"]);
    let execution_value = obj
        .remove("execution")
        .or_else(|| obj.remove("executionTarget"))
        .or_else(|| obj.remove("execution_target"))
        .or_else(|| obj.remove("runtime"));
    let sandbox_enabled_value = obj
        .remove("sandboxEnabled")
        .or_else(|| obj.remove("sandbox_enabled"))
        .or_else(|| obj.remove("sandboxed"));
    let sandbox_image_value = obj
        .remove("sandboxImage")
        .or_else(|| obj.remove("sandbox_image"));

    if !obj.contains_key("sandbox")
        && let Some(value) = execution_value
    {
        obj.insert("sandbox".to_string(), value);
    }

    if let Some(sandbox) = obj.get_mut("sandbox") {
        if let Value::Object(sandbox_obj) = sandbox {
            if let Some(value) = sandbox_enabled_value
                && !sandbox_obj.contains_key("enabled")
            {
                sandbox_obj.insert("enabled".to_string(), value);
            }
            if let Some(value) = sandbox_image_value
                && !sandbox_obj.contains_key("image")
            {
                sandbox_obj.insert("image".to_string(), value);
            }
        }
    } else if sandbox_enabled_value.is_some() || sandbox_image_value.is_some() {
        let mut sandbox_obj = Map::new();
        if let Some(value) = sandbox_enabled_value {
            sandbox_obj.insert("enabled".to_string(), value);
        }
        if let Some(value) = sandbox_image_value {
            sandbox_obj.insert("image".to_string(), value);
        }
        obj.insert("sandbox".to_string(), Value::Object(sandbox_obj));
    }

    if let Some(sandbox) = obj.get_mut("sandbox") {
        normalize_sandbox_value(sandbox, "sandbox")?;
    }
    Ok(())
}

fn normalize_job_value(job: &Value) -> Result<Value> {
    let mut normalized = job.clone();
    let obj = normalized
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("job must be an object"))?;
    normalize_session_target_field(obj);
    normalize_sandbox_field(obj)?;
    normalize_wake_mode_field(obj)?;

    let session_target_hint = obj
        .get("sessionTarget")
        .and_then(Value::as_str)
        .map(str::to_string);

    let schedule = obj
        .get_mut("schedule")
        .ok_or_else(|| anyhow::anyhow!("missing `schedule`"))?;
    normalize_schedule_value(schedule)?;

    let payload = obj
        .get_mut("payload")
        .ok_or_else(|| anyhow::anyhow!("missing `payload`"))?;
    normalize_payload_value(payload, session_target_hint.as_deref())?;

    Ok(normalized)
}

fn normalize_patch_value(patch: &Value) -> Result<Value> {
    let mut normalized = patch.clone();
    let obj = normalized
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("patch must be an object"))?;
    normalize_session_target_field(obj);
    normalize_sandbox_field(obj)?;
    normalize_wake_mode_field(obj)?;

    let session_target_hint = obj
        .get("sessionTarget")
        .and_then(Value::as_str)
        .map(str::to_string);

    if let Some(schedule) = obj.get_mut("schedule") {
        normalize_schedule_value(schedule)?;
    }
    if let Some(payload) = obj.get_mut("payload") {
        normalize_payload_value(payload, session_target_hint.as_deref())?;
    }

    Ok(normalized)
}

#[async_trait]
impl AgentTool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Manage scheduled tasks (reminders, recurring jobs, cron schedules).\n\
         \n\
         For reminders and recurring tasks that should produce a response in the \
         main conversation (e.g. \"tell me a joke every day at 9am\"), use:\n\
         - sessionTarget: \"main\"\n\
         - payload.kind: \"systemEvent\"\n\
         - payload.text: a message that will be injected as if the user typed it \
           (the agent will then process it and respond). Write the text as an \
           instruction to yourself, e.g. \"Tell me a funny joke.\"\n\
         \n\
         For isolated background tasks (no main session interaction), use:\n\
         - sessionTarget: \"isolated\"\n\
         - payload.kind: \"agentTurn\"\n\
         - payload.message: the prompt for the isolated agent run\n\
         \n\
         Optional execution controls for agent turns:\n\
         - payload.model: model id for this job\n\
         - sandbox.enabled: true for sandbox execution, false for host\n\
         - sandbox.image: optional sandbox image override"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "list", "add", "update", "remove", "run", "runs"],
                    "description": "The action to perform"
                },
                "job": {
                    "type": "object",
                    "description": "Job specification (for 'add' action)",
                    "properties": {
                        "name": { "type": "string", "description": "Human-readable job name" },
                        "schedule": {
                            "type": "object",
                            "description": "Schedule object. Use {kind:'at', at_ms}, {kind:'every', every_ms, anchor_ms?}, or {kind:'cron', expr, tz?}. This tool also accepts shorthand schedule strings/numbers at runtime.",
                            "properties": {
                                "kind": { "type": "string", "enum": ["at", "every", "cron"] },
                                "at_ms": { "type": "integer", "description": "Used when kind='at'" },
                                "every_ms": { "type": "integer", "description": "Used when kind='every'" },
                                "anchor_ms": { "type": "integer", "description": "Optional anchor when kind='every'" },
                                "expr": { "type": "string", "description": "Cron expression used when kind='cron'" },
                                "tz": { "type": "string", "description": "Optional timezone used when kind='cron'" }
                            },
                            "required": ["kind"]
                        },
                        "payload": {
                            "type": "object",
                            "description": "What to do. Use {kind:'systemEvent', text} for main-session reminders or {kind:'agentTurn', message, model?, timeout_secs?, deliver?, channel?, to?}. `payload.model` selects the LLM for that job. This tool also accepts a shorthand message string at runtime.",
                            "properties": {
                                "kind": { "type": "string", "enum": ["systemEvent", "agentTurn"] },
                                "text": { "type": "string" },
                                "message": { "type": "string" },
                                "model": { "type": "string" },
                                "timeout_secs": { "type": "integer" },
                                "deliver": { "type": "boolean" },
                                "channel": { "type": "string" },
                                "to": { "type": "string" }
                            },
                            "required": ["kind"]
                        },
                        "sessionTarget": { "type": "string", "enum": ["main", "isolated"], "default": "isolated" },
                        "sandbox": {
                            "type": "object",
                            "description": "Execution environment for agent turns. Use {enabled:false} for host execution, or {enabled:true, image?} for sandbox execution.",
                            "properties": {
                                "enabled": { "type": "boolean", "description": "true = sandbox execution, false = host execution" },
                                "image": { "type": "string", "description": "Optional sandbox image tag when sandbox is enabled" }
                            }
                        },
                        "execution": {
                            "type": "object",
                            "description": "Alias for sandbox settings. Use {target:'host'|'sandbox', image?}.",
                            "properties": {
                                "target": { "type": "string", "enum": ["host", "sandbox"] },
                                "image": { "type": "string" }
                            }
                        },
                        "deleteAfterRun": { "type": "boolean", "default": false },
                        "enabled": { "type": "boolean", "default": true },
                        "wakeMode": { "type": "string", "enum": ["now", "nextHeartbeat"], "default": "nextHeartbeat", "description": "Whether to trigger an immediate heartbeat after this job fires" }
                    },
                    "required": ["name", "schedule", "payload"]
                },
                "patch": {
                    "type": "object",
                    "description": "Fields to update (for 'update' action)"
                },
                "id": {
                    "type": "string",
                    "description": "Job ID (for update/remove/run/runs)"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force-run even if disabled (for 'run' action)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max run records to return (for 'runs' action, default 20)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

        match action {
            "status" => {
                let status = self.service.status().await;
                Ok(serde_json::to_value(status)?)
            },
            "list" => {
                let jobs = self.service.list().await;
                Ok(serde_json::to_value(jobs)?)
            },
            "add" => {
                let job_val = params
                    .get("job")
                    .ok_or_else(|| anyhow::anyhow!("missing 'job' parameter for add"))?;
                let normalized = normalize_job_value(job_val)?;
                let create: CronJobCreate = serde_json::from_value(normalized)
                    .map_err(|e| anyhow::anyhow!("invalid job spec: {e}"))?;
                let job = self.service.add(create).await?;
                Ok(serde_json::to_value(job)?)
            },
            "update" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for update"))?;
                let patch_val = params
                    .get("patch")
                    .ok_or_else(|| anyhow::anyhow!("missing 'patch' for update"))?;
                let normalized = normalize_patch_value(patch_val)?;
                let patch: CronJobPatch = serde_json::from_value(normalized)
                    .map_err(|e| anyhow::anyhow!("invalid patch: {e}"))?;
                let job = self.service.update(id, patch).await?;
                Ok(serde_json::to_value(job)?)
            },
            "remove" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for remove"))?;
                self.service.remove(id).await?;
                Ok(json!({ "removed": id }))
            },
            "run" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for run"))?;
                let force = params
                    .get("force")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.service.run(id, force).await?;
                Ok(json!({ "ran": id }))
            },
            "runs" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("missing 'id' for runs"))?;
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let runs = self.service.runs(id, limit).await?;
                Ok(serde_json::to_value(runs)?)
            },
            _ => bail!("unknown cron action: {action}"),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
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
}
