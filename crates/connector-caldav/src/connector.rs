use std::{
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

use {
    moltis_caldav::{
        client::{CalDavClient, CalDavConnectionOptions, LibDavCalDavClient},
        ical::{EventComponentIndex, IndexedProperty, index_event_components},
        types::{CalendarInfo, CalendarObject, CalendarObjectResult, CalendarResourceMetadata},
    },
    moltis_connectors::{ConnectorItemInput, SourceDisposition, SourceObservation, SourceStateMap},
    sha2::{Digest, Sha256},
    time::{Date, Month},
};

use crate::{
    CalDavAccountConfig, CalDavConnectorError, CalDavDatasetConfig, CalDavFilters, CalDavSnapshot,
    CalendarBody, CalendarSelection, EventBody, ParseStatus, Result,
};

const MULTIGET_BATCH_SIZE: usize = 100;
const MAX_CALENDARS: usize = 1_000;
const MAX_RESOURCES_PER_SYNC: usize = 50_000;
const MAX_CALENDAR_METADATA_BYTES: usize = 64 * 1024;
const MAX_RESOURCE_METADATA_BYTES: usize = 64 * 1024;
const MAX_SYNC_METADATA_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SYNC_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEARCH_TEXT_BYTES: usize = 16 * 1024;
const MAX_USER_ADDRESSES: usize = 256;
const MAX_USER_ADDRESS_BYTES: usize = 4 * 1024;
const MAX_USER_ADDRESS_METADATA_BYTES: usize = 64 * 1024;
const SCHEMA_VERSION: u32 = 1;
const FILTER_REASON_UNPARSED: &str = "unparsed_ical";
const FILTER_REASON_DATE_RANGE: &str = "outside_date_range";
const FILTER_REASON_ACCEPTANCE: &str = "not_accepted_by_account";
const FILTER_REASON_NO_COMPONENT: &str = "no_matching_component";

#[derive(Debug, Clone, Copy, Default)]
pub struct CalDavConnector;

impl CalDavConnector {
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn test_connection(&self, config: &CalDavAccountConfig) -> Result<Vec<CalendarInfo>> {
        let started = Instant::now();
        let client = connect(config).await?;
        let calendars = self.test_connection_with_client(&client).await?;
        record_operation("test_connection", started, calendars.len());
        Ok(calendars)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn test_connection_with_client(
        &self,
        client: &dyn CalDavClient,
    ) -> Result<Vec<CalendarInfo>> {
        let calendars = client.list_calendars().await?;
        if calendars.len() > MAX_CALENDARS {
            return Err(limit_error("calendar count exceeds connector limit"));
        }
        for calendar in &calendars {
            if serde_json::to_vec(calendar)?.len() > MAX_CALENDAR_METADATA_BYTES {
                return Err(limit_error("calendar metadata exceeds connector limit"));
            }
        }
        ensure_unique(
            calendars.iter().map(|calendar| calendar.href.as_str()),
            "calendar list contains duplicate hrefs",
        )?;
        Ok(calendars)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_dataset(
        &self,
        config: &CalDavAccountConfig,
        dataset_config: &CalDavDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
    ) -> Result<CalDavSnapshot> {
        let started = Instant::now();
        let client = connect(config).await?;
        let snapshot = self
            .sync_with_client(
                &client,
                &config.username,
                dataset_config,
                existing,
                plan_revision,
            )
            .await?;
        record_operation("sync", started, snapshot.source_observations.len());
        Ok(snapshot)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn sync_with_client(
        &self,
        client: &dyn CalDavClient,
        account_username: &str,
        dataset_config: &CalDavDatasetConfig,
        existing: SourceStateMap,
        plan_revision: u64,
    ) -> Result<CalDavSnapshot> {
        dataset_config.validate()?;
        let user_addresses = if dataset_config.filters.accepted_by_account {
            client.list_user_addresses().await?
        } else {
            Vec::new()
        };
        let filter_plan =
            EventFilterPlan::new(&dataset_config.filters, account_username, user_addresses)?;
        let discovered = self.test_connection_with_client(client).await?;
        let calendars = select_calendars(discovered, &dataset_config.selection)?;

        let mut items = Vec::new();
        let mut source_observations = Vec::new();
        let mut resource_count: usize = 0;
        let mut resource_metadata_bytes: usize = 0;
        let mut body_bytes: usize = 0;
        for calendar in &calendars {
            let calendar_remote_id = calendar_remote_id(&calendar.href)?;
            items.push(calendar_item(calendar_remote_id.clone(), calendar)?);
            source_observations.push(included_observation(
                calendar_remote_id,
                calendar.collection_etag.clone(),
                plan_revision,
            ));

            let resources = client.list_event_resources(&calendar.href).await?;
            resource_count = resource_count
                .checked_add(resources.len())
                .ok_or_else(|| limit_error("calendar resource count exceeds connector limit"))?;
            if resource_count > MAX_RESOURCES_PER_SYNC {
                return Err(limit_error(
                    "calendar resource count exceeds connector limit",
                ));
            }
            for resource in &resources {
                let bytes = resource
                    .href
                    .len()
                    .checked_add(resource.etag.as_ref().map_or(0, String::len))
                    .ok_or_else(|| {
                        limit_error("calendar resource metadata exceeds connector limit")
                    })?;
                if bytes > MAX_RESOURCE_METADATA_BYTES {
                    return Err(limit_error(
                        "calendar resource metadata exceeds connector limit",
                    ));
                }
                resource_metadata_bytes = resource_metadata_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| limit_error("calendar sync metadata exceeds connector limit"))?;
                if resource_metadata_bytes > MAX_SYNC_METADATA_BYTES {
                    return Err(limit_error(
                        "calendar sync metadata exceeds connector limit",
                    ));
                }
            }
            ensure_unique(
                resources.iter().map(|resource| resource.href.as_str()),
                "calendar resource list contains duplicate hrefs",
            )?;
            sync_calendar_resources(
                client,
                calendar,
                resources,
                &existing,
                &filter_plan,
                plan_revision,
                &mut items,
                &mut source_observations,
                &mut resource_metadata_bytes,
                &mut body_bytes,
            )
            .await?;
        }

        Ok(CalDavSnapshot {
            items,
            source_observations,
            calendars,
        })
    }
}

async fn connect(config: &CalDavAccountConfig) -> Result<LibDavCalDavClient> {
    let (_, addresses) = config.resolve_connection_target().await?;
    let options = CalDavConnectionOptions {
        allow_insecure_http: config.allow_insecure_http,
        request_timeout: Duration::from_secs(config.timeout_seconds),
    };
    LibDavCalDavClient::connect_direct_to_addresses_with_options(
        &config.server_url,
        &config.username,
        &config.password,
        options,
        addresses,
    )
    .await
    .map_err(Into::into)
}

fn select_calendars(
    calendars: Vec<CalendarInfo>,
    selection: &CalendarSelection,
) -> Result<Vec<CalendarInfo>> {
    let CalendarSelection::Selected { calendar_hrefs } = selection else {
        return Ok(calendars);
    };
    let selected = calendar_hrefs
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let result = calendars
        .into_iter()
        .filter(|calendar| selected.contains(calendar.href.as_str()))
        .collect::<Vec<_>>();
    if result.len() != selected.len() {
        return Err(CalDavConnectorError::DatasetConfig(
            "one or more selected calendars were not found",
        ));
    }
    Ok(result)
}

async fn sync_calendar_resources(
    client: &dyn CalDavClient,
    calendar: &CalendarInfo,
    resources: Vec<CalendarResourceMetadata>,
    existing: &SourceStateMap,
    filter_plan: &EventFilterPlan,
    plan_revision: u64,
    items: &mut Vec<ConnectorItemInput>,
    source_observations: &mut Vec<SourceObservation>,
    metadata_bytes: &mut usize,
    body_bytes: &mut usize,
) -> Result<()> {
    let mut fetch_hrefs = Vec::new();
    for resource in resources {
        let remote_id = event_remote_id(&calendar.href, &resource.href)?;
        let unchanged = resource.etag.as_ref().is_some_and(|etag| {
            existing.get(&remote_id).is_some_and(|state| {
                state.remote_version.as_ref() == Some(etag)
                    && state.evaluated_plan_revision == plan_revision
            })
        });
        if unchanged {
            let state = existing.get(&remote_id).ok_or_else(|| {
                CalDavConnectorError::ServerResponse(
                    "unchanged calendar resource has no source state".to_string(),
                )
            })?;
            source_observations.push(SourceObservation {
                remote_id,
                remote_version: resource.etag,
                disposition: state.disposition,
                filter_reason: state.filter_reason.clone(),
                evaluated_plan_revision: plan_revision,
            });
        } else {
            fetch_hrefs.push(resource.href);
        }
    }

    for batch in fetch_hrefs.chunks(MULTIGET_BATCH_SIZE) {
        let results = client.fetch_event_resources(&calendar.href, batch).await?;
        let fetched = validate_multiget(batch, results)?;
        for href in batch {
            let result = fetched.get(href).ok_or_else(|| {
                CalDavConnectorError::ServerResponse(
                    "calendar multiget omitted a requested resource".to_string(),
                )
            })?;
            match result {
                CalendarObjectResult::Found(object) => {
                    let etag_bytes = object.etag.as_ref().map_or(0, String::len);
                    let fetched_metadata_bytes =
                        object.href.len().checked_add(etag_bytes).ok_or_else(|| {
                            limit_error("calendar resource metadata exceeds connector limit")
                        })?;
                    if fetched_metadata_bytes > MAX_RESOURCE_METADATA_BYTES {
                        return Err(limit_error(
                            "calendar resource metadata exceeds connector limit",
                        ));
                    }
                    *metadata_bytes = metadata_bytes.checked_add(etag_bytes).ok_or_else(|| {
                        limit_error("calendar sync metadata exceeds connector limit")
                    })?;
                    if *metadata_bytes > MAX_SYNC_METADATA_BYTES {
                        return Err(limit_error(
                            "calendar sync metadata exceeds connector limit",
                        ));
                    }
                    if object.ical.len() > MAX_RESOURCE_BYTES {
                        return Err(limit_error(
                            "calendar resource body exceeds connector limit",
                        ));
                    }
                    *body_bytes = body_bytes.checked_add(object.ical.len()).ok_or_else(|| {
                        limit_error("calendar sync body size exceeds connector limit")
                    })?;
                    if *body_bytes > MAX_SYNC_BODY_BYTES {
                        return Err(limit_error(
                            "calendar sync body size exceeds connector limit",
                        ));
                    }
                    let remote_id = event_remote_id(&calendar.href, &object.href)?;
                    let processed = process_event(
                        remote_id,
                        &calendar.href,
                        object,
                        filter_plan,
                        plan_revision,
                    )?;
                    if let Some(item) = processed.item {
                        items.push(item);
                    }
                    source_observations.push(processed.observation);
                },
                CalendarObjectResult::NotFound { .. } => {},
                CalendarObjectResult::Failed { status, .. } => {
                    return Err(CalDavConnectorError::ServerResponse(format!(
                        "calendar multiget failed with HTTP status {status}"
                    )));
                },
            }
        }
    }
    Ok(())
}

fn validate_multiget(
    requested: &[String],
    results: Vec<CalendarObjectResult>,
) -> Result<BTreeMap<String, CalendarObjectResult>> {
    let requested = requested.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut indexed = BTreeMap::new();
    for result in results {
        let href = result_href(&result);
        if !requested.contains(href) {
            return Err(CalDavConnectorError::ServerResponse(
                "calendar multiget returned an unrequested resource".to_string(),
            ));
        }
        if indexed.insert(href.to_string(), result).is_some() {
            return Err(CalDavConnectorError::ServerResponse(
                "calendar multiget returned duplicate hrefs".to_string(),
            ));
        }
    }
    if indexed.len() != requested.len() {
        return Err(CalDavConnectorError::ServerResponse(
            "calendar multiget omitted a requested resource".to_string(),
        ));
    }
    Ok(indexed)
}

fn result_href(result: &CalendarObjectResult) -> &str {
    match result {
        CalendarObjectResult::Found(object) => &object.href,
        CalendarObjectResult::NotFound { href } | CalendarObjectResult::Failed { href, .. } => href,
    }
}

fn ensure_unique<'a>(values: impl Iterator<Item = &'a str>, message: &'static str) -> Result<()> {
    let mut seen = HashSet::new();
    if values.into_iter().all(|value| seen.insert(value)) {
        Ok(())
    } else {
        Err(CalDavConnectorError::ServerResponse(message.to_string()))
    }
}

fn limit_error(message: &'static str) -> CalDavConnectorError {
    CalDavConnectorError::ServerResponse(message.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventFilterPlan {
    start_date: Option<Date>,
    end_date: Option<Date>,
    accepted_addresses: Option<HashSet<String>>,
}

impl EventFilterPlan {
    fn new(
        filters: &CalDavFilters,
        account_username: &str,
        user_addresses: Vec<String>,
    ) -> Result<Self> {
        let (start_date, end_date) = filters.date_bounds()?;
        let accepted_addresses = filters
            .accepted_by_account
            .then(|| bounded_user_addresses(user_addresses, account_username))
            .transpose()?;
        Ok(Self {
            start_date,
            end_date,
            accepted_addresses,
        })
    }

    const fn is_active(&self) -> bool {
        self.start_date.is_some() || self.end_date.is_some() || self.accepted_addresses.is_some()
    }

    fn disposition(
        &self,
        components: &[EventComponentIndex],
        parse_status: ParseStatus,
    ) -> (SourceDisposition, Option<String>) {
        if !self.is_active() {
            return (SourceDisposition::Included, None);
        }
        if parse_status == ParseStatus::Unparsed {
            return (
                SourceDisposition::Filtered,
                Some(FILTER_REASON_UNPARSED.to_string()),
            );
        }
        if components
            .iter()
            .any(|component| self.matches_component(component))
        {
            return (SourceDisposition::Included, None);
        }

        let reason = if (self.start_date.is_some() || self.end_date.is_some())
            && !components
                .iter()
                .any(|component| self.matches_date(component))
        {
            FILTER_REASON_DATE_RANGE
        } else if self.accepted_addresses.is_some() {
            FILTER_REASON_ACCEPTANCE
        } else {
            FILTER_REASON_NO_COMPONENT
        };
        (SourceDisposition::Filtered, Some(reason.to_string()))
    }

    fn matches_component(&self, component: &EventComponentIndex) -> bool {
        self.matches_date(component) && self.matches_acceptance(component)
    }

    fn matches_date(&self, component: &EventComponentIndex) -> bool {
        if self.start_date.is_none() && self.end_date.is_none() {
            return true;
        }
        let Some(date) = component.dtstart.as_ref().and_then(component_start_date) else {
            return false;
        };
        self.start_date.is_none_or(|start| date >= start)
            && self.end_date.is_none_or(|end| date < end)
    }

    fn matches_acceptance(&self, component: &EventComponentIndex) -> bool {
        let Some(addresses) = self.accepted_addresses.as_ref() else {
            return true;
        };
        component.attendees.iter().any(|attendee| {
            addresses.contains(&normalized_calendar_address(&attendee.value).to_ascii_lowercase())
                && attendee.parameters.get("PARTSTAT").is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case("ACCEPTED"))
                })
        })
    }
}

fn bounded_user_addresses(
    user_addresses: Vec<String>,
    account_username: &str,
) -> Result<HashSet<String>> {
    if user_addresses.len() > MAX_USER_ADDRESSES {
        return Err(limit_error(
            "calendar user address count exceeds connector limit",
        ));
    }

    let use_username_fallback = user_addresses.is_empty();
    let mut metadata_bytes = 0_usize;
    let mut addresses = HashSet::new();
    for address in user_addresses {
        if address.len() > MAX_USER_ADDRESS_BYTES {
            return Err(limit_error(
                "calendar user address metadata exceeds connector limit",
            ));
        }
        metadata_bytes = metadata_bytes
            .checked_add(address.len())
            .ok_or_else(|| limit_error("calendar user address metadata exceeds connector limit"))?;
        if metadata_bytes > MAX_USER_ADDRESS_METADATA_BYTES {
            return Err(limit_error(
                "calendar user address metadata exceeds connector limit",
            ));
        }
        let normalized = normalized_calendar_address(&address).to_ascii_lowercase();
        if !normalized.is_empty() {
            addresses.insert(normalized);
        }
    }
    if use_username_fallback {
        if account_username.len() > MAX_USER_ADDRESS_BYTES {
            return Err(limit_error(
                "calendar user address metadata exceeds connector limit",
            ));
        }
        let normalized = normalized_calendar_address(account_username).to_ascii_lowercase();
        if !normalized.is_empty() {
            addresses.insert(normalized);
        }
    }
    Ok(addresses)
}

fn component_start_date(property: &IndexedProperty) -> Option<Date> {
    let value = property.value.as_bytes();
    if value.len() < 8 || !value[..8].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = std::str::from_utf8(&value[..4]).ok()?.parse().ok()?;
    let month = std::str::from_utf8(&value[4..6])
        .ok()?
        .parse::<u8>()
        .ok()
        .and_then(|month| Month::try_from(month).ok())?;
    let day = std::str::from_utf8(&value[6..8]).ok()?.parse().ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

fn normalized_calendar_address(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("mailto:"))
        .map_or(trimmed, |_| &trimmed[7..])
}

struct ProcessedEvent {
    item: Option<ConnectorItemInput>,
    observation: SourceObservation,
}

fn process_event(
    remote_id: String,
    calendar_href: &str,
    object: &CalendarObject,
    filter_plan: &EventFilterPlan,
    plan_revision: u64,
) -> Result<ProcessedEvent> {
    let (components, parse_status) = match index_event_components(&object.ical) {
        Ok(components) => (components, ParseStatus::Parsed),
        Err(_) => (Vec::new(), ParseStatus::Unparsed),
    };
    let (disposition, filter_reason) = filter_plan.disposition(&components, parse_status);
    let item = (disposition == SourceDisposition::Included)
        .then(|| {
            event_item(
                remote_id.clone(),
                calendar_href,
                object,
                components,
                parse_status,
            )
        })
        .transpose()?;
    Ok(ProcessedEvent {
        item,
        observation: SourceObservation {
            remote_id,
            remote_version: object.etag.clone(),
            disposition,
            filter_reason,
            evaluated_plan_revision: plan_revision,
        },
    })
}

fn included_observation(
    remote_id: String,
    remote_version: Option<String>,
    plan_revision: u64,
) -> SourceObservation {
    SourceObservation {
        remote_id,
        remote_version,
        disposition: SourceDisposition::Included,
        filter_reason: None,
        evaluated_plan_revision: plan_revision,
    }
}

fn calendar_item(remote_id: String, calendar: &CalendarInfo) -> Result<ConnectorItemInput> {
    let body = CalendarBody {
        schema_version: SCHEMA_VERSION,
        metadata: calendar.clone(),
    };
    Ok(ConnectorItemInput {
        remote_id,
        kind: "calendar".to_string(),
        remote_version: calendar.collection_etag.clone(),
        occurred_at: None,
        updated_at: None,
        search_text: bounded_search_text([
            Some(calendar.href.as_str()),
            calendar.display_name.as_deref(),
            calendar.description.as_deref(),
        ]),
        content_hash: content_hash(&serde_json::to_vec(&body)?),
        body_json: serde_json::to_value(body)?,
    })
}

fn event_item(
    remote_id: String,
    calendar_href: &str,
    object: &CalendarObject,
    components: Vec<EventComponentIndex>,
    parse_status: ParseStatus,
) -> Result<ConnectorItemInput> {
    let uid = components
        .first()
        .and_then(|component| component.uid.as_ref())
        .map(|property| property.value.clone());
    let occurred_at = components
        .first()
        .and_then(|component| component.dtstart.as_ref())
        .map(|property| property.value.clone());
    let search_text = event_search_text(calendar_href, object, &components);
    let body = EventBody {
        schema_version: SCHEMA_VERSION,
        calendar_href: calendar_href.to_string(),
        resource_href: object.href.clone(),
        uid,
        components,
        raw_ical: object.ical.clone(),
        parse_status,
    };
    Ok(ConnectorItemInput {
        remote_id,
        kind: "event".to_string(),
        remote_version: object.etag.clone(),
        occurred_at,
        updated_at: None,
        search_text,
        body_json: serde_json::to_value(body)?,
        content_hash: content_hash(object.ical.as_bytes()),
    })
}

fn event_search_text(
    calendar_href: &str,
    object: &CalendarObject,
    components: &[EventComponentIndex],
) -> String {
    let component_values = components.iter().flat_map(|component| {
        [
            component.uid.as_ref(),
            component.summary.as_ref(),
            component.location.as_ref(),
            component.organizer.as_ref(),
            component.dtstart.as_ref(),
            component.dtend.as_ref(),
        ]
        .into_iter()
        .flatten()
        .chain(component.attendees.iter())
        .chain(component.description.iter())
        .map(|property| Some(property.value.as_str()))
    });
    bounded_search_text(
        [Some(calendar_href), Some(object.href.as_str())]
            .into_iter()
            .chain(component_values),
    )
}

fn bounded_search_text<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let mut text = String::new();
    for value in values.into_iter().flatten().map(str::trim) {
        if value.is_empty() || text.len() >= MAX_SEARCH_TEXT_BYTES {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let remaining = MAX_SEARCH_TEXT_BYTES.saturating_sub(text.len());
        if value.len() <= remaining {
            text.push_str(value);
            continue;
        }
        let mut end = remaining;
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        text.push_str(&value[..end]);
    }
    text
}

fn calendar_remote_id(href: &str) -> Result<String> {
    serde_json::to_string(&("calendar", href)).map_err(Into::into)
}

fn event_remote_id(calendar_href: &str, resource_href: &str) -> Result<String> {
    serde_json::to_string(&("event", calendar_href, resource_href)).map_err(Into::into)
}

fn content_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn record_operation(operation: &'static str, started: Instant, item_count: usize) {
    #[cfg(feature = "metrics")]
    {
        metrics::counter!("moltis_connector_caldav_operations_total", "operation" => operation)
            .increment(1);
        metrics::histogram!("moltis_connector_caldav_operation_duration_seconds", "operation" => operation)
            .record(started.elapsed().as_secs_f64());
        metrics::histogram!("moltis_connector_caldav_observed_items", "operation" => operation)
            .record(item_count as f64);
    }
    #[cfg(feature = "tracing")]
    tracing::debug!(
        operation,
        item_count,
        elapsed_ms = started.elapsed().as_millis(),
        "CalDAV connector operation completed"
    );
    #[cfg(not(any(feature = "metrics", feature = "tracing")))]
    let _ = (operation, started, item_count);
}
