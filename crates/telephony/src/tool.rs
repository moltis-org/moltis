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

/// Agent tool that allows the LLM to make and manage phone calls.
pub struct VoiceCallTool {
    /// Account ID → CallManager for each active telephony account.
    managers: Arc<RwLock<Vec<(String, Arc<RwLock<CallManager>>)>>>,
    /// Default account to use when not specified.
    default_account: Option<String>,
    /// Default webhook base URL for callbacks.
    webhook_base_url: String,
}

impl VoiceCallTool {
    pub fn new(webhook_base_url: String) -> Self {
        Self {
            managers: Arc::new(RwLock::new(Vec::new())),
            default_account: None,
            webhook_base_url,
        }
    }

    /// Register a call manager for an account.
    pub async fn add_manager(&self, account_id: String, manager: Arc<RwLock<CallManager>>) {
        self.managers.write().await.push((account_id, manager));
    }

    async fn resolve_manager(
        &self,
        account_id: Option<&str>,
    ) -> anyhow::Result<(String, Arc<RwLock<CallManager>>)> {
        let managers = self.managers.read().await;
        if managers.is_empty() {
            anyhow::bail!("no telephony accounts configured");
        }

        if let Some(aid) = account_id.or(self.default_account.as_deref()) {
            let found = managers
                .iter()
                .find(|(id, _)| id == aid)
                .ok_or_else(|| anyhow::anyhow!("account {aid} not found"))?;
            Ok((found.0.clone(), Arc::clone(&found.1)))
        } else {
            let first = &managers[0];
            Ok((first.0.clone(), Arc::clone(&first.1)))
        }
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

                let (acct, mgr) = self.resolve_manager(account_id).await?;
                let manager = mgr.read().await;

                // Build webhook URLs.
                let status_url = format!("{}/telephony/{acct}/status", self.webhook_base_url);
                let answer_url = format!("{}/telephony/{acct}/answer", self.webhook_base_url);

                // Get from_number from active calls context (simplified).
                let from = "+10000000000"; // Placeholder — resolved from account config at runtime.

                let call_id = manager
                    .initiate(from, to, mode, message, &acct, &status_url, &answer_url)
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

                let (_acct, mgr) = self.resolve_manager(account_id).await?;
                mgr.read().await.hangup(call_id).await?;

                Ok(json!({ "status": "ended", "call_id": call_id }))
            },
            "get_status" => {
                let call_id = params["call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'call_id' is required"))?;

                let (_acct, mgr) = self.resolve_manager(account_id).await?;
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

                let (_acct, mgr) = self.resolve_manager(account_id).await?;
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
        let tool = VoiceCallTool::new("https://example.com/api".into());
        let mgr = Arc::new(RwLock::new(CallManager::new(
            Box::new(MockProvider::new()),
            60,
        )));
        tool.add_manager("test-acct".into(), mgr).await;
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
