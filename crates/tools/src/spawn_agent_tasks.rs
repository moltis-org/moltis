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
const DEFAULT_STALE_RUNNING_TTL_HOURS: i64 = 48;
const CLEANUP_INTERVAL_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnTaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SpawnTaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpawnTaskRetention {
    completed_ttl: Duration,
    failed_ttl: Duration,
    running_ttl: Duration,
    cleanup_interval: Duration,
}

impl Default for SpawnTaskRetention {
    fn default() -> Self {
        Self {
            completed_ttl: Duration::hours(DEFAULT_TASK_TTL_HOURS),
            failed_ttl: Duration::hours(DEFAULT_TASK_TTL_HOURS),
            running_ttl: Duration::hours(DEFAULT_STALE_RUNNING_TTL_HOURS),
            cleanup_interval: Duration::seconds(CLEANUP_INTERVAL_SECS),
        }
    }
}

impl SpawnTaskRetention {
    #[must_use]
    pub fn new(
        completed_ttl: Duration,
        failed_ttl: Duration,
        running_ttl: Duration,
        cleanup_interval: Duration,
    ) -> Self {
        Self {
            completed_ttl,
            failed_ttl,
            running_ttl,
            cleanup_interval,
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
    fn is_expired(&self, now: OffsetDateTime, retention: &SpawnTaskRetention) -> bool {
        match (self.status.clone(), self.finished_at) {
            // Completed/failed tasks expire after one TTL from completion.
            (SpawnTaskStatus::Completed, Some(finished_at)) => {
                finished_at + retention.completed_ttl <= now
            },
            (SpawnTaskStatus::Failed | SpawnTaskStatus::Cancelled, Some(finished_at)) => {
                finished_at + retention.failed_ttl <= now
            },
            // Running tasks that never completed get a separate stale-running TTL.
            (SpawnTaskStatus::Running, _) => self.started_at + retention.running_ttl <= now,
            // Malformed terminal records without finished_at should not be retained forever.
            (_, None) => self.started_at + retention.failed_ttl <= now,
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
    abort_handles: RwLock<HashMap<String, futures::future::AbortHandle>>,
    retention: SpawnTaskRetention,
    last_cleanup: std::sync::Mutex<Option<OffsetDateTime>>,
}

impl Default for SpawnTaskStore {
    fn default() -> Self {
        Self::new(SpawnTaskRetention::default())
    }
}

impl SpawnTaskStore {
    pub fn new(retention: SpawnTaskRetention) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            abort_handles: RwLock::new(HashMap::new()),
            retention,
            last_cleanup: std::sync::Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_retention(retention: SpawnTaskRetention) -> Self {
        Self::new(retention)
    }

    #[tracing::instrument(skip(self, task), fields(model = %model))]
    pub async fn insert_running(
        &self,
        task: String,
        session_key: Option<String>,
        model: String,
        preset: Option<String>,
        abort_handle: futures::future::AbortHandle,
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
        self.abort_handles
            .write()
            .await
            .insert(entry.id.clone(), abort_handle);
        entry
    }

    #[tracing::instrument(skip(self, update))]
    pub async fn complete(&self, id: &str, update: SpawnTaskUpdate) {
        let mut tasks = self.tasks.write().await;
        let Some(task) = tasks.get_mut(id) else {
            return;
        };
        if task.status == SpawnTaskStatus::Cancelled {
            return;
        }
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
        drop(tasks);

        self.abort_handles.write().await.remove(id);
    }

    pub async fn cancel(&self, id: &str, session_key: Option<&str>) -> crate::Result<Value> {
        let now = OffsetDateTime::now_utc();
        self.cleanup_expired(now).await;
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .get_mut(id)
            .ok_or_else(|| Error::message(format!("spawn task not found: {id}")))?;
        task.assert_access(session_key)?;
        if task.status != SpawnTaskStatus::Running {
            let mut value = task.status_json(now);
            value["cancelled"] = false.into();
            return Ok(value);
        }

        task.status = SpawnTaskStatus::Cancelled;
        task.finished_at = Some(now);
        task.error = Some("cancelled by caller".to_string());
        let mut value = task.status_json(now);
        value["cancelled"] = true.into();
        drop(tasks);

        if let Some(handle) = self.abort_handles.write().await.remove(id) {
            handle.abort();
        }
        Ok(value)
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
        let mut expired_ids = Vec::new();
        tasks.retain(|id, task| {
            let keep = !task.is_expired(now, &self.retention);
            if !keep {
                expired_ids.push(id.clone());
            }
            keep
        });
        let expired = before - tasks.len();
        drop(tasks);
        if !expired_ids.is_empty() {
            let mut abort_handles = self.abort_handles.write().await;
            for id in expired_ids {
                if let Some(handle) = abort_handles.remove(&id) {
                    handle.abort();
                }
            }
        }

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
        if last_cleanup.is_some_and(|last| last + self.retention.cleanup_interval > now) {
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

#[derive(Clone)]
pub struct SpawnCancelTool {
    store: Arc<SpawnTaskStore>,
}

impl SpawnCancelTool {
    pub fn new(store: Arc<SpawnTaskStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for SpawnCancelTool {
    fn name(&self) -> &str {
        "cancel_spawn"
    }

    fn description(&self) -> &str {
        "Cancel a running non-blocking spawn_agent task by task_id."
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
        Ok(self.store.cancel(id, session_key).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abort_handle() -> futures::future::AbortHandle {
        let (handle, _registration) = futures::future::AbortHandle::new_pair();
        handle
    }

    #[tokio::test]
    async fn stale_running_tasks_are_cleaned_up() {
        let store = SpawnTaskStore::with_retention(SpawnTaskRetention::new(
            Duration::hours(1),
            Duration::hours(1),
            Duration::milliseconds(1),
            Duration::milliseconds(0),
        ));
        let task = store
            .insert_running(
                "zombie task".to_string(),
                None,
                "mock-model".to_string(),
                None,
                abort_handle(),
            )
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;

        let status = store.status(&task.id, None).await;

        match status {
            Ok(value) => panic!("expected stale task to be cleaned up, got {value:?}"),
            Err(err) => assert!(err.to_string().contains("not found")),
        }
    }

    #[tokio::test]
    async fn cleanup_is_amortized_between_polls() {
        let store = SpawnTaskStore::with_retention(SpawnTaskRetention::new(
            Duration::hours(1),
            Duration::hours(1),
            Duration::hours(1),
            Duration::minutes(1),
        ));
        let task = store
            .insert_running(
                "active task".to_string(),
                None,
                "mock-model".to_string(),
                None,
                abort_handle(),
            )
            .await;

        if let Err(err) = store.status(&task.id, None).await {
            panic!("expected status lookup to succeed: {err}");
        }
        let first_cleanup = *store
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Err(err) = store.status(&task.id, None).await {
            panic!("expected second status lookup to succeed: {err}");
        }
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
                abort_handle(),
            )
            .await;
        store
            .insert_running(
                "task b".to_string(),
                Some("session-2".to_string()),
                "model".to_string(),
                None,
                abort_handle(),
            )
            .await;
        store
            .insert_running(
                "task c".to_string(),
                Some("session-1".to_string()),
                "model".to_string(),
                None,
                abort_handle(),
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

    #[tokio::test]
    async fn cancel_marks_running_task_cancelled() {
        let store = SpawnTaskStore::default();
        let task = store
            .insert_running(
                "cancel me".to_string(),
                Some("session-1".to_string()),
                "model".to_string(),
                None,
                abort_handle(),
            )
            .await;

        let cancelled = match store.cancel(&task.id, Some("session-1")).await {
            Ok(value) => value,
            Err(err) => panic!("expected cancellation to succeed: {err}"),
        };

        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(cancelled["cancelled"], true);
        assert_eq!(cancelled["error"], "cancelled by caller");
    }

    #[tokio::test]
    async fn cancel_enforces_session_key() {
        let store = SpawnTaskStore::default();
        let task = store
            .insert_running(
                "private task".to_string(),
                Some("session-1".to_string()),
                "model".to_string(),
                None,
                abort_handle(),
            )
            .await;

        let denied = store.cancel(&task.id, Some("session-2")).await;

        match denied {
            Ok(value) => panic!("expected cancellation to be denied, got {value:?}"),
            Err(err) => assert!(err.to_string().contains("access denied")),
        }
    }

    #[test]
    fn cancel_spawn_tool_requires_task_id() {
        let tool = SpawnCancelTool::new(Arc::new(SpawnTaskStore::default()));
        assert_eq!(tool.parameters_schema()["required"][0], "task_id");
    }
}
