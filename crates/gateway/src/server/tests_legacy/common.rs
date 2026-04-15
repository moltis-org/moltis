use std::sync::MutexGuard;

use {async_trait::async_trait, moltis_common::types::ReplyPayload, tokio::sync::Mutex};

pub(crate) struct LocalModelConfigTestGuard {
    _lock: MutexGuard<'static, ()>,
}

impl LocalModelConfigTestGuard {
    pub(crate) fn new() -> Self {
        Self {
            _lock: crate::config_override_test_lock(),
        }
    }
}

impl Drop for LocalModelConfigTestGuard {
    fn drop(&mut self) {
        moltis_config::clear_config_dir();
        moltis_config::clear_data_dir();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeliveredMessage {
    pub(crate) account_id: String,
    pub(crate) to: String,
    pub(crate) text: String,
    pub(crate) reply_to: Option<String>,
}

#[derive(Default)]
pub(crate) struct RecordingChannelOutbound {
    pub(crate) delivered: Mutex<Vec<DeliveredMessage>>,
}

#[async_trait]
impl moltis_channels::ChannelOutbound for RecordingChannelOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        self.delivered.lock().await.push(DeliveredMessage {
            account_id: account_id.to_string(),
            to: to.to_string(),
            text: text.to_string(),
            reply_to: reply_to.map(ToString::to_string),
        });
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

pub(crate) fn cron_delivery_request() -> moltis_cron::service::AgentTurnRequest {
    moltis_cron::service::AgentTurnRequest {
        message: "Run background summary".to_string(),
        model: None,
        timeout_secs: None,
        deliver: true,
        channel: Some("bot-main".to_string()),
        to: Some("123456".to_string()),
        session_target: moltis_cron::types::SessionTarget::Isolated,
        sandbox: moltis_cron::types::CronSandboxConfig::default(),
    }
}
