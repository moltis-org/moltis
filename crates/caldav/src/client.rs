//! CalDAV client trait and `libdav`-backed implementation.

use std::sync::Arc;

use {
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
};

use crate::types::{
    CalendarInfo, CreatedEvent, EventSummary, NewEvent, TimeRange, UpdateEvent, UpdatedEvent,
};

/// Trait for CalDAV server interactions.
///
/// This allows mocking in tests without a real server.
#[async_trait]
pub trait CalDavClient: Send + Sync {
    /// Discover calendars available on this account.
    async fn list_calendars(&self) -> Result<Vec<CalendarInfo>>;

    /// List events in a calendar, optionally filtered by time range.
    async fn list_events(
        &self,
        calendar_href: &str,
        range: Option<TimeRange>,
    ) -> Result<Vec<EventSummary>>;

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
    inner: libdav::CalDavClient<
        tower_http::auth::AddAuthorization<
            hyper_util::client::legacy::Client<
                hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
                String,
            >,
        >,
    >,
}

/// Type alias for the libdav HTTPS connector stack.
type HyperHttpsClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    String,
>;

impl LibDavCalDavClient {
    /// Connect to a CalDAV server.
    ///
    /// Uses service discovery to locate the CalDAV context path.
    pub async fn connect(
        base_url: &str,
        username: &str,
        password: &Secret<String>,
    ) -> Result<Self> {
        let uri: http::Uri = base_url
            .parse()
            .map_err(|e| anyhow!("invalid CalDAV URL '{base_url}': {e}"))?;

        let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|e| anyhow!("failed to load native TLS roots: {e}"))?
            .https_or_http()
            .enable_http1()
            .build();

        let https_client: HyperHttpsClient =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(https_connector);

        let authed_client = tower_http::auth::AddAuthorization::basic(
            https_client,
            username,
            password.expose_secret(),
        );

        let webdav = libdav::dav::WebDavClient::new(uri, authed_client);

        let caldav_client = libdav::CalDavClient::bootstrap_via_service_discovery(webdav)
            .await
            .map_err(|e| anyhow!("CalDAV service discovery failed: {e}"))?;

        Ok(Self {
            inner: caldav_client,
        })
    }

    /// Find calendar home set URLs for the current user.
    async fn find_calendar_homes(&self) -> Result<Vec<http::Uri>> {
        let principal = self
            .inner
            .find_current_user_principal()
            .await
            .map_err(|e| anyhow!("failed to find user principal: {e}"))?;

        match principal {
            Some(principal_uri) => {
                let response = self
                    .inner
                    .request(libdav::caldav::FindCalendarHomeSet::new(&principal_uri))
                    .await
                    .map_err(|e| anyhow!("failed to find calendar home set: {e}"))?;
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

#[async_trait]
impl CalDavClient for LibDavCalDavClient {
    async fn list_calendars(&self) -> Result<Vec<CalendarInfo>> {
        let homes = self.find_calendar_homes().await?;
        let mut calendars = Vec::new();

        for home_url in &homes {
            let found = self
                .inner
                .request(libdav::caldav::FindCalendars::new(home_url))
                .await
                .map_err(|e| anyhow!("failed to find calendars: {e}"))?;

            for cal in found.calendars {
                let display_name = self
                    .inner
                    .request(libdav::dav::GetProperty::new(
                        &cal.href,
                        &libdav::names::DISPLAY_NAME,
                    ))
                    .await
                    .ok()
                    .and_then(|r| r.value);

                let color = self
                    .inner
                    .request(libdav::dav::GetProperty::new(
                        &cal.href,
                        &libdav::names::CALENDAR_COLOUR,
                    ))
                    .await
                    .ok()
                    .and_then(|r| r.value);

                let description = self
                    .inner
                    .request(libdav::dav::GetProperty::new(
                        &cal.href,
                        &libdav::names::CALENDAR_DESCRIPTION,
                    ))
                    .await
                    .ok()
                    .and_then(|r| r.value);

                calendars.push(CalendarInfo {
                    href: cal.href,
                    display_name,
                    color,
                    description,
                });
            }
        }

        Ok(calendars)
    }

    async fn list_events(
        &self,
        calendar_href: &str,
        _range: Option<TimeRange>,
    ) -> Result<Vec<EventSummary>> {
        // Fetch all calendar resources (iCal data + etags)
        let response = self
            .inner
            .request(libdav::caldav::GetCalendarResources::new(calendar_href))
            .await
            .map_err(|e| anyhow!("failed to fetch calendar resources: {e}"))?;

        let mut events = Vec::new();
        for resource in &response.resources {
            if let Ok(ref content) = resource.content {
                match crate::ical::parse_events(&content.data, &resource.href, &content.etag) {
                    Ok(parsed) => events.extend(parsed),
                    Err(e) => {
                        #[cfg(feature = "tracing")]
                        tracing::debug!(
                            href = %resource.href,
                            error = %e,
                            "skipping unparseable calendar resource"
                        );
                    },
                }
            }
        }

        Ok(events)
    }

    async fn create_event(&self, calendar_href: &str, event: NewEvent) -> Result<CreatedEvent> {
        let uid = format!("{}@moltis", uuid::Uuid::new_v4());
        let ical_data = crate::ical::build_vevent(&event, &uid);

        let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);

        let put_request =
            libdav::dav::PutResource::new(&event_href).create(ical_data, "text/calendar");

        let response = self
            .inner
            .request(put_request)
            .await
            .map_err(|e| anyhow!("failed to create event: {e}"))?;

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
            .inner
            .request(libdav::caldav::GetCalendarResources::new(href).with_hrefs([href]))
            .await
            .map_err(|e| anyhow!("failed to fetch event for update: {e}"))?;

        let resource = resources
            .resources
            .first()
            .ok_or_else(|| anyhow!("event not found at {href}"))?;

        let content = resource
            .content
            .as_ref()
            .map_err(|status| anyhow!("server returned {status} for event at {href}"))?;

        let merged = crate::ical::merge_updates(&content.data, &updates)?;

        let put_request = libdav::dav::PutResource::new(href).update(merged, "text/calendar", etag);

        let response = self
            .inner
            .request(put_request)
            .await
            .map_err(|e| anyhow!("failed to update event: {e}"))?;

        Ok(UpdatedEvent {
            href: href.to_string(),
            etag: response.etag,
        })
    }

    async fn delete_event(&self, href: &str, etag: &str) -> Result<()> {
        let delete_request = libdav::dav::Delete::new(href).with_etag(etag);

        self.inner
            .request(delete_request)
            .await
            .map_err(|e| anyhow!("failed to delete event: {e}"))?;

        Ok(())
    }
}

/// Thread-safe shared CalDAV client.
pub type SharedCalDavClient = Arc<dyn CalDavClient>;
