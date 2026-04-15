use serde_json::json;

use crate::{
    BrowserService, ChatService, NoopBrowserService, ServiceResult,
    interfaces::model_service_not_configured_error,
};

struct SlowShutdownBrowserService;

struct DefaultRefreshChatService;

#[async_trait::async_trait]
impl ChatService for DefaultRefreshChatService {
    async fn send(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn abort(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn cancel_queued(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn history(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!([]))
    }

    async fn inject(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn clear(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn compact(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn context(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn raw_prompt(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn full_context(&self, _params: serde_json::Value) -> ServiceResult {
        Ok(json!({}))
    }
}

#[async_trait::async_trait]
impl BrowserService for SlowShutdownBrowserService {
    async fn request(&self, _p: serde_json::Value) -> ServiceResult {
        Err("not used".into())
    }

    async fn shutdown(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn noop_browser_service_lifecycle_methods() {
    let svc = NoopBrowserService;
    svc.cleanup_idle().await;
    svc.shutdown().await;
    assert!(
        svc.shutdown_with_grace(std::time::Duration::from_millis(10))
            .await
    );
    svc.close_all().await;
}

#[tokio::test]
async fn noop_browser_service_request_returns_error() {
    let svc = NoopBrowserService;
    let result = svc.request(serde_json::json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn browser_shutdown_with_grace_times_out() {
    let svc = SlowShutdownBrowserService;
    assert!(
        !svc.shutdown_with_grace(std::time::Duration::from_millis(5))
            .await
    );
}

#[test]
fn model_service_not_configured_error_returns_expected_message() {
    let error = model_service_not_configured_error("models.test");
    assert_eq!(error.to_string(), "model service not configured");
}

#[tokio::test]
async fn chat_service_default_refresh_prompt_memory_returns_not_configured() {
    let svc = DefaultRefreshChatService;
    let error = match svc
        .refresh_prompt_memory(json!({ "sessionKey": "session-a" }))
        .await
    {
        Ok(value) => panic!("default refresh should be unavailable, got {value:?}"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "chat not configured");
}
