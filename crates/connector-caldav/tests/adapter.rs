use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use {
    async_trait::async_trait,
    moltis_caldav::{
        client::CalDavClient,
        error::Result as ClientResult,
        types::{
            CalendarInfo, CalendarObject, CalendarObjectResult, CalendarResourceMetadata,
            CreatedEvent, EventSummary, NewEvent, TimeRange, UpdateEvent, UpdatedEvent,
        },
    },
    moltis_connector_caldav::{
        CalDavAccountConfig, CalDavConnector, CalDavDatasetConfig, CalDavFilters, CalendarSelection,
    },
    moltis_connectors::{SourceDisposition, SourceState, SourceStateMap},
    secrecy::Secret,
    sha2::{Digest, Sha256},
    time::OffsetDateTime,
};

const ACCOUNT_USERNAME: &str = "account@example.test";
const PLAN_REVISION: u64 = 7;

#[derive(Default)]
struct FakeState {
    user_addresses: Vec<String>,
    calendars: Vec<CalendarInfo>,
    resources: HashMap<String, Vec<CalendarResourceMetadata>>,
    fetched: HashMap<String, CalendarObjectResult>,
    batches: Vec<Vec<String>>,
    omit_response_for: Option<String>,
}

#[derive(Default)]
struct FakeClient {
    state: Mutex<FakeState>,
}

impl FakeClient {
    fn with_state(state: FakeState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[async_trait]
impl CalDavClient for FakeClient {
    async fn list_user_addresses(&self) -> ClientResult<Vec<String>> {
        Ok(self.state().user_addresses.clone())
    }

    async fn list_calendars(&self) -> ClientResult<Vec<CalendarInfo>> {
        Ok(self.state().calendars.clone())
    }

    async fn list_events(
        &self,
        _calendar_href: &str,
        _range: Option<TimeRange>,
    ) -> ClientResult<Vec<EventSummary>> {
        Err(moltis_caldav::Error::Protocol(
            "unused fake method".to_string(),
        ))
    }

    async fn list_event_resources(
        &self,
        calendar_href: &str,
    ) -> ClientResult<Vec<CalendarResourceMetadata>> {
        Ok(self
            .state()
            .resources
            .get(calendar_href)
            .cloned()
            .unwrap_or_default())
    }

    async fn fetch_event_resources(
        &self,
        _calendar_href: &str,
        hrefs: &[String],
    ) -> ClientResult<Vec<CalendarObjectResult>> {
        let mut state = self.state();
        state.batches.push(hrefs.to_vec());
        Ok(hrefs
            .iter()
            .filter(|href| state.omit_response_for.as_ref() != Some(href))
            .filter_map(|href| state.fetched.get(href).cloned())
            .collect())
    }

    async fn create_event(
        &self,
        _calendar_href: &str,
        _event: NewEvent,
    ) -> ClientResult<CreatedEvent> {
        Err(moltis_caldav::Error::Protocol(
            "unused fake method".to_string(),
        ))
    }

    async fn update_event(
        &self,
        _href: &str,
        _etag: &str,
        _updates: UpdateEvent,
    ) -> ClientResult<UpdatedEvent> {
        Err(moltis_caldav::Error::Protocol(
            "unused fake method".to_string(),
        ))
    }

    async fn delete_event(&self, _href: &str, _etag: &str) -> ClientResult<()> {
        Err(moltis_caldav::Error::Protocol(
            "unused fake method".to_string(),
        ))
    }
}

fn calendar(href: &str) -> CalendarInfo {
    CalendarInfo {
        href: href.to_string(),
        display_name: Some("Calendar".to_string()),
        color: Some("#112233".to_string()),
        description: Some("Description".to_string()),
        collection_etag: Some("\"calendar-v1\"".to_string()),
        supports_sync: true,
    }
}

fn metadata(href: &str, etag: Option<&str>) -> CalendarResourceMetadata {
    CalendarResourceMetadata {
        href: href.to_string(),
        etag: etag.map(String::from),
    }
}

fn found(href: &str, etag: Option<&str>, ical: &str) -> CalendarObjectResult {
    CalendarObjectResult::Found(CalendarObject {
        href: href.to_string(),
        etag: etag.map(String::from),
        ical: ical.to_string(),
    })
}

fn event_id(calendar_href: &str, href: &str) -> String {
    serde_json::to_string(&("event", calendar_href, href))
        .unwrap_or_else(|error| panic!("test tuple serializes: {error}"))
}

fn basic_ical(uid: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20260806T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn source_state(
    remote_version: Option<&str>,
    disposition: SourceDisposition,
    filter_reason: Option<&str>,
    evaluated_plan_revision: u64,
) -> SourceState {
    SourceState {
        dataset_id: "dataset".to_string(),
        remote_id: "source".to_string(),
        remote_version: remote_version.map(String::from),
        disposition,
        filter_reason: filter_reason.map(String::from),
        evaluated_plan_revision,
        last_seen_run_id: "run".to_string(),
        observed_at: OffsetDateTime::UNIX_EPOCH,
    }
}

#[tokio::test]
async fn all_and_selected_calendar_modes() {
    let all_client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/"), calendar("/b/")],
        ..FakeState::default()
    });
    let connector = CalDavConnector;
    let all = connector
        .sync_with_client(
            &all_client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("all sync succeeds: {error}"));
    assert_eq!(all.calendars.len(), 2);
    assert_eq!(all.items.len(), 2);

    let selected = connector
        .sync_with_client(
            &all_client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig {
                selection: CalendarSelection::Selected {
                    calendar_hrefs: vec!["/b/".to_string()],
                },
                ..CalDavDatasetConfig::default()
            },
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("selected sync succeeds: {error}"));
    assert_eq!(selected.calendars, vec![calendar("/b/")]);
    assert_eq!(selected.items.len(), 1);
}

#[tokio::test]
async fn unchanged_etag_skips_fetch_and_removed_resource_is_unobserved() {
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(
            "/a/kept.ics",
            Some("\"v1\""),
        )])]),
        ..FakeState::default()
    });
    let kept_id = event_id("/a/", "/a/kept.ics");
    let removed_id = event_id("/a/", "/a/removed.ics");
    let existing = SourceStateMap::from([
        (
            kept_id.clone(),
            source_state(
                Some("\"v1\""),
                SourceDisposition::Included,
                None,
                PLAN_REVISION,
            ),
        ),
        (
            removed_id.clone(),
            source_state(
                Some("\"old\""),
                SourceDisposition::Included,
                None,
                PLAN_REVISION,
            ),
        ),
    ]);

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            existing,
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));

    assert!(client.state().batches.is_empty());
    assert!(
        snapshot
            .source_observations
            .iter()
            .any(|observation| observation.remote_id == kept_id)
    );
    assert!(
        !snapshot
            .source_observations
            .iter()
            .any(|observation| observation.remote_id == removed_id)
    );
    assert_eq!(
        snapshot
            .items
            .iter()
            .filter(|item| item.kind == "event")
            .count(),
        0
    );
}

#[tokio::test]
async fn changed_and_missing_etags_are_fetched() {
    let changed_ical = basic_ical("changed");
    let no_etag_ical = basic_ical("no-etag");
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![
            metadata("/a/changed.ics", Some("\"v2\"")),
            metadata("/a/no-etag.ics", None),
        ])]),
        fetched: HashMap::from([
            (
                "/a/changed.ics".to_string(),
                found("/a/changed.ics", Some("\"v2\""), &changed_ical),
            ),
            (
                "/a/no-etag.ics".to_string(),
                found("/a/no-etag.ics", None, &no_etag_ical),
            ),
        ]),
        ..FakeState::default()
    });
    let existing = SourceStateMap::from([
        (
            event_id("/a/", "/a/changed.ics"),
            source_state(
                Some("\"v1\""),
                SourceDisposition::Included,
                None,
                PLAN_REVISION,
            ),
        ),
        (
            event_id("/a/", "/a/no-etag.ics"),
            source_state(None, SourceDisposition::Included, None, PLAN_REVISION),
        ),
    ]);

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            existing,
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));

    assert_eq!(client.state().batches[0].len(), 2);
    assert_eq!(
        snapshot
            .items
            .iter()
            .filter(|item| item.kind == "event")
            .count(),
        2
    );
}

#[tokio::test]
async fn multiget_batches_are_bounded_to_one_hundred() {
    let resources = (0..205)
        .map(|index| metadata(&format!("/a/{index}.ics"), None))
        .collect::<Vec<_>>();
    let fetched = resources
        .iter()
        .map(|resource| {
            (
                resource.href.clone(),
                found(&resource.href, None, &basic_ical(&resource.href)),
            )
        })
        .collect::<HashMap<_, _>>();
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), resources)]),
        fetched,
        ..FakeState::default()
    });

    CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));

    assert_eq!(
        client
            .state()
            .batches
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        vec![100, 100, 5]
    );
}

#[tokio::test]
async fn oversized_calendar_responses_are_rejected() {
    let calendars = (0..1_001)
        .map(|index| calendar(&format!("/calendars/{index}/")))
        .collect();
    let client = FakeClient::with_state(FakeState {
        calendars,
        ..FakeState::default()
    });
    let error = match CalDavConnector.test_connection_with_client(&client).await {
        Ok(_) => panic!("oversized calendar list should be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("calendar count exceeds connector limit")
    );

    let calendar_href = "/calendars/work/";
    let mut resources = HashMap::new();
    resources.insert(calendar_href.to_owned(), vec![metadata(
        &"X".repeat(64 * 1024 + 1),
        Some("1"),
    )]);
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar(calendar_href)],
        resources,
        ..FakeState::default()
    });
    let error = match CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            SourceStateMap::default(),
            PLAN_REVISION,
        )
        .await
    {
        Ok(_) => panic!("oversized resource metadata should be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("resource metadata exceeds connector limit")
    );

    let resource_href = "/calendars/work/large.ics";
    let huge_etag = "X".repeat(64 * 1024 + 1);
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar(calendar_href)],
        resources: HashMap::from([(calendar_href.to_owned(), vec![metadata(
            resource_href,
            Some("1"),
        )])]),
        fetched: HashMap::from([(
            resource_href.to_owned(),
            found(
                resource_href,
                Some(&huge_etag),
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
            ),
        )]),
        ..FakeState::default()
    });
    let error = match CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            SourceStateMap::default(),
            PLAN_REVISION,
        )
        .await
    {
        Ok(_) => panic!("oversized multiget metadata should be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("resource metadata exceeds connector limit")
    );

    let mut resources = HashMap::new();
    resources.insert(calendar_href.to_owned(), vec![metadata(
        resource_href,
        Some("1"),
    )]);
    let mut fetched = HashMap::new();
    fetched.insert(
        resource_href.to_owned(),
        found(resource_href, Some("1"), &"X".repeat(4 * 1024 * 1024 + 1)),
    );
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar(calendar_href)],
        resources,
        fetched,
        ..FakeState::default()
    });
    let error = match CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            SourceStateMap::default(),
            PLAN_REVISION,
        )
        .await
    {
        Ok(_) => panic!("oversized calendar body should be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("resource body exceeds connector limit")
    );
}

#[tokio::test]
async fn missing_or_failed_multiget_prevents_snapshot() {
    let missing_client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(
            "/a/event.ics",
            Some("\"v1\""),
        )])]),
        omit_response_for: Some("/a/event.ics".to_string()),
        ..FakeState::default()
    });
    assert!(
        CalDavConnector
            .sync_with_client(
                &missing_client,
                ACCOUNT_USERNAME,
                &CalDavDatasetConfig::default(),
                BTreeMap::new(),
                PLAN_REVISION,
            )
            .await
            .is_err()
    );

    let failed_client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(
            "/a/event.ics",
            Some("\"v1\""),
        )])]),
        fetched: HashMap::from([("/a/event.ics".to_string(), CalendarObjectResult::Failed {
            href: "/a/event.ics".to_string(),
            status: 503,
        })]),
        ..FakeState::default()
    });
    assert!(
        CalDavConnector
            .sync_with_client(
                &failed_client,
                ACCOUNT_USERNAME,
                &CalDavDatasetConfig::default(),
                BTreeMap::new(),
                PLAN_REVISION,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn duplicate_resource_hrefs_prevent_snapshot() {
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![
            metadata("/a/event.ics", Some("\"v1\"")),
            metadata("/a/event.ics", Some("\"v1\"")),
        ])]),
        ..FakeState::default()
    });

    assert!(
        CalDavConnector
            .sync_with_client(
                &client,
                ACCOUNT_USERNAME,
                &CalDavDatasetConfig::default(),
                BTreeMap::new(),
                PLAN_REVISION,
            )
            .await
            .is_err()
    );
    assert!(client.state().batches.is_empty());
}

#[tokio::test]
async fn not_found_race_is_not_observed() {
    let href = "/a/gone.ics";
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(href, Some("\"v1\""))])]),
        fetched: HashMap::from([(href.to_string(), CalendarObjectResult::NotFound {
            href: href.to_string(),
        })]),
        ..FakeState::default()
    });

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));

    assert!(
        !snapshot
            .source_observations
            .iter()
            .any(|observation| observation.remote_id == event_id("/a/", href))
    );
    assert_eq!(snapshot.items.len(), 1);
}

#[tokio::test]
async fn recurrence_raw_and_index_are_preserved() {
    let raw = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:series@example.test\r\n",
        "DTSTART;TZID=Europe/Paris:20260806T090000\r\n",
        "RRULE:FREQ=WEEKLY;COUNT=3\r\n",
        "EXDATE;TZID=Europe/Paris:20260813T090000\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(
            "/a/series.ics",
            Some("\"v1\""),
        )])]),
        fetched: HashMap::from([(
            "/a/series.ics".to_string(),
            found("/a/series.ics", Some("\"v1\""), raw),
        )]),
        ..FakeState::default()
    });

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));
    let event = snapshot
        .items
        .iter()
        .find(|item| item.kind == "event")
        .unwrap_or_else(|| panic!("event item exists"));

    assert_eq!(event.body_json["raw_ical"], raw);
    assert_eq!(event.body_json["parse_status"], "parsed");
    assert_eq!(event.body_json["uid"], "series@example.test");
    assert_eq!(
        event.body_json["components"][0]["rrule"][0]["value"],
        "FREQ=WEEKLY;COUNT=3"
    );
    assert_eq!(event.remote_version.as_deref(), Some("\"v1\""));
    assert_eq!(
        event.content_hash,
        format!("{:x}", Sha256::digest(raw.as_bytes()))
    );
    assert_eq!(event.occurred_at.as_deref(), Some("20260806T090000"));
}

#[tokio::test]
async fn invalid_ical_is_preserved_without_parser_details() {
    let raw = "not an iCalendar payload";
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata("/a/bad.ics", None)])]),
        fetched: HashMap::from([("/a/bad.ics".to_string(), found("/a/bad.ics", None, raw))]),
        ..FakeState::default()
    });

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            BTreeMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));
    let body = &snapshot.items[1].body_json;
    assert_eq!(body["raw_ical"], raw);
    assert_eq!(body["parse_status"], "unparsed");
    assert_eq!(body["components"], serde_json::json!([]));
    assert!(body.get("error").is_none());
}

#[tokio::test]
async fn filters_are_component_scoped_and_emit_metadata_only_observations() {
    let inside = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n",
        "UID:inside\r\nDTSTART:20260806T230000Z\r\n",
        "ATTENDEE;PARTSTAT=accepted:MAILTO:ACCOUNT@EXAMPLE.TEST\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    let exclusive_end = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n",
        "UID:end\r\nDTSTART:20260807T000000Z\r\n",
        "ATTENDEE;PARTSTAT=ACCEPTED:mailto:account@example.test\r\n",
        "RDATE:20260806T090000Z\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    let split_matches = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\nUID:date-only\r\nDTSTART:20260806T090000Z\r\n",
        "ATTENDEE;PARTSTAT=DECLINED:mailto:account@example.test\r\nEND:VEVENT\r\n",
        "BEGIN:VEVENT\r\nUID:acceptance-only\r\nDTSTART:20260808T090000Z\r\n",
        "ATTENDEE;PARTSTAT=ACCEPTED:mailto:account@example.test\r\nEND:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );
    let hrefs = ["/a/inside.ics", "/a/end.ics", "/a/split.ics", "/a/bad.ics"];
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([(
            "/a/".to_string(),
            hrefs
                .iter()
                .map(|href| metadata(href, Some("\"v1\"")))
                .collect(),
        )]),
        fetched: HashMap::from([
            (
                hrefs[0].to_string(),
                found(hrefs[0], Some("\"v1\""), inside),
            ),
            (
                hrefs[1].to_string(),
                found(hrefs[1], Some("\"v1\""), exclusive_end),
            ),
            (
                hrefs[2].to_string(),
                found(hrefs[2], Some("\"v1\""), split_matches),
            ),
            (
                hrefs[3].to_string(),
                found(hrefs[3], Some("\"v1\""), "invalid"),
            ),
        ]),
        ..FakeState::default()
    });
    let config = CalDavDatasetConfig {
        filters: CalDavFilters {
            start_date: Some("2026-08-06".to_string()),
            end_date: Some("2026-08-07".to_string()),
            accepted_by_account: true,
        },
        ..CalDavDatasetConfig::default()
    };

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &config,
            SourceStateMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("filtered sync succeeds: {error}"));

    assert_eq!(snapshot.items.len(), 2, "calendar plus included event");
    let observation = |href: &str| {
        snapshot
            .source_observations
            .iter()
            .find(|observation| observation.remote_id == event_id("/a/", href))
            .unwrap_or_else(|| panic!("observation exists for {href}"))
    };
    assert_eq!(
        observation(hrefs[0]).disposition,
        SourceDisposition::Included
    );
    assert_eq!(
        observation(hrefs[1]).filter_reason.as_deref(),
        Some("outside_date_range")
    );
    assert_eq!(
        observation(hrefs[2]).filter_reason.as_deref(),
        Some("not_accepted_by_account")
    );
    assert_eq!(
        observation(hrefs[3]).filter_reason.as_deref(),
        Some("unparsed_ical")
    );
    assert_eq!(
        snapshot.source_observations[0].disposition,
        SourceDisposition::Included,
        "calendar records are always included"
    );
}

#[tokio::test]
async fn accepted_filter_matches_all_server_user_addresses_instead_of_login_username() {
    let primary = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:primary\r\n",
        "ATTENDEE;PARTSTAT=ACCEPTED:MAILTO:PRIMARY@EXAMPLE.TEST\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    let alias = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:alias\r\n",
        "ATTENDEE;PARTSTAT=accepted:mailto:alias@example.test\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    let login = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:login\r\n",
        "ATTENDEE;PARTSTAT=ACCEPTED:mailto:login-user@example.test\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    let hrefs = ["/a/primary.ics", "/a/alias.ics", "/a/login.ics"];
    let client = FakeClient::with_state(FakeState {
        user_addresses: vec![
            "mailto:primary@example.test".to_string(),
            "MAILTO:PRIMARY@EXAMPLE.TEST".to_string(),
            "mailto:alias@example.test".to_string(),
        ],
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([(
            "/a/".to_string(),
            hrefs
                .iter()
                .map(|href| metadata(href, Some("\"v1\"")))
                .collect(),
        )]),
        fetched: HashMap::from([
            (
                hrefs[0].to_string(),
                found(hrefs[0], Some("\"v1\""), primary),
            ),
            (hrefs[1].to_string(), found(hrefs[1], Some("\"v1\""), alias)),
            (hrefs[2].to_string(), found(hrefs[2], Some("\"v1\""), login)),
        ]),
        ..FakeState::default()
    });
    let config = CalDavDatasetConfig {
        filters: CalDavFilters {
            accepted_by_account: true,
            ..CalDavFilters::default()
        },
        ..CalDavDatasetConfig::default()
    };

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            "login-user@example.test",
            &config,
            SourceStateMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("filtered sync succeeds: {error}"));

    assert_eq!(
        snapshot
            .items
            .iter()
            .filter(|item| item.kind == "event")
            .count(),
        2
    );
    let login_observation = snapshot
        .source_observations
        .iter()
        .find(|observation| observation.remote_id == event_id("/a/", hrefs[2]))
        .unwrap_or_else(|| panic!("login attendee observation exists"));
    assert_eq!(login_observation.disposition, SourceDisposition::Filtered);
    assert_eq!(
        login_observation.filter_reason.as_deref(),
        Some("not_accepted_by_account")
    );
}

#[tokio::test]
async fn user_address_count_and_metadata_are_bounded() {
    let config = CalDavDatasetConfig {
        filters: CalDavFilters {
            accepted_by_account: true,
            ..CalDavFilters::default()
        },
        ..CalDavDatasetConfig::default()
    };
    let count_client = FakeClient::with_state(FakeState {
        user_addresses: (0..257)
            .map(|index| format!("mailto:user-{index}@example.test"))
            .collect(),
        ..FakeState::default()
    });

    let count_error = match CalDavConnector
        .sync_with_client(
            &count_client,
            ACCOUNT_USERNAME,
            &config,
            SourceStateMap::new(),
            PLAN_REVISION,
        )
        .await
    {
        Ok(_) => panic!("excessive address count must fail"),
        Err(error) => error,
    };
    assert!(
        count_error
            .to_string()
            .contains("user address count exceeds connector limit")
    );

    let metadata_client = FakeClient::with_state(FakeState {
        user_addresses: (0..17)
            .map(|index| format!("mailto:{index}-{}", "x".repeat(4_080)))
            .collect(),
        ..FakeState::default()
    });
    let metadata_error = match CalDavConnector
        .sync_with_client(
            &metadata_client,
            ACCOUNT_USERNAME,
            &config,
            SourceStateMap::new(),
            PLAN_REVISION,
        )
        .await
    {
        Ok(_) => panic!("excessive address metadata must fail"),
        Err(error) => error,
    };
    assert!(
        metadata_error
            .to_string()
            .contains("user address metadata exceeds connector limit")
    );
}

#[tokio::test]
async fn plan_revision_controls_etag_skip_and_preserves_filtered_state() {
    let href = "/a/event.ics";
    let remote_id = event_id("/a/", href);
    let stale_client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(href, Some("\"v1\""))])]),
        fetched: HashMap::from([(
            href.to_string(),
            found(href, Some("\"v1\""), &basic_ical("event")),
        )]),
        ..FakeState::default()
    });
    let stale = SourceStateMap::from([(
        remote_id.clone(),
        source_state(
            Some("\"v1\""),
            SourceDisposition::Filtered,
            Some("old_filter"),
            PLAN_REVISION - 1,
        ),
    )]);

    CalDavConnector
        .sync_with_client(
            &stale_client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            stale,
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("revision mismatch refetches: {error}"));
    assert_eq!(stale_client.state().batches, vec![vec![href.to_string()]]);

    let current_client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(href, Some("\"v1\""))])]),
        ..FakeState::default()
    });
    let current = SourceStateMap::from([(
        remote_id.clone(),
        source_state(
            Some("\"v1\""),
            SourceDisposition::Filtered,
            Some("outside_date_range"),
            PLAN_REVISION,
        ),
    )]);
    let snapshot = CalDavConnector
        .sync_with_client(
            &current_client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            current,
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("matching revision skips: {error}"));

    assert!(current_client.state().batches.is_empty());
    let observation = snapshot
        .source_observations
        .iter()
        .find(|observation| observation.remote_id == remote_id)
        .unwrap_or_else(|| panic!("filtered source is observed"));
    assert_eq!(observation.disposition, SourceDisposition::Filtered);
    assert_eq!(
        observation.filter_reason.as_deref(),
        Some("outside_date_range")
    );
    assert_eq!(observation.evaluated_plan_revision, PLAN_REVISION);
}

#[tokio::test]
async fn search_text_is_bounded_and_excludes_unindexed_raw_ical() {
    let long_description = "x".repeat(20 * 1024);
    let raw = format!(
        concat!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:search-uid\r\n",
            "SUMMARY:Planning Needle\r\nLOCATION:Room Seven\r\n",
            "ORGANIZER:mailto:owner@example.test\r\n",
            "ATTENDEE;PARTSTAT=ACCEPTED:mailto:guest@example.test\r\n",
            "DESCRIPTION:{long_description}\r\n",
            "X-PRIVATE:RAW-ICS-MUST-NOT-BE-SEARCHABLE\r\n",
            "DTSTART:20260806T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        ),
        long_description = long_description,
    );
    let href = "/a/search.ics";
    let client = FakeClient::with_state(FakeState {
        calendars: vec![calendar("/a/")],
        resources: HashMap::from([("/a/".to_string(), vec![metadata(href, Some("\"v1\""))])]),
        fetched: HashMap::from([(href.to_string(), found(href, Some("\"v1\""), &raw))]),
        ..FakeState::default()
    });

    let snapshot = CalDavConnector
        .sync_with_client(
            &client,
            ACCOUNT_USERNAME,
            &CalDavDatasetConfig::default(),
            SourceStateMap::new(),
            PLAN_REVISION,
        )
        .await
        .unwrap_or_else(|error| panic!("sync succeeds: {error}"));
    let event = snapshot
        .items
        .iter()
        .find(|item| item.kind == "event")
        .unwrap_or_else(|| panic!("event item exists"));

    assert!(event.search_text.contains("Planning Needle"));
    assert!(event.search_text.contains("Room Seven"));
    assert!(event.search_text.contains("owner@example.test"));
    assert!(event.search_text.len() <= 16 * 1024);
    assert!(!event.search_text.contains("RAW-ICS-MUST-NOT-BE-SEARCHABLE"));
    assert!(
        event.body_json["raw_ical"]
            .as_str()
            .is_some_and(|value| value.contains("RAW-ICS-MUST-NOT-BE-SEARCHABLE"))
    );
}

#[tokio::test]
async fn private_url_is_blocked_unless_explicitly_allowed() {
    let blocked = CalDavAccountConfig {
        server_url: "https://127.0.0.1:9/caldav".to_string(),
        username: "user".to_string(),
        password: Secret::new("password".to_string()),
        timeout_seconds: 1,
        allow_insecure_http: false,
        allow_private_network: false,
    };
    let blocked_error = CalDavConnector.test_connection(&blocked).await;
    assert!(blocked_error.is_err());
    assert_eq!(
        blocked_error
            .err()
            .map(|error| error.to_string())
            .as_deref(),
        Some("CalDAV server failed network safety validation")
    );

    let allowed = CalDavAccountConfig {
        allow_private_network: true,
        ..blocked
    };
    assert!(allowed.validate_connection_target().await.is_ok());
}

#[test]
fn config_validation_redacts_and_never_serializes_password() {
    let parsed: CalDavAccountConfig = serde_json::from_value(serde_json::json!({
        "server_url": "https://calendar.example.test/dav",
        "username": "account@example.test",
        "password": "super-secret"
    }))
    .unwrap_or_else(|error| panic!("config deserializes: {error}"));
    assert_eq!(parsed.timeout_seconds, 30);
    let debug = format!("{parsed:?}");
    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("account@example.test"));
    assert!(!debug.contains("calendar.example.test"));
    let view = serde_json::to_string(&parsed.view())
        .unwrap_or_else(|error| panic!("view serializes: {error}"));
    assert!(!view.contains("super-secret"));
    assert!(parsed.validate().is_ok());

    let credential_url: CalDavAccountConfig = serde_json::from_value(serde_json::json!({
        "server_url": "https://user:password@calendar.example.test/dav",
        "username": "user",
        "password": "password"
    }))
    .unwrap_or_else(|error| panic!("config deserializes: {error}"));
    assert!(credential_url.validate().is_err());

    let query_url: CalDavAccountConfig = serde_json::from_value(serde_json::json!({
        "server_url": "https://calendar.example.test/dav?token=secret",
        "username": "user",
        "password": "password"
    }))
    .unwrap_or_else(|error| panic!("config deserializes: {error}"));
    assert!(query_url.validate().is_err());
}

#[test]
fn selected_calendar_validation_uses_exact_deduplication() {
    let duplicate = CalDavDatasetConfig {
        selection: CalendarSelection::Selected {
            calendar_hrefs: vec!["/calendar".to_string(), "/calendar".to_string()],
        },
        ..CalDavDatasetConfig::default()
    };
    assert!(duplicate.validate().is_err());

    let distinct = CalDavDatasetConfig {
        selection: CalendarSelection::Selected {
            calendar_hrefs: vec!["/calendar".to_string(), "/calendar/".to_string()],
        },
        ..CalDavDatasetConfig::default()
    };
    assert!(distinct.validate().is_ok());
}

#[test]
fn dataset_config_defaults_and_serializes_as_camel_case() {
    let parsed: CalDavDatasetConfig = serde_json::from_value(serde_json::json!({
        "selection": {
            "mode": "selected",
            "calendar_hrefs": ["/calendar/"]
        }
    }))
    .unwrap_or_else(|error| panic!("legacy config deserializes: {error}"));

    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.filters, CalDavFilters::default());
    let serialized = serde_json::to_value(parsed)
        .unwrap_or_else(|error| panic!("dataset config serializes: {error}"));
    assert_eq!(serialized["schemaVersion"], 1);
    assert_eq!(
        serialized["selection"]["calendarHrefs"],
        serde_json::json!(["/calendar/"])
    );
    assert_eq!(serialized["filters"]["acceptedByAccount"], false);
    assert!(serialized.get("schema_version").is_none());
    assert!(serialized["selection"].get("calendar_hrefs").is_none());
}

#[test]
fn dataset_config_validates_schema_and_absolute_date_ordering() {
    let unsupported = CalDavDatasetConfig {
        schema_version: 2,
        ..CalDavDatasetConfig::default()
    };
    assert!(unsupported.validate().is_err());

    for invalid in ["20260806", "2026-02-30", "2026-8-06"] {
        let config = CalDavDatasetConfig {
            filters: CalDavFilters {
                start_date: Some(invalid.to_string()),
                ..CalDavFilters::default()
            },
            ..CalDavDatasetConfig::default()
        };
        assert!(config.validate().is_err(), "{invalid} must be rejected");
    }

    let reversed = CalDavDatasetConfig {
        filters: CalDavFilters {
            start_date: Some("2026-08-07".to_string()),
            end_date: Some("2026-08-07".to_string()),
            accepted_by_account: false,
        },
        ..CalDavDatasetConfig::default()
    };
    assert!(reversed.validate().is_err());
}
