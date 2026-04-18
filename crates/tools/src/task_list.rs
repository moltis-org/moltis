//! Shared task list tool for inter-agent task coordination.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tokio::sync::RwLock,
};

use {
    crate::{
        Error,
        params::{require_str, str_param, str_param_any},
    },
    moltis_agents::tool_registry::AgentTool,
};

/// Status of a task in the shared list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = Error;

    fn from_str(input: &str) -> crate::Result<Self> {
        match input {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            other => Err(Error::message(format!("unknown task status: {other}"))),
        }
    }
}

/// A single task in the shared list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
    /// Which list this task belongs to. Always populated since creation
    /// associates tasks with a list. Persisted to disk as part of the
    /// task JSON; deserialization defaults to empty for legacy files.
    #[serde(default)]
    pub list_id: String,
}

/// File-backed store for one logical task list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub next_id: u64,
    pub tasks: HashMap<String, Task>,
}

impl Default for TaskList {
    fn default() -> Self {
        Self {
            next_id: 1,
            tasks: HashMap::new(),
        }
    }
}

/// Thread-safe, file-backed task store.
pub struct TaskStore {
    data_dir: PathBuf,
    lists: RwLock<HashMap<String, TaskList>>,
}

impl TaskStore {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            data_dir: base_dir.join("tasks"),
            lists: RwLock::new(HashMap::new()),
        }
    }

    fn file_path(&self, list_id: &str) -> PathBuf {
        self.data_dir.join(format!("{list_id}.json"))
    }

    async fn ensure_list(&self, list_id: &str) -> crate::Result<()> {
        let mut lists = self.lists.write().await;
        if lists.contains_key(list_id) {
            return Ok(());
        }

        let path = self.file_path(list_id);
        let list = if path.exists() {
            let data = tokio::fs::read_to_string(&path).await.map_err(|e| {
                Error::message(format!("failed to read task list '{list_id}': {e}"))
            })?;
            serde_json::from_str::<TaskList>(&data).map_err(|e| {
                Error::message(format!("failed to parse task list '{list_id}' JSON: {e}"))
            })?
        } else {
            TaskList::default()
        };
        lists.insert(list_id.to_string(), list);
        Ok(())
    }

    async fn persist(&self, list_id: &str) -> crate::Result<()> {
        let lists = self.lists.read().await;
        let Some(list) = lists.get(list_id) else {
            return Ok(());
        };
        tokio::fs::create_dir_all(&self.data_dir)
            .await
            .map_err(|e| Error::message(format!("failed to create task dir: {e}")))?;
        let payload = serde_json::to_string_pretty(list).map_err(|e| {
            Error::message(format!("failed to serialize task list '{list_id}': {e}"))
        })?;
        tokio::fs::write(self.file_path(list_id), payload)
            .await
            .map_err(|e| Error::message(format!("failed to write task list '{list_id}': {e}")))?;
        Ok(())
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub async fn create(
        &self,
        list_id: &str,
        subject: String,
        description: String,
    ) -> crate::Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists
            .get_mut(list_id)
            .ok_or_else(|| Error::message(format!("missing task list: {list_id}")))?;

        let id = list.next_id.to_string();
        list.next_id = list.next_id.saturating_add(1);
        let now = Self::now();
        let task = Task {
            id: id.clone(),
            subject,
            description,
            status: TaskStatus::Pending,
            owner: None,
            blocked_by: Vec::new(),
            created_at: now,
            updated_at: now,
            list_id: list_id.to_string(),
        };
        list.tasks.insert(id, task.clone());
        drop(lists);
        self.persist(list_id).await?;
        Ok(task)
    }

    /// Return all list IDs that have persisted files or contain tasks.
    /// Filters out phantom in-memory lists created by failed lookups.
    ///
    /// Note: this scans the filesystem and holds a read lock, so results
    /// may be stale if a concurrent write creates or deletes a list file
    /// between the directory scan and the filter. This is acceptable for
    /// a discovery API — callers should not assume strong consistency.
    pub async fn list_ids(&self) -> crate::Result<Vec<String>> {
        // Ensure every persisted list is loaded.
        if self.data_dir.exists() {
            let mut entries = tokio::fs::read_dir(&self.data_dir)
                .await
                .map_err(|e| Error::message(format!("failed to read task dir: {e}")))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| Error::message(format!("failed to read task dir entry: {e}")))?
            {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    self.ensure_list(&stem).await?;
                }
            }
        }
        let lists = self.lists.read().await;
        let mut ids: Vec<String> = lists
            .iter()
            .filter(|(id, list)| {
                // Keep if persisted on disk or has tasks.
                self.file_path(id).exists() || !list.tasks.is_empty()
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// List tasks from a single list, or all lists when `list_id` is `"*"`.
    pub async fn list_tasks(
        &self,
        list_id: &str,
        status_filter: Option<&TaskStatus>,
    ) -> crate::Result<Vec<Task>> {
        if list_id == "*" {
            return self.list_all_tasks(status_filter).await;
        }
        self.ensure_list(list_id).await?;
        let lists = self.lists.read().await;
        let list = lists
            .get(list_id)
            .ok_or_else(|| Error::message(format!("missing task list: {list_id}")))?;

        let mut tasks: Vec<Task> = list
            .tasks
            .values()
            .filter(|t| status_filter.is_none_or(|s| &t.status == s))
            .cloned()
            .collect();
        tasks.sort_by_key(|t| t.id.parse::<u64>().unwrap_or(0));
        Ok(tasks)
    }

    /// List tasks across every known list.
    async fn list_all_tasks(&self, status_filter: Option<&TaskStatus>) -> crate::Result<Vec<Task>> {
        let ids = self.list_ids().await?;
        // Collect as (list_id, numeric_id, task) for stable cross-list ordering.
        let mut all: Vec<(String, u64, Task)> = Vec::new();
        let lists = self.lists.read().await;
        for id in &ids {
            let Some(list) = lists.get(id) else {
                continue;
            };
            for task in list
                .tasks
                .values()
                .filter(|t| status_filter.is_none_or(|s| &t.status == s))
            {
                let mut t = task.clone();
                t.list_id = id.clone();
                all.push((id.clone(), t.id.parse::<u64>().unwrap_or(0), t));
            }
        }
        all.sort_by_key(|(list_id, num, _)| (list_id.clone(), *num));
        Ok(all.into_iter().map(|(_, _, t)| t).collect())
    }

    pub async fn get(&self, list_id: &str, task_id: &str) -> crate::Result<Option<Task>> {
        self.ensure_list(list_id).await?;
        let lists = self.lists.read().await;
        let list = lists
            .get(list_id)
            .ok_or_else(|| Error::message(format!("missing task list: {list_id}")))?;
        Ok(list.tasks.get(task_id).cloned())
    }

    pub async fn update(
        &self,
        list_id: &str,
        task_id: &str,
        status: Option<TaskStatus>,
        subject: Option<String>,
        description: Option<String>,
        owner: Option<String>,
        blocked_by: Option<Vec<String>>,
    ) -> crate::Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists
            .get_mut(list_id)
            .ok_or_else(|| Error::message(format!("missing task list: {list_id}")))?;
        let task = list
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| Error::message(format!("task not found: {task_id}")))?;

        if let Some(status) = status {
            task.status = status;
        }
        if let Some(subject) = subject {
            task.subject = subject;
        }
        if let Some(description) = description {
            task.description = description;
        }
        if let Some(owner) = owner {
            task.owner = Some(owner);
        }
        if let Some(blocked_by) = blocked_by {
            task.blocked_by = blocked_by;
        }
        task.updated_at = Self::now();

        let updated = task.clone();
        drop(lists);
        self.persist(list_id).await?;
        Ok(updated)
    }

    /// Atomically claim a pending task and set it to in-progress.
    pub async fn claim(&self, list_id: &str, task_id: &str, owner: &str) -> crate::Result<Task> {
        self.ensure_list(list_id).await?;
        let mut lists = self.lists.write().await;
        let list = lists
            .get_mut(list_id)
            .ok_or_else(|| Error::message(format!("missing task list: {list_id}")))?;

        let (status, deps) = {
            let task = list
                .tasks
                .get(task_id)
                .ok_or_else(|| Error::message(format!("task not found: {task_id}")))?;
            (task.status.clone(), task.blocked_by.clone())
        };

        if status != TaskStatus::Pending {
            return Err(Error::message(format!(
                "task {task_id} cannot be claimed: current status is {}",
                status.as_str()
            )));
        }

        let blocked: Vec<String> = deps
            .iter()
            .filter(|dep_id| {
                list.tasks
                    .get(dep_id.as_str())
                    .is_some_and(|dep| dep.status != TaskStatus::Completed)
            })
            .cloned()
            .collect();
        if !blocked.is_empty() {
            return Err(Error::message(format!(
                "task {task_id} is blocked by incomplete tasks: {}",
                blocked.join(", ")
            )));
        }

        let task = list
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| Error::message(format!("task not found: {task_id}")))?;
        task.owner = Some(owner.to_string());
        task.status = TaskStatus::InProgress;
        task.updated_at = Self::now();

        let claimed = task.clone();
        drop(lists);
        self.persist(list_id).await?;
        Ok(claimed)
    }
}

/// Tool wrapper around [`TaskStore`].
pub struct TaskListTool {
    store: Arc<TaskStore>,
}

impl TaskListTool {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            store: Arc::new(TaskStore::new(base_dir)),
        }
    }
}

#[async_trait]
impl AgentTool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "Manage a shared task list for coordinated multi-agent execution. \
         Actions: create, list, list_lists, get, update, claim."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "list_lists", "get", "update", "claim"],
                    "description": "Task list action to perform. Use list_lists to discover all lists. Use list with list_id=\"*\" for all tasks."
                },
                "list_id": {
                    "type": "string",
                    "description": "Task list identifier. Use \"*\" (or omit) to list across all lists. Pass a specific ID to scope to one list."
                },
                "id": {
                    "type": "string",
                    "description": "Task ID for get/update/claim."
                },
                "subject": {
                    "type": "string",
                    "description": "Task subject for create/update."
                },
                "description": {
                    "type": "string",
                    "description": "Task description for create/update."
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "Task status for list/update."
                },
                "owner": {
                    "type": "string",
                    "description": "Task owner for update/claim."
                },
                "blocked_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of task IDs that block this task."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let action = require_str(&params, "action")?;
        let list_id = str_param_any(&params, &["list_id", "listId"]).unwrap_or("default");

        match action {
            "create" => {
                let subject = require_str(&params, "subject")?.to_string();
                let description = str_param(&params, "description").unwrap_or("").to_string();
                let task = self.store.create(list_id, subject, description).await?;
                Ok(serde_json::json!({
                    "ok": true,
                    "task": task,
                }))
            },
            "list" => {
                let status = str_param(&params, "status")
                    .map(str::parse::<TaskStatus>)
                    .transpose()?;
                let effective_id = if list_id == "default"
                    && params.get("list_id").is_none()
                    && params.get("listId").is_none()
                {
                    // When list_id is truly omitted, default to "*" so agents
                    // see tasks from all lists without guessing.
                    "*"
                } else {
                    list_id
                };
                let tasks = self.store.list_tasks(effective_id, status.as_ref()).await?;
                Ok(serde_json::json!({
                    "ok": true,
                    "tasks": tasks,
                    "count": tasks.len(),
                }))
            },
            "list_lists" => {
                let ids = self.store.list_ids().await?;
                Ok(serde_json::json!({
                    "ok": true,
                    "list_ids": ids,
                    "count": ids.len(),
                }))
            },
            "get" => {
                let id = require_str(&params, "id")?;
                let task = self.store.get(list_id, id).await?;
                Ok(serde_json::json!({
                    "ok": task.is_some(),
                    "task": task,
                }))
            },
            "update" => {
                let id = require_str(&params, "id")?;
                let status = str_param(&params, "status")
                    .map(str::parse::<TaskStatus>)
                    .transpose()?;
                let subject = str_param(&params, "subject").map(String::from);
                let description = str_param(&params, "description").map(String::from);
                let owner = str_param(&params, "owner").map(String::from);
                let blocked_by = params
                    .get("blocked_by")
                    .and_then(serde_json::Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(String::from)
                            .collect::<Vec<_>>()
                    });
                let task = self
                    .store
                    .update(list_id, id, status, subject, description, owner, blocked_by)
                    .await?;
                Ok(serde_json::json!({
                    "ok": true,
                    "task": task,
                }))
            },
            "claim" => {
                let id = require_str(&params, "id")?;
                let owner = str_param_any(&params, &["owner", "_session_key"])
                    .unwrap_or("agent")
                    .to_string();
                let task = self.store.claim(list_id, id, &owner).await?;
                Ok(serde_json::json!({
                    "ok": true,
                    "task": task,
                }))
            },
            _ => Err(Error::message(format!("unknown task_list action: {action}")).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    fn tool(tmp: &tempfile::TempDir) -> TaskListTool {
        TaskListTool::new(tmp.path())
    }

    #[tokio::test]
    async fn create_and_list_tasks() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "first",
                "description": "desc"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "list"
            }))
            .await?;
        assert_eq!(result["count"], 1);
        assert_eq!(result["tasks"][0]["subject"], "first");
        assert_eq!(result["tasks"][0]["status"], "pending");
        Ok(())
    }

    #[tokio::test]
    async fn claim_moves_task_to_in_progress() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);
        let created = task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "work"
            }))
            .await?;
        let id = created["task"]["id"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("missing task id"))?;

        let claimed = task_tool
            .execute(serde_json::json!({
                "action": "claim",
                "id": id,
                "owner": "worker-a"
            }))
            .await?;
        assert_eq!(claimed["task"]["status"], "in_progress");
        assert_eq!(claimed["task"]["owner"], "worker-a");
        Ok(())
    }

    #[tokio::test]
    async fn claim_rejects_non_pending_task() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);
        let created = task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "work"
            }))
            .await?;
        let id = created["task"]["id"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("missing task id"))?;

        task_tool
            .execute(serde_json::json!({
                "action": "update",
                "id": id,
                "status": "completed"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "claim",
                "id": id,
                "owner": "worker-a"
            }))
            .await;
        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected claim failure"))?;
        assert!(err.to_string().contains("cannot be claimed"));
        Ok(())
    }

    #[tokio::test]
    async fn claim_rejects_when_blocked_dependencies_incomplete() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);
        let dep = task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "dep"
            }))
            .await?;
        let dep_id = dep["task"]["id"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("missing dep id"))?;

        let main = task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "main"
            }))
            .await?;
        let main_id = main["task"]["id"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("missing main id"))?;

        task_tool
            .execute(serde_json::json!({
                "action": "update",
                "id": main_id,
                "blocked_by": [dep_id]
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "claim",
                "id": main_id
            }))
            .await;
        let err = result
            .err()
            .ok_or_else(|| std::io::Error::other("expected blocked claim failure"))?;
        assert!(err.to_string().contains("blocked by incomplete tasks"));
        Ok(())
    }

    #[tokio::test]
    async fn list_without_list_id_returns_all_tasks() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        // Create tasks in two different lists.
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "CURRICULUM_1",
                "subject": "task-a",
                "description": "in list 1"
            }))
            .await?;
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "CURRICULUM_2",
                "subject": "task-b",
                "description": "in list 2"
            }))
            .await?;

        // Omitting list_id should now default to "*" and return both.
        let result = task_tool
            .execute(serde_json::json!({
                "action": "list"
            }))
            .await?;
        assert_eq!(result["count"], 2);

        let subjects: Vec<&str> = result["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["subject"].as_str().unwrap())
            .collect();
        assert!(subjects.contains(&"task-a"));
        assert!(subjects.contains(&"task-b"));
        Ok(())
    }

    #[tokio::test]
    async fn list_with_wildcard_returns_all_tasks() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "ALPHA",
                "subject": "alpha-task"
            }))
            .await?;
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "subject": "default-task"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "list",
                "list_id": "*"
            }))
            .await?;
        assert_eq!(result["count"], 2);
        Ok(())
    }

    #[tokio::test]
    async fn list_lists_returns_all_known_ids() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "LIST_X",
                "subject": "x"
            }))
            .await?;
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "LIST_Y",
                "subject": "y"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "list_lists"
            }))
            .await?;
        assert_eq!(result["count"], 2);

        let ids: Vec<&str> = result["list_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(ids.contains(&"LIST_X"));
        assert!(ids.contains(&"LIST_Y"));
        Ok(())
    }

    #[tokio::test]
    async fn explicit_default_list_id_still_scopes() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "default",
                "subject": "in-default"
            }))
            .await?;
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "OTHER",
                "subject": "in-other"
            }))
            .await?;

        // Explicitly passing list_id="default" should only return default tasks.
        let result = task_tool
            .execute(serde_json::json!({
                "action": "list",
                "list_id": "default"
            }))
            .await?;
        assert_eq!(result["count"], 1);
        assert_eq!(result["tasks"][0]["subject"], "in-default");
        Ok(())
    }

    #[tokio::test]
    async fn wildcard_list_includes_list_id_field() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "LIST_A",
                "subject": "a-task"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "list",
                "list_id": "*"
            }))
            .await?;
        assert_eq!(result["count"], 1);
        assert_eq!(result["tasks"][0]["list_id"], "LIST_A");
        Ok(())
    }

    #[tokio::test]
    async fn single_list_includes_list_id_field() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "X",
                "subject": "x-task"
            }))
            .await?;

        let result = task_tool
            .execute(serde_json::json!({
                "action": "list",
                "list_id": "X"
            }))
            .await?;
        assert_eq!(result["tasks"][0]["list_id"], "X");
        Ok(())
    }

    #[tokio::test]
    async fn list_lists_excludes_phantom_empty_lists() -> TestResult<()> {
        let tmp = tempfile::tempdir()?;
        let task_tool = tool(&tmp);

        // Create a real list with a task.
        task_tool
            .execute(serde_json::json!({
                "action": "create",
                "list_id": "REAL",
                "subject": "real-task"
            }))
            .await?;

        // Try to get from a non-existent list — this creates a phantom empty list.
        let get_result = task_tool
            .execute(serde_json::json!({
                "action": "get",
                "list_id": "PHANTOM",
                "id": "999"
            }))
            .await?;
        assert_eq!(get_result["ok"], false);

        // list_lists should only return REAL, not PHANTOM.
        let result = task_tool
            .execute(serde_json::json!({
                "action": "list_lists"
            }))
            .await?;
        assert_eq!(result["count"], 1);
        assert_eq!(result["list_ids"][0], "REAL");
        Ok(())
    }
}
