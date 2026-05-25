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
const CLEANUP_INTERVAL_SECS: i64 = 60;

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
        match self.finished_at {
            // Completed/failed tasks expire after one TTL from completion.
            Some(finished_at) => finished_at + ttl <= now,
            // Running tasks that never completed get a grace period of 2× TTL
            // from their start time before being reaped as stale.
            None => self.started_at + ttl + ttl <= now,
        }
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
    cleanup_interval: Duration,
    last_cleanup: std::sync::Mutex<Option<OffsetDateTime>>,
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
            cleanup_interval: Duration::seconds(CLEANUP_INTERVAL_SECS),
            last_cleanup: std::sync::Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_cleanup_interval(ttl: Duration, cleanup_interval: Duration) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            ttl,
            cleanup_interval,
            last_cleanup: std::sync::Mutex::new(None),
        }
    }

    #[tracing::instrument(skip(self, task), fields(model = %model))]
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

    #[tracing::instrument(skip(self, update))]
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

    #[tracing::instrument(skip(self))]
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

    #[tracing::instrument(skip(self))]
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

    pub async fn list(&self, session_key: Option<&str>) -> Vec<Value> {
        let now = OffsetDateTime::now_utc();
        self.cleanup_expired(now).await;
        let tasks = self.tasks.read().await;
        tasks
            .values()
            .filter(|task| task.session_key.as_deref() == session_key)
            .map(|task| task.status_json(now))
            .collect()
    }

    async fn cleanup_expired(&self, now: OffsetDateTime) {
        if !self.should_cleanup(now) {
            return;
        }
        let mut tasks = self.tasks.write().await;
        let before = tasks.len();
        tasks.retain(|_, task| !task.is_expired(now, self.ttl));
        let expired = before - tasks.len();

        #[cfg(feature = "metrics")]
        if expired > 0 {
            use moltis_metrics::{counter, spawn as spawn_metrics};
            counter!(spawn_metrics::TASKS_EXPIRED_TOTAL).increment(expired as u64);
        }

        let _ = expired; // silence unused warning when metrics feature is off
    }

    fn should_cleanup(&self, now: OffsetDateTime) -> bool {
        let mut last_cleanup = self
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last_cleanup.is_some_and(|last| last + self.cleanup_interval > now) {
            return false;
        }
        *last_cleanup = Some(now);
        true
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

    #[tracing::instrument(skip(self, params))]
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
        "Fetch the result of a non-blocking spawn_agent task. Returns the current state; check status before using text because running tasks have no final text yet."
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

    #[tracing::instrument(skip(self, params))]
    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let id = str_param(&params, "task_id")
            .ok_or_else(|| Error::message("missing required parameter: task_id"))?;
        let session_key = str_param(&params, "_session_key");
        Ok(self.store.result(id, session_key).await?)
    }
}

#[derive(Clone)]
pub struct SpawnListTool {
    store: Arc<SpawnTaskStore>,
}

impl SpawnListTool {
    pub fn new(store: Arc<SpawnTaskStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for SpawnListTool {
    fn name(&self) -> &str {
        "spawn_list"
    }

    fn description(&self) -> &str {
        "List all non-blocking spawn_agent tasks visible to the current session. Useful for recovering task IDs after context loss."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    #[tracing::instrument(skip(self, params))]
    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let session_key = str_param(&params, "_session_key");
        let tasks = self.store.list(session_key).await;
        Ok(serde_json::json!({ "tasks": tasks }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_running_tasks_are_cleaned_up() {
        let store = SpawnTaskStore::with_cleanup_interval(
            Duration::milliseconds(1),
            Duration::milliseconds(0),
        );
        let task = store
            .insert_running(
                "zombie task".to_string(),
                None,
                "mock-model".to_string(),
                None,
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;

        let status = store.status(&task.id, None).await;

        assert!(status.is_err());
        assert!(status.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn cleanup_is_amortized_between_polls() {
        let store = SpawnTaskStore::with_cleanup_interval(Duration::hours(1), Duration::minutes(1));
        let task = store
            .insert_running(
                "active task".to_string(),
                None,
                "mock-model".to_string(),
                None,
            )
            .await;

        store.status(&task.id, None).await.unwrap();
        let first_cleanup = *store
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        store.status(&task.id, None).await.unwrap();
        let second_cleanup = *store
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        assert_eq!(first_cleanup, second_cleanup);
    }

    #[test]
    fn spawn_result_description_warns_running_results_have_no_text() {
        let tool = SpawnResultTool::new(Arc::new(SpawnTaskStore::default()));
        let description = tool.description();

        assert!(description.contains("check status"));
        assert!(description.contains("running tasks have no final text"));
    }

    #[tokio::test]
    async fn list_returns_tasks_for_matching_session() {
        let store = SpawnTaskStore::default();
        store
            .insert_running(
                "task a".to_string(),
                Some("session-1".to_string()),
                "model".to_string(),
                None,
            )
            .await;
        store
            .insert_running(
                "task b".to_string(),
                Some("session-2".to_string()),
                "model".to_string(),
                None,
            )
            .await;
        store
            .insert_running(
                "task c".to_string(),
                Some("session-1".to_string()),
                "model".to_string(),
                None,
            )
            .await;

        let session_1_tasks = store.list(Some("session-1")).await;
        assert_eq!(session_1_tasks.len(), 2);

        let session_2_tasks = store.list(Some("session-2")).await;
        assert_eq!(session_2_tasks.len(), 1);

        let no_session_tasks = store.list(None).await;
        assert_eq!(no_session_tasks.len(), 0);
    }

    #[test]
    fn spawn_list_tool_has_no_required_params() {
        let tool = SpawnListTool::new(Arc::new(SpawnTaskStore::default()));
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }
}
