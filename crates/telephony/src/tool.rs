//! Agent tool for voice calls.
//!
//! Exposes `voice_call` to the agent loop, enabling agents to initiate and
//! manage phone calls programmatically.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    moltis_channels::ChannelPlugin as _,
    serde_json::{Value, json},
    std::sync::Arc,
    tokio::sync::RwLock,
    tracing::debug,
};

use crate::{manager::CallManager, plugin::TelephonyPlugin, types::CallMode};

struct ResolvedToolAccount {
    account_id: String,
    manager: Arc<RwLock<CallManager>>,
    from_number: String,
    webhook_base_url: String,
}

/// Agent tool that allows the LLM to make and manage phone calls.
pub struct VoiceCallTool {
    telephony_plugin: Arc<RwLock<TelephonyPlugin>>,
    /// Default account to use when not specified.
    default_account: Option<String>,
    /// Default webhook base URL for callbacks.
    webhook_base_url: String,
}

impl VoiceCallTool {
    pub fn new(webhook_base_url: String, telephony_plugin: Arc<RwLock<TelephonyPlugin>>) -> Self {
        Self {
            telephony_plugin,
            default_account: None,
            webhook_base_url,
        }
    }

    async fn resolve_account(
        &self,
        account_id: Option<&str>,
    ) -> anyhow::Result<ResolvedToolAccount> {
        let plugin = self.telephony_plugin.read().await;
        let account_ids = plugin.account_ids();
        if account_ids.is_empty() {
            anyhow::bail!("no telephony accounts configured");
        }

        let selected_account_id = if let Some(aid) = account_id.or(self.default_account.as_deref())
        {
            if !plugin.has_account(aid) {
                anyhow::bail!("account {aid} not found");
            }
            aid.to_string()
        } else {
            account_ids
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("no telephony accounts configured"))?
        };

        let manager = plugin
            .call_manager(&selected_account_id)
            .ok_or_else(|| anyhow::anyhow!("account {selected_account_id} has no call manager"))?;
        let from_number = plugin
            .caller_number(&selected_account_id)
            .unwrap_or_default();
        let webhook_base_url = plugin
            .account_config_json(&selected_account_id)
            .and_then(|config| {
                config["webhook_url"]
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| self.webhook_base_url.clone());

        Ok(ResolvedToolAccount {
            account_id: selected_account_id,
            manager,
            from_number,
            webhook_base_url,
        })
    }
}

#[async_trait]
impl AgentTool for VoiceCallTool {
    fn name(&self) -> &str {
        "voice_call"
    }

    fn description(&self) -> &str {
        "Make and manage phone calls. Actions: initiate_call, end_call, get_status, send_dtmf."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["initiate_call", "end_call", "get_status", "send_dtmf"],
                    "description": "The action to perform."
                },
                "to": {
                    "type": "string",
                    "description": "Phone number to call (E.164 format, e.g. +15551234567). Required for initiate_call."
                },
                "message": {
                    "type": "string",
                    "description": "Message to speak when the call connects. Used with initiate_call."
                },
                "mode": {
                    "type": "string",
                    "enum": ["notify", "conversation"],
                    "description": "Call mode. 'notify' delivers a message and hangs up. 'conversation' enables multi-turn interaction. Default: conversation."
                },
                "call_id": {
                    "type": "string",
                    "description": "Call ID for end_call, get_status, send_dtmf actions."
                },
                "digits": {
                    "type": "string",
                    "description": "DTMF digits to send (0-9, *, #, w for wait). Used with send_dtmf."
                },
                "account_id": {
                    "type": "string",
                    "description": "Telephony account to use. Optional, defaults to first configured account."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing action"))?;

        let account_id = params["account_id"].as_str();

        match action {
            "initiate_call" => {
                let to = params["to"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'to' phone number is required"))?;
                let message = params["message"].as_str();
                let mode = match params["mode"].as_str() {
                    Some("notify") => CallMode::Notify,
                    _ => CallMode::Conversation,
                };

                let account = self.resolve_account(account_id).await?;
                if account.from_number.is_empty() {
                    anyhow::bail!(
                        "no from_number configured for account {}",
                        account.account_id
                    );
                }
                if account.webhook_base_url.trim().is_empty() {
                    anyhow::bail!(
                        "no webhook_url or server.external_url configured for account {}",
                        account.account_id
                    );
                }
                let manager = account.manager.read().await;

                let status_url = format!(
                    "{}/api/channels/telephony/{}/status",
                    account.webhook_base_url.trim_end_matches('/'),
                    account.account_id,
                );
                let answer_url = format!(
                    "{}/api/channels/telephony/{}/answer",
                    account.webhook_base_url.trim_end_matches('/'),
                    account.account_id,
                );

                let call_id = manager
                    .initiate(
                        &account.from_number,
                        to,
                        mode,
                        message,
                        &account.account_id,
                        &status_url,
                        &answer_url,
                    )
                    .await?;

                debug!(call_id = %call_id, to = %to, "voice_call tool: call initiated");

                Ok(json!({
                    "status": "initiated",
                    "call_id": call_id,
                    "to": to,
                    "mode": format!("{mode:?}").to_lowercase()
                }))
            },
            "end_call" => {
                let call_id = params["call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'call_id' is required"))?;

                let account = self.resolve_account(account_id).await?;
                account.manager.read().await.hangup(call_id).await?;

                Ok(json!({ "status": "ended", "call_id": call_id }))
            },
            "get_status" => {
                let call_id = params["call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'call_id' is required"))?;

                let account = self.resolve_account(account_id).await?;
                let record = account
                    .manager
                    .read()
                    .await
                    .get_call(call_id)
                    .ok_or_else(|| anyhow::anyhow!("call {call_id} not found"))?;

                Ok(json!({
                    "call_id": record.call_id,
                    "state": record.state,
                    "from": record.from,
                    "to": record.to,
                    "direction": record.direction,
                    "mode": record.mode,
                    "transcript_entries": record.transcript.len(),
                }))
            },
            "send_dtmf" => {
                let call_id = params["call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'call_id' is required"))?;
                let digits = params["digits"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'digits' is required"))?;

                let account = self.resolve_account(account_id).await?;
                let manager = account.manager.read().await;
                let record = manager
                    .get_call(call_id)
                    .ok_or_else(|| anyhow::anyhow!("call {call_id} not found"))?;

                let provider_id = record
                    .provider_call_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("no provider call ID"))?;

                manager
                    .provider()
                    .read()
                    .await
                    .send_dtmf(provider_id, digits)
                    .await?;

                Ok(json!({ "status": "sent", "digits": digits }))
            },
            other => anyhow::bail!("unknown action: {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{config::TelephonyAccountConfig, providers::mock::MockProvider},
    };

    fn mock_manager() -> Arc<RwLock<CallManager>> {
        Arc::new(RwLock::new(CallManager::new(
            Box::new(MockProvider::new()),
            60,
        )))
    }

    async fn test_tool() -> VoiceCallTool {
        let mut plugin = TelephonyPlugin::new();
        let mgr = mock_manager();
        plugin.set_test_account(
            "test-acct".into(),
            TelephonyAccountConfig {
                from_number: "+15551111111".into(),
                webhook_url: Some("https://example.com".into()),
                ..TelephonyAccountConfig::default()
            },
            mgr,
        );
        VoiceCallTool::new("https://example.com".into(), Arc::new(RwLock::new(plugin)))
    }

    #[tokio::test]
    async fn resolves_current_manager_after_account_reload() {
        let mut plugin = TelephonyPlugin::new();
        let old_manager = mock_manager();
        plugin.set_test_account(
            "test-acct".into(),
            TelephonyAccountConfig {
                from_number: "+15551111111".into(),
                webhook_url: Some("https://example.com".into()),
                ..TelephonyAccountConfig::default()
            },
            Arc::clone(&old_manager),
        );
        let plugin = Arc::new(RwLock::new(plugin));
        let tool = VoiceCallTool::new("https://example.com".into(), Arc::clone(&plugin));

        let new_manager = mock_manager();
        plugin.write().await.set_test_account(
            "test-acct".into(),
            TelephonyAccountConfig {
                from_number: "+15552222222".into(),
                webhook_url: Some("https://example.com".into()),
                ..TelephonyAccountConfig::default()
            },
            Arc::clone(&new_manager),
        );

        let result = tool
            .execute(json!({
                "action": "initiate_call",
                "account_id": "test-acct",
                "to": "+15559876543",
            }))
            .await
            .unwrap_or_else(|error| panic!("call should use reloaded manager: {error}"));

        assert_eq!(result["status"], "initiated");
        assert!(old_manager.read().await.active_calls().is_empty());

        let new_calls = new_manager.read().await.active_calls();
        assert_eq!(new_calls.len(), 1);
        assert_eq!(new_calls[0].from, "+15552222222");
    }

    #[tokio::test]
    async fn initiate_call_returns_call_id() {
        let tool = test_tool().await;
        let result = tool
            .execute(json!({
                "action": "initiate_call",
                "to": "+15559876543",
                "message": "Hello from the agent",
            }))
            .await
            .unwrap_or_default();

        assert_eq!(result["status"], "initiated");
        assert!(result["call_id"].is_string());
    }

    #[tokio::test]
    async fn get_status_returns_call_info() {
        let tool = test_tool().await;
        let init_result = tool
            .execute(json!({
                "action": "initiate_call",
                "to": "+15559876543",
            }))
            .await
            .unwrap_or_default();

        let call_id = init_result["call_id"].as_str().unwrap_or("");
        let status = tool
            .execute(json!({
                "action": "get_status",
                "call_id": call_id,
            }))
            .await
            .unwrap_or_default();

        assert_eq!(status["state"], "initiated");
        assert_eq!(status["to"], "+15559876543");
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let tool = test_tool().await;
        let result = tool.execute(json!({"action": "fly_to_moon"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_to_number_errors() {
        let tool = test_tool().await;
        let result = tool.execute(json!({"action": "initiate_call"})).await;
        assert!(result.is_err());
    }
}
