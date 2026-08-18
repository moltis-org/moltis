use {
    moltis_connectors::{ConnectorItemInput, SourceObservation},
    serde::{Deserialize, Serialize},
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailProfile {
    #[serde(default)]
    pub email_address: String,
    #[serde(default)]
    pub messages_total: u64,
    #[serde(default)]
    pub threads_total: u64,
    #[serde(default)]
    pub history_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessageRef {
    pub id: String,
    #[serde(default)]
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessagePage {
    #[serde(default)]
    pub messages: Vec<GmailMessageRef>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub result_size_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessagePartBody {
    #[serde(default)]
    pub attachment_id: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessagePart {
    #[serde(default)]
    pub part_id: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<GmailHeader>,
    #[serde(default)]
    pub body: GmailMessagePartBody,
    #[serde(default)]
    pub parts: Vec<GmailMessagePart>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailApiMessage {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub snippet: String,
    #[serde(default)]
    pub history_id: Option<String>,
    #[serde(default)]
    pub internal_date: Option<String>,
    #[serde(default)]
    pub payload: GmailMessagePart,
    #[serde(default)]
    pub size_estimate: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmailBodyFormat {
    PlainText,
    HtmlDerived,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub attachment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessageBody {
    pub schema_version: u32,
    pub id: String,
    pub thread_id: String,
    pub history_id: Option<String>,
    pub internal_date: Option<String>,
    pub label_ids: Vec<String>,
    pub snippet: String,
    pub headers: Vec<GmailHeader>,
    pub body: Option<String>,
    pub body_format: GmailBodyFormat,
    pub attachments: Vec<GmailAttachment>,
    pub size_estimate: u64,
}

impl GmailMessageBody {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GmailSnapshot {
    pub items: Vec<ConnectorItemInput>,
    pub source_observations: Vec<SourceObservation>,
    pub profile: Option<GmailProfile>,
}
