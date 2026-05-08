//! Project-aware wrapper for code-index tools.
//!
//! Wraps the raw code-index tools (`codebase_search`, `codebase_peek`,
//! `codebase_status`) to check whether code indexing is enabled for the
//! given project before delegating execution.

use std::sync::Arc;

use {
    async_trait::async_trait, moltis_agents::tool_registry::AgentTool,
    moltis_projects::ProjectStore, serde_json::json,
};

/// Wraps a code-index tool, returning a "disabled" response when
/// `code_index_enabled` is `false` for the specified project.
pub struct ProjectAwareCodeIndexTool {
    inner: Arc<dyn AgentTool>,
    project_store: Arc<dyn ProjectStore>,
}

impl ProjectAwareCodeIndexTool {
    pub fn new(inner: Box<dyn AgentTool>, project_store: Arc<dyn ProjectStore>) -> Self {
        Self {
            inner: Arc::from(inner),
            project_store,
        }
    }

    /// Extract the `project_id` from tool parameters.
    fn project_id(params: &serde_json::Value) -> Option<String> {
        params
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Check whether code indexing is enabled for the given project.
    async fn is_enabled(&self, project_id: &str) -> bool {
        match self.project_store.get(project_id).await {
            Ok(Some(project)) => project.code_index_enabled,
            _ => true, // Default to enabled if project not found (e.g. agent passes raw id)
        }
    }
}

#[async_trait]
impl AgentTool for ProjectAwareCodeIndexTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        self.inner.warmup().await
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        if let Some(ref pid) = Self::project_id(&params)
            && !self.is_enabled(pid).await
        {
            return Ok(json!({
                "disabled": true,
                "message": format!("Code indexing is disabled for project '{pid}'. Enable it in project settings."),
            }));
        }

        self.inner.execute(params).await
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_projects::{Project, ProjectStore},
        std::{collections::HashMap, path::PathBuf},
        tokio::sync::Mutex,
    };

    /// In-memory mock project store for testing.
    struct MockProjectStore {
        projects: Mutex<HashMap<String, Project>>,
    }

    impl MockProjectStore {
        fn new(projects: Vec<Project>) -> Self {
            let map = projects.into_iter().map(|p| (p.id.clone(), p)).collect();
            Self {
                projects: Mutex::new(map),
            }
        }
    }

    fn test_project(id: &str, code_index_enabled: bool) -> Project {
        Project {
            id: id.to_string(),
            label: id.to_string(),
            directory: PathBuf::from(format!("/tmp/{id}")),
            system_prompt: None,
            auto_worktree: false,
            setup_command: None,
            teardown_command: None,
            branch_prefix: None,
            sandbox_image: None,
            detected: false,
            code_index_enabled,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    #[async_trait]
    impl ProjectStore for MockProjectStore {
        async fn list(&self) -> moltis_projects::Result<Vec<Project>> {
            Ok(self.projects.lock().await.values().cloned().collect())
        }

        async fn get(&self, id: &str) -> moltis_projects::Result<Option<Project>> {
            Ok(self.projects.lock().await.get(id).cloned())
        }

        async fn upsert(&self, project: Project) -> moltis_projects::Result<()> {
            self.projects
                .lock()
                .await
                .insert(project.id.clone(), project);
            Ok(())
        }

        async fn delete(&self, id: &str) -> moltis_projects::Result<()> {
            self.projects.lock().await.remove(id);
            Ok(())
        }
    }

    /// Mock inner tool that records calls and returns a fixed response.
    struct MockInnerTool {
        name: &'static str,
        call_count: Mutex<usize>,
        last_params: Mutex<Option<serde_json::Value>>,
        response: serde_json::Value,
    }

    impl MockInnerTool {
        fn new(response: serde_json::Value) -> Self {
            Self {
                name: "mock_tool",
                call_count: Mutex::new(0),
                last_params: Mutex::new(None),
                response,
            }
        }
    }

    #[async_trait]
    impl AgentTool for MockInnerTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "mock tool for testing"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "query": { "type": "string" }
                }
            })
        }

        async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            *self.call_count.lock().await += 1;
            *self.last_params.lock().await = Some(params);
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn delegates_to_inner_when_enabled() {
        let store: Arc<dyn ProjectStore> = Arc::new(MockProjectStore::new(vec![test_project(
            "enabled-proj",
            true,
        )]));
        let inner = MockInnerTool::new(json!({"results": []}));
        let wrapper = ProjectAwareCodeIndexTool::new(Box::new(inner), store);

        let result = wrapper
            .execute(json!({
                "project_id": "enabled-proj",
                "query": "foo"
            }))
            .await
            .unwrap();

        assert_eq!(result, json!({"results": []}));
    }

    #[tokio::test]
    async fn returns_disabled_when_code_index_off() {
        let store: Arc<dyn ProjectStore> = Arc::new(MockProjectStore::new(vec![test_project(
            "disabled-proj",
            false,
        )]));
        let inner = MockInnerTool::new(json!({"results": []}));
        let wrapper = ProjectAwareCodeIndexTool::new(Box::new(inner), Arc::clone(&store));

        let result = wrapper
            .execute(json!({
                "project_id": "disabled-proj",
                "query": "foo"
            }))
            .await
            .unwrap();

        assert_eq!(result["disabled"], json!(true));
        assert!(
            result["message"]
                .as_str()
                .is_some_and(|m| m.contains("disabled-proj"))
        );
    }

    #[tokio::test]
    async fn delegates_when_project_not_found() {
        let store: Arc<dyn ProjectStore> = Arc::new(MockProjectStore::new(vec![]));
        let inner = MockInnerTool::new(json!({"results": []}));
        let wrapper = ProjectAwareCodeIndexTool::new(Box::new(inner), store);

        let result = wrapper
            .execute(json!({
                "project_id": "unknown-proj",
                "query": "foo"
            }))
            .await
            .unwrap();

        // Unknown project defaults to enabled → delegates to inner
        assert_eq!(result, json!({"results": []}));
    }

    #[tokio::test]
    async fn delegates_when_no_project_id_param() {
        let store: Arc<dyn ProjectStore> = Arc::new(MockProjectStore::new(vec![test_project(
            "some-proj",
            false,
        )]));
        let inner = MockInnerTool::new(json!({"ok": true}));
        let wrapper = ProjectAwareCodeIndexTool::new(Box::new(inner), store);

        let result = wrapper
            .execute(json!({
                "query": "foo"
            }))
            .await
            .unwrap();

        // No project_id → skip gating, delegate
        assert_eq!(result, json!({"ok": true}));
    }

    #[tokio::test]
    async fn forwards_name_and_schema() {
        let store: Arc<dyn ProjectStore> = Arc::new(MockProjectStore::new(vec![]));
        let inner = MockInnerTool::new(json!(null));
        let wrapper = ProjectAwareCodeIndexTool::new(Box::new(inner), store);

        assert_eq!(wrapper.name(), "mock_tool");
        assert_eq!(wrapper.description(), "mock tool for testing");
        assert!(wrapper.parameters_schema().is_object());
    }
}
