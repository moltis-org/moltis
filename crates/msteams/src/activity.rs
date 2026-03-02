use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct TeamsActivity {
    #[serde(rename = "type")]
    pub activity_type: String,
    pub id: Option<String>,
    pub text: Option<String>,
    #[serde(rename = "serviceUrl")]
    pub service_url: Option<String>,
    pub from: Option<ActivityAccount>,
    pub recipient: Option<ActivityAccount>,
    pub conversation: Option<ActivityConversation>,
    pub entities: Option<Vec<ActivityEntity>>,
    #[serde(rename = "channelData")]
    pub channel_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivityAccount {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "aadObjectId")]
    pub aad_object_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivityConversation {
    pub id: Option<String>,
    #[serde(rename = "conversationType")]
    pub conversation_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivityEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub mentioned: Option<ActivityAccount>,
}

impl TeamsActivity {
    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation.as_ref()?.id.as_deref()
    }

    pub fn sender_id(&self) -> Option<String> {
        self.from
            .as_ref()
            .and_then(|from| from.aad_object_id.clone().or_else(|| from.id.clone()))
    }

    pub fn sender_name(&self) -> Option<String> {
        self.from.as_ref().and_then(|from| from.name.clone())
    }

    pub fn is_group_chat(&self) -> bool {
        if let Some(conv) = self
            .conversation
            .as_ref()
            .and_then(|c| c.conversation_type.as_deref())
            && conv.eq_ignore_ascii_case("personal")
        {
            return false;
        }

        if let Some(data) = self.channel_data.as_ref() {
            return data.get("team").is_some() || data.get("channel").is_some();
        }

        true
    }

    pub fn bot_is_mentioned(&self) -> bool {
        let recipient_id = self.recipient.as_ref().and_then(|r| r.id.as_deref());
        let Some(recipient_id) = recipient_id else {
            return false;
        };
        self.entities
            .as_ref()
            .map(|entities| {
                entities.iter().any(|entity| {
                    entity.entity_type.eq_ignore_ascii_case("mention")
                        && entity
                            .mentioned
                            .as_ref()
                            .and_then(|m| m.id.as_deref())
                            .is_some_and(|id| id == recipient_id)
                })
            })
            .unwrap_or(false)
    }

    pub fn cleaned_text(&self) -> Option<String> {
        let mut text = self.text.clone()?;
        while let Some(start) = text.find("<at>") {
            if let Some(end_rel) = text[start + 4..].find("</at>") {
                let end = start + 4 + end_rel + 5;
                text.replace_range(start..end, "");
            } else {
                break;
            }
        }

        let text = text.trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
