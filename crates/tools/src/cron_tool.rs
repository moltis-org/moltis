//! Agent-callable cron tool for managing scheduled jobs.

use std::sync::Arc;

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    serde_json::{Value, json},
};

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_cron::{
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
         - payload.message: the prompt for the isolated agent run"
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
                            "description": "Schedule: {kind:'at', at_ms}, {kind:'every', every_ms, anchor_ms?}, or {kind:'cron', expr, tz?}",
                            "properties": {
                                "kind": { "type": "string", "enum": ["at", "every", "cron"] },
                                "at_ms": { "type": "integer" },
                                "every_ms": { "type": "integer" },
                                "anchor_ms": { "type": "integer" },
                                "expr": { "type": "string" },
                                "tz": { "type": "string" }
                            },
                            "required": ["kind"]
                        },
                        "payload": {
                            "type": "object",
                            "description": "What to do: {kind:'systemEvent', text} or {kind:'agentTurn', message, model?, deliver?, channel?, to?}",
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
                        "deleteAfterRun": { "type": "boolean", "default": false },
                        "enabled": { "type": "boolean", "default": true }
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
                let create: CronJobCreate = serde_json::from_value(job_val.clone())
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
                let patch: CronJobPatch = serde_json::from_value(patch_val.clone())
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
}
