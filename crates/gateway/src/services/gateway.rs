// ── Bundled services ────────────────────────────────────────────────────────

use super::*;

/// All domain services the gateway delegates to.
pub struct GatewayServices {
    pub agent: Arc<dyn AgentService>,
    pub session: Arc<dyn SessionService>,
    pub channel: Arc<dyn ChannelService>,
    pub config: Arc<dyn ConfigService>,
    pub cron: Arc<dyn CronService>,
    pub webhooks: Arc<dyn WebhooksService>,
    pub chat: Arc<dyn ChatService>,
    pub tts: Arc<dyn TtsService>,
    pub stt: Arc<dyn SttService>,
    pub skills: Arc<dyn SkillsService>,
    pub mcp: Arc<dyn McpService>,
    pub browser: Arc<dyn BrowserService>,
    pub usage: Arc<dyn UsageService>,
    pub exec_approval: Arc<dyn ExecApprovalService>,
    pub onboarding: Arc<dyn OnboardingService>,
    pub update: Arc<dyn UpdateService>,
    pub model: Arc<dyn ModelService>,
    pub web_login: Arc<dyn WebLoginService>,
    pub voicewake: Arc<dyn VoicewakeService>,
    pub logs: Arc<dyn LogsService>,
    pub provider_setup: Arc<dyn ProviderSetupService>,
    pub project: Arc<dyn ProjectService>,
    pub local_llm: Arc<dyn LocalLlmService>,
    pub external_agent: Arc<dyn ExternalAgentService>,
    pub network_audit: Arc<dyn crate::network_audit::NetworkAuditService>,
    /// Optional channel registry for direct plugin access (thread context, etc.).
    pub channel_registry: Option<Arc<moltis_channels::ChannelRegistry>>,
    /// Optional persisted channel store for safe config mutations.
    pub channel_store: Option<Arc<dyn moltis_channels::store::ChannelStore>>,
    /// Optional channel outbound for sending replies back to channels.
    channel_outbound: Option<Arc<dyn moltis_channels::ChannelOutbound>>,
    /// Optional channel stream outbound for edit-in-place channel streaming.
    channel_stream_outbound: Option<Arc<dyn moltis_channels::ChannelStreamOutbound>>,
    /// Optional session metadata for cross-service access (e.g. channel binding).
    pub session_metadata: Option<Arc<moltis_sessions::metadata::SqliteSessionMetadata>>,
    /// Optional session store for message-index lookups (e.g. deduplication).
    pub session_store: Option<Arc<moltis_sessions::store::SessionStore>>,
    /// Optional session share store for immutable snapshot links.
    pub session_share_store: Option<Arc<crate::share_store::ShareStore>>,
    /// Optional agent persona store for multi-agent support.
    pub agent_persona_store: Option<Arc<crate::agent_persona::AgentPersonaStore>>,
    /// Optional voice persona store for named voice identities.
    pub voice_persona_store: Option<Arc<crate::voice_persona::VoicePersonaStore>>,
    /// Shared agents config (presets) for spawn_agent and RPC sync.
    pub agents_config: Option<Arc<tokio::sync::RwLock<moltis_config::AgentsConfig>>>,
    /// Typed telephony plugin for RPC call initiation/hangup.
    #[cfg(feature = "telephony")]
    pub telephony_plugin: Option<Arc<tokio::sync::RwLock<moltis_telephony::TelephonyPlugin>>>,
}

impl GatewayServices {
    pub fn with_chat(mut self, chat: Arc<dyn ChatService>) -> Self {
        self.chat = chat;
        self
    }

    pub fn with_session(mut self, session: Arc<dyn SessionService>) -> Self {
        self.session = session;
        self
    }

    pub fn with_model(mut self, model: Arc<dyn ModelService>) -> Self {
        self.model = model;
        self
    }

    pub fn with_cron(mut self, cron: Arc<dyn CronService>) -> Self {
        self.cron = cron;
        self
    }

    pub fn with_webhooks(mut self, webhooks: Arc<dyn WebhooksService>) -> Self {
        self.webhooks = webhooks;
        self
    }

    pub fn with_provider_setup(mut self, ps: Arc<dyn ProviderSetupService>) -> Self {
        self.provider_setup = ps;
        self
    }

    pub fn with_channel_registry(
        mut self,
        registry: Arc<moltis_channels::ChannelRegistry>,
    ) -> Self {
        self.channel_registry = Some(registry);
        self
    }

    pub fn with_channel_store(
        mut self,
        store: Arc<dyn moltis_channels::store::ChannelStore>,
    ) -> Self {
        self.channel_store = Some(store);
        self
    }

    pub fn with_channel_outbound(
        mut self,
        outbound: Arc<dyn moltis_channels::ChannelOutbound>,
    ) -> Self {
        self.channel_outbound = Some(outbound);
        self
    }

    pub fn with_channel_stream_outbound(
        mut self,
        outbound: Arc<dyn moltis_channels::ChannelStreamOutbound>,
    ) -> Self {
        self.channel_stream_outbound = Some(outbound);
        self
    }

    #[cfg(feature = "telephony")]
    pub fn with_telephony_plugin(
        mut self,
        plugin: Arc<tokio::sync::RwLock<moltis_telephony::TelephonyPlugin>>,
    ) -> Self {
        self.telephony_plugin = Some(plugin);
        self
    }

    pub fn channel_outbound_arc(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
        self.channel_outbound.clone()
    }

    pub fn channel_stream_outbound_arc(
        &self,
    ) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
        self.channel_stream_outbound.clone()
    }

    /// Create a service bundle with all noop implementations.
    pub fn noop() -> Self {
        Self {
            agent: Arc::new(NoopAgentService),
            session: Arc::new(NoopSessionService),
            channel: Arc::new(NoopChannelService),
            config: Arc::new(NoopConfigService),
            cron: Arc::new(NoopCronService),
            webhooks: Arc::new(NoopWebhooksService),
            chat: Arc::new(NoopChatService),
            tts: Arc::new(NoopTtsService),
            stt: Arc::new(NoopSttService),
            skills: Arc::new(NoopSkillsService),
            mcp: Arc::new(NoopMcpService),
            browser: Arc::new(NoopBrowserService),
            usage: Arc::new(NoopUsageService),
            exec_approval: Arc::new(NoopExecApprovalService),
            onboarding: Arc::new(NoopOnboardingService),
            update: Arc::new(NoopUpdateService),
            model: Arc::new(NoopModelService),
            web_login: Arc::new(NoopWebLoginService),
            voicewake: Arc::new(NoopVoicewakeService),
            logs: Arc::new(NoopLogsService),
            provider_setup: Arc::new(NoopProviderSetupService),
            project: Arc::new(NoopProjectService),
            local_llm: Arc::new(NoopLocalLlmService),
            external_agent: Arc::new(NoopExternalAgentService),
            network_audit: Arc::new(crate::network_audit::NoopNetworkAuditService),
            channel_registry: None,
            channel_store: None,
            channel_outbound: None,
            channel_stream_outbound: None,
            session_metadata: None,
            session_store: None,
            session_share_store: None,
            agent_persona_store: None,
            voice_persona_store: None,
            agents_config: None,
            #[cfg(feature = "telephony")]
            telephony_plugin: None,
        }
    }

    pub fn with_local_llm(mut self, local_llm: Arc<dyn LocalLlmService>) -> Self {
        self.local_llm = local_llm;
        self
    }

    pub fn with_network_audit(
        mut self,
        svc: Arc<dyn crate::network_audit::NetworkAuditService>,
    ) -> Self {
        self.network_audit = svc;
        self
    }

    pub fn with_external_agent(mut self, external_agent: Arc<dyn ExternalAgentService>) -> Self {
        self.external_agent = external_agent;
        self
    }

    pub fn with_onboarding(mut self, onboarding: Arc<dyn OnboardingService>) -> Self {
        self.onboarding = onboarding;
        self
    }

    pub fn with_project(mut self, project: Arc<dyn ProjectService>) -> Self {
        self.project = project;
        self
    }

    pub fn with_session_metadata(
        mut self,
        meta: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
    ) -> Self {
        self.session_metadata = Some(meta);
        self
    }

    pub fn with_session_store(mut self, store: Arc<moltis_sessions::store::SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    pub fn with_session_share_store(mut self, store: Arc<crate::share_store::ShareStore>) -> Self {
        self.session_share_store = Some(store);
        self
    }

    pub fn with_agent_persona_store(
        mut self,
        store: Arc<crate::agent_persona::AgentPersonaStore>,
    ) -> Self {
        self.agent_persona_store = Some(store);
        self
    }

    pub fn with_voice_persona_store(
        mut self,
        store: Arc<crate::voice_persona::VoicePersonaStore>,
    ) -> Self {
        self.voice_persona_store = Some(store);
        self
    }

    pub fn with_agents_config(
        mut self,
        agents_config: Arc<tokio::sync::RwLock<moltis_config::AgentsConfig>>,
    ) -> Self {
        self.agents_config = Some(agents_config);
        self
    }

    pub fn with_tts(mut self, tts: Arc<dyn TtsService>) -> Self {
        self.tts = tts;
        self
    }

    pub fn with_stt(mut self, stt: Arc<dyn SttService>) -> Self {
        self.stt = stt;
        self
    }

    /// Create a [`Services`] bundle with an injected `chat` and `system_info`.
    ///
    /// Clones all other service `Arc`s (cheap pointer bumps) into the shared
    /// bundle. The `system_info` service is provided separately because it
    /// needs the fully-constructed `GatewayState` which isn't available during
    /// `GatewayServices` construction.
    pub fn to_services_with_chat(
        &self,
        system_info: Arc<dyn SystemInfoService>,
        chat: Arc<dyn ChatService>,
    ) -> Arc<Services> {
        Arc::new(Services {
            agent: self.agent.clone(),
            session: self.session.clone(),
            channel: self.channel.clone(),
            config: self.config.clone(),
            cron: self.cron.clone(),
            chat,
            tts: self.tts.clone(),
            stt: self.stt.clone(),
            skills: self.skills.clone(),
            mcp: self.mcp.clone(),
            browser: self.browser.clone(),
            usage: self.usage.clone(),
            exec_approval: self.exec_approval.clone(),
            onboarding: self.onboarding.clone(),
            update: self.update.clone(),
            model: self.model.clone(),
            web_login: self.web_login.clone(),
            voicewake: self.voicewake.clone(),
            logs: self.logs.clone(),
            provider_setup: self.provider_setup.clone(),
            project: self.project.clone(),
            local_llm: self.local_llm.clone(),
            system_info,
            external_agent: self.external_agent.clone(),
        })
    }

    pub fn to_services(&self, system_info: Arc<dyn SystemInfoService>) -> Arc<Services> {
        self.to_services_with_chat(system_info, self.chat.clone())
    }
}
