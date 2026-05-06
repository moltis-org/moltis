//! Microsoft Graph API client for Teams operations.
//!
//! Provides thread context (message history), reactions, and message management
//! via the Microsoft Graph API.

use {secrecy::ExposeSecret, tracing::debug};

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const GRAPH_API_BETA: &str = "https://graph.microsoft.com/beta";

/// A message from the Graph API.
#[derive(Debug, Clone)]
pub struct GraphMessage {
    pub id: String,
    pub body_content: Option<String>,
    pub from_user_id: Option<String>,
    pub from_user_name: Option<String>,
    pub is_bot: bool,
    pub created_at: Option<String>,
}

/// Summary of reactions on a message.
#[derive(Debug, Clone)]
pub struct ReactionSummary {
    pub reaction_type: String,
    pub user_name: Option<String>,
}

/// Valid Teams reaction types.
pub const TEAMS_REACTION_TYPES: &[&str] = &["like", "heart", "laugh", "surprised", "sad", "angry"];

/// Fetch recent messages from a chat via Graph API.
pub async fn fetch_chat_messages(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<GraphMessage>> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/messages?$top={limit}&$orderby=createdDateTime desc",
        urlencoding::encode(chat_id),
    );

    let resp = http
        .get(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API fetch messages failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    let messages = json["value"]
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_graph_message).collect())
        .unwrap_or_default();

    Ok(messages)
}

/// Add a reaction to a message via Graph API (beta).
pub async fn add_reaction(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    message_id: &str,
    reaction_type: &str,
) -> anyhow::Result<()> {
    if !TEAMS_REACTION_TYPES.contains(&reaction_type) {
        anyhow::bail!(
            "unsupported reaction type '{reaction_type}'; valid types: {}",
            TEAMS_REACTION_TYPES.join(", ")
        );
    }

    let url = format!(
        "{GRAPH_API_BETA}/chats/{}/messages/{}/setReaction",
        urlencoding::encode(chat_id),
        urlencoding::encode(message_id),
    );

    let resp = http
        .post(&url)
        .bearer_auth(token.expose_secret())
        .json(&serde_json::json!({ "reactionType": reaction_type }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API setReaction failed ({status}): {body}");
    }

    debug!(chat_id, message_id, reaction_type, "reaction added");
    Ok(())
}

/// Remove a reaction from a message via Graph API (beta).
pub async fn remove_reaction(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    message_id: &str,
    reaction_type: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "{GRAPH_API_BETA}/chats/{}/messages/{}/unsetReaction",
        urlencoding::encode(chat_id),
        urlencoding::encode(message_id),
    );

    let resp = http
        .post(&url)
        .bearer_auth(token.expose_secret())
        .json(&serde_json::json!({ "reactionType": reaction_type }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API unsetReaction failed ({status}): {body}");
    }

    Ok(())
}

/// List reactions on a message via Graph API.
pub async fn list_reactions(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    message_id: &str,
) -> anyhow::Result<Vec<ReactionSummary>> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/messages/{}",
        urlencoding::encode(chat_id),
        urlencoding::encode(message_id),
    );

    let resp = http
        .get(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API get message failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    let reactions = json["reactions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    Some(ReactionSummary {
                        reaction_type: r["reactionType"].as_str()?.to_string(),
                        user_name: r["user"]["displayName"].as_str().map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(reactions)
}

// ── Message retrieval ────────────────────────────────────────────────────────

/// Fetch a single message by ID via Graph API.
pub async fn get_message(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    message_id: &str,
) -> anyhow::Result<GraphMessage> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/messages/{}",
        urlencoding::encode(chat_id),
        urlencoding::encode(message_id),
    );

    let resp = http
        .get(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API get message failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    parse_graph_message(&json)
        .ok_or_else(|| anyhow::anyhow!("Graph API returned invalid message format"))
}

// ── Search ──────────────────────────────────────────────────────────────────

/// Search result from the Graph API.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub messages: Vec<GraphMessage>,
}

/// Search messages in a chat via Graph API.
///
/// Uses the `$search` OData parameter for body content search. Requires the
/// `ConsistencyLevel: eventual` header.
pub async fn search_messages(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    query: &str,
    from_user: Option<&str>,
    limit: usize,
) -> anyhow::Result<SearchResult> {
    // Sanitize query — remove double quotes to prevent OData injection.
    let sanitized = query.replace('"', "");
    let top = limit.clamp(1, 50);

    let base_url = format!(
        "{GRAPH_API_BASE}/chats/{}/messages",
        urlencoding::encode(chat_id),
    );

    let search_val = format!("\"{sanitized}\"");
    let mut query_params: Vec<(&str, String)> =
        vec![("$search", search_val), ("$top", top.to_string())];
    if let Some(user) = from_user {
        let escaped = user.replace('\'', "''");
        query_params.push(("$filter", format!("from/user/displayName eq '{escaped}'")));
    }

    let resp = http
        .get(&base_url)
        .query(&query_params)
        .bearer_auth(token.expose_secret())
        .header("ConsistencyLevel", "eventual")
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API search messages failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    let messages = json["value"]
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_graph_message).collect())
        .unwrap_or_default();

    Ok(SearchResult { messages })
}

// ── Member info ─────────────────────────────────────────────────────────────

/// User profile information from Microsoft Graph.
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub job_title: Option<String>,
    pub user_principal_name: Option<String>,
    pub office_location: Option<String>,
}

/// Fetch user profile information via Graph API.
pub async fn get_member_info(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    user_id: &str,
) -> anyhow::Result<MemberInfo> {
    let url = format!(
        "{GRAPH_API_BASE}/users/{}?$select=id,displayName,mail,jobTitle,userPrincipalName,officeLocation",
        urlencoding::encode(user_id),
    );

    let resp = http
        .get(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API get user failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    Ok(MemberInfo {
        id: json["id"].as_str().map(String::from),
        display_name: json["displayName"].as_str().map(String::from),
        mail: json["mail"].as_str().map(String::from),
        job_title: json["jobTitle"].as_str().map(String::from),
        user_principal_name: json["userPrincipalName"].as_str().map(String::from),
        office_location: json["officeLocation"].as_str().map(String::from),
    })
}

// ── Pinned messages ─────────────────────────────────────────────────────────

/// A pinned message in a chat.
#[derive(Debug, Clone)]
pub struct PinnedMessage {
    /// The pinned-message resource ID (needed for unpin).
    pub pinned_message_id: String,
    /// The underlying message ID.
    pub message_id: Option<String>,
    /// The message text (available if `$expand=message` was used).
    pub text: Option<String>,
}

/// Pin a message in a chat via Graph API.
///
/// Returns the pinned-message resource ID needed for unpinning.
pub async fn pin_message(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    message_id: &str,
) -> anyhow::Result<String> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/pinnedMessages",
        urlencoding::encode(chat_id),
    );

    let resp = http
        .post(&url)
        .bearer_auth(token.expose_secret())
        .json(&serde_json::json!({
            "message@odata.bind": format!(
                "https://graph.microsoft.com/v1.0/chats/{}/messages/{}",
                urlencoding::encode(chat_id),
                urlencoding::encode(message_id),
            ),
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API pin message failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    json["id"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("pin response missing id"))
}

/// Unpin a message in a chat via Graph API.
///
/// Requires the pinned-message resource ID (from `pin_message` or `list_pins`),
/// NOT the message ID.
pub async fn unpin_message(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
    pinned_message_id: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/pinnedMessages/{}",
        urlencoding::encode(chat_id),
        urlencoding::encode(pinned_message_id),
    );

    let resp = http
        .delete(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API unpin message failed ({status}): {body}");
    }

    Ok(())
}

/// List all pinned messages in a chat via Graph API.
pub async fn list_pins(
    http: &reqwest::Client,
    token: &secrecy::Secret<String>,
    chat_id: &str,
) -> anyhow::Result<Vec<PinnedMessage>> {
    let url = format!(
        "{GRAPH_API_BASE}/chats/{}/pinnedMessages?$expand=message",
        urlencoding::encode(chat_id),
    );

    let resp = http
        .get(&url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Graph API list pins failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    let pins = json["value"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let pinned_message_id = p["id"].as_str()?.to_string();
                    let message = &p["message"];
                    let message_id = message["id"].as_str().map(String::from);
                    let text = message["body"]["content"].as_str().map(String::from);
                    Some(PinnedMessage {
                        pinned_message_id,
                        message_id,
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(pins)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_graph_message(msg: &serde_json::Value) -> Option<GraphMessage> {
    let id = msg["id"].as_str()?.to_string();
    let body_content = msg["body"]["content"].as_str().map(String::from);
    let from = &msg["from"];
    let from_user_id = from["user"]["id"].as_str().map(String::from);
    let from_user_name = from["user"]["displayName"]
        .as_str()
        .or_else(|| from["application"]["displayName"].as_str())
        .map(String::from);
    let is_bot = from["application"].is_object();
    let created_at = msg["createdDateTime"].as_str().map(String::from);

    Some(GraphMessage {
        id,
        body_content,
        from_user_id,
        from_user_name,
        is_bot,
        created_at,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_graph_message_basic() {
        let json = serde_json::json!({
            "id": "msg-1",
            "body": { "content": "hello world" },
            "from": {
                "user": { "id": "user-1", "displayName": "Alice" }
            },
            "createdDateTime": "2026-01-01T00:00:00Z",
        });
        let msg = parse_graph_message(&json).unwrap();
        assert_eq!(msg.id, "msg-1");
        assert_eq!(msg.body_content.as_deref(), Some("hello world"));
        assert_eq!(msg.from_user_id.as_deref(), Some("user-1"));
        assert_eq!(msg.from_user_name.as_deref(), Some("Alice"));
        assert!(!msg.is_bot);
    }

    #[test]
    fn parse_graph_message_bot() {
        let json = serde_json::json!({
            "id": "msg-2",
            "body": { "content": "bot reply" },
            "from": {
                "application": { "id": "app-1", "displayName": "Bot" }
            },
            "createdDateTime": "2026-01-01T00:01:00Z",
        });
        let msg = parse_graph_message(&json).unwrap();
        assert!(msg.is_bot);
        assert_eq!(msg.from_user_name.as_deref(), Some("Bot"));
    }

    #[test]
    fn parse_graph_message_missing_id_returns_none() {
        let json = serde_json::json!({
            "body": { "content": "no id" },
        });
        assert!(parse_graph_message(&json).is_none());
    }

    #[test]
    fn reaction_type_validation() {
        assert!(TEAMS_REACTION_TYPES.contains(&"like"));
        assert!(TEAMS_REACTION_TYPES.contains(&"heart"));
        assert!(!TEAMS_REACTION_TYPES.contains(&"thumbsup"));
    }
}
