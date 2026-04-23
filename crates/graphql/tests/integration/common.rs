use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use {
    moltis_service_traits::{ServiceResult, Services},
    serde_json::{Value, json},
    tokio::sync::broadcast,
};

/// Central mock that records calls and returns preset responses.
/// Used by all mock service implementations below.
pub(crate) struct MockDispatch {
    responses: Mutex<HashMap<String, Value>>,
    calls: Mutex<Vec<(String, Value)>>,
}

impl MockDispatch {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn set_response(&self, method: &str, response: Value) {
        self.responses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(method.to_string(), response);
    }

    fn call(&self, method: &str, params: Value) -> ServiceResult {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((method.to_string(), params));
        let responses = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        match responses.get(method) {
            Some(v) => Ok(v.clone()),
            None => Err(format!("no mock response for {method}").into()),
        }
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub(crate) fn last_call(&self) -> Option<(String, Value)> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned()
    }
}

macro_rules! mock_svc_struct {
    ($name:ident) => {
        struct $name(Arc<MockDispatch>);
    };
}

mock_svc_struct!(MockAgent);
mock_svc_struct!(MockSession);
mock_svc_struct!(MockChannel);
mock_svc_struct!(MockConfig);
mock_svc_struct!(MockCron);
mock_svc_struct!(MockChat);
mock_svc_struct!(MockTts);
mock_svc_struct!(MockStt);
mock_svc_struct!(MockSkills);
mock_svc_struct!(MockMcp);
mock_svc_struct!(MockBrowser);
mock_svc_struct!(MockUsage);
mock_svc_struct!(MockExecApproval);
mock_svc_struct!(MockOnboarding);
mock_svc_struct!(MockUpdate);
mock_svc_struct!(MockModel);
mock_svc_struct!(MockWebLogin);
mock_svc_struct!(MockVoicewake);
mock_svc_struct!(MockLogs);
mock_svc_struct!(MockProviderSetup);
mock_svc_struct!(MockProject);
mock_svc_struct!(MockLocalLlm);
mock_svc_struct!(MockSystemInfo);

#[async_trait::async_trait]
impl moltis_service_traits::AgentService for MockAgent {
    async fn run(&self, params: Value) -> ServiceResult {
        self.0.call("agent", params)
    }

    async fn run_wait(&self, params: Value) -> ServiceResult {
        self.0.call("agent.wait", params)
    }

    async fn identity_get(&self) -> ServiceResult {
        self.0.call("agent.identity.get", json!({}))
    }

    async fn list(&self) -> ServiceResult {
        self.0.call("agents.list", json!({}))
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::SessionService for MockSession {
    async fn list(&self) -> ServiceResult {
        self.0.call("sessions.list", json!({}))
    }

    async fn preview(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.preview", p)
    }

    async fn resolve(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.resolve", p)
    }

    async fn patch(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.patch", p)
    }

    async fn voice_generate(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.voice.generate", p)
    }

    async fn share_create(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.share.create", p)
    }

    async fn share_list(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.share.list", p)
    }

    async fn share_revoke(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.share.revoke", p)
    }

    async fn reset(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.reset", p)
    }

    async fn delete(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.delete", p)
    }

    async fn compact(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.compact", p)
    }

    async fn search(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.search", p)
    }

    async fn fork(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.fork", p)
    }

    async fn branches(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.branches", p)
    }

    async fn run_detail(&self, p: Value) -> ServiceResult {
        self.0.call("sessions.run_detail", p)
    }

    async fn clear_all(&self) -> ServiceResult {
        self.0.call("sessions.clear_all", json!({}))
    }

    async fn mark_seen(&self, _key: &str) {}
}

#[async_trait::async_trait]
impl moltis_service_traits::ChannelService for MockChannel {
    async fn status(&self) -> ServiceResult {
        self.0.call("channels.status", json!({}))
    }

    async fn logout(&self, p: Value) -> ServiceResult {
        self.0.call("channels.logout", p)
    }

    async fn send(&self, p: Value) -> ServiceResult {
        self.0.call("send", p)
    }

    async fn add(&self, p: Value) -> ServiceResult {
        self.0.call("channels.add", p)
    }

    async fn remove(&self, p: Value) -> ServiceResult {
        self.0.call("channels.remove", p)
    }

    async fn update(&self, p: Value) -> ServiceResult {
        self.0.call("channels.update", p)
    }

    async fn retry_ownership(&self, p: Value) -> ServiceResult {
        self.0.call("channels.retry_ownership", p)
    }

    async fn senders_list(&self, p: Value) -> ServiceResult {
        self.0.call("channels.senders.list", p)
    }

    async fn sender_approve(&self, p: Value) -> ServiceResult {
        self.0.call("channels.senders.approve", p)
    }

    async fn sender_deny(&self, p: Value) -> ServiceResult {
        self.0.call("channels.senders.deny", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ConfigService for MockConfig {
    async fn get(&self, p: Value) -> ServiceResult {
        self.0.call("config.get", p)
    }

    async fn set(&self, p: Value) -> ServiceResult {
        self.0.call("config.set", p)
    }

    async fn apply(&self, p: Value) -> ServiceResult {
        self.0.call("config.apply", p)
    }

    async fn patch(&self, p: Value) -> ServiceResult {
        self.0.call("config.patch", p)
    }

    async fn schema(&self) -> ServiceResult {
        self.0.call("config.schema", json!({}))
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::CronService for MockCron {
    async fn list(&self) -> ServiceResult {
        self.0.call("cron.list", json!({}))
    }

    async fn status(&self) -> ServiceResult {
        self.0.call("cron.status", json!({}))
    }

    async fn add(&self, p: Value) -> ServiceResult {
        self.0.call("cron.add", p)
    }

    async fn update(&self, p: Value) -> ServiceResult {
        self.0.call("cron.update", p)
    }

    async fn remove(&self, p: Value) -> ServiceResult {
        self.0.call("cron.remove", p)
    }

    async fn run(&self, p: Value) -> ServiceResult {
        self.0.call("cron.run", p)
    }

    async fn runs(&self, p: Value) -> ServiceResult {
        self.0.call("cron.runs", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ChatService for MockChat {
    async fn send(&self, p: Value) -> ServiceResult {
        self.0.call("chat.send", p)
    }

    async fn abort(&self, p: Value) -> ServiceResult {
        self.0.call("chat.abort", p)
    }

    async fn cancel_queued(&self, p: Value) -> ServiceResult {
        self.0.call("chat.cancel_queued", p)
    }

    async fn history(&self, p: Value) -> ServiceResult {
        self.0.call("chat.history", p)
    }

    async fn inject(&self, p: Value) -> ServiceResult {
        self.0.call("chat.inject", p)
    }

    async fn clear(&self, p: Value) -> ServiceResult {
        self.0.call("chat.clear", p)
    }

    async fn compact(&self, p: Value) -> ServiceResult {
        self.0.call("chat.compact", p)
    }

    async fn context(&self, p: Value) -> ServiceResult {
        self.0.call("chat.context", p)
    }

    async fn raw_prompt(&self, p: Value) -> ServiceResult {
        self.0.call("chat.raw_prompt", p)
    }

    async fn full_context(&self, p: Value) -> ServiceResult {
        self.0.call("chat.full_context", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::TtsService for MockTts {
    async fn status(&self) -> ServiceResult {
        self.0.call("tts.status", json!({}))
    }

    async fn providers(&self) -> ServiceResult {
        self.0.call("tts.providers", json!({}))
    }

    async fn enable(&self, p: Value) -> ServiceResult {
        self.0.call("tts.enable", p)
    }

    async fn disable(&self) -> ServiceResult {
        self.0.call("tts.disable", json!({}))
    }

    async fn convert(&self, p: Value) -> ServiceResult {
        self.0.call("tts.convert", p)
    }

    async fn set_provider(&self, p: Value) -> ServiceResult {
        self.0.call("tts.setProvider", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::SttService for MockStt {
    async fn status(&self) -> ServiceResult {
        self.0.call("stt.status", json!({}))
    }

    async fn providers(&self) -> ServiceResult {
        self.0.call("stt.providers", json!({}))
    }

    async fn transcribe(&self, p: Value) -> ServiceResult {
        self.0.call("stt.transcribe", p)
    }

    async fn transcribe_bytes(
        &self,
        _audio: bytes::Bytes,
        _format: &str,
        _provider: Option<&str>,
        _language: Option<&str>,
        _prompt: Option<&str>,
    ) -> ServiceResult {
        self.0.call("stt.transcribe_bytes", json!({}))
    }

    async fn set_provider(&self, p: Value) -> ServiceResult {
        self.0.call("stt.setProvider", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::SkillsService for MockSkills {
    async fn status(&self) -> ServiceResult {
        self.0.call("skills.status", json!({}))
    }

    async fn bins(&self) -> ServiceResult {
        self.0.call("skills.bins", json!({}))
    }

    async fn install(&self, p: Value) -> ServiceResult {
        self.0.call("skills.install", p)
    }

    async fn update(&self, p: Value) -> ServiceResult {
        self.0.call("skills.update", p)
    }

    async fn list(&self) -> ServiceResult {
        self.0.call("skills.list", json!({}))
    }

    async fn remove(&self, p: Value) -> ServiceResult {
        self.0.call("skills.remove", p)
    }

    async fn repos_list(&self) -> ServiceResult {
        self.0.call("skills.repos.list", json!({}))
    }

    async fn repos_list_full(&self) -> ServiceResult {
        self.0.call("skills.repos.list_full", json!({}))
    }

    async fn repos_remove(&self, p: Value) -> ServiceResult {
        self.0.call("skills.repos.remove", p)
    }

    async fn repos_export(&self, p: Value) -> ServiceResult {
        self.0.call("skills.repos.export", p)
    }

    async fn repos_import(&self, p: Value) -> ServiceResult {
        self.0.call("skills.repos.import", p)
    }

    async fn repos_unquarantine(&self, p: Value) -> ServiceResult {
        self.0.call("skills.repos.unquarantine", p)
    }

    async fn emergency_disable(&self) -> ServiceResult {
        self.0.call("skills.emergency_disable", json!({}))
    }

    async fn skill_enable(&self, p: Value) -> ServiceResult {
        self.0.call("skills.skill.enable", p)
    }

    async fn skill_disable(&self, p: Value) -> ServiceResult {
        self.0.call("skills.skill.disable", p)
    }

    async fn skill_trust(&self, p: Value) -> ServiceResult {
        self.0.call("skills.skill.trust", p)
    }

    async fn skill_detail(&self, p: Value) -> ServiceResult {
        self.0.call("skills.skill.detail", p)
    }

    async fn install_dep(&self, p: Value) -> ServiceResult {
        self.0.call("skills.install_dep", p)
    }

    async fn security_status(&self) -> ServiceResult {
        self.0.call("skills.security.status", json!({}))
    }

    async fn security_scan(&self) -> ServiceResult {
        self.0.call("skills.security.scan", json!({}))
    }

    async fn skill_save(&self, p: Value) -> ServiceResult {
        self.0.call("skills.skill.save", p)
    }

    async fn bundled_categories(&self) -> ServiceResult {
        self.0.call("skills.bundled.categories", json!({}))
    }

    async fn bundled_toggle_category(&self, p: Value) -> ServiceResult {
        self.0.call("skills.bundled.toggle_category", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::McpService for MockMcp {
    async fn list(&self) -> ServiceResult {
        self.0.call("mcp.list", json!({}))
    }

    async fn add(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.add", p)
    }

    async fn remove(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.remove", p)
    }

    async fn enable(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.enable", p)
    }

    async fn disable(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.disable", p)
    }

    async fn status(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.status", p)
    }

    async fn tools(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.tools", p)
    }

    async fn restart(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.restart", p)
    }

    async fn update(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.update", p)
    }

    async fn reauth(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.reauth", p)
    }

    async fn oauth_start(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.oauth.start", p)
    }

    async fn oauth_complete(&self, p: Value) -> ServiceResult {
        self.0.call("mcp.oauth.complete", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::BrowserService for MockBrowser {
    async fn request(&self, p: Value) -> ServiceResult {
        self.0.call("browser.request", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::UsageService for MockUsage {
    async fn status(&self) -> ServiceResult {
        self.0.call("usage.status", json!({}))
    }

    async fn cost(&self, p: Value) -> ServiceResult {
        self.0.call("usage.cost", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ExecApprovalService for MockExecApproval {
    async fn get(&self) -> ServiceResult {
        self.0.call("exec.approvals.get", json!({}))
    }

    async fn set(&self, p: Value) -> ServiceResult {
        self.0.call("exec.approvals.set", p)
    }

    async fn node_get(&self, p: Value) -> ServiceResult {
        self.0.call("exec.approvals.node.get", p)
    }

    async fn node_set(&self, p: Value) -> ServiceResult {
        self.0.call("exec.approvals.node.set", p)
    }

    async fn request(&self, p: Value) -> ServiceResult {
        self.0.call("exec.approval.request", p)
    }

    async fn resolve(&self, p: Value) -> ServiceResult {
        self.0.call("exec.approval.resolve", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::OnboardingService for MockOnboarding {
    async fn wizard_start(&self, p: Value) -> ServiceResult {
        self.0.call("wizard.start", p)
    }

    async fn wizard_next(&self, p: Value) -> ServiceResult {
        self.0.call("wizard.next", p)
    }

    async fn wizard_cancel(&self) -> ServiceResult {
        self.0.call("wizard.cancel", json!({}))
    }

    async fn wizard_status(&self) -> ServiceResult {
        self.0.call("wizard.status", json!({}))
    }

    async fn identity_get(&self) -> ServiceResult {
        self.0.call("onboarding.identity.get", json!({}))
    }

    async fn identity_update(&self, p: Value) -> ServiceResult {
        self.0.call("agent.identity.update", p)
    }

    async fn identity_update_soul(&self, soul: Option<String>) -> ServiceResult {
        self.0
            .call("agent.identity.update_soul", json!({ "soul": soul }))
    }

    async fn openclaw_detect(&self) -> ServiceResult {
        self.0.call("openclaw.detect", json!({}))
    }

    async fn openclaw_scan(&self) -> ServiceResult {
        self.0.call("openclaw.scan", json!({}))
    }

    async fn openclaw_import(&self, p: Value) -> ServiceResult {
        self.0.call("openclaw.import", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::UpdateService for MockUpdate {
    async fn run(&self, p: Value) -> ServiceResult {
        self.0.call("update.run", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ModelService for MockModel {
    async fn list(&self) -> ServiceResult {
        self.0.call("models.list", json!({}))
    }

    async fn list_all(&self) -> ServiceResult {
        self.0.call("models.list_all", json!({}))
    }

    async fn disable(&self, p: Value) -> ServiceResult {
        self.0.call("models.disable", p)
    }

    async fn enable(&self, p: Value) -> ServiceResult {
        self.0.call("models.enable", p)
    }

    async fn detect_supported(&self, p: Value) -> ServiceResult {
        self.0.call("models.detect_supported", p)
    }

    async fn cancel_detect(&self) -> ServiceResult {
        self.0.call("models.cancel_detect", json!({}))
    }

    async fn test(&self, p: Value) -> ServiceResult {
        self.0.call("models.test", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::WebLoginService for MockWebLogin {
    async fn start(&self, p: Value) -> ServiceResult {
        self.0.call("web.login.start", p)
    }

    async fn wait(&self, p: Value) -> ServiceResult {
        self.0.call("web.login.wait", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::VoicewakeService for MockVoicewake {
    async fn get(&self) -> ServiceResult {
        self.0.call("voicewake.get", json!({}))
    }

    async fn set(&self, p: Value) -> ServiceResult {
        self.0.call("voicewake.set", p)
    }

    async fn wake(&self, p: Value) -> ServiceResult {
        self.0.call("wake", p)
    }

    async fn talk_mode(&self, p: Value) -> ServiceResult {
        self.0.call("talk.mode", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::LogsService for MockLogs {
    async fn tail(&self, p: Value) -> ServiceResult {
        self.0.call("logs.tail", p)
    }

    async fn list(&self, p: Value) -> ServiceResult {
        self.0.call("logs.list", p)
    }

    async fn status(&self) -> ServiceResult {
        self.0.call("logs.status", json!({}))
    }

    async fn ack(&self) -> ServiceResult {
        self.0.call("logs.ack", json!({}))
    }

    fn log_file_path(&self) -> Option<std::path::PathBuf> {
        None
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ProviderSetupService for MockProviderSetup {
    async fn available(&self) -> ServiceResult {
        self.0.call("providers.available", json!({}))
    }

    async fn save_key(&self, p: Value) -> ServiceResult {
        self.0.call("providers.save_key", p)
    }

    async fn oauth_start(&self, p: Value) -> ServiceResult {
        self.0.call("providers.oauth.start", p)
    }

    async fn oauth_complete(&self, p: Value) -> ServiceResult {
        self.0.call("providers.oauth.complete", p)
    }

    async fn oauth_status(&self, p: Value) -> ServiceResult {
        self.0.call("providers.oauth.status", p)
    }

    async fn remove_key(&self, p: Value) -> ServiceResult {
        self.0.call("providers.remove_key", p)
    }

    async fn validate_key(&self, p: Value) -> ServiceResult {
        self.0.call("providers.validate_key", p)
    }

    async fn save_model(&self, p: Value) -> ServiceResult {
        self.0.call("providers.save_model", p)
    }

    async fn save_models(&self, p: Value) -> ServiceResult {
        self.0.call("providers.save_models", p)
    }

    async fn add_custom(&self, p: Value) -> ServiceResult {
        self.0.call("providers.add_custom", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::ProjectService for MockProject {
    async fn list(&self) -> ServiceResult {
        self.0.call("projects.list", json!({}))
    }

    async fn get(&self, p: Value) -> ServiceResult {
        self.0.call("projects.get", p)
    }

    async fn upsert(&self, p: Value) -> ServiceResult {
        self.0.call("projects.upsert", p)
    }

    async fn delete(&self, p: Value) -> ServiceResult {
        self.0.call("projects.delete", p)
    }

    async fn detect(&self, p: Value) -> ServiceResult {
        self.0.call("projects.detect", p)
    }

    async fn complete_path(&self, p: Value) -> ServiceResult {
        self.0.call("projects.complete_path", p)
    }

    async fn context(&self, p: Value) -> ServiceResult {
        self.0.call("projects.context", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::LocalLlmService for MockLocalLlm {
    async fn system_info(&self) -> ServiceResult {
        self.0.call("providers.local.system_info", json!({}))
    }

    async fn models(&self) -> ServiceResult {
        self.0.call("providers.local.models", json!({}))
    }

    async fn configure(&self, p: Value) -> ServiceResult {
        self.0.call("providers.local.configure", p)
    }

    async fn status(&self) -> ServiceResult {
        self.0.call("providers.local.status", json!({}))
    }

    async fn search_hf(&self, p: Value) -> ServiceResult {
        self.0.call("providers.local.search_hf", p)
    }

    async fn configure_custom(&self, p: Value) -> ServiceResult {
        self.0.call("providers.local.configure_custom", p)
    }

    async fn remove_model(&self, p: Value) -> ServiceResult {
        self.0.call("providers.local.remove_model", p)
    }
}

#[async_trait::async_trait]
impl moltis_service_traits::SystemInfoService for MockSystemInfo {
    async fn health(&self) -> ServiceResult {
        self.0.call("health", json!({}))
    }

    async fn status(&self) -> ServiceResult {
        self.0.call("status", json!({}))
    }

    async fn system_presence(&self) -> ServiceResult {
        self.0.call("system-presence", json!({}))
    }

    async fn node_list(&self) -> ServiceResult {
        self.0.call("node.list", json!({}))
    }

    async fn node_describe(&self, p: Value) -> ServiceResult {
        self.0.call("node.describe", p)
    }

    async fn hooks_list(&self) -> ServiceResult {
        self.0.call("hooks.list", json!({}))
    }

    async fn heartbeat_status(&self) -> ServiceResult {
        self.0.call("heartbeat.status", json!({}))
    }

    async fn heartbeat_runs(&self, p: Value) -> ServiceResult {
        self.0.call("heartbeat.runs", p)
    }
}

fn build_mock_services(mock: &Arc<MockDispatch>) -> Arc<Services> {
    Arc::new(Services {
        agent: Arc::new(MockAgent(mock.clone())),
        session: Arc::new(MockSession(mock.clone())),
        channel: Arc::new(MockChannel(mock.clone())),
        config: Arc::new(MockConfig(mock.clone())),
        cron: Arc::new(MockCron(mock.clone())),
        chat: Arc::new(MockChat(mock.clone())),
        tts: Arc::new(MockTts(mock.clone())),
        stt: Arc::new(MockStt(mock.clone())),
        skills: Arc::new(MockSkills(mock.clone())),
        mcp: Arc::new(MockMcp(mock.clone())),
        browser: Arc::new(MockBrowser(mock.clone())),
        usage: Arc::new(MockUsage(mock.clone())),
        exec_approval: Arc::new(MockExecApproval(mock.clone())),
        onboarding: Arc::new(MockOnboarding(mock.clone())),
        update: Arc::new(MockUpdate(mock.clone())),
        model: Arc::new(MockModel(mock.clone())),
        web_login: Arc::new(MockWebLogin(mock.clone())),
        voicewake: Arc::new(MockVoicewake(mock.clone())),
        logs: Arc::new(MockLogs(mock.clone())),
        provider_setup: Arc::new(MockProviderSetup(mock.clone())),
        project: Arc::new(MockProject(mock.clone())),
        local_llm: Arc::new(MockLocalLlm(mock.clone())),
        system_info: Arc::new(MockSystemInfo(mock.clone())),
    })
}

pub(crate) fn build_test_schema(
    mock: Arc<MockDispatch>,
) -> (
    moltis_graphql::MoltisSchema,
    broadcast::Sender<(String, Value)>,
) {
    let (tx, _) = broadcast::channel(16);
    let services = build_mock_services(&mock);
    let schema = moltis_graphql::build_schema(services, tx.clone());
    (schema, tx)
}
