use std::path::PathBuf;

use {async_trait::async_trait, serde_json::Value, tracing::warn};

use crate::{ServiceError, ServiceResult};

#[async_trait]
pub trait AgentService: Send + Sync {
    async fn run(&self, params: Value) -> ServiceResult;
    async fn run_wait(&self, params: Value) -> ServiceResult;
    async fn identity_get(&self) -> ServiceResult;
    async fn list(&self) -> ServiceResult;
}

pub struct NoopAgentService;

#[async_trait]
impl AgentService for NoopAgentService {
    async fn run(&self, _params: Value) -> ServiceResult {
        Err("agent service not configured".into())
    }

    async fn run_wait(&self, _params: Value) -> ServiceResult {
        Err("agent service not configured".into())
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "name": "moltis", "avatar": null }))
    }

    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

#[async_trait]
pub trait SessionService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn preview(&self, params: Value) -> ServiceResult;
    async fn resolve(&self, params: Value) -> ServiceResult;
    async fn patch(&self, params: Value) -> ServiceResult;
    async fn voice_generate(&self, params: Value) -> ServiceResult;
    async fn share_create(&self, params: Value) -> ServiceResult;
    async fn share_list(&self, params: Value) -> ServiceResult;
    async fn share_revoke(&self, params: Value) -> ServiceResult;
    async fn reset(&self, params: Value) -> ServiceResult;
    async fn delete(&self, params: Value) -> ServiceResult;
    async fn compact(&self, params: Value) -> ServiceResult;
    async fn search(&self, params: Value) -> ServiceResult;
    async fn fork(&self, params: Value) -> ServiceResult;
    async fn branches(&self, params: Value) -> ServiceResult;
    async fn run_detail(&self, params: Value) -> ServiceResult;
    async fn clear_all(&self) -> ServiceResult;
    async fn mark_seen(&self, key: &str);
}

pub struct NoopSessionService;

#[async_trait]
impl SessionService for NoopSessionService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn preview(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn resolve(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn patch(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn voice_generate(&self, _p: Value) -> ServiceResult {
        Err("session voice generation not available".into())
    }

    async fn share_create(&self, _p: Value) -> ServiceResult {
        Err("session sharing not available".into())
    }

    async fn share_list(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn share_revoke(&self, _p: Value) -> ServiceResult {
        Err("session sharing not available".into())
    }

    async fn reset(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn delete(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn compact(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn search(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn fork(&self, _p: Value) -> ServiceResult {
        Err("session forking not available".into())
    }

    async fn branches(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn run_detail(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn clear_all(&self) -> ServiceResult {
        Ok(serde_json::json!({ "deleted": 0 }))
    }

    async fn mark_seen(&self, _key: &str) {}
}

#[async_trait]
pub trait ChannelService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn logout(&self, params: Value) -> ServiceResult;
    async fn send(&self, params: Value) -> ServiceResult;
    async fn add(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn retry_ownership(&self, params: Value) -> ServiceResult;
    async fn senders_list(&self, params: Value) -> ServiceResult;
    async fn sender_approve(&self, params: Value) -> ServiceResult;
    async fn sender_deny(&self, params: Value) -> ServiceResult;

    async fn oauth_start(&self, params: Value) -> ServiceResult {
        let _ = params;
        Err("channel OAuth not supported".into())
    }

    async fn oauth_complete(&self, params: Value) -> ServiceResult {
        let _ = params;
        Err("channel OAuth not supported".into())
    }
}

pub struct NoopChannelService;

#[async_trait]
impl ChannelService for NoopChannelService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "channels": [] }))
    }

    async fn logout(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn send(&self, _p: Value) -> ServiceResult {
        Err("no channels configured".into())
    }

    async fn add(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn remove(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn update(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn retry_ownership(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn senders_list(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn sender_approve(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn sender_deny(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }
}

#[async_trait]
pub trait ConfigService: Send + Sync {
    async fn get(&self, params: Value) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn apply(&self, params: Value) -> ServiceResult;
    async fn patch(&self, params: Value) -> ServiceResult;
    async fn schema(&self) -> ServiceResult;
}

pub struct NoopConfigService;

#[async_trait]
impl ConfigService for NoopConfigService {
    async fn get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn apply(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn patch(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn schema(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

#[async_trait]
pub trait CronService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn add(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn run(&self, params: Value) -> ServiceResult;
    async fn runs(&self, params: Value) -> ServiceResult;
}

pub struct NoopCronService;

#[async_trait]
impl CronService for NoopCronService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "running": false }))
    }

    async fn add(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn update(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn remove(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn run(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn runs(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

#[async_trait]
pub trait WebhooksService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn get(&self, params: Value) -> ServiceResult;
    async fn create(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn delete(&self, params: Value) -> ServiceResult;
    async fn deliveries(&self, params: Value) -> ServiceResult;
    async fn delivery_get(&self, params: Value) -> ServiceResult;
    async fn delivery_payload(&self, params: Value) -> ServiceResult;
    async fn delivery_actions(&self, params: Value) -> ServiceResult;
    async fn profiles(&self) -> ServiceResult;
}

pub struct NoopWebhooksService;

#[async_trait]
impl WebhooksService for NoopWebhooksService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn get(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn create(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn update(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn delete(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn deliveries(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn delivery_get(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn delivery_payload(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn delivery_actions(&self, _params: Value) -> ServiceResult {
        Err("webhooks not configured".into())
    }

    async fn profiles(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

#[async_trait]
pub trait ChatService: Send + Sync {
    async fn send(&self, params: Value) -> ServiceResult;

    async fn send_sync(&self, params: Value) -> ServiceResult {
        self.send(params).await
    }

    async fn abort(&self, params: Value) -> ServiceResult;
    async fn cancel_queued(&self, params: Value) -> ServiceResult;
    async fn history(&self, params: Value) -> ServiceResult;
    async fn inject(&self, params: Value) -> ServiceResult;
    async fn clear(&self, params: Value) -> ServiceResult;
    async fn compact(&self, params: Value) -> ServiceResult;
    async fn context(&self, params: Value) -> ServiceResult;
    async fn raw_prompt(&self, params: Value) -> ServiceResult;
    async fn full_context(&self, params: Value) -> ServiceResult;

    async fn refresh_prompt_memory(&self, _params: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn active(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({ "active": false }))
    }

    async fn active_session_keys(&self) -> Vec<String> {
        Vec::new()
    }

    async fn active_thinking_text(&self, _session_key: &str) -> Option<String> {
        None
    }

    async fn active_voice_pending(&self, _session_key: &str) -> bool {
        false
    }

    async fn peek(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({ "active": false }))
    }
}

pub struct NoopChatService;

#[async_trait]
impl ChatService for NoopChatService {
    async fn send(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn abort(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn cancel_queued(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "cleared": 0 }))
    }

    async fn history(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn inject(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn clear(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn compact(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn context(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "session": {}, "project": null, "tools": [], "providers": [] }))
    }

    async fn raw_prompt(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn full_context(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn refresh_prompt_memory(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }
}

#[async_trait]
pub trait TtsService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn providers(&self) -> ServiceResult;
    async fn enable(&self, params: Value) -> ServiceResult;
    async fn disable(&self) -> ServiceResult;
    async fn convert(&self, params: Value) -> ServiceResult;
    async fn set_provider(&self, params: Value) -> ServiceResult;
}

pub struct NoopTtsService;

#[async_trait]
impl TtsService for NoopTtsService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "enabled": false }))
    }

    async fn providers(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn enable(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }

    async fn disable(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn convert(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }

    async fn set_provider(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }
}

#[async_trait]
pub trait SttService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn providers(&self) -> ServiceResult;
    async fn transcribe(&self, params: Value) -> ServiceResult;

    async fn transcribe_bytes(
        &self,
        audio: bytes::Bytes,
        format: &str,
        provider: Option<&str>,
        language: Option<&str>,
        prompt: Option<&str>,
    ) -> ServiceResult;

    async fn set_provider(&self, params: Value) -> ServiceResult;
}

pub struct NoopSttService;

#[async_trait]
impl SttService for NoopSttService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "enabled": false, "configured": false }))
    }

    async fn providers(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn transcribe(&self, _params: Value) -> ServiceResult {
        Err("STT not available".into())
    }

    async fn transcribe_bytes(
        &self,
        _audio: bytes::Bytes,
        _format: &str,
        _provider: Option<&str>,
        _language: Option<&str>,
        _prompt: Option<&str>,
    ) -> ServiceResult {
        Err("STT not available".into())
    }

    async fn set_provider(&self, _params: Value) -> ServiceResult {
        Err("STT not available".into())
    }
}

#[async_trait]
pub trait McpService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn add(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn enable(&self, params: Value) -> ServiceResult;
    async fn disable(&self, params: Value) -> ServiceResult;
    async fn status(&self, params: Value) -> ServiceResult;
    async fn tools(&self, params: Value) -> ServiceResult;
    async fn restart(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn reauth(&self, params: Value) -> ServiceResult;
    async fn oauth_start(&self, params: Value) -> ServiceResult;
    async fn oauth_complete(&self, params: Value) -> ServiceResult;

    async fn update_request_timeout(&self, _request_timeout_secs: u64) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true }))
    }
}

pub struct NoopMcpService;

#[async_trait]
impl McpService for NoopMcpService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!({ "servers": [] }))
    }

    async fn add(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn remove(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn enable(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn disable(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn status(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn tools(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn restart(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn update(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn reauth(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn oauth_start(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn oauth_complete(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }
}

#[async_trait]
pub trait SkillsService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn bins(&self) -> ServiceResult;
    async fn install(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn list(&self) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn repos_list(&self) -> ServiceResult;
    async fn repos_list_full(&self) -> ServiceResult;
    async fn repos_remove(&self, params: Value) -> ServiceResult;
    async fn repos_export(&self, params: Value) -> ServiceResult;
    async fn repos_import(&self, params: Value) -> ServiceResult;
    async fn repos_unquarantine(&self, params: Value) -> ServiceResult;
    async fn emergency_disable(&self) -> ServiceResult;
    async fn skill_enable(&self, params: Value) -> ServiceResult;
    async fn skill_disable(&self, params: Value) -> ServiceResult;
    async fn skill_trust(&self, params: Value) -> ServiceResult;
    async fn skill_detail(&self, params: Value) -> ServiceResult;
    async fn install_dep(&self, params: Value) -> ServiceResult;
    async fn security_status(&self) -> ServiceResult;
    async fn security_scan(&self) -> ServiceResult;
    async fn skill_save(&self, params: Value) -> ServiceResult;
}

pub struct NoopSkillsStub;

#[async_trait]
impl SkillsService for NoopSkillsStub {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "installed": [] }))
    }

    async fn bins(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn install(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn update(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn remove(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn repos_list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn repos_list_full(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn repos_remove(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn repos_export(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn repos_import(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn repos_unquarantine(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn emergency_disable(&self) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn skill_enable(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn skill_disable(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn skill_trust(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn skill_detail(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn install_dep(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn security_status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn security_scan(&self) -> ServiceResult {
        Err("skills service not configured".into())
    }

    async fn skill_save(&self, _params: Value) -> ServiceResult {
        Err("skills service not configured".into())
    }
}

#[async_trait]
pub trait BrowserService: Send + Sync {
    async fn request(&self, params: Value) -> ServiceResult;

    async fn warmup(&self) {}
    async fn cleanup_idle(&self) {}
    async fn shutdown(&self) {}

    async fn shutdown_with_grace(&self, grace: std::time::Duration) -> bool {
        tokio::time::timeout(grace, self.shutdown()).await.is_ok()
    }

    async fn close_all(&self) {}
}

pub struct NoopBrowserService;

#[async_trait]
impl BrowserService for NoopBrowserService {
    async fn request(&self, _p: Value) -> ServiceResult {
        Err("browser not available".into())
    }
}

#[async_trait]
pub trait UsageService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn cost(&self, params: Value) -> ServiceResult;
}

pub struct NoopUsageService;

#[async_trait]
impl UsageService for NoopUsageService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "totalCost": 0, "requests": 0 }))
    }

    async fn cost(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "cost": 0 }))
    }
}

#[async_trait]
pub trait ExecApprovalService: Send + Sync {
    async fn get(&self) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn node_get(&self, params: Value) -> ServiceResult;
    async fn node_set(&self, params: Value) -> ServiceResult;
    async fn request(&self, params: Value) -> ServiceResult;
    async fn resolve(&self, params: Value) -> ServiceResult;
}

pub struct NoopExecApprovalService;

#[async_trait]
impl ExecApprovalService for NoopExecApprovalService {
    async fn get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "mode": "always" }))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn node_get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "mode": "always" }))
    }

    async fn node_set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn request(&self, _p: Value) -> ServiceResult {
        Err("approvals not configured".into())
    }

    async fn resolve(&self, _p: Value) -> ServiceResult {
        Err("approvals not configured".into())
    }
}

#[async_trait]
pub trait OnboardingService: Send + Sync {
    async fn wizard_start(&self, params: Value) -> ServiceResult;
    async fn wizard_next(&self, params: Value) -> ServiceResult;
    async fn wizard_cancel(&self) -> ServiceResult;
    async fn wizard_status(&self) -> ServiceResult;
    async fn identity_get(&self) -> ServiceResult;
    async fn identity_update(&self, params: Value) -> ServiceResult;
    async fn identity_update_soul(&self, soul: Option<String>) -> ServiceResult;
    async fn openclaw_detect(&self) -> ServiceResult;
    async fn openclaw_scan(&self) -> ServiceResult;
    async fn openclaw_import(&self, params: Value) -> ServiceResult;
}

pub struct NoopOnboardingService;

#[async_trait]
impl OnboardingService for NoopOnboardingService {
    async fn wizard_start(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0 }))
    }

    async fn wizard_next(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0, "done": true }))
    }

    async fn wizard_cancel(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn wizard_status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "active": false }))
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "name": "moltis", "avatar": null }))
    }

    async fn identity_update(&self, _params: Value) -> ServiceResult {
        Err("onboarding service not configured".into())
    }

    async fn identity_update_soul(&self, _soul: Option<String>) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn openclaw_detect(&self) -> ServiceResult {
        Ok(serde_json::json!({ "detected": false }))
    }

    async fn openclaw_scan(&self) -> ServiceResult {
        Err("onboarding service not configured".into())
    }

    async fn openclaw_import(&self, _params: Value) -> ServiceResult {
        Err("onboarding service not configured".into())
    }
}

#[async_trait]
pub trait UpdateService: Send + Sync {
    async fn run(&self, params: Value) -> ServiceResult;
}

pub struct NoopUpdateService;

#[async_trait]
impl UpdateService for NoopUpdateService {
    async fn run(&self, _p: Value) -> ServiceResult {
        Err("update not available".into())
    }
}

#[async_trait]
pub trait ModelService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn list_all(&self) -> ServiceResult;
    async fn disable(&self, params: Value) -> ServiceResult;
    async fn enable(&self, params: Value) -> ServiceResult;
    async fn detect_supported(&self, params: Value) -> ServiceResult;
    async fn cancel_detect(&self) -> ServiceResult;
    async fn test(&self, params: Value) -> ServiceResult;
}

pub struct NoopModelService;

pub(crate) fn model_service_not_configured_error(operation: &'static str) -> ServiceError {
    warn!(
        operation,
        "model service not configured (gateway services not fully initialized)"
    );
    "model service not configured".into()
}

#[async_trait]
impl ModelService for NoopModelService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn list_all(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn disable(&self, _params: Value) -> ServiceResult {
        Err(model_service_not_configured_error("models.disable"))
    }

    async fn enable(&self, _params: Value) -> ServiceResult {
        Err(model_service_not_configured_error("models.enable"))
    }

    async fn detect_supported(&self, _params: Value) -> ServiceResult {
        Err(model_service_not_configured_error(
            "models.detect_supported",
        ))
    }

    async fn cancel_detect(&self) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true, "cancelled": false }))
    }

    async fn test(&self, _params: Value) -> ServiceResult {
        Err(model_service_not_configured_error("models.test"))
    }
}

#[async_trait]
pub trait WebLoginService: Send + Sync {
    async fn start(&self, params: Value) -> ServiceResult;
    async fn wait(&self, params: Value) -> ServiceResult;
}

pub struct NoopWebLoginService;

#[async_trait]
impl WebLoginService for NoopWebLoginService {
    async fn start(&self, _p: Value) -> ServiceResult {
        Err("web login not available".into())
    }

    async fn wait(&self, _p: Value) -> ServiceResult {
        Err("web login not available".into())
    }
}

#[async_trait]
pub trait VoicewakeService: Send + Sync {
    async fn get(&self) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn wake(&self, params: Value) -> ServiceResult;
    async fn talk_mode(&self, params: Value) -> ServiceResult;
}

pub struct NoopVoicewakeService;

#[async_trait]
impl VoicewakeService for NoopVoicewakeService {
    async fn get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "enabled": false }))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn wake(&self, _p: Value) -> ServiceResult {
        Err("voicewake not available".into())
    }

    async fn talk_mode(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

#[async_trait]
pub trait LogsService: Send + Sync {
    async fn tail(&self, params: Value) -> ServiceResult;
    async fn list(&self, params: Value) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn ack(&self) -> ServiceResult;
    fn log_file_path(&self) -> Option<PathBuf>;
}

pub struct NoopLogsService;

#[async_trait]
impl LogsService for NoopLogsService {
    async fn tail(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "subscribed": true }))
    }

    async fn list(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "entries": [] }))
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({
            "unseen_warns": 0,
            "unseen_errors": 0,
            "enabled_levels": { "debug": false, "trace": false }
        }))
    }

    async fn ack(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    fn log_file_path(&self) -> Option<PathBuf> {
        None
    }
}

#[async_trait]
pub trait ProviderSetupService: Send + Sync {
    async fn available(&self) -> ServiceResult;
    async fn save_key(&self, params: Value) -> ServiceResult;
    async fn oauth_start(&self, params: Value) -> ServiceResult;
    async fn oauth_complete(&self, params: Value) -> ServiceResult;
    async fn oauth_status(&self, params: Value) -> ServiceResult;
    async fn remove_key(&self, params: Value) -> ServiceResult;
    async fn validate_key(&self, params: Value) -> ServiceResult;
    async fn save_model(&self, params: Value) -> ServiceResult;
    async fn save_models(&self, params: Value) -> ServiceResult;
    async fn add_custom(&self, params: Value) -> ServiceResult;
}

pub struct NoopProviderSetupService;

#[async_trait]
impl ProviderSetupService for NoopProviderSetupService {
    async fn available(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn save_key(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn oauth_start(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn oauth_complete(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn oauth_status(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn remove_key(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn validate_key(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn save_model(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn save_models(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn add_custom(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }
}

#[async_trait]
pub trait ProjectService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn get(&self, params: Value) -> ServiceResult;
    async fn upsert(&self, params: Value) -> ServiceResult;
    async fn delete(&self, params: Value) -> ServiceResult;
    async fn detect(&self, params: Value) -> ServiceResult;
    async fn complete_path(&self, params: Value) -> ServiceResult;
    async fn context(&self, params: Value) -> ServiceResult;
}

pub struct NoopProjectService;

#[async_trait]
impl ProjectService for NoopProjectService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!(null))
    }

    async fn upsert(&self, _p: Value) -> ServiceResult {
        Err("project service not configured".into())
    }

    async fn delete(&self, _p: Value) -> ServiceResult {
        Err("project service not configured".into())
    }

    async fn detect(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn complete_path(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn context(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!(null))
    }
}

#[async_trait]
pub trait LocalLlmService: Send + Sync {
    async fn system_info(&self) -> ServiceResult;
    async fn models(&self) -> ServiceResult;
    async fn configure(&self, params: Value) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn search_hf(&self, params: Value) -> ServiceResult;
    async fn configure_custom(&self, params: Value) -> ServiceResult;
    async fn remove_model(&self, params: Value) -> ServiceResult;
}

pub struct NoopLocalLlmService;

#[async_trait]
impl LocalLlmService for NoopLocalLlmService {
    async fn system_info(&self) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn models(&self) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn configure(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "status": "unavailable" }))
    }

    async fn search_hf(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn configure_custom(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn remove_model(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }
}

#[async_trait]
pub trait SystemInfoService: Send + Sync {
    async fn health(&self) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn system_presence(&self) -> ServiceResult;
    async fn node_list(&self) -> ServiceResult;
    async fn node_describe(&self, params: Value) -> ServiceResult;
    async fn hooks_list(&self) -> ServiceResult;
    async fn heartbeat_status(&self) -> ServiceResult;
    async fn heartbeat_runs(&self, params: Value) -> ServiceResult;
}

pub struct NoopSystemInfoService;

#[async_trait]
impl SystemInfoService for NoopSystemInfoService {
    async fn health(&self) -> ServiceResult {
        Ok(serde_json::json!({ "ok": true, "connections": 0 }))
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({
            "hostname": "unknown",
            "version": "0.0.0",
            "connections": 0,
            "uptimeMs": 0,
        }))
    }

    async fn system_presence(&self) -> ServiceResult {
        Ok(serde_json::json!({ "clients": [], "nodes": [] }))
    }

    async fn node_list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn node_describe(&self, _params: Value) -> ServiceResult {
        Err("system info service not configured".into())
    }

    async fn hooks_list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn heartbeat_status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "config": null }))
    }

    async fn heartbeat_runs(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}
