//! CalDAV client trait and `libdav`-backed implementation.

use std::{
    future::{Future, Ready, ready},
    io,
    net::SocketAddr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use {
    async_trait::async_trait,
    hyper_util::client::legacy::connect::{HttpConnector, dns::Name},
    secrecy::{ExposeSecret, Secret},
    tower_service::Service,
};

use crate::{
    error::{Error, Result},
    types::{
        CalendarInfo, CalendarObject, CalendarObjectResult, CalendarResourceMetadata, CreatedEvent,
        EventSummary, NewEvent, TimeRange, UpdateEvent, UpdatedEvent,
    },
};

/// Default timeout applied to discovery and CalDAV protocol operations.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Security and timeout options for CalDAV connections.
#[derive(Debug, Clone)]
pub struct CalDavConnectionOptions {
    /// Permit cleartext HTTP. This should only be enabled for trusted local test servers.
    pub allow_insecure_http: bool,
    /// Maximum duration of each protocol operation.
    pub request_timeout: Duration,
}

impl Default for CalDavConnectionOptions {
    fn default() -> Self {
        Self {
            allow_insecure_http: false,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

/// Trait for CalDAV server interactions.
///
/// This allows mocking in tests without a real server.
#[async_trait]
pub trait CalDavClient: Send + Sync {
    /// List the calendar user addresses associated with the authenticated principal.
    async fn list_user_addresses(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Discover calendars available on this account.
    async fn list_calendars(&self) -> Result<Vec<CalendarInfo>>;

    /// List events in a calendar, optionally filtered by time range.
    async fn list_events(
        &self,
        calendar_href: &str,
        range: Option<TimeRange>,
    ) -> Result<Vec<EventSummary>>;

    /// List VEVENT resources and their change-detection metadata.
    async fn list_event_resources(
        &self,
        calendar_href: &str,
    ) -> Result<Vec<CalendarResourceMetadata>>;

    /// Fetch specific VEVENT resources, retaining a result for every server response.
    async fn fetch_event_resources(
        &self,
        calendar_href: &str,
        hrefs: &[String],
    ) -> Result<Vec<CalendarObjectResult>>;

    /// Create a new event in the given calendar.
    async fn create_event(&self, calendar_href: &str, event: NewEvent) -> Result<CreatedEvent>;

    /// Update an existing event.
    async fn update_event(
        &self,
        href: &str,
        etag: &str,
        updates: UpdateEvent,
    ) -> Result<UpdatedEvent>;

    /// Delete an event.
    async fn delete_event(&self, href: &str, etag: &str) -> Result<()>;
}

/// `libdav`-backed CalDAV client using hyper + tower for HTTP.
pub struct LibDavCalDavClient {
    inner: libdav::CalDavClient<AuthenticatedHyperHttpsClient>,
    request_timeout: Duration,
}

/// Type alias for the libdav HTTPS connector stack.
type HyperHttpsClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<HttpConnector<PinnedResolver>>,
    String,
>;

type AuthenticatedHyperHttpsClient = tower_http::auth::AddAuthorization<HyperHttpsClient>;

#[derive(Clone)]
struct PinnedResolver {
    addresses: Arc<[SocketAddr]>,
}

struct PinnedAddresses {
    addresses: Arc<[SocketAddr]>,
    index: usize,
}

impl Iterator for PinnedAddresses {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let address = self.addresses.get(self.index).copied()?;
        self.index += 1;
        Some(address)
    }
}

impl Service<Name> for PinnedResolver {
    type Error = io::Error;
    type Future = Ready<std::result::Result<Self::Response, Self::Error>>;
    type Response = PinnedAddresses;

    fn poll_ready(
        &mut self,
        _context: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _name: Name) -> Self::Future {
        ready(Ok(PinnedAddresses {
            addresses: Arc::clone(&self.addresses),
            index: 0,
        }))
    }
}

impl LibDavCalDavClient {
    /// Connect to a CalDAV server.
    ///
    /// Uses service discovery to locate the CalDAV context path.
    pub async fn connect(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
    ) -> Result<Self> {
        Self::connect_with_options(
            base_url,
            username,
            password,
            CalDavConnectionOptions::default(),
        )
        .await
    }

    /// Connect using service discovery and explicit security/timeout options.
    pub async fn connect_with_options(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
        options: CalDavConnectionOptions,
    ) -> Result<Self> {
        Self::connect_inner(base_url, username, password, options, true, None).await
    }

    /// Connect directly to the supplied CalDAV context URL without service discovery.
    pub async fn connect_direct(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
    ) -> Result<Self> {
        Self::connect_direct_with_options(
            base_url,
            username,
            password,
            CalDavConnectionOptions::default(),
        )
        .await
    }

    /// Connect directly with explicit security/timeout options.
    pub async fn connect_direct_with_options(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
        options: CalDavConnectionOptions,
    ) -> Result<Self> {
        Self::connect_inner(base_url, username, password, options, false, None).await
    }

    /// Connect directly while pinning prevalidated socket addresses.
    pub async fn connect_direct_to_addresses_with_options(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
        options: CalDavConnectionOptions,
        addresses: Vec<SocketAddr>,
    ) -> Result<Self> {
        Self::connect_inner(
            base_url,
            username,
            password,
            options,
            false,
            Some(addresses),
        )
        .await
    }

    async fn connect_inner(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
        options: CalDavConnectionOptions,
        use_service_discovery: bool,
        addresses: Option<Vec<SocketAddr>>,
    ) -> Result<Self> {
        let uri = validate_base_uri(base_url, options.allow_insecure_http)?;
        let addresses = match addresses {
            Some(addresses) if !addresses.is_empty() => addresses,
            Some(_) => {
                return Err(Error::Protocol(
                    "CalDAV connection has no resolved addresses".to_owned(),
                ));
            },
            None => resolve_uri_addresses(&uri).await?,
        };

        let mut http_connector = HttpConnector::new_with_resolver(PinnedResolver {
            addresses: addresses.into(),
        });
        http_connector.enforce_http(false);
        let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|e| Error::Protocol(format!("failed to load native TLS roots: {e}")))?
            .https_or_http()
            .enable_http1()
            .wrap_connector(http_connector);

        let https_client: HyperHttpsClient =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(https_connector);

        let authed_client = tower_http::auth::AddAuthorization::basic(
            https_client,
            username,
            password.expose_secret(),
        )
        .as_sensitive(true);

        let webdav = libdav::dav::WebDavClient::new(uri, authed_client);

        let caldav_client = if use_service_discovery {
            tokio::time::timeout(
                options.request_timeout,
                libdav::CalDavClient::bootstrap_via_service_discovery(webdav),
            )
            .await
            .map_err(|_| Error::Timeout("service discovery".to_string()))?
            .map_err(|e| Error::Protocol(format!("CalDAV service discovery failed: {e}")))?
        } else {
            libdav::CalDavClient::new(webdav)
        };

        Ok(Self {
            inner: caldav_client,
            request_timeout: options.request_timeout,
        })
    }

    async fn protocol<T, E, F>(&self, operation: &'static str, future: F) -> Result<T>
    where
        F: Future<Output = std::result::Result<T, E>>,
        E: std::fmt::Display,
    {
        tokio::time::timeout(self.request_timeout, future)
            .await
            .map_err(|_| Error::Timeout(operation.to_string()))?
            .map_err(|error| Error::Protocol(format!("{operation} failed: {error}")))
    }

    /// Find calendar home set URLs for the current user.
    async fn find_calendar_homes(&self) -> Result<Vec<http::Uri>> {
        let principal = self
            .protocol(
                "find user principal",
                self.inner.find_current_user_principal(),
            )
            .await?;

        match principal {
            Some(principal_uri) => {
                let response = self
                    .protocol(
                        "find calendar home set",
                        self.inner
                            .request(libdav::caldav::FindCalendarHomeSet::new(&principal_uri)),
                    )
                    .await?;
                if response.home_sets.is_empty() {
                    Ok(vec![self.inner.base_url().clone()])
                } else {
                    Ok(response.home_sets)
                }
            },
            None => Ok(vec![self.inner.base_url().clone()]),
        }
    }
}

/// Build the server-side `calendar-query` REPORT that filters a collection
/// by VEVENT time range.
///
/// Per RFC 4791 the `time-range` element must sit inside
/// `comp-filter name="VEVENT"`, nested in `comp-filter name="VCALENDAR"` —
/// servers (e.g. Nextcloud) silently ignore filters at the wrong level.
/// `start`/`end` must be iCalendar UTC basic format (`YYYYMMDDTHHMMSSZ`).
fn build_time_range_query<'a>(
    calendar_href: &'a str,
    start: &'a str,
    end: &'a str,
) -> Result<libdav::caldav::ListCalendarResources<'a>> {
    libdav::caldav::ListCalendarResources::new(calendar_href)
        .with_component_and_time_range("VEVENT", Some(start), Some(end))
        .map_err(|e| Error::Validation(format!("invalid time-range filter: {e}")))
}

struct GetExpandedCalendarResources<'a> {
    inner: libdav::caldav::GetCalendarResources<'a>,
    start: &'a str,
    end: &'a str,
}

impl<'a> GetExpandedCalendarResources<'a> {
    fn new(collection_href: &'a str, hrefs: &[String], start: &'a str, end: &'a str) -> Self {
        Self {
            inner: libdav::caldav::GetCalendarResources::new(collection_href).with_hrefs(hrefs),
            start,
            end,
        }
    }
}

impl libdav::requests::DavRequest for GetExpandedCalendarResources<'_> {
    type Error<E> = libdav::dav::WebDavError<E>;
    type ParseError = libdav::requests::ParseResponseError;
    type Response = libdav::caldav::GetCalendarResourcesResponse;

    fn prepare_request(
        &self,
    ) -> std::result::Result<libdav::requests::PreparedRequest, http::Error> {
        let mut request = self.inner.prepare_request()?;
        request.body = request.body.replacen(
            "<C:calendar-data/>",
            &format!(
                r#"<C:calendar-data><C:expand start="{}" end="{}"/></C:calendar-data>"#,
                self.start, self.end
            ),
            1,
        );
        Ok(request)
    }

    fn parse_response(
        &self,
        parts: &http::response::Parts,
        body: &[u8],
    ) -> std::result::Result<Self::Response, Self::ParseError> {
        self.inner.parse_response(parts, body)
    }
}

async fn resolve_uri_addresses(uri: &http::Uri) -> Result<Vec<SocketAddr>> {
    let host = uri
        .host()
        .ok_or_else(|| Error::InvalidUrl("CalDAV URL must include a host".to_owned()))?;
    let port = uri.port_u16().unwrap_or_else(|| {
        if uri.scheme_str() == Some("http") {
            80
        } else {
            443
        }
    });
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| Error::Protocol(format!("CalDAV DNS resolution failed: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(Error::Protocol(
            "CalDAV DNS resolution returned no addresses".to_owned(),
        ));
    }
    Ok(addresses)
}

fn validate_base_uri(base_url: &str, allow_insecure_http: bool) -> Result<http::Uri> {
    let uri: http::Uri = base_url
        .parse()
        .map_err(|_| Error::InvalidUrl("invalid CalDAV URL".to_string()))?;
    let authority = uri
        .authority()
        .ok_or_else(|| Error::InvalidUrl("CalDAV URL must include a host".to_string()))?;
    if authority.as_str().contains('@') {
        return Err(Error::InvalidUrl(
            "CalDAV URL must not contain credentials".to_string(),
        ));
    }
    match uri.scheme_str() {
        Some("https") => {},
        Some("http") if allow_insecure_http => {},
        Some("http") => {
            return Err(Error::InvalidUrl(
                "CalDAV URL must use HTTPS unless insecure HTTP is explicitly allowed".to_string(),
            ));
        },
        _ => {
            return Err(Error::InvalidUrl(
                "CalDAV URL must use HTTP or HTTPS".to_string(),
            ));
        },
    }
    Ok(uri)
}

#[async_trait]
impl CalDavClient for LibDavCalDavClient {
    async fn list_user_addresses(&self) -> Result<Vec<String>> {
        list_user_addresses_for(&self.inner, self.request_timeout).await
    }

    async fn list_calendars(&self) -> Result<Vec<CalendarInfo>> {
        let homes = self.find_calendar_homes().await?;
        let mut calendars = Vec::new();

        for home_url in &homes {
            let found = self
                .protocol(
                    "find calendars",
                    self.inner
                        .request(libdav::caldav::FindCalendars::new(home_url)),
                )
                .await?;

            for cal in found.calendars {
                let display_name = self
                    .protocol(
                        "get calendar display name",
                        self.inner.request(libdav::dav::GetProperty::new(
                            &cal.href,
                            &libdav::names::DISPLAY_NAME,
                        )),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.value);

                let color = self
                    .protocol(
                        "get calendar colour",
                        self.inner.request(libdav::dav::GetProperty::new(
                            &cal.href,
                            &libdav::names::CALENDAR_COLOUR,
                        )),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.value);

                let description = self
                    .protocol(
                        "get calendar description",
                        self.inner.request(libdav::dav::GetProperty::new(
                            &cal.href,
                            &libdav::names::CALENDAR_DESCRIPTION,
                        )),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.value);

                calendars.push(CalendarInfo {
                    href: cal.href,
                    display_name,
                    color,
                    description,
                    collection_etag: cal.etag,
                    supports_sync: cal.supports_sync,
                });
            }
        }

        Ok(calendars)
    }

    async fn list_events(
        &self,
        calendar_href: &str,
        range: Option<TimeRange>,
    ) -> Result<Vec<EventSummary>> {
        // With a range, ask the server which resources match first
        // (calendar-query REPORT with a VCALENDAR > VEVENT time-range
        // filter), then fetch only those via calendar-multiget. Without a
        // range, fetch everything in the collection.
        let matching_resources = match &range {
            Some(r) => {
                let start = crate::time_filter::to_ical_utc(&r.start)?;
                let end = crate::time_filter::to_ical_utc(&r.end)?;
                let listed = self
                    .protocol(
                        "query calendar time range",
                        self.inner
                            .request(build_time_range_query(calendar_href, &start, &end)?),
                    )
                    .await?;
                if listed.resources.is_empty() {
                    return Ok(Vec::new());
                }
                Some((
                    start,
                    end,
                    listed
                        .resources
                        .into_iter()
                        .map(|resource| resource.href)
                        .collect::<Vec<_>>(),
                ))
            },
            None => None,
        };

        let response = match &matching_resources {
            Some((start, end, hrefs)) => {
                self.protocol(
                    "fetch expanded calendar resources",
                    self.inner.request(GetExpandedCalendarResources::new(
                        calendar_href,
                        hrefs,
                        start,
                        end,
                    )),
                )
                .await?
            },
            None => {
                self.protocol(
                    "fetch calendar resources",
                    self.inner
                        .request(libdav::caldav::GetCalendarResources::new(calendar_href)),
                )
                .await?
            },
        };

        let mut events = Vec::new();
        for resource in &response.resources {
            let content = resource.content.as_ref().map_err(|status| {
                Error::Protocol(format!(
                    "server returned status {} while fetching calendar resource {}",
                    status.as_u16(),
                    resource.href
                ))
            })?;
            let parsed = crate::ical::parse_events(&content.data, &resource.href, &content.etag)
                .map_err(|error| {
                    Error::IcalParse(format!(
                        "failed to parse calendar resource {}: {error}",
                        resource.href
                    ))
                })?;
            events.extend(parsed);
        }

        Ok(events)
    }

    async fn list_event_resources(
        &self,
        calendar_href: &str,
    ) -> Result<Vec<CalendarResourceMetadata>> {
        let request = libdav::caldav::ListCalendarResources::new(calendar_href)
            .with_component("VEVENT")
            .map_err(|error| {
                Error::Protocol(format!("failed to create VEVENT resource query: {error}"))
            })?;
        let response = self
            .protocol("list VEVENT resources", self.inner.request(request))
            .await?;

        convert_listed_resources(response.resources)
    }

    async fn fetch_event_resources(
        &self,
        calendar_href: &str,
        hrefs: &[String],
    ) -> Result<Vec<CalendarObjectResult>> {
        if hrefs.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .protocol(
                "fetch VEVENT resources",
                self.inner.request(
                    libdav::caldav::GetCalendarResources::new(calendar_href).with_hrefs(hrefs),
                ),
            )
            .await?;

        let results = convert_fetched_resources(response.resources);
        if hrefs
            .iter()
            .any(|href| !results.iter().any(|result| result_href(result) == href))
        {
            return Err(Error::Protocol(
                "calendar multiget response omitted one or more requested resources".to_string(),
            ));
        }

        Ok(results)
    }

    async fn create_event(&self, calendar_href: &str, event: NewEvent) -> Result<CreatedEvent> {
        let uid = format!("{}@moltis", uuid::Uuid::new_v4());
        let ical_data = crate::ical::build_vevent(&event, &uid);

        let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);

        let put_request =
            libdav::dav::PutResource::new(&event_href).create(ical_data, "text/calendar");

        let response = self
            .protocol("create event", self.inner.request(put_request))
            .await?;

        Ok(CreatedEvent {
            href: event_href,
            etag: response.etag,
            uid,
        })
    }

    async fn update_event(
        &self,
        href: &str,
        etag: &str,
        updates: UpdateEvent,
    ) -> Result<UpdatedEvent> {
        // First fetch the existing resource to get current iCal data
        let resources = self
            .protocol(
                "fetch event for update",
                self.inner
                    .request(libdav::caldav::GetCalendarResources::new(href).with_hrefs([href])),
            )
            .await?;

        let resource = resources
            .resources
            .first()
            .ok_or_else(|| Error::NotFound(format!("event not found at {href}")))?;

        let content = resource.content.as_ref().map_err(|status| {
            Error::Protocol(format!("server returned {status} for event at {href}"))
        })?;

        let merged = crate::ical::merge_updates(&content.data, &updates)?;

        let put_request = libdav::dav::PutResource::new(href).update(merged, "text/calendar", etag);

        let response = self
            .protocol("update event", self.inner.request(put_request))
            .await?;

        Ok(UpdatedEvent {
            href: href.to_string(),
            etag: response.etag,
        })
    }

    async fn delete_event(&self, href: &str, etag: &str) -> Result<()> {
        let delete_request = libdav::dav::Delete::new(href).with_etag(etag);

        self.protocol("delete event", self.inner.request(delete_request))
            .await?;

        Ok(())
    }
}

async fn list_user_addresses_for<C>(
    client: &libdav::CalDavClient<C>,
    request_timeout: Duration,
) -> Result<Vec<String>>
where
    C: Service<http::Request<String>, Response = http::Response<hyper::body::Incoming>>
        + Send
        + Sync
        + 'static,
    C::Error: std::error::Error + Send + Sync,
{
    let principal = tokio::time::timeout(request_timeout, client.find_current_user_principal())
        .await
        .map_err(|_| Error::Timeout("find user principal".to_string()))?
        .map_err(|error| Error::Protocol(format!("find user principal failed: {error}")))?;
    let Some(principal) = principal else {
        return Ok(Vec::new());
    };
    let response = tokio::time::timeout(
        request_timeout,
        client.request(libdav::caldav::GetUserAddressSet::new(&principal)),
    )
    .await
    .map_err(|_| Error::Timeout("get calendar user address set".to_string()))?
    .map_err(|error| Error::Protocol(format!("get calendar user address set failed: {error}")))?;
    Ok(response.addresses)
}

fn convert_listed_resources(
    resources: Vec<libdav::dav::ListedResource>,
) -> Result<Vec<CalendarResourceMetadata>> {
    resources
        .into_iter()
        .map(|resource| {
            if let Some(status) = resource.status
                && !status.is_success()
            {
                return Err(Error::Protocol(format!(
                    "server returned status {} while listing calendar resource {}",
                    status.as_u16(),
                    resource.href
                )));
            }
            Ok(CalendarResourceMetadata {
                href: resource.href,
                etag: resource.etag,
            })
        })
        .collect()
}

fn convert_fetched_resources(resources: Vec<libdav::FetchedResource>) -> Vec<CalendarObjectResult> {
    resources
        .into_iter()
        .map(|resource| match resource.content {
            Ok(content) => CalendarObjectResult::Found(CalendarObject {
                href: resource.href,
                etag: Some(content.etag),
                ical: content.data,
            }),
            Err(status) if status == http::StatusCode::NOT_FOUND => {
                CalendarObjectResult::NotFound {
                    href: resource.href,
                }
            },
            Err(status) => CalendarObjectResult::Failed {
                href: resource.href,
                status: status.as_u16(),
            },
        })
        .collect()
}

fn result_href(result: &CalendarObjectResult) -> &str {
    match result {
        CalendarObjectResult::Found(object) => &object.href,
        CalendarObjectResult::NotFound { href } | CalendarObjectResult::Failed { href, .. } => href,
    }
}

/// Thread-safe shared CalDAV client.
pub type SharedCalDavClient = Arc<dyn CalDavClient>;

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        pin::Pin,
        str::FromStr,
        sync::Mutex,
        task::{Context, Poll},
    };

    use {
        hyper::{
            Request, Response,
            body::{Body, Bytes, Frame, SizeHint},
            service::service_fn,
        },
        hyper_util::rt::TokioIo,
    };

    use {super::*, libdav::requests::DavRequest};

    #[test]
    fn time_range_query_nests_time_range_under_vcalendar_vevent() {
        let query =
            build_time_range_query("/cal/personal/", "20260101T000000Z", "20260201T000000Z")
                .unwrap();
        let prepared = query.prepare_request().unwrap();

        assert_eq!(
            prepared.method,
            http::Method::from_bytes(b"REPORT").unwrap()
        );
        assert!(prepared.body.contains(concat!(
            r#"<C:comp-filter name="VCALENDAR">"#,
            r#"<C:comp-filter name="VEVENT">"#,
            r#"<C:time-range start="20260101T000000Z" end="20260201T000000Z"/>"#,
            r#"</C:comp-filter></C:comp-filter>"#,
        )));
    }

    #[test]
    fn expanded_resource_query_uses_requested_range() {
        let request = GetExpandedCalendarResources::new(
            "/cal/personal/",
            &["/cal/personal/series.ics".to_string()],
            "20260201T000000Z",
            "20260301T000000Z",
        );
        let prepared = request.prepare_request().unwrap();

        assert!(prepared.body.contains(concat!(
            r#"<C:calendar-data><C:expand start="20260201T000000Z" "#,
            r#"end="20260301T000000Z"/></C:calendar-data>"#,
        )));
        assert!(
            prepared
                .body
                .contains("<D:href>/cal/personal/series.ics</D:href>")
        );
    }

    #[test]
    fn expanded_resource_response_returns_recurring_occurrence_in_range() {
        let request = GetExpandedCalendarResources::new(
            "/cal/personal/",
            &["/cal/personal/series.ics".to_string()],
            "20260201T000000Z",
            "20260301T000000Z",
        );
        let response = Response::builder()
            .status(http::StatusCode::MULTI_STATUS)
            .body(())
            .unwrap();
        let (parts, ()) = response.into_parts();
        let body = br#"<?xml version="1.0" encoding="utf-8"?>
<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <response>
    <href>/cal/personal/series.ics</href>
    <propstat>
      <prop>
        <getetag>"series-etag"</getetag>
        <C:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:daily-series@example.test
RECURRENCE-ID:20260205T090000Z
SUMMARY:Daily standup
DTSTART:20260205T090000Z
DTEND:20260205T093000Z
END:VEVENT
END:VCALENDAR
</C:calendar-data>
      </prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
</multistatus>"#;

        let fetched = request.parse_response(&parts, body).unwrap();
        let content = fetched.resources[0].content.as_ref().unwrap();
        let events =
            crate::ical::parse_events(&content.data, &fetched.resources[0].href, &content.etag)
                .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid.as_deref(), Some("daily-series@example.test"));
        assert_eq!(events[0].start.as_deref(), Some("2026-02-05T09:00:00"));
    }

    #[test]
    fn time_range_query_uses_utc_z_timestamps_from_iso_input() {
        let start = crate::time_filter::to_ical_utc("2026-01-01T02:00:00+02:00").unwrap();
        let end = crate::time_filter::to_ical_utc("2026-02-01").unwrap();
        let query = build_time_range_query("/cal/personal/", &start, &end).unwrap();
        let prepared = query.prepare_request().unwrap();

        assert!(
            prepared
                .body
                .contains(r#"<C:time-range start="20260101T000000Z" end="20260201T000000Z"/>"#)
        );
    }

    #[test]
    fn time_range_query_rejects_non_utc_timestamps() {
        let result = build_time_range_query("/cal/personal/", "2026-01-01T00:00:00", "2026-02-01");
        assert!(matches!(result, Err(Error::Validation(_))));
    }

    #[tokio::test]
    async fn pinned_resolver_never_performs_a_second_hostname_lookup() {
        let address = SocketAddr::from(([203, 0, 113, 7], 8443));
        let mut resolver = PinnedResolver {
            addresses: vec![address].into(),
        };

        let resolved = resolver
            .call(Name::from_str("different.invalid").unwrap())
            .await
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(resolved, vec![address]);
    }

    struct TestBody(Option<Bytes>);

    impl TestBody {
        fn new(value: &'static str) -> Self {
            Self(Some(Bytes::from_static(value.as_bytes())))
        }
    }

    impl Body for TestBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(self.0.take().map(|data| Ok(Frame::data(data))))
        }

        fn size_hint(&self) -> SizeHint {
            let length = self.0.as_ref().map_or(0, |data| data.len()) as u64;
            SizeHint::with_exact(length)
        }
    }

    #[tokio::test]
    async fn lists_multiple_user_addresses_for_authenticated_principal() {
        const PRINCIPAL_RESPONSE: &str = r#"<?xml version="1.0"?>
            <multistatus xmlns="DAV:">
                <response>
                    <href>/dav/</href>
                    <propstat><prop><current-user-principal>
                        <href>/principals/calendar-user/</href>
                    </current-user-principal></prop>
                    <status>HTTP/1.1 200 OK</status></propstat>
                </response>
            </multistatus>"#;
        const ADDRESS_RESPONSE: &str = r#"<?xml version="1.0"?>
            <multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
                <response>
                    <href>/principals/calendar-user/</href>
                    <propstat><prop><C:calendar-user-address-set>
                        <href>mailto:accepted@example.test</href>
                        <href>mailto:alias@example.test</href>
                    </C:calendar-user-address-set></prop>
                    <status>HTTP/1.1 200 OK</status></propstat>
                </response>
            </multistatus>"#;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |request: Request<hyper::body::Incoming>| {
                        let requests = Arc::clone(&server_requests);
                        async move {
                            requests.lock().unwrap().push((
                                request.uri().path().to_string(),
                                request
                                    .headers()
                                    .get(http::header::AUTHORIZATION)
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_string),
                            ));
                            let body = match request.uri().path() {
                                "/dav/" => PRINCIPAL_RESPONSE,
                                "/principals/calendar-user/" => ADDRESS_RESPONSE,
                                path => panic!("unexpected request path: {path}"),
                            };
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(http::StatusCode::MULTI_STATUS)
                                    .header(http::header::CONTENT_TYPE, "application/xml")
                                    .body(TestBody::new(body))
                                    .unwrap(),
                            )
                        }
                    }),
                )
                .await
                .unwrap();
        });

        let http_client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(HttpConnector::new());
        let http_client =
            tower_http::auth::AddAuthorization::basic(http_client, "login-user", "password")
                .as_sensitive(true);
        let webdav = libdav::dav::WebDavClient::new(
            format!("http://{address}/dav/").parse().unwrap(),
            http_client,
        );
        let client = libdav::CalDavClient::new(webdav);

        assert_eq!(
            list_user_addresses_for(&client, Duration::from_secs(2))
                .await
                .unwrap(),
            vec![
                "mailto:accepted@example.test".to_string(),
                "mailto:alias@example.test".to_string(),
            ]
        );
        drop(client);
        server.await.unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|(_, authorization)| authorization.as_deref()
                    == Some("Basic bG9naW4tdXNlcjpwYXNzd29yZA=="))
        );
    }

    #[test]
    fn converts_listed_resource_metadata() {
        let resources = vec![libdav::dav::ListedResource {
            href: "/calendars/work/event-1.ics".to_string(),
            status: Some(http::StatusCode::OK),
            content_type: Some("text/calendar".to_string()),
            etag: Some("\"etag-1\"".to_string()),
            resource_type: libdav::ResourceType::default(),
        }];

        let converted = convert_listed_resources(resources).unwrap();

        assert_eq!(converted, vec![CalendarResourceMetadata {
            href: "/calendars/work/event-1.ics".to_string(),
            etag: Some("\"etag-1\"".to_string()),
        }]);
    }

    #[test]
    fn converts_each_fetched_resource_result() {
        let resources = vec![
            libdav::FetchedResource {
                href: "/calendars/work/found.ics".to_string(),
                content: Ok(libdav::FetchedResourceContent {
                    data: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_string(),
                    etag: "\"etag-found\"".to_string(),
                }),
            },
            libdav::FetchedResource {
                href: "/calendars/work/missing.ics".to_string(),
                content: Err(http::StatusCode::NOT_FOUND),
            },
            libdav::FetchedResource {
                href: "/calendars/work/forbidden.ics".to_string(),
                content: Err(http::StatusCode::FORBIDDEN),
            },
        ];

        let converted = convert_fetched_resources(resources);

        assert_eq!(converted, vec![
            CalendarObjectResult::Found(CalendarObject {
                href: "/calendars/work/found.ics".to_string(),
                etag: Some("\"etag-found\"".to_string()),
                ical: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".to_string(),
            }),
            CalendarObjectResult::NotFound {
                href: "/calendars/work/missing.ics".to_string(),
            },
            CalendarObjectResult::Failed {
                href: "/calendars/work/forbidden.ics".to_string(),
                status: 403,
            },
        ]);
    }

    #[test]
    fn insecure_http_requires_explicit_option() {
        assert!(validate_base_uri("http://calendar.example.test", false).is_err());
        assert!(validate_base_uri("http://calendar.example.test", true).is_ok());
        assert!(validate_base_uri("https://calendar.example.test", false).is_ok());
    }
}
