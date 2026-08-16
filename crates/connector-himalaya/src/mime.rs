use {
    mail_parser::{MessageParser, MimeHeaders},
    sha2::{Digest, Sha256},
};

use crate::{
    HimalayaAttachment, HimalayaBackend, HimalayaEnvelope, HimalayaMessageBody, MimeStatus,
};

pub(crate) const MAX_RAW_MESSAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_BODY_TEXT_BYTES: usize = 128 * 1024;
const MAX_BODY_PARTS: usize = 128;
const MAX_ATTACHMENTS: usize = 256;
const MAX_ATTACHMENT_NAME_BYTES: usize = 512;

#[must_use]
pub fn parse_message(
    backend: HimalayaBackend,
    mailboxes: Vec<String>,
    envelope: HimalayaEnvelope,
    raw: &[u8],
) -> HimalayaMessageBody {
    if raw.len() > MAX_RAW_MESSAGE_BYTES {
        return envelope_only(
            backend,
            mailboxes,
            envelope,
            MimeStatus::TooLarge,
            raw.len(),
            None,
        );
    }

    let sha = format!("{:x}", Sha256::digest(raw));
    let Some(message) = MessageParser::default().parse(raw) else {
        return envelope_only(
            backend,
            mailboxes,
            envelope,
            MimeStatus::Unparsed,
            raw.len(),
            Some(sha),
        );
    };

    let mut remaining = MAX_BODY_TEXT_BYTES;
    let plain_text_bodies = message
        .text_bodies()
        .take(MAX_BODY_PARTS)
        .filter_map(|part| part.text_contents())
        .filter_map(|text| {
            let bounded = bounded_utf8(text, remaining);
            remaining = remaining.saturating_sub(bounded.len());
            (!bounded.is_empty()).then_some(bounded)
        })
        .collect();
    let attachments = message
        .attachments()
        .take(MAX_ATTACHMENTS)
        .map(|part| {
            let content_type = part.content_type().map_or_else(
                || "application/octet-stream".to_owned(),
                |content_type| {
                    content_type.c_subtype.as_ref().map_or_else(
                        || content_type.c_type.to_string(),
                        |subtype| format!("{}/{}", content_type.c_type, subtype),
                    )
                },
            );
            HimalayaAttachment {
                filename: part
                    .attachment_name()
                    .map(|name| bounded_utf8(name, MAX_ATTACHMENT_NAME_BYTES)),
                content_type,
                size: part.len(),
            }
        })
        .collect();
    HimalayaMessageBody {
        schema_version: 1,
        backend,
        mailboxes,
        envelope,
        plain_text_bodies,
        attachments,
        mime_status: MimeStatus::Parsed,
        raw_size: raw.len(),
        raw_sha256: Some(sha),
    }
}

pub(crate) fn envelope_only(
    backend: HimalayaBackend,
    mailboxes: Vec<String>,
    envelope: HimalayaEnvelope,
    mime_status: MimeStatus,
    raw_size: usize,
    raw_sha256: Option<String>,
) -> HimalayaMessageBody {
    HimalayaMessageBody {
        schema_version: 1,
        backend,
        mailboxes,
        envelope,
        plain_text_bodies: Vec::new(),
        attachments: Vec::new(),
        mime_status,
        raw_size,
        raw_sha256,
    }
}

fn bounded_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> HimalayaEnvelope {
        HimalayaEnvelope {
            id: "42".to_owned(),
            subject: Some("hello".to_owned()),
            from: Vec::new(),
            to: Vec::new(),
            date: None,
            flags: Vec::new(),
            message_id: None,
            has_attachment: Some(true),
            uid: None,
        }
    }

    #[test]
    fn parses_text_non_utf8_and_attachment_metadata_without_bytes() {
        let raw = concat!(
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=x\r\n\r\n",
            "--x\r\nContent-Type: text/plain; charset=iso-8859-1\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n\r\ncaf=E9\r\n",
            "--x\r\nContent-Type: application/octet-stream; name=a.bin\r\n",
            "Content-Disposition: attachment; filename=a.bin\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--x--\r\n"
        );
        let body = parse_message(
            HimalayaBackend::Jmap,
            vec!["Inbox".to_owned()],
            envelope(),
            raw.as_bytes(),
        );
        assert_eq!(body.mime_status, MimeStatus::Parsed);
        assert!(
            body.plain_text_bodies
                .iter()
                .any(|text| text.contains("café"))
        );
        assert_eq!(body.attachments.len(), 1);
        assert_eq!(body.attachments[0].filename.as_deref(), Some("a.bin"));
        assert_eq!(body.attachments[0].size, 3);
        let encoded = serde_json::to_vec(&body).unwrap_or_default();
        assert!(!encoded.windows(3).any(|bytes| bytes == [0, 1, 2]));
    }

    #[test]
    fn oversized_message_is_envelope_only() {
        let raw = vec![b'x'; MAX_RAW_MESSAGE_BYTES + 1];
        let body = parse_message(
            HimalayaBackend::Maildir,
            vec!["Inbox".to_owned()],
            envelope(),
            &raw,
        );
        assert_eq!(body.mime_status, MimeStatus::TooLarge);
        assert!(body.raw_sha256.is_none());
        assert!(body.plain_text_bodies.is_empty());
    }
}
