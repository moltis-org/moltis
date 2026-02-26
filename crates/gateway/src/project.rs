use std::sync::Arc;

use {async_trait::async_trait, serde_json::Value, tracing::warn};

use moltis_projects::{
    ProjectStore,
    complete::complete_path,
    context::load_context_files,
    detect::{auto_detect, default_scan_dirs},
};

use crate::services::{ProjectService, ServiceError, ServiceResult};

pub struct LiveProjectService {
    store: Arc<dyn ProjectStore>,
}

impl LiveProjectService {
    pub fn new(store: Arc<dyn ProjectStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ProjectService for LiveProjectService {
    async fn list(&self) -> ServiceResult {
        let projects = self.store.list().await.map_err(ServiceError::message)?;
        Ok(serde_json::to_value(projects)?)
    }

    async fn get(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id' parameter".to_string())?;
        let project = self.store.get(id).await.map_err(ServiceError::message)?;
        Ok(serde_json::to_value(project)?)
    }

    async fn upsert(&self, params: Value) -> ServiceResult {
        let project: moltis_projects::Project = serde_json::from_value(params)?;
        self.store
            .upsert(project.clone())
            .await
            .map_err(ServiceError::message)?;
        Ok(serde_json::to_value(project)?)
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id' parameter".to_string())?;
        self.store.delete(id).await.map_err(ServiceError::message)?;
        Ok(serde_json::json!({"deleted": id}))
    }

    async fn detect(&self, params: Value) -> ServiceResult {
        let dirs: Vec<String> = params
            .get("directories")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let existing = self.store.list().await.map_err(ServiceError::message)?;
        let known_ids: Vec<String> = existing.iter().map(|p| p.id.clone()).collect();

        let dir_refs: Vec<std::path::PathBuf> = if dirs.is_empty() {
            default_scan_dirs()
        } else {
            dirs.iter().map(std::path::PathBuf::from).collect()
        };
        let dir_slices: Vec<&std::path::Path> = dir_refs.iter().map(|p| p.as_path()).collect();
        let detected = auto_detect(&dir_slices, &known_ids);

        for p in &detected {
            if let Err(e) = self.store.upsert(p.clone()).await {
                warn!(id = %p.id, error = %e, "failed to persist detected project");
            }
        }

        Ok(serde_json::to_value(detected)?)
    }

    async fn complete_path(&self, params: Value) -> ServiceResult {
        let partial = params.get("partial").and_then(|v| v.as_str()).unwrap_or("");
        let results: Vec<String> = complete_path(partial)
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        Ok(serde_json::to_value(results)?)
    }

    async fn context(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id' parameter".to_string())?;
        let project = self
            .store
            .get(id)
            .await
            .map_err(ServiceError::message)?
            .ok_or_else(|| format!("project '{id}' not found"))?;

        let context_files =
            load_context_files(&project.directory).map_err(ServiceError::message)?;
        let entries: Vec<Value> = context_files
            .iter()
            .map(|cf| {
                serde_json::json!({
                    "path": cf.path.to_string_lossy(),
                    "content": cf.content,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "project": project,
            "context_files": entries,
        }))
    }
}
