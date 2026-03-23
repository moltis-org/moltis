use serde::{Deserialize, Serialize};

/// Unique identifier for an agent.
pub type AgentId = String;

/// Unique identifier for a channel account.
pub type AccountId = String;

/// Unique identifier for a peer (user on a channel).
pub type PeerId = String;

/// Channel identifier (e.g. "telegram", "discord", "whatsapp").
pub type ChannelId = String;

/// Chat type for routing and session scoping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChatType {
    Dm,
    Group,
    Channel,
}

/// Normalized inbound message context (mirrors MsgContext from TypeScript).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgContext {
    pub body: String,
    pub from: PeerId,
    pub to: String,
    pub channel: ChannelId,
    pub account_id: AccountId,
    pub chat_type: ChatType,
    pub session_key: String,
    pub reply_to_id: Option<String>,
    pub media_path: Option<String>,
    pub media_url: Option<String>,
    pub group_id: Option<String>,
    pub guild_id: Option<String>,
    pub team_id: Option<String>,
    pub sender_name: Option<String>,
}

/// Outbound reply payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyPayload {
    pub text: String,
    pub media: Option<MediaAttachment>,
    pub reply_to_id: Option<String>,
    pub silent: bool,
}

/// Media attachment for outbound messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub url: String,
    pub mime_type: String,
    /// Optional original filename (e.g. "report.pdf"). Channel outbounds use
    /// this instead of a generic placeholder when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_attachment_round_trip_with_filename() {
        let original = MediaAttachment {
            url: "data:application/pdf;base64,abc".to_string(),
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"filename\":\"report.pdf\""));
        let decoded: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.filename.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn media_attachment_round_trip_without_filename() {
        let original = MediaAttachment {
            url: "data:image/png;base64,xyz".to_string(),
            mime_type: "image/png".to_string(),
            filename: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        // filename should be omitted entirely
        assert!(!json.contains("filename"));
        let decoded: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.filename, None);
    }

    #[test]
    fn media_attachment_deserialize_without_filename_field() {
        // Backward compatibility: old JSON without filename field
        let json = r#"{"url":"data:image/png;base64,xyz","mime_type":"image/png"}"#;
        let decoded: MediaAttachment = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.filename, None);
        assert_eq!(decoded.mime_type, "image/png");
    }
}
