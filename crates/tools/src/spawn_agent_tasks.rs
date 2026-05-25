use std::{collections::HashMap, sync::Arc};

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::Value,
    time::{Duration, OffsetDateTime},
    tokio::sync::RwLock,
    uuid::Uuid,
};

use crate::{error::Error, params::str_param};

const DEFAULT_TASK_TTL_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnTaskStatus {
    Running,
    Completed,
    Failed,
}

impl SpawnTaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpawnTaskUpdate {
    pub text: Option<String>,
    pub iterations: usize,
    pub tool_calls_made: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpawnTask {
    pub id: String,
    pub task: String,
    pub session_key: Option<String>,
    pub status: SpawnTaskStatus,
    pub model: String,
    pub preset: Option<String>,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
    pub text: Option<String>,
    pub iterations: usize,
    pub tool_calls_made: usize,
    pub error: Option<String>,
}

impl SpawnTask {
    fn is_expired(&self, now: OffsetDateTime, ttl: Duration) -> bool {
        let Some(finished_at) = self.finished_at else {
            return false;
        };
        finished_at + ttl <= now
    }

    fn assert_access(&self, session_key: Option<&str>) -> crate::Result<()> {
        if self.session_key.as_deref() == session_key {
            return Ok(());
        }
        Err(Error::message("spawn task access denied"))
    }

    fn elapsed_secs(&self, now: OffsetDateTime) -> i64 {
        (self.finished_at.unwrap_or(now) - self.started_at).whole_seconds()
    }

    fn status_json(&self, now: OffsetDateTime) -> Value {
        serde_json::json!({
            "task_id": self.id,
            "status": self.status.as_str(),
            "task": self.task,
            "model": self.model,
            "preset": self.preset,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "elapsed_secs": self.elapsed_secs(now),
            "iterations": self.iterations,
            "tool_calls_made": self.tool_calls_made,
            "error": self.error,
        })
    }

    fn result_json(&self, now: OffsetDateTime) -> Value {
        let mut value = self.status_json(now);
        value["text"] = self.text.clone().into();
        value
    }
}

#[derive(Debug)]
pub struct SpawnTaskStore {
    tasks: RwLock<HashMap<String, SpawnTask>>,
    ttl: Duration,
}

impl Default for SpawnTaskStore {
    fn default() -> Self {
        Self::new(Duration::hours(DEFAULT_TASK_TTL_HOURS))
    }
}

impl SpawnTaskStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub async fn insert_running(
        &self,
        task: String,
        session_key: Option<String>,
        model: String,
        preset: Option<String>,
    ) -> SpawnTask {
        let entry = SpawnTask {
            id: Uuid::new_v4().to_string(),
            task,
            session_key,
            status: SpawnTaskStatus::Running,
            model,
            preset,
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
            text: None,
            iterations: 0,
            tool_calls_made: 0,
            error: None,
        };
        self.tasks
            .write()
            .await
            .insert(entry.id.clone(), entry.clone());
        entry
    }

    pub async fn complete(&self, id: &str, update: SpawnTaskUpdate) {
        if let Some(task) = self.tasks.write().await.get_mut(id) {
            task.status = if update.error.is_some() {
                SpawnTaskStatus::Failed
            } else {
                SpawnTaskStatus::Completed
            };
            task.finished_at = Some(OffsetDateTime::now_utc());
            task.text = update.text;
            task.iterations = update.iterations;
            task.tool_calls_made = update.tool_calls_made;
            task.error = update.error;
        }
    }

    pub async fn status(&self, id: &str, session_key: Option<&str>) -> crate::Result<Value> {
        let now = OffsetDateTime::now_utc();
        self.cleanup_expired(now).await;
        let tasks = self.tasks.read().await;
        let task = tasks
            .get(id)
            .ok_or_else(|| Error::message(format!("spawn task not found: {id}")))?;
        task.assert_access(session_key)?;
        Ok(task.status_json(now))
    }

    pub async fn result(&self, id: &str, session_key: Option<&str>) -> crate::Result<Value> {
        let now = OffsetDateTime::now_utc();
        self.cleanup_expired(now).await;
        let tasks = self.tasks.read().await;
        let task = tasks
            .get(id)
            .ok_or_else(|| Error::message(format!("spawn task not found: {id}")))?;
        task.assert_access(session_key)?;
        Ok(task.result_json(now))
    }

    async fn cleanup_expired(&self, now: OffsetDateTime) {
        let mut tasks = self.tasks.write().await;
        tasks.retain(|_, task| !task.is_expired(now, self.ttl));
    }
}

#[derive(Clone)]
pub struct SpawnStatusTool {
    store: Arc<SpawnTaskStore>,
}

impl SpawnStatusTool {
    pub fn new(store: Arc<SpawnTaskStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for SpawnStatusTool {
    fn name(&self) -> &str {
        "spawn_status"
    }

    fn description(&self) -> &str {
        "Check the status of a non-blocking spawn_agent task."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID returned by spawn_agent with nonblocking=true."
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let id = str_param(&params, "task_id")
            .ok_or_else(|| Error::message("missing required parameter: task_id"))?;
        let session_key = str_param(&params, "_session_key");
        Ok(self.store.status(id, session_key).await?)
    }
}

#[derive(Clone)]
pub struct SpawnResultTool {
    store: Arc<SpawnTaskStore>,
}

impl SpawnResultTool {
    pub fn new(store: Arc<SpawnTaskStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for SpawnResultTool {
    fn name(&self) -> &str {
        "spawn_result"
    }

    fn description(&self) -> &str {
        "Fetch the result of a non-blocking spawn_agent task."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID returned by spawn_agent with nonblocking=true."
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let id = str_param(&params, "task_id")
            .ok_or_else(|| Error::message("missing required parameter: task_id"))?;
        let session_key = str_param(&params, "_session_key");
        Ok(self.store.result(id, session_key).await?)
    }
}
