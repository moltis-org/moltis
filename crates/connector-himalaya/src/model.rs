use {
    moltis_connectors::{ConnectorItemInput, SourceObservation},
    serde::{Deserialize, Deserializer, Serialize, de},
};

use crate::HimalayaBackend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaAccount {
    #[serde(alias = "account")]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_backends")]
    pub backends: Vec<HimalayaBackend>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaMailbox {
    pub id: String,
    #[serde(default, alias = "name", alias = "displayName", alias = "display_name")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaAddress {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(alias = "address", alias = "email")]
    pub addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaFlag {
    pub raw: String,
    pub iana: Option<HimalayaIanaFlag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HimalayaIanaFlag {
    Seen,
    Answered,
    Flagged,
    Draft,
    Deleted,
    Forwarded,
    Junk,
    NotJunk,
    Phishing,
    Important,
    MdnSent,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaEnvelope {
    #[serde(deserialize_with = "deserialize_id")]
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default, deserialize_with = "deserialize_addresses")]
    pub from: Vec<HimalayaAddress>,
    #[serde(default, deserialize_with = "deserialize_addresses")]
    pub to: Vec<HimalayaAddress>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub flags: Vec<HimalayaFlag>,
    #[serde(
        default,
        rename = "message-id",
        alias = "message_id",
        alias = "messageId"
    )]
    pub message_id: Option<String>,
    #[serde(
        default,
        rename = "has-attachment",
        alias = "has_attachment",
        alias = "hasAttachment"
    )]
    pub has_attachment: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_u32")]
    pub uid: Option<u32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrInteger {
    String(String),
    Unsigned(u64),
    Signed(i64),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrManyAddresses {
    One(HimalayaAddress),
    Many(Vec<HimalayaAddress>),
}

fn deserialize_addresses<'de, D>(deserializer: D) -> Result<Vec<HimalayaAddress>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(
        match Option::<OneOrManyAddresses>::deserialize(deserializer)? {
            Some(OneOrManyAddresses::One(address)) => vec![address],
            Some(OneOrManyAddresses::Many(addresses)) => addresses,
            None => Vec::new(),
        },
    )
}

fn deserialize_backends<'de, D>(deserializer: D) -> Result<Vec<HimalayaBackend>, D::Error>
where
    D: Deserializer<'de>,
{
    let backends = Vec::<String>::deserialize(deserializer)?;
    Ok(backends
        .into_iter()
        .filter_map(|backend| match backend.as_str() {
            "imap" => Some(HimalayaBackend::Imap),
            "jmap" => Some(HimalayaBackend::Jmap),
            "gmail" => Some(HimalayaBackend::Gmail),
            "msgraph" => Some(HimalayaBackend::Msgraph),
            "maildir" => Some(HimalayaBackend::Maildir),
            "m2dir" => Some(HimalayaBackend::M2dir),
            _ => None,
        })
        .collect())
}

fn deserialize_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    match StringOrInteger::deserialize(deserializer)? {
        StringOrInteger::String(value) if !value.is_empty() => Ok(value),
        StringOrInteger::Unsigned(value) => Ok(value.to_string()),
        StringOrInteger::Signed(value) if value >= 0 => Ok(value.to_string()),
        _ => Err(de::Error::custom(
            "message ID must be nonempty and nonnegative",
        )),
    }
}

fn deserialize_optional_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<StringOrInteger>::deserialize(deserializer)?;
    value
        .map(|value| match value {
            StringOrInteger::String(value) => value.parse::<u32>().map_err(de::Error::custom),
            StringOrInteger::Unsigned(value) => u32::try_from(value).map_err(de::Error::custom),
            StringOrInteger::Signed(value) => u32::try_from(value).map_err(de::Error::custom),
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaAttachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MimeStatus {
    Parsed,
    Unparsed,
    TooLarge,
    NotRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HimalayaMessageBody {
    pub schema_version: u32,
    pub backend: HimalayaBackend,
    pub mailboxes: Vec<String>,
    pub envelope: HimalayaEnvelope,
    pub plain_text_bodies: Vec<String>,
    pub attachments: Vec<HimalayaAttachment>,
    pub mime_status: MimeStatus,
    pub raw_size: usize,
    pub raw_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HimalayaSnapshot {
    pub items: Vec<ConnectorItemInput>,
    pub source_observations: Vec<SourceObservation>,
    pub mailboxes: Vec<HimalayaMailbox>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_accepts_the_v2_json_shape() -> serde_json::Result<()> {
        let envelope: HimalayaEnvelope = serde_json::from_str(
            r#"{
                "id": "42",
                "from": [{"name":"Sender","email":"from@example.com"}],
                "to": [{"name":"Recipient","email":"to@example.com"}],
                "flags": [{"raw":"\\Seen","iana":"seen"}],
                "message-id":"<42@example.com>",
                "has-attachment":null,
                "additiveField":"ignored"
            }"#,
        )?;
        assert_eq!(envelope.id, "42");
        assert_eq!(envelope.from.len(), 1);
        assert_eq!(envelope.to.len(), 1);
        assert_eq!(envelope.flags[0].iana, Some(HimalayaIanaFlag::Seen));
        assert_eq!(envelope.message_id.as_deref(), Some("<42@example.com>"));
        assert_eq!(envelope.has_attachment, None);
        Ok(())
    }

    #[test]
    fn account_accepts_v2_backends_and_ignores_send_only_backends() -> serde_json::Result<()> {
        let account: HimalayaAccount =
            serde_json::from_str(r#"{"name":"work","default":true,"backends":["imap","smtp"]}"#)?;
        assert_eq!(account.name, "work");
        assert_eq!(account.backends, vec![HimalayaBackend::Imap]);
        Ok(())
    }
}
