//! Live cron service implementation wiring the cron crate into gateway services.

use std::sync::Arc;

use {async_trait::async_trait, serde_json::Value, tracing::error};

use moltis_cron::{
    service::CronService,
    types::{CronJobCreate, CronJobPatch},
};

use crate::services::{CronService as CronServiceTrait, ServiceError, ServiceResult};

/// Gateway-facing cron service backed by the real [`moltis_cron::service::CronService`].
pub struct LiveCronService {
    inner: Arc<CronService>,
}

impl LiveCronService {
    pub fn new(inner: Arc<CronService>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &Arc<CronService> {
        &self.inner
    }
}

#[async_trait]
impl CronServiceTrait for LiveCronService {
    async fn list(&self) -> ServiceResult {
        let jobs = self.inner.list().await;
        Ok(serde_json::to_value(jobs)?)
    }

    async fn status(&self) -> ServiceResult {
        let status = self.inner.status().await;
        Ok(serde_json::to_value(status)?)
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let create: CronJobCreate = serde_json::from_value(params)
            .map_err(|e| ServiceError::message(format!("invalid job spec: {e}")))?;
        let job = self.inner.add(create).await.map_err(|e| {
            error!(error = %e, "cron add failed");
            ServiceError::message(e)
        })?;
        Ok(serde_json::to_value(job)?)
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        let patch: CronJobPatch = serde_json::from_value(
            params
                .get("patch")
                .cloned()
                .unwrap_or(Value::Object(Default::default())),
        )
        .map_err(|e| ServiceError::message(format!("invalid patch: {e}")))?;
        let job = self
            .inner
            .update(id, patch)
            .await
            .map_err(ServiceError::message)?;
        Ok(serde_json::to_value(job)?)
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        self.inner.remove(id).await.map_err(ServiceError::message)?;
        Ok(serde_json::json!({ "removed": id }))
    }

    async fn run(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        self.inner
            .run(id, force)
            .await
            .map_err(ServiceError::message)?;
        Ok(serde_json::json!({ "ran": id }))
    }

    async fn runs(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id'".to_string())?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let runs = self
            .inner
            .runs(id, limit)
            .await
            .map_err(ServiceError::message)?;
        Ok(serde_json::to_value(runs)?)
    }
}
