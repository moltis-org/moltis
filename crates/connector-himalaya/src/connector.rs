use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use {
    moltis_connectors::{ConnectorItemInput, SourceDisposition, SourceObservation, SourceStateMap},
    serde::{Deserialize, de::DeserializeOwned},
    sha2::{Digest, Sha256},
};

use crate::{
    HimalayaAccount, HimalayaAccountConfig, HimalayaBackend, HimalayaConnectorError,
    HimalayaDatasetConfig, HimalayaEnvelope, HimalayaInvocation, HimalayaMailbox,
    HimalayaMessageBody, HimalayaRunner, HimalayaSnapshot, MimeStatus, ProcessHimalayaRunner,
    Result, mime, validate_v2_version,
};

const PAGE_SIZE: u32 = 100;
const MAX_AGGREGATE_RAW_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEARCH_TEXT_BYTES: usize = 16 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;

struct MessageCandidate {
    remote_id: String,
    remote_version: String,
    envelope: HimalayaEnvelope,
    mailboxes: Vec<String>,
}

pub struct HimalayaConnector {
    runner: Arc<dyn HimalayaRunner>,
}

impl Default for HimalayaConnector {
    fn default() -> Self {
        Self::new(Arc::new(ProcessHimalayaRunner::default()))
    }
}

impl HimalayaConnector {
    #[must_use]
    pub fn new(runner: Arc<dyn HimalayaRunner>) -> Self {
        Self { runner }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn discover_accounts(&self) -> Result<Vec<HimalayaAccount>> {
        let started = Instant::now();
        self.check_version().await?;
        let output = self.runner.run(&HimalayaInvocation::AccountList).await?;
        let accounts = parse_accounts(&output.stdout, output.stdout_truncated)?;
        validate_accounts(&accounts)?;
        record_operation("discover_accounts", started, accounts.len());
        Ok(accounts)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn discover_mailboxes(
        &self,
        config: &HimalayaAccountConfig,
    ) -> Result<Vec<HimalayaMailbox>> {
        config.validate()?;
        self.check_version().await?;
        self.list_mailboxes(config).await
    }

    pub async fn test_account(
        &self,
        config: &HimalayaAccountConfig,
    ) -> Result<Vec<HimalayaMailbox>> {
        self.discover_mailboxes(config).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_dataset(
        &self,
        account: &HimalayaAccountConfig,
        dataset: &HimalayaDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
    ) -> Result<HimalayaSnapshot> {
        let started = Instant::now();
        account.validate()?;
        dataset.validate(account.backend)?;
        if plan_revision == 0 {
            return Err(HimalayaConnectorError::DatasetConfig(
                "plan revision must be positive",
            ));
        }
        self.check_version().await?;
        let discovered = self.list_mailboxes(account).await?;
        let mailboxes = select_mailboxes(discovered, &dataset.mailbox_ids)?;
        let mut candidates: Vec<MessageCandidate> = Vec::new();
        let mut candidate_indices: HashMap<String, usize> = HashMap::new();
        let mut remaining = usize::try_from(dataset.max_messages).unwrap_or(usize::MAX);

        for mailbox in &mailboxes {
            if remaining == 0 {
                break;
            }
            let uid_validity = if account.backend == HimalayaBackend::Imap {
                Some(self.imap_uid_validity(account, &mailbox.id).await?)
            } else {
                None
            };
            let envelopes = self
                .list_envelopes(
                    account,
                    dataset,
                    &mailbox.id,
                    usize::try_from(dataset.max_messages).unwrap_or(usize::MAX),
                )
                .await?;
            for envelope in envelopes {
                let remote_id = remote_id(account.backend, &mailbox.id, uid_validity, &envelope)?;
                let remote_version = envelope_version(&envelope)?;
                if let Some(index) = candidate_indices.get(&remote_id).copied() {
                    let candidate = &mut candidates[index];
                    if candidate.mailboxes.iter().any(|id| id == &mailbox.id) {
                        return Err(HimalayaConnectorError::Response(
                            "envelope traversal returned a duplicate message identity".to_owned(),
                        ));
                    }
                    if candidate.remote_version != remote_version {
                        return Err(HimalayaConnectorError::Response(
                            "the same message had conflicting envelope metadata".to_owned(),
                        ));
                    }
                    candidate.mailboxes.push(mailbox.id.clone());
                    continue;
                }
                candidate_indices.insert(remote_id.clone(), candidates.len());
                candidates.push(MessageCandidate {
                    remote_id,
                    remote_version,
                    envelope,
                    mailboxes: vec![mailbox.id.clone()],
                });
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    break;
                }
            }
            if let Some(before) = uid_validity {
                let after = self.imap_uid_validity(account, &mailbox.id).await?;
                if before != after {
                    return Err(HimalayaConnectorError::Response(
                        "IMAP UIDVALIDITY changed during traversal".to_owned(),
                    ));
                }
            }
        }

        let mut items = Vec::new();
        let mut observations = Vec::with_capacity(candidates.len());
        let mut aggregate_raw_bytes = 0_usize;
        for candidate in candidates {
            let unchanged = existing.get(&candidate.remote_id).is_some_and(|state| {
                state.remote_version.as_deref() == Some(candidate.remote_version.as_str())
                    && state.evaluated_plan_revision == plan_revision
            });
            observations.push(SourceObservation {
                remote_id: candidate.remote_id.clone(),
                remote_version: Some(candidate.remote_version.clone()),
                disposition: SourceDisposition::Included,
                filter_reason: None,
                evaluated_plan_revision: plan_revision,
            });
            if unchanged {
                continue;
            }
            let body = self
                .message_body(
                    account,
                    dataset.include_bodies,
                    candidate.mailboxes,
                    candidate.envelope,
                    &mut aggregate_raw_bytes,
                )
                .await?;
            items.push(message_item(
                candidate.remote_id,
                candidate.remote_version,
                body,
            )?);
        }
        record_operation("sync", started, observations.len());
        Ok(HimalayaSnapshot {
            items,
            source_observations: observations,
            mailboxes,
        })
    }

    async fn check_version(&self) -> Result<()> {
        let output = self.runner.run(&HimalayaInvocation::Version).await?;
        if output.stdout_truncated {
            return Err(HimalayaConnectorError::OutputLimit);
        }
        validate_v2_version(&output.stdout)
    }

    async fn list_mailboxes(&self, config: &HimalayaAccountConfig) -> Result<Vec<HimalayaMailbox>> {
        let output = self
            .runner
            .run(&HimalayaInvocation::MailboxList {
                account: config.account_name.clone(),
                backend: config.backend,
            })
            .await?;
        let mailboxes = parse_mailboxes(&output.stdout, output.stdout_truncated)?;
        validate_mailboxes(&mailboxes)?;
        Ok(mailboxes)
    }

    async fn imap_uid_validity(
        &self,
        account: &HimalayaAccountConfig,
        mailbox: &str,
    ) -> Result<u32> {
        let output = self
            .runner
            .run(&HimalayaInvocation::ImapStatus {
                account: account.account_name.clone(),
                mailbox: mailbox.to_owned(),
            })
            .await?;
        parse_imap_status(&output.stdout, output.stdout_truncated)
    }

    async fn list_envelopes(
        &self,
        account: &HimalayaAccountConfig,
        dataset: &HimalayaDatasetConfig,
        mailbox: &str,
        limit: usize,
    ) -> Result<Vec<HimalayaEnvelope>> {
        let mut result = Vec::new();
        let mut page = 1_u32;
        let mut page_hashes = HashSet::new();
        let mut metadata_bytes = 0_usize;
        while result.len() < limit {
            let requested = (limit - result.len()).min(PAGE_SIZE as usize);
            let page_size = u32::try_from(requested).unwrap_or(PAGE_SIZE);
            let output = self
                .runner
                .run(&HimalayaInvocation::EnvelopePage {
                    account: account.account_name.clone(),
                    backend: account.backend,
                    mailbox: mailbox.to_owned(),
                    query: dataset.query.clone().filter(|query| !query.is_empty()),
                    page,
                    page_size,
                })
                .await?;
            if output.stdout_truncated {
                return Err(HimalayaConnectorError::OutputLimit);
            }
            metadata_bytes = metadata_bytes
                .checked_add(output.stdout.len())
                .ok_or_else(metadata_limit_error)?;
            if metadata_bytes > MAX_METADATA_BYTES {
                return Err(metadata_limit_error());
            }
            let envelopes = parse_envelopes(&output.stdout, false)?;
            validate_envelopes(&envelopes)?;
            if envelopes.len() > requested {
                return Err(HimalayaConnectorError::Response(
                    "envelope page exceeded its requested bound".to_owned(),
                ));
            }
            let page_hash = Sha256::digest(serde_json::to_vec(&envelopes)?);
            if !page_hashes.insert(page_hash.to_vec()) {
                return Err(HimalayaConnectorError::Response(
                    "envelope traversal repeated a page".to_owned(),
                ));
            }
            let count = envelopes.len();
            result.extend(envelopes);
            if count < requested || count == 0 {
                break;
            }
            page = page.checked_add(1).ok_or_else(|| {
                HimalayaConnectorError::Response("envelope page number overflowed".to_owned())
            })?;
        }
        Ok(result)
    }

    async fn message_body(
        &self,
        account: &HimalayaAccountConfig,
        include_bodies: bool,
        mailboxes: Vec<String>,
        envelope: HimalayaEnvelope,
        aggregate_raw_bytes: &mut usize,
    ) -> Result<HimalayaMessageBody> {
        let mailbox = mailboxes.first().ok_or_else(|| {
            HimalayaConnectorError::Response("message candidate omitted its mailbox".to_owned())
        })?;
        if !include_bodies {
            return Ok(mime::envelope_only(
                account.backend,
                mailboxes,
                envelope,
                MimeStatus::NotRequested,
                0,
                None,
            ));
        }
        let output = self
            .runner
            .run(&HimalayaInvocation::MessageReadRaw {
                account: account.account_name.clone(),
                backend: account.backend,
                mailbox: mailbox.clone(),
                id: envelope.id.clone(),
            })
            .await?;
        let observed_size = output
            .stdout
            .len()
            .saturating_add(usize::from(output.stdout_truncated));
        *aggregate_raw_bytes = aggregate_raw_bytes
            .checked_add(observed_size)
            .ok_or_else(raw_limit_error)?;
        if *aggregate_raw_bytes > MAX_AGGREGATE_RAW_BYTES {
            return Err(raw_limit_error());
        }
        if output.stdout_truncated {
            return Ok(mime::envelope_only(
                account.backend,
                mailboxes,
                envelope,
                MimeStatus::TooLarge,
                observed_size,
                None,
            ));
        }
        Ok(mime::parse_message(
            account.backend,
            mailboxes,
            envelope,
            &output.stdout,
        ))
    }
}

fn metadata_limit_error() -> HimalayaConnectorError {
    HimalayaConnectorError::Response("envelope metadata exceeded its safe limit".to_owned())
}

fn raw_limit_error() -> HimalayaConnectorError {
    HimalayaConnectorError::Response("aggregate raw MIME exceeded its safe limit".to_owned())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AccountResponse {
    List(Vec<HimalayaAccount>),
    Wrapped { accounts: Vec<HimalayaAccount> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MailboxEntry {
    Object(HimalayaMailbox),
    Id(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MailboxResponse {
    List(Vec<MailboxEntry>),
    Wrapped { mailboxes: Vec<MailboxEntry> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EnvelopeResponse {
    List(Vec<HimalayaEnvelope>),
    Wrapped { envelopes: Vec<HimalayaEnvelope> },
}

#[derive(Deserialize)]
struct ImapStatus {
    #[serde(
        rename = "uid-validity",
        alias = "uid_validity",
        alias = "uidValidity",
        alias = "uidvalidity"
    )]
    uid_validity: u64,
}

fn parse_json<T: DeserializeOwned>(bytes: &[u8], truncated: bool) -> Result<T> {
    if truncated {
        return Err(HimalayaConnectorError::OutputLimit);
    }
    serde_json::from_slice(bytes).map_err(|error| {
        HimalayaConnectorError::Response(format!("command returned invalid JSON: {error}"))
    })
}

fn parse_accounts(bytes: &[u8], truncated: bool) -> Result<Vec<HimalayaAccount>> {
    let accounts = match parse_json(bytes, truncated)? {
        AccountResponse::List(accounts) | AccountResponse::Wrapped { accounts } => accounts,
    };
    Ok(accounts
        .into_iter()
        .filter(|account| !account.backends.is_empty())
        .collect())
}

fn parse_mailboxes(bytes: &[u8], truncated: bool) -> Result<Vec<HimalayaMailbox>> {
    let entries = match parse_json(bytes, truncated)? {
        MailboxResponse::List(entries) | MailboxResponse::Wrapped { mailboxes: entries } => entries,
    };
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            MailboxEntry::Object(mailbox) => mailbox,
            MailboxEntry::Id(id) => HimalayaMailbox {
                id,
                display_name: None,
            },
        })
        .collect())
}

fn parse_envelopes(bytes: &[u8], truncated: bool) -> Result<Vec<HimalayaEnvelope>> {
    Ok(match parse_json(bytes, truncated)? {
        EnvelopeResponse::List(envelopes) | EnvelopeResponse::Wrapped { envelopes } => envelopes,
    })
}

fn parse_imap_status(bytes: &[u8], truncated: bool) -> Result<u32> {
    let status: ImapStatus = parse_json(bytes, truncated)?;
    let uid_validity = u32::try_from(status.uid_validity).map_err(|_| {
        HimalayaConnectorError::Response("IMAP UIDVALIDITY is out of range".to_owned())
    })?;
    if uid_validity == 0 {
        return Err(HimalayaConnectorError::Response(
            "IMAP UIDVALIDITY must be positive".to_owned(),
        ));
    }
    Ok(uid_validity)
}

fn validate_accounts(accounts: &[HimalayaAccount]) -> Result<()> {
    let mut names = HashSet::new();
    if accounts.iter().any(|account| {
        account.name.trim().is_empty()
            || account.name.len() > 256
            || account.backends.is_empty()
            || !names.insert(account.name.as_str())
    }) {
        return Err(HimalayaConnectorError::Response(
            "account list contains invalid or duplicate names".to_owned(),
        ));
    }
    Ok(())
}

fn validate_mailboxes(mailboxes: &[HimalayaMailbox]) -> Result<()> {
    let mut ids = HashSet::new();
    if mailboxes.len() > 10_000
        || mailboxes.iter().any(|mailbox| {
            mailbox.id.trim().is_empty()
                || mailbox.id.len() > 1024
                || !ids.insert(mailbox.id.as_str())
        })
    {
        return Err(HimalayaConnectorError::Response(
            "mailbox list contains invalid or duplicate IDs".to_owned(),
        ));
    }
    Ok(())
}

fn validate_envelopes(envelopes: &[HimalayaEnvelope]) -> Result<()> {
    if envelopes.iter().any(|envelope| {
        envelope.id.is_empty()
            || envelope.id.len() > 1024
            || envelope.id.chars().any(char::is_control)
            || envelope.uid == Some(0)
            || envelope.flags.len() > 256
            || envelope
                .flags
                .iter()
                .any(|flag| flag.raw.is_empty() || flag.raw.len() > 1024)
            || envelope.from.len() > 256
            || envelope.to.len() > 256
    }) {
        return Err(HimalayaConnectorError::Response(
            "envelope list contains an invalid ID or field range".to_owned(),
        ));
    }
    Ok(())
}

fn select_mailboxes(
    discovered: Vec<HimalayaMailbox>,
    selected: &[String],
) -> Result<Vec<HimalayaMailbox>> {
    let mut by_id = discovered
        .into_iter()
        .map(|mailbox| (mailbox.id.clone(), mailbox))
        .collect::<HashMap<_, _>>();
    selected
        .iter()
        .map(|id| {
            by_id
                .remove(id)
                .ok_or(HimalayaConnectorError::DatasetConfig(
                    "a selected mailbox was not found",
                ))
        })
        .collect()
}

fn remote_id(
    backend: HimalayaBackend,
    mailbox: &str,
    uid_validity: Option<u32>,
    envelope: &HimalayaEnvelope,
) -> Result<String> {
    match backend {
        HimalayaBackend::Imap => {
            let validity = uid_validity.ok_or_else(|| {
                HimalayaConnectorError::Response("IMAP UIDVALIDITY was omitted".to_owned())
            })?;
            let uid = envelope
                .uid
                .or_else(|| envelope.id.parse::<u32>().ok())
                .ok_or_else(|| {
                    HimalayaConnectorError::Response("IMAP envelope omitted a valid UID".to_owned())
                })?;
            if uid == 0 {
                return Err(HimalayaConnectorError::Response(
                    "IMAP UID must be positive".to_owned(),
                ));
            }
            serde_json::to_string(&("himalaya-message-v1", backend, mailbox, validity, uid))
                .map_err(Into::into)
        },
        HimalayaBackend::Jmap | HimalayaBackend::Gmail | HimalayaBackend::Msgraph => {
            serde_json::to_string(&("himalaya-message-v1", backend, &envelope.id))
                .map_err(Into::into)
        },
        HimalayaBackend::Maildir | HimalayaBackend::M2dir => {
            serde_json::to_string(&("himalaya-message-v1", backend, mailbox, &envelope.id))
                .map_err(Into::into)
        },
    }
}

fn envelope_version(envelope: &HimalayaEnvelope) -> Result<String> {
    let bytes = serde_json::to_vec(&("himalaya-envelope-v1", envelope))?;
    Ok(format!("v1:{:x}", Sha256::digest(bytes)))
}

fn message_item(
    remote_id: String,
    remote_version: String,
    body: HimalayaMessageBody,
) -> Result<ConnectorItemInput> {
    let search_text = message_search_text(&body);
    let bytes = serde_json::to_vec(&body)?;
    Ok(ConnectorItemInput {
        remote_id,
        kind: "message".to_owned(),
        remote_version: Some(remote_version),
        occurred_at: body.envelope.date.clone(),
        updated_at: None,
        body_json: serde_json::to_value(body)?,
        search_text,
        content_hash: format!("{:x}", Sha256::digest(bytes)),
    })
}

fn message_search_text(body: &HimalayaMessageBody) -> String {
    let mut text = String::new();
    let values = body
        .envelope
        .subject
        .iter()
        .chain(body.envelope.message_id.iter())
        .chain(body.envelope.from.iter().map(|from| &from.addr))
        .chain(body.envelope.to.iter().map(|to| &to.addr))
        .chain(body.plain_text_bodies.iter());
    for value in values {
        if !text.is_empty() && text.len() < MAX_SEARCH_TEXT_BYTES {
            text.push('\n');
        }
        let remaining = MAX_SEARCH_TEXT_BYTES.saturating_sub(text.len());
        if remaining == 0 {
            break;
        }
        let mut end = value.len().min(remaining);
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        text.push_str(&value[..end]);
    }
    text
}

fn record_operation(operation: &'static str, started: Instant, item_count: usize) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!("moltis_connector_himalaya_operations_total", "operation" => operation)
            .increment(1);
        metrics::histogram!("moltis_connector_himalaya_operation_duration_seconds", "operation" => operation)
            .record(started.elapsed().as_secs_f64());
    }
    #[cfg(feature = "tracing")]
    tracing::debug!(
        operation,
        item_count,
        elapsed_ms = started.elapsed().as_millis(),
        "Himalaya connector operation completed"
    );
    #[cfg(not(any(feature = "metrics", feature = "tracing")))]
    let _ = (operation, started, item_count);
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use async_trait::async_trait;

    use {super::*, crate::HimalayaOutput};

    struct FakeRunner {
        calls: Mutex<Vec<HimalayaInvocation>>,
        responses: Mutex<VecDeque<Result<HimalayaOutput>>>,
    }

    impl FakeRunner {
        fn new(responses: impl IntoIterator<Item = Result<HimalayaOutput>>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }

        fn calls(&self) -> Vec<HimalayaInvocation> {
            self.calls
                .lock()
                .map_or_else(|error| error.into_inner().clone(), |calls| calls.clone())
        }
    }

    #[async_trait]
    impl HimalayaRunner for FakeRunner {
        async fn run(&self, invocation: &HimalayaInvocation) -> Result<HimalayaOutput> {
            self.calls
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(invocation.clone());
            self.responses
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front()
                .unwrap_or_else(|| {
                    Err(HimalayaConnectorError::Response(
                        "fake response queue exhausted".to_owned(),
                    ))
                })
        }
    }

    fn output(value: impl Into<Vec<u8>>) -> Result<HimalayaOutput> {
        Ok(HimalayaOutput::complete(value))
    }

    fn account(backend: HimalayaBackend) -> HimalayaAccountConfig {
        HimalayaAccountConfig {
            schema_version: 1,
            account_name: "work".to_owned(),
            backend,
        }
    }

    fn dataset(max_messages: u32) -> HimalayaDatasetConfig {
        HimalayaDatasetConfig {
            schema_version: 1,
            mailbox_ids: vec!["INBOX".to_owned()],
            query: None,
            max_messages,
            include_bodies: true,
        }
    }

    #[tokio::test]
    async fn parses_discovery_contracts_and_rejects_old_version() {
        let runner = Arc::new(FakeRunner::new([
            output("himalaya 2.0.1"),
            output(r#"{"accounts":[{"name":"work","default":true,"backends":["imap","smtp"]}]}"#),
        ]));
        let connector = HimalayaConnector::new(runner);
        assert_eq!(
            connector
                .discover_accounts()
                .await
                .unwrap_or_default()
                .len(),
            1
        );

        let old = Arc::new(FakeRunner::new([output("himalaya 1.9.0")]));
        assert!(
            HimalayaConnector::new(old)
                .discover_accounts()
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn paginates_with_bounds_and_builds_imap_stable_ids() -> Result<()> {
        let first = (1..=100)
            .map(|uid| serde_json::json!({"id": uid, "subject": "s"}))
            .collect::<Vec<_>>();
        let second = serde_json::json!([{"id":101,"uid":101,"message-id":"meta"}]);
        let mut responses = vec![
            output("himalaya 2.2.0"),
            output(r#"["INBOX"]"#),
            output(r#"{"uidValidity":77}"#),
            output(serde_json::to_vec(&first)?),
            output(serde_json::to_vec(&second)?),
            output(r#"{"uidValidity":77}"#),
        ];
        responses.extend((0..101).map(|_| output(b"Subject: x\r\n\r\nbody".to_vec())));
        let runner = Arc::new(FakeRunner::new(responses));
        let connector = HimalayaConnector::new(runner.clone());
        let snapshot = connector
            .sync_dataset(
                &account(HimalayaBackend::Imap),
                &dataset(101),
                SourceStateMap::new(),
                1,
            )
            .await?;
        assert_eq!(snapshot.items.len(), 101);
        assert_eq!(
            snapshot.items[0].remote_id,
            r#"["himalaya-message-v1","imap","INBOX",77,1]"#
        );
        let pages = runner
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                HimalayaInvocation::EnvelopePage {
                    page, page_size, ..
                } => Some((page, page_size)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(pages, vec![(1, 100), (2, 1)]);
        Ok(())
    }

    #[tokio::test]
    async fn unchanged_version_skips_raw_mime() -> Result<()> {
        let envelope: HimalayaEnvelope = serde_json::from_value(serde_json::json!({
            "id": "abc",
            "subject": "same"
        }))?;
        let version = envelope_version(&envelope)?;
        let remote_id = remote_id(HimalayaBackend::Jmap, "INBOX", None, &envelope)?;
        let state = moltis_connectors::SourceState {
            dataset_id: "d".to_owned(),
            remote_id: remote_id.clone(),
            remote_version: Some(version),
            disposition: SourceDisposition::Included,
            filter_reason: None,
            evaluated_plan_revision: 9,
            last_seen_run_id: "old".to_owned(),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        let runner = Arc::new(FakeRunner::new([
            output("himalaya 2.0.0"),
            output(r#"["INBOX"]"#),
            output(r#"[{"id":"abc","subject":"same"}]"#),
        ]));
        let snapshot = HimalayaConnector::new(runner.clone())
            .sync_dataset(
                &account(HimalayaBackend::Jmap),
                &dataset(1),
                [(remote_id, state)].into_iter().collect(),
                9,
            )
            .await?;
        assert!(snapshot.items.is_empty());
        assert!(
            !runner
                .calls()
                .iter()
                .any(|call| { matches!(call, HimalayaInvocation::MessageReadRaw { .. }) })
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeated_page_and_command_failure_abort_snapshot() {
        let page = serde_json::to_vec(
            &(1..=100)
                .map(|id| serde_json::json!({"id":id}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let repeated = Arc::new(FakeRunner::new([
            output("himalaya 2.0.0"),
            output(r#"["INBOX"]"#),
            output(page.clone()),
            output(page),
        ]));
        assert!(
            HimalayaConnector::new(repeated)
                .sync_dataset(
                    &account(HimalayaBackend::Jmap),
                    &dataset(101),
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );

        let failed = Arc::new(FakeRunner::new([
            output("himalaya 2.0.0"),
            output(r#"["INBOX"]"#),
            output(r#"[{"id":"1"}]"#),
            Err(HimalayaConnectorError::CommandFailed),
        ]));
        assert!(
            HimalayaConnector::new(failed)
                .sync_dataset(
                    &account(HimalayaBackend::Maildir),
                    &dataset(1),
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn uidvalidity_change_aborts_the_complete_snapshot() {
        let mut config = dataset(1);
        config.include_bodies = false;
        let runner = Arc::new(FakeRunner::new([
            output("himalaya 2.0.0"),
            output(r#"["INBOX"]"#),
            output(r#"{"uidvalidity":7}"#),
            output(r#"[{"id":"9"}]"#),
            output(r#"{"uidvalidity":8}"#),
        ]));
        assert!(
            HimalayaConnector::new(runner)
                .sync_dataset(
                    &account(HimalayaBackend::Imap),
                    &config,
                    SourceStateMap::new(),
                    1,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn global_backend_ids_merge_selected_mailboxes() -> Result<()> {
        let mut config = dataset(2);
        config.mailbox_ids = vec!["inbox".to_owned(), "all".to_owned()];
        let first = r#"[{"id":"global-id","subject":"same"}]"#;
        let second = r#"[
            {"id":"global-id","subject":"same"},
            {"id":"second-id","subject":"second"}
        ]"#;
        let runner = Arc::new(FakeRunner::new([
            output("himalaya 2.0.0"),
            output(r#"[{"id":"inbox","name":"Inbox"},{"id":"all","name":"All"}]"#),
            output(first),
            output(second),
            output(b"Subject: same\r\n\r\nbody".to_vec()),
            output(b"Subject: second\r\n\r\nbody".to_vec()),
        ]));
        let snapshot = HimalayaConnector::new(runner.clone())
            .sync_dataset(
                &account(HimalayaBackend::Jmap),
                &config,
                SourceStateMap::new(),
                1,
            )
            .await?;

        assert_eq!(snapshot.items.len(), 2);
        assert_eq!(snapshot.source_observations.len(), 2);
        assert_eq!(
            snapshot.items[0].body_json["mailboxes"],
            serde_json::json!(["inbox", "all"])
        );
        assert_eq!(
            runner
                .calls()
                .into_iter()
                .filter(|call| matches!(call, HimalayaInvocation::MessageReadRaw { .. }))
                .count(),
            2
        );
        Ok(())
    }
}
