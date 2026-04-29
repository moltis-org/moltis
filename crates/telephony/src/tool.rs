//! Agent tool for voice calls.
//!
//! Exposes `voice_call` to the agent loop, enabling agents to initiate and
//! manage phone calls programmatically.

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::sync::Arc,
    tokio::sync::RwLock,
    tracing::debug,
};

use crate::{manager::CallManager, types::CallMode};

/// Per-account state stored in the tool.
struct ToolAccount {
    manager: Arc<RwLock<CallManager>>,
    from_number: String,
}

/// Agent tool that allows the LLM to make and manage phone calls.
pub struct VoiceCallTool {
    /// Account ID → per-account state.
    accounts: Arc<RwLock<Vec<(String, ToolAccount)>>>,
    /// Default account to use when not specified.
    default_account: Option<String>,
    /// Default webhook base URL for callbacks.
    webhook_base_url: String,
}

impl VoiceCallTool {
    pub fn new(webhook_base_url: String) -> Self {
        Self {
            accounts: Arc::new(RwLock::new(Vec::new())),
            default_account: None,
            webhook_base_url,
        }
    }

    /// Register a call manager for an account.
    pub async fn add_manager(
        &self,
        account_id: String,
        manager: Arc<RwLock<CallManager>>,
        from_number: String,
    ) {
        self.accounts.write().await.push((account_id, ToolAccount {
            manager,
            from_number,
        }));
    }

    async fn resolve_account(
        &self,
        account_id: Option<&str>,
    ) -> anyhow::Result<(String, Arc<RwLock<CallManager>>, String)> {
        let accounts = self.accounts.read().await;
        if accounts.is_empty() {
            anyhow::bail!("no telephony accounts configured");
        }

        let (id, acct) = if let Some(aid) = account_id.or(self.default_account.as_deref()) {
            accounts
                .iter()
                .find(|(id, _)| id == aid)
                .ok_or_else(|| anyhow::anyhow!("account {aid} not found"))?
        } else {
            &accounts[0]
        };
        Ok((
            id.clone(),
            Arc::clone(&acct.manager),
            acct.from_number.clone(),
        ))
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

                let (acct, mgr, from_number) = self.resolve_account(account_id).await?;
                if from_number.is_empty() {
                    anyhow::bail!("no from_number configured for account {acct}");
                }
                let manager = mgr.read().await;

                let status_url = format!(
                    "{}/api/channels/telephony/{acct}/status",
                    self.webhook_base_url
                );
                let answer_url = format!(
                    "{}/api/channels/telephony/{acct}/answer",
                    self.webhook_base_url
                );

                let call_id = manager
                    .initiate(
                        &from_number,
                        to,
                        mode,
                        message,
                        &acct,
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

                let (_acct, mgr, _from) = self.resolve_account(account_id).await?;
                mgr.read().await.hangup(call_id).await?;

                Ok(json!({ "status": "ended", "call_id": call_id }))
            },
            "get_status" => {
                let call_id = params["call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'call_id' is required"))?;

                let (_acct, mgr, _from) = self.resolve_account(account_id).await?;
                let record = mgr
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

                let (_acct, mgr, _from) = self.resolve_account(account_id).await?;
                let manager = mgr.read().await;
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
    use {super::*, crate::providers::mock::MockProvider};

    async fn test_tool() -> VoiceCallTool {
        let tool = VoiceCallTool::new("https://example.com".into());
        let mgr = Arc::new(RwLock::new(CallManager::new(
            Box::new(MockProvider::new()),
            60,
        )));
        tool.add_manager("test-acct".into(), mgr, "+15551111111".into())
            .await;
        tool
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
