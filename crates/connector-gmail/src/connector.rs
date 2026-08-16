use {
    base64::{
        Engine,
        engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    },
    futures::StreamExt,
    moltis_connectors::{ConnectorItemInput, SourceDisposition, SourceObservation, SourceStateMap},
    sha2::{Digest, Sha256},
    std::{collections::HashSet, path::PathBuf, sync::Arc, time::Instant},
    time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339},
};

use crate::{
    GmailAccountConfig, GmailApiMessage, GmailAttachment, GmailBodyFormat, GmailClient,
    GmailConnectorError, GmailDatasetConfig, GmailHeader, GmailMessageBody, GmailMessagePart,
    GmailMessageRef, GmailProfile, GmailSnapshot, NativeGmailClient, Result,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_PAGES: usize = 2_000;
const MAX_ID_BYTES: usize = 512;
const MAX_PAGE_TOKEN_BYTES: usize = 4 * 1024;
const MAX_THREAD_ID_BYTES: usize = 512;
const MAX_HISTORY_ID_BYTES: usize = 128;
const MAX_SNIPPET_BYTES: usize = 32 * 1024;
const MAX_LABELS: usize = 1_000;
const MAX_LABEL_BYTES: usize = 512;
const MAX_HEADERS: usize = 2_000;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADER_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_MIME_DEPTH: usize = 32;
const MAX_MIME_PARTS: usize = 2_000;
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 1_000;
const MAX_SEARCH_TEXT_BYTES: usize = 16 * 1024;
const MESSAGE_FETCH_CONCURRENCY: usize = 8;

pub struct GmailConnector {
    client: Arc<dyn GmailClient>,
}

impl GmailConnector {
    /// Creates a connector from a host-controlled, existing Google authorized-user file.
    /// The file is read once, never modified, and its path is not part of account config.
    pub async fn new(authorized_user_path: PathBuf) -> Result<Self> {
        let client = NativeGmailClient::from_authorized_user_file(&authorized_user_path).await?;
        Ok(Self {
            client: Arc::new(client),
        })
    }

    #[must_use]
    pub fn with_client(client: Arc<dyn GmailClient>) -> Self {
        Self { client }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn test_connection(&self, account: &GmailAccountConfig) -> Result<GmailProfile> {
        let started = Instant::now();
        account.validate()?;
        let profile = self.client.profile().await?;
        validate_profile(&profile)?;
        record_operation("test_connection", started, 0);
        Ok(profile)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_dataset(
        &self,
        account: &GmailAccountConfig,
        dataset: &GmailDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
    ) -> Result<GmailSnapshot> {
        let started = Instant::now();
        account.validate()?;
        dataset.validate()?;

        // Gmail list references contain no history/version metadata, so existing
        // source state cannot safely suppress a full message fetch.
        drop(existing);
        let profile = self.client.profile().await?;
        validate_profile(&profile)?;
        let references = self.list_all(dataset).await?;
        let mut items = Vec::with_capacity(references.len());
        let mut source_observations = Vec::with_capacity(references.len());
        let mut aggregate_bytes = 0_usize;

        let mut messages =
            futures::stream::iter(references.into_iter().map(|reference| async move {
                let message = self.client.get_message(&reference.id).await?;
                Ok::<_, GmailConnectorError>((reference, message))
            }))
            .buffered(MESSAGE_FETCH_CONCURRENCY);
        while let Some(result) = messages.next().await {
            let (reference, message) = result?;
            if message.id != reference.id {
                return Err(response_error(
                    "Gmail get response id did not match requested id",
                ));
            }
            let item = message_item(message, dataset.include_body)?;
            aggregate_bytes = aggregate_bytes
                .checked_add(serde_json::to_vec(&item.body_json)?.len())
                .ok_or_else(|| response_error("Gmail snapshot exceeds aggregate size limit"))?;
            if aggregate_bytes > MAX_SNAPSHOT_BYTES {
                return Err(response_error(
                    "Gmail snapshot exceeds aggregate size limit",
                ));
            }
            source_observations.push(SourceObservation {
                remote_id: item.remote_id.clone(),
                remote_version: item.remote_version.clone(),
                disposition: SourceDisposition::Included,
                filter_reason: None,
                evaluated_plan_revision: plan_revision,
            });
            items.push(item);
        }

        record_operation("sync", started, items.len());
        Ok(GmailSnapshot {
            items,
            source_observations,
            profile: Some(profile),
        })
    }

    async fn list_all(&self, dataset: &GmailDatasetConfig) -> Result<Vec<GmailMessageRef>> {
        let limit = usize::try_from(dataset.max_messages)
            .map_err(|_| response_error("maxMessages does not fit this platform"))?;
        let mut messages = Vec::with_capacity(limit);
        let mut ids = HashSet::new();
        let mut page_tokens = HashSet::new();
        let mut page_token: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let remaining = limit.saturating_sub(messages.len());
            if remaining == 0 {
                return Ok(messages);
            }
            let max_results = u32::try_from(remaining.min(500))
                .map_err(|_| response_error("Gmail page size does not fit u32"))?;
            let page = self
                .client
                .list_messages(&dataset.query, page_token.as_deref(), max_results)
                .await?;
            if page.messages.len() > remaining || page.messages.len() > max_results as usize {
                return Err(response_error(
                    "Gmail list returned more messages than requested",
                ));
            }
            for message in page.messages {
                validate_id("message id", &message.id, MAX_ID_BYTES)?;
                if !message.thread_id.is_empty() {
                    validate_id("thread id", &message.thread_id, MAX_THREAD_ID_BYTES)?;
                }
                if !ids.insert(message.id.clone()) {
                    return Err(response_error("Gmail list returned duplicate message ids"));
                }
                messages.push(message);
            }
            let Some(next_token) = page.next_page_token else {
                return Ok(messages);
            };
            validate_id("page token", &next_token, MAX_PAGE_TOKEN_BYTES)?;
            if !page_tokens.insert(next_token.clone()) {
                return Err(response_error("Gmail list repeated a page token"));
            }
            page_token = Some(next_token);
        }
        Err(response_error("Gmail list exceeded pagination limit"))
    }
}

fn validate_profile(profile: &GmailProfile) -> Result<()> {
    if profile.email_address.len() > MAX_HEADER_BYTES
        || profile.history_id.len() > MAX_HISTORY_ID_BYTES
    {
        return Err(response_error(
            "Gmail profile metadata exceeds connector limit",
        ));
    }
    Ok(())
}

fn message_item(message: GmailApiMessage, include_body: bool) -> Result<ConnectorItemInput> {
    validate_message_metadata(&message)?;
    let occurred_at = message
        .internal_date
        .as_deref()
        .map(parse_internal_date)
        .transpose()?;
    let (body, body_format, attachments) = extract_content(&message.payload, include_body)?;
    let normalized = GmailMessageBody {
        schema_version: SCHEMA_VERSION,
        id: message.id.clone(),
        thread_id: message.thread_id,
        history_id: message.history_id.clone(),
        internal_date: message.internal_date,
        label_ids: message.label_ids,
        snippet: message.snippet,
        headers: message.payload.headers,
        body,
        body_format,
        attachments,
        size_estimate: message.size_estimate,
    };
    let search_text = message_search_text(&normalized);
    let encoded = serde_json::to_vec(&normalized)?;
    Ok(ConnectorItemInput {
        remote_id: message.id,
        kind: "message".to_owned(),
        remote_version: message.history_id,
        occurred_at,
        updated_at: None,
        body_json: serde_json::to_value(normalized)?,
        search_text,
        content_hash: format!("{:x}", Sha256::digest(encoded)),
    })
}

fn validate_message_metadata(message: &GmailApiMessage) -> Result<()> {
    validate_id("message id", &message.id, MAX_ID_BYTES)?;
    validate_id("thread id", &message.thread_id, MAX_THREAD_ID_BYTES)?;
    if message
        .history_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > MAX_HISTORY_ID_BYTES)
    {
        return Err(response_error("invalid Gmail history id"));
    }
    if message.snippet.len() > MAX_SNIPPET_BYTES {
        return Err(response_error("Gmail snippet exceeds connector limit"));
    }
    if message.label_ids.len() > MAX_LABELS
        || message
            .label_ids
            .iter()
            .any(|label| label.is_empty() || label.len() > MAX_LABEL_BYTES)
    {
        return Err(response_error("Gmail labels exceed connector limit"));
    }
    validate_headers(&message.payload.headers)
}

fn validate_headers(headers: &[GmailHeader]) -> Result<()> {
    if headers.len() > MAX_HEADERS {
        return Err(response_error("Gmail header count exceeds connector limit"));
    }
    let total = headers.iter().try_fold(0_usize, |total, header| {
        if header.name.is_empty()
            || header.name.len() > MAX_HEADER_BYTES
            || header.value.len() > MAX_HEADER_BYTES
        {
            return Err(response_error("Gmail header exceeds connector limit"));
        }
        total
            .checked_add(header.name.len())
            .and_then(|value| value.checked_add(header.value.len()))
            .ok_or_else(|| response_error("Gmail headers exceed connector limit"))
    })?;
    if total > MAX_HEADER_TOTAL_BYTES {
        return Err(response_error("Gmail headers exceed connector limit"));
    }
    Ok(())
}

fn validate_id(name: &'static str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(response_error(&format!("invalid Gmail {name}")));
    }
    Ok(())
}

fn parse_internal_date(value: &str) -> Result<String> {
    let milliseconds = value
        .parse::<i64>()
        .map_err(|_| response_error("Gmail internalDate must be milliseconds since epoch"))?;
    OffsetDateTime::UNIX_EPOCH
        .checked_add(Duration::milliseconds(milliseconds))
        .ok_or_else(|| response_error("Gmail internalDate is out of range"))?
        .format(&Rfc3339)
        .map_err(|_| response_error("failed to format Gmail internalDate"))
}

struct MimeScan<'a> {
    plain: Option<&'a str>,
    html: Option<&'a str>,
    attachments: Vec<GmailAttachment>,
    parts: usize,
}

fn extract_content(
    payload: &GmailMessagePart,
    include_body: bool,
) -> Result<(Option<String>, GmailBodyFormat, Vec<GmailAttachment>)> {
    let mut scan = MimeScan {
        plain: None,
        html: None,
        attachments: Vec::new(),
        parts: 0,
    };
    scan_part(payload, 0, &mut scan)?;
    if !include_body {
        return Ok((None, GmailBodyFormat::Unavailable, scan.attachments));
    }
    if let Some(data) = scan.plain {
        return Ok((
            Some(decode_body(data)?),
            GmailBodyFormat::PlainText,
            scan.attachments,
        ));
    }
    if let Some(data) = scan.html {
        let html = decode_body(data)?;
        let text = html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
            .string_from_read(html.as_bytes(), MAX_BODY_BYTES)
            .map_err(|error| {
                GmailConnectorError::MessageBody(format!("failed to convert HTML body: {error}"))
            })?;
        if text.len() > MAX_BODY_BYTES {
            return Err(GmailConnectorError::MessageBody(
                "HTML-derived body exceeds connector limit".to_owned(),
            ));
        }
        return Ok((
            Some(text.trim().to_owned()),
            GmailBodyFormat::HtmlDerived,
            scan.attachments,
        ));
    }
    Ok((None, GmailBodyFormat::Unavailable, scan.attachments))
}

fn scan_part<'a>(part: &'a GmailMessagePart, depth: usize, scan: &mut MimeScan<'a>) -> Result<()> {
    if depth > MAX_MIME_DEPTH {
        return Err(GmailConnectorError::MessageBody(
            "MIME nesting exceeds connector limit".to_owned(),
        ));
    }
    scan.parts = scan
        .parts
        .checked_add(1)
        .ok_or_else(|| GmailConnectorError::MessageBody("too many MIME parts".to_owned()))?;
    if scan.parts > MAX_MIME_PARTS {
        return Err(GmailConnectorError::MessageBody(
            "MIME part count exceeds connector limit".to_owned(),
        ));
    }
    validate_headers(&part.headers)?;
    if !part.filename.is_empty()
        || part.body.attachment_id.is_some()
        || part.mime_type.eq_ignore_ascii_case("message/rfc822")
    {
        if scan.attachments.len() >= MAX_ATTACHMENTS {
            return Err(GmailConnectorError::MessageBody(
                "attachment count exceeds connector limit".to_owned(),
            ));
        }
        if part.filename.len() > MAX_HEADER_BYTES
            || part.mime_type.len() > MAX_HEADER_BYTES
            || part
                .body
                .attachment_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > MAX_ID_BYTES)
        {
            return Err(GmailConnectorError::MessageBody(
                "attachment metadata exceeds connector limit".to_owned(),
            ));
        }
        scan.attachments.push(GmailAttachment {
            filename: part.filename.clone(),
            mime_type: part.mime_type.clone(),
            size: part.body.size,
            attachment_id: part.body.attachment_id.clone(),
        });
        return Ok(());
    } else if let Some(data) = part.body.data.as_deref() {
        match part.mime_type.to_ascii_lowercase().as_str() {
            "text/plain" if scan.plain.is_none() => scan.plain = Some(data),
            "text/html" if scan.html.is_none() => scan.html = Some(data),
            _ => {},
        }
    }
    for child in &part.parts {
        scan_part(child, depth + 1, scan)?;
    }
    Ok(())
}

fn decode_body(data: &str) -> Result<String> {
    if data.len() > MAX_BODY_BYTES.saturating_mul(2) {
        return Err(GmailConnectorError::MessageBody(
            "encoded body exceeds connector limit".to_owned(),
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(data)
        .or_else(|_| URL_SAFE.decode(data))
        .map_err(|_| GmailConnectorError::MessageBody("body is not valid base64url".to_owned()))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(GmailConnectorError::MessageBody(
            "decoded body exceeds connector limit".to_owned(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| GmailConnectorError::MessageBody("body is not valid UTF-8".to_owned()))
}

fn message_search_text(message: &GmailMessageBody) -> String {
    let values = ["Subject", "From", "To", "Cc", "Date"]
        .into_iter()
        .filter_map(|name| message.header(name))
        .chain(std::iter::once(message.snippet.as_str()))
        .chain(message.body.as_deref());
    bounded_text(values)
}

fn bounded_text<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut output = String::new();
    for value in values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if output.len() >= MAX_SEARCH_TEXT_BYTES {
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        let remaining = MAX_SEARCH_TEXT_BYTES.saturating_sub(output.len());
        let mut end = value.len().min(remaining);
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&value[..end]);
    }
    output
}

fn response_error(message: &str) -> GmailConnectorError {
    GmailConnectorError::ServerResponse(message.to_owned())
}

fn record_operation(operation: &'static str, started: Instant, item_count: usize) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!("moltis_connector_gmail_operations_total", "operation" => operation)
            .increment(1);
        metrics::histogram!("moltis_connector_gmail_operation_duration_seconds", "operation" => operation)
            .record(started.elapsed().as_secs_f64());
        metrics::histogram!("moltis_connector_gmail_observed_items", "operation" => operation)
            .record(item_count as f64);
    }
    #[cfg(feature = "tracing")]
    tracing::debug!(
        operation,
        item_count,
        elapsed_ms = started.elapsed().as_millis(),
        "Gmail connector operation completed"
    );
    #[cfg(not(any(feature = "metrics", feature = "tracing")))]
    let _ = (operation, started, item_count);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use {
        super::*,
        crate::{GmailMessagePage, GmailMessagePartBody},
        async_trait::async_trait,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
        std::{
            collections::HashMap,
            sync::{
                Mutex,
                atomic::{AtomicUsize, Ordering},
            },
        },
    };

    #[derive(Default)]
    struct FakeState {
        pages: Vec<GmailMessagePage>,
        messages: HashMap<String, GmailApiMessage>,
        fail_id: Option<String>,
        list_calls: usize,
    }

    struct FakeClient {
        state: Mutex<FakeState>,
    }

    impl FakeClient {
        fn new(pages: Vec<GmailMessagePage>, messages: Vec<GmailApiMessage>) -> Self {
            Self {
                state: Mutex::new(FakeState {
                    pages,
                    messages: messages
                        .into_iter()
                        .map(|message| (message.id.clone(), message))
                        .collect(),
                    ..FakeState::default()
                }),
            }
        }
    }

    #[async_trait]
    impl GmailClient for FakeClient {
        async fn profile(&self) -> Result<GmailProfile> {
            Ok(GmailProfile {
                email_address: "owner@example.com".to_owned(),
                history_id: "10".to_owned(),
                ..GmailProfile::default()
            })
        }

        async fn list_messages(
            &self,
            _query: &str,
            _page_token: Option<&str>,
            _max_results: u32,
        ) -> Result<GmailMessagePage> {
            let mut state = self.state.lock().unwrap();
            let index = state.list_calls;
            state.list_calls += 1;
            state
                .pages
                .get(index)
                .cloned()
                .ok_or_else(|| response_error("unexpected list call"))
        }

        async fn get_message(&self, id: &str) -> Result<GmailApiMessage> {
            let state = self.state.lock().unwrap();
            if state.fail_id.as_deref() == Some(id) {
                return Err(response_error("injected get failure"));
            }
            state
                .messages
                .get(id)
                .cloned()
                .ok_or_else(|| response_error("missing fake message"))
        }
    }

    struct ConcurrencyClient {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        ids: Vec<String>,
    }

    #[async_trait]
    impl GmailClient for ConcurrencyClient {
        async fn profile(&self) -> Result<GmailProfile> {
            Ok(GmailProfile {
                email_address: "owner@example.com".to_owned(),
                history_id: "10".to_owned(),
                ..GmailProfile::default()
            })
        }

        async fn list_messages(
            &self,
            _query: &str,
            _page_token: Option<&str>,
            _max_results: u32,
        ) -> Result<GmailMessagePage> {
            Ok(GmailMessagePage {
                messages: self
                    .ids
                    .iter()
                    .map(|id| GmailMessageRef {
                        id: id.clone(),
                        thread_id: format!("thread-{id}"),
                    })
                    .collect(),
                ..GmailMessagePage::default()
            })
        }

        async fn get_message(&self, id: &str) -> Result<GmailApiMessage> {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(message(id, id))
        }
    }

    fn message(id: &str, text: &str) -> GmailApiMessage {
        GmailApiMessage {
            id: id.to_owned(),
            thread_id: format!("thread-{id}"),
            history_id: Some(format!("history-{id}")),
            internal_date: Some("1700000000000".to_owned()),
            snippet: "snippet".to_owned(),
            payload: GmailMessagePart {
                mime_type: "multipart/alternative".to_owned(),
                headers: vec![GmailHeader {
                    name: "Subject".to_owned(),
                    value: format!("Subject {id}"),
                }],
                parts: vec![GmailMessagePart {
                    mime_type: "multipart/mixed".to_owned(),
                    parts: vec![
                        GmailMessagePart {
                            mime_type: "text/html".to_owned(),
                            body: GmailMessagePartBody {
                                data: Some(URL_SAFE_NO_PAD.encode("<b>html</b>")),
                                ..GmailMessagePartBody::default()
                            },
                            ..GmailMessagePart::default()
                        },
                        GmailMessagePart {
                            mime_type: "text/plain".to_owned(),
                            body: GmailMessagePartBody {
                                data: Some(URL_SAFE_NO_PAD.encode(text)),
                                ..GmailMessagePartBody::default()
                            },
                            ..GmailMessagePart::default()
                        },
                        GmailMessagePart {
                            mime_type: "application/pdf".to_owned(),
                            filename: "doc.pdf".to_owned(),
                            body: GmailMessagePartBody {
                                attachment_id: Some("attachment-1".to_owned()),
                                size: 42,
                                data: None,
                            },
                            ..GmailMessagePart::default()
                        },
                    ],
                    ..GmailMessagePart::default()
                }],
                ..GmailMessagePart::default()
            },
            ..GmailApiMessage::default()
        }
    }

    fn page(ids: &[&str], next: Option<&str>) -> GmailMessagePage {
        GmailMessagePage {
            messages: ids
                .iter()
                .map(|id| GmailMessageRef {
                    id: (*id).to_owned(),
                    thread_id: format!("thread-{id}"),
                })
                .collect(),
            next_page_token: next.map(ToOwned::to_owned),
            result_size_estimate: ids.len() as u64,
        }
    }

    #[tokio::test]
    async fn paginates_and_normalizes_recursive_mime() {
        let client = Arc::new(FakeClient::new(
            vec![page(&["one"], Some("next")), page(&["two"], None)],
            vec![message("one", "plain one"), message("two", "plain two")],
        ));
        let connector = GmailConnector::with_client(client);
        let snapshot = connector
            .sync_dataset(
                &GmailAccountConfig::default(),
                &GmailDatasetConfig::default(),
                SourceStateMap::new(),
                7,
            )
            .await
            .unwrap();

        assert_eq!(snapshot.items.len(), 2);
        assert_eq!(snapshot.items[0].remote_id, "one");
        assert_eq!(
            snapshot.items[0].remote_version.as_deref(),
            Some("history-one")
        );
        assert_eq!(
            snapshot.items[0].occurred_at.as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert!(snapshot.items[0].search_text.contains("plain one"));
        assert_eq!(snapshot.items[0].content_hash.len(), 64);
        let body: GmailMessageBody =
            serde_json::from_value(snapshot.items[0].body_json.clone()).unwrap();
        assert_eq!(body.body.as_deref(), Some("plain one"));
        assert_eq!(body.body_format, GmailBodyFormat::PlainText);
        assert_eq!(body.attachments.len(), 1);
        assert_eq!(snapshot.source_observations[0].evaluated_plan_revision, 7);
    }

    #[tokio::test]
    async fn detects_duplicate_ids_and_repeated_tokens() {
        let duplicate = GmailConnector::with_client(Arc::new(FakeClient::new(
            vec![page(&["one"], Some("next")), page(&["one"], None)],
            vec![message("one", "text")],
        )));
        assert!(
            duplicate
                .sync_dataset(
                    &GmailAccountConfig::default(),
                    &GmailDatasetConfig::default(),
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );

        let repeated = GmailConnector::with_client(Arc::new(FakeClient::new(
            vec![page(&[], Some("same")), page(&[], Some("same"))],
            Vec::new(),
        )));
        assert!(
            repeated
                .sync_dataset(
                    &GmailAccountConfig::default(),
                    &GmailDatasetConfig::default(),
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn any_message_failure_fails_the_snapshot() {
        let client = Arc::new(FakeClient::new(vec![page(&["one", "two"], None)], vec![
            message("one", "one"),
            message("two", "two"),
        ]));
        client.state.lock().unwrap().fail_id = Some("two".to_owned());
        let connector = GmailConnector::with_client(client);
        assert!(
            connector
                .sync_dataset(
                    &GmailAccountConfig::default(),
                    &GmailDatasetConfig::default(),
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn message_fetches_use_bounded_concurrency() {
        let client = Arc::new(ConcurrencyClient {
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            ids: (0..20).map(|id| id.to_string()).collect(),
        });
        let connector = GmailConnector::with_client(client.clone());
        let snapshot = connector
            .sync_dataset(
                &GmailAccountConfig::default(),
                &GmailDatasetConfig::default(),
                SourceStateMap::new(),
                1,
            )
            .await
            .unwrap();

        assert_eq!(snapshot.items.len(), 20);
        let peak = client.peak.load(Ordering::SeqCst);
        assert!(peak > 1);
        assert!(peak <= MESSAGE_FETCH_CONCURRENCY);
    }

    #[test]
    fn html_fallback_is_distinguished_and_malformed_base64_fails() {
        let html = GmailMessagePart {
            mime_type: "text/html".to_owned(),
            body: GmailMessagePartBody {
                data: Some(URL_SAFE_NO_PAD.encode("<p>Hello &amp; world</p>")),
                ..GmailMessagePartBody::default()
            },
            ..GmailMessagePart::default()
        };
        let (body, format, _) = extract_content(&html, true).unwrap();
        assert_eq!(body.as_deref(), Some("Hello & world"));
        assert_eq!(format, GmailBodyFormat::HtmlDerived);

        let invalid = GmailMessagePart {
            mime_type: "text/plain".to_owned(),
            body: GmailMessagePartBody {
                data: Some("%%%".to_owned()),
                ..GmailMessagePartBody::default()
            },
            ..GmailMessagePart::default()
        };
        assert!(extract_content(&invalid, true).is_err());
    }

    #[test]
    fn attached_message_children_never_become_the_stored_body() {
        let payload = GmailMessagePart {
            mime_type: "multipart/mixed".to_owned(),
            parts: vec![GmailMessagePart {
                mime_type: "message/rfc822".to_owned(),
                filename: String::new(),
                body: GmailMessagePartBody {
                    attachment_id: None,
                    size: 42,
                    data: None,
                },
                parts: vec![GmailMessagePart {
                    mime_type: "text/plain".to_owned(),
                    body: GmailMessagePartBody {
                        data: Some(URL_SAFE_NO_PAD.encode("attached secret")),
                        ..GmailMessagePartBody::default()
                    },
                    ..GmailMessagePart::default()
                }],
                ..GmailMessagePart::default()
            }],
            ..GmailMessagePart::default()
        };

        let (body, format, attachments) = extract_content(&payload, true).unwrap();
        assert_eq!(body, None);
        assert_eq!(format, GmailBodyFormat::Unavailable);
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn hash_is_stable_and_changes_with_content() {
        let first = message_item(message("one", "same"), true).unwrap();
        let second = message_item(message("one", "same"), true).unwrap();
        let changed = message_item(message("one", "changed"), true).unwrap();
        assert_eq!(first.content_hash, second.content_hash);
        assert_ne!(first.content_hash, changed.content_hash);
    }
}
