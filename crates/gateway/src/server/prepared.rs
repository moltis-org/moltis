use std::{path::PathBuf, sync::Arc};

use crate::{auth_webauthn::SharedWebAuthnRegistry, methods::MethodRegistry, state::GatewayState};

#[cfg(feature = "tailscale")]
use crate::tailscale::TailscaleMode;

/// Core gateway state produced by [`super::prepare_gateway_core`].
///
/// Contains everything needed to build an HTTP server on top of the core, but
/// no HTTP/transport-specific types. Non-HTTP consumers (TUI, tests) can stop
/// at this level.
pub struct PreparedGatewayCore {
    /// Shared gateway state (sessions, services, config, etc.).
    pub state: Arc<GatewayState>,
    /// RPC method registry.
    pub methods: Arc<MethodRegistry>,
    /// WebAuthn registry for passkey auth.
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    /// MS Teams webhook plugin (always present, may be empty).
    pub msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>,
    /// Slack webhook plugin.
    #[cfg(feature = "slack")]
    pub slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
    /// Push notification service.
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<crate::push::PushService>>,
    /// Network audit buffer (trusted-network proxy).
    #[cfg(feature = "trusted-network")]
    pub audit_buffer: Option<crate::network_audit::NetworkAuditBuffer>,
    /// Sandbox router for container backends.
    pub sandbox_router: Arc<moltis_tools::sandbox::SandboxRouter>,
    /// Browser service for lifecycle management.
    pub browser_for_lifecycle: Arc<dyn crate::services::BrowserService>,
    /// Cron scheduler service. **Callers must invoke
    /// [`CronService::start()`] to activate the scheduler**; without it,
    /// scheduled jobs will not execute.
    pub cron_service: Arc<moltis_cron::service::CronService>,
    /// Log buffer for real-time log streaming.
    pub log_buffer: Option<crate::logs::LogBuffer>,
    /// Browser tool for warmup after listener is ready.
    pub browser_tool_for_warmup: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
    /// Loaded configuration snapshot.
    pub config: moltis_config::schema::MoltisConfig,
    /// Resolved data directory.
    pub data_dir: PathBuf,
    /// Human-readable provider summary for the startup banner.
    pub provider_summary: String,
    /// Number of configured MCP servers.
    pub mcp_configured_count: usize,
    /// OpenClaw detection status string.
    pub openclaw_status: String,
    /// One-time setup code (when auth setup is pending).
    pub setup_code_display: Option<String>,
    /// Resolved port.
    pub port: u16,
    /// Whether TLS is active for this gateway instance.
    pub tls_enabled: bool,
    /// Tailscale mode.
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    /// Whether to reset tailscale on exit.
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
    /// Shutdown sender for the trusted-network proxy.  Retained here so the
    /// proxy task is not cancelled when `prepare_gateway` returns (dropping
    /// the sender closes the watch channel and triggers immediate shutdown).
    #[cfg(feature = "trusted-network")]
    pub _proxy_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}
