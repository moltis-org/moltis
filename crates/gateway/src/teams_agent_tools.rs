//! Agent tools for Microsoft Teams Graph API operations.
//!
//! These tools give agents the ability to search messages, look up members,
//! pin/unpin messages, and edit/delete messages in Teams conversations.

use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
    std::sync::Arc,
    tokio::sync::RwLock,
};

type TeamsPlugin = Arc<RwLock<moltis_msteams::MsTeamsPlugin>>;

/// Resolve account_id: use the provided value or fall back to the first
/// configured Teams account.
async fn resolve_account(plugin: &TeamsPlugin, params: &Value) -> Result<String> {
    if let Some(id) = params.get("account_id").and_then(Value::as_str) {
        let id = id.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    let p = plugin.read().await;
    let ids = p.account_ids();
    ids.into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no Teams accounts configured"))
}

// ── Search Messages ──────────────────────────────────────────────────────────

pub struct TeamsSearchMessagesTool {
    plugin: TeamsPlugin,
}

impl TeamsSearchMessagesTool {
    pub fn new(plugin: TeamsPlugin) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl AgentTool for TeamsSearchMessagesTool {
    fn name(&self) -> &str {
        "teams_search_messages"
    }

    fn description(&self) -> &str {
        "Search message history in a Microsoft Teams chat or channel. Returns matching messages with sender, text, and timestamp. Requires Graph API permissions (Chat.Read.All)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["chat_id", "query"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Teams account ID. Defaults to the first configured account."
                },
                "chat_id": {
                    "type": "string",
                    "description": "The Teams conversation/chat ID to search in."
                },
                "query": {
                    "type": "string",
                    "description": "Search text to find in message bodies."
                },
                "from": {
                    "type": "string",
                    "description": "Optional: filter by sender display name."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (1-50, default 25).",
                    "default": 25
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = resolve_account(&self.plugin, &params).await?;
        let chat_id = params["chat_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: chat_id"))?;
        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;
        let from = params.get("from").and_then(Value::as_str);
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(25) as usize;

        let (http, token) = {
            let p = self.plugin.read().await;
            p.graph_client(&account_id).await?
        };

        let result =
            moltis_msteams::graph::search_messages(&http, &token, chat_id, query, from, limit)
                .await?;

        let messages: Vec<Value> = result
            .messages
            .iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "text": m.body_content,
                    "from": m.from_user_name,
                    "from_id": m.from_user_id,
                    "is_bot": m.is_bot,
                    "created_at": m.created_at,
                })
            })
            .collect();

        Ok(json!({ "ok": true, "messages": messages, "count": messages.len() }))
    }
}

// ── Member Info ──────────────────────────────────────────────────────────────

pub struct TeamsMemberInfoTool {
    plugin: TeamsPlugin,
}

impl TeamsMemberInfoTool {
    pub fn new(plugin: TeamsPlugin) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl AgentTool for TeamsMemberInfoTool {
    fn name(&self) -> &str {
        "teams_member_info"
    }

    fn description(&self) -> &str {
        "Look up a Microsoft Teams user's profile information (name, email, job title, office location). Requires Graph API permissions (User.Read.All)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["user_id"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Teams account ID. Defaults to the first configured account."
                },
                "user_id": {
                    "type": "string",
                    "description": "The AAD Object ID or User Principal Name of the user to look up."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = resolve_account(&self.plugin, &params).await?;
        let user_id = params["user_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: user_id"))?;

        let (http, token) = {
            let p = self.plugin.read().await;
            p.graph_client(&account_id).await?
        };

        let info = moltis_msteams::graph::get_member_info(&http, &token, user_id).await?;

        Ok(json!({
            "ok": true,
            "id": info.id,
            "display_name": info.display_name,
            "email": info.mail,
            "job_title": info.job_title,
            "user_principal_name": info.user_principal_name,
            "office_location": info.office_location,
        }))
    }
}

// ── Pin Message ──────────────────────────────────────────────────────────────

pub struct TeamsPinMessageTool {
    plugin: TeamsPlugin,
}

impl TeamsPinMessageTool {
    pub fn new(plugin: TeamsPlugin) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl AgentTool for TeamsPinMessageTool {
    fn name(&self) -> &str {
        "teams_pin_message"
    }

    fn description(&self) -> &str {
        "Pin or unpin a message in a Microsoft Teams chat, or list all pinned messages. Pinned messages appear at the top of the chat for all participants."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["chat_id", "action"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Teams account ID. Defaults to the first configured account."
                },
                "chat_id": {
                    "type": "string",
                    "description": "The Teams conversation/chat ID."
                },
                "action": {
                    "type": "string",
                    "enum": ["pin", "unpin", "list"],
                    "description": "Action to perform: pin a message, unpin a message, or list all pinned messages."
                },
                "message_id": {
                    "type": "string",
                    "description": "The message ID to pin (required for 'pin' action)."
                },
                "pinned_message_id": {
                    "type": "string",
                    "description": "The pinned-message resource ID to unpin (required for 'unpin' action). Get this from the 'list' action."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = resolve_account(&self.plugin, &params).await?;
        let chat_id = params["chat_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: chat_id"))?;
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        let (http, token) = {
            let p = self.plugin.read().await;
            p.graph_client(&account_id).await?
        };

        match action {
            "pin" => {
                let message_id = params["message_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'pin' action requires message_id"))?;
                let pinned_id =
                    moltis_msteams::graph::pin_message(&http, &token, chat_id, message_id).await?;
                Ok(json!({
                    "ok": true,
                    "action": "pin",
                    "pinned_message_id": pinned_id,
                    "message_id": message_id,
                }))
            },
            "unpin" => {
                let pinned_message_id = params["pinned_message_id"].as_str().ok_or_else(|| {
                    anyhow::anyhow!("'unpin' action requires pinned_message_id (from 'list')")
                })?;
                moltis_msteams::graph::unpin_message(&http, &token, chat_id, pinned_message_id)
                    .await?;
                Ok(json!({ "ok": true, "action": "unpin" }))
            },
            "list" => {
                let pins = moltis_msteams::graph::list_pins(&http, &token, chat_id).await?;
                let items: Vec<Value> = pins
                    .iter()
                    .map(|p| {
                        json!({
                            "pinned_message_id": p.pinned_message_id,
                            "message_id": p.message_id,
                            "text": p.text,
                        })
                    })
                    .collect();
                Ok(json!({ "ok": true, "action": "list", "pins": items, "count": items.len() }))
            },
            other => Err(anyhow::anyhow!(
                "unknown action '{other}'; use 'pin', 'unpin', or 'list'"
            )),
        }
    }
}

// ── Edit / Delete Message ────────────────────────────────────────────────────

pub struct TeamsEditMessageTool {
    plugin: TeamsPlugin,
}

impl TeamsEditMessageTool {
    pub fn new(plugin: TeamsPlugin) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl AgentTool for TeamsEditMessageTool {
    fn name(&self) -> &str {
        "teams_edit_message"
    }

    fn description(&self) -> &str {
        "Edit or delete a previously sent message in a Microsoft Teams conversation. Only messages sent by the bot can be edited or deleted."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["chat_id", "activity_id", "action"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Teams account ID. Defaults to the first configured account."
                },
                "chat_id": {
                    "type": "string",
                    "description": "The Teams conversation/chat ID containing the message."
                },
                "activity_id": {
                    "type": "string",
                    "description": "The activity ID of the message to edit or delete."
                },
                "action": {
                    "type": "string",
                    "enum": ["edit", "delete"],
                    "description": "Whether to edit the message text or delete it entirely."
                },
                "text": {
                    "type": "string",
                    "description": "New message text (required for 'edit' action)."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = resolve_account(&self.plugin, &params).await?;
        let chat_id = params["chat_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: chat_id"))?;
        let activity_id = params["activity_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: activity_id"))?;
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        match action {
            "edit" => {
                let text = params["text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'edit' action requires text"))?;
                let p = self.plugin.read().await;
                p.edit_message(&account_id, chat_id, activity_id, text)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!({ "ok": true, "action": "edit" }))
            },
            "delete" => {
                let p = self.plugin.read().await;
                p.delete_message(&account_id, chat_id, activity_id)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!({ "ok": true, "action": "delete" }))
            },
            other => Err(anyhow::anyhow!(
                "unknown action '{other}'; use 'edit' or 'delete'"
            )),
        }
    }
}

// ── Read Message ─────────────────────────────────────────────────────────────

pub struct TeamsReadMessageTool {
    plugin: TeamsPlugin,
}

impl TeamsReadMessageTool {
    pub fn new(plugin: TeamsPlugin) -> Self {
        Self { plugin }
    }
}

#[async_trait]
impl AgentTool for TeamsReadMessageTool {
    fn name(&self) -> &str {
        "teams_read_message"
    }

    fn description(&self) -> &str {
        "Read a specific message by ID from a Microsoft Teams chat, including its reactions. Requires Graph API permissions (Chat.Read.All)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["chat_id", "message_id"],
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Teams account ID. Defaults to the first configured account."
                },
                "chat_id": {
                    "type": "string",
                    "description": "The Teams conversation/chat ID."
                },
                "message_id": {
                    "type": "string",
                    "description": "The message ID to read."
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let account_id = resolve_account(&self.plugin, &params).await?;
        let chat_id = params["chat_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: chat_id"))?;
        let message_id = params["message_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: message_id"))?;

        let (http, token) = {
            let p = self.plugin.read().await;
            p.graph_client(&account_id).await?
        };

        let msg = moltis_msteams::graph::get_message(&http, &token, chat_id, message_id).await?;

        Ok(json!({
            "ok": true,
            "id": msg.id,
            "text": msg.body_content,
            "from": msg.from_user_name,
            "from_id": msg.from_user_id,
            "is_bot": msg.is_bot,
            "created_at": msg.created_at,
        }))
    }
}
