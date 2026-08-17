//! Typed structs for CalDAV operations.

use serde::{Deserialize, Serialize};

/// Information about a discovered calendar collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    /// Server-relative href path to the calendar.
    pub href: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// Calendar colour (CSS hex string, e.g. "#FF5733").
    pub color: Option<String>,
    /// Optional description text.
    pub description: Option<String>,
    /// ETag for the calendar collection, when advertised by the server.
    #[serde(default)]
    pub collection_etag: Option<String>,
    /// Whether the collection advertises RFC 6578 sync support.
    #[serde(default)]
    pub supports_sync: bool,
}

/// Metadata for a VEVENT resource in a calendar collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarResourceMetadata {
    /// Server-relative href path to the calendar object.
    pub href: String,
    /// ETag for change detection, when supplied by the server.
    pub etag: Option<String>,
}

/// A fetched calendar object. The raw iCalendar data is canonical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarObject {
    /// Server-relative href path to the calendar object.
    pub href: String,
    /// ETag for change detection, when supplied by the server.
    pub etag: Option<String>,
    /// Unmodified iCalendar resource data.
    pub ical: String,
}

/// Per-resource result from a calendar multiget request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarObjectResult {
    /// The resource was fetched successfully.
    Found(CalendarObject),
    /// The resource no longer exists.
    NotFound { href: String },
    /// The server failed to return this resource.
    Failed { href: String, status: u16 },
}

/// Summary of a single calendar event (returned by `list_events`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSummary {
    /// Server-relative href path to the event resource.
    pub href: String,
    /// ETag for conditional updates.
    pub etag: String,
    /// iCalendar UID.
    pub uid: Option<String>,
    /// Event title.
    pub summary: Option<String>,
    /// Start date/time as ISO 8601 string.
    pub start: Option<String>,
    /// End date/time as ISO 8601 string.
    pub end: Option<String>,
    /// Whether this is an all-day event.
    pub all_day: bool,
    /// Event location.
    pub location: Option<String>,
}

/// Parameters for creating a new event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEvent {
    /// Event title (required).
    pub summary: String,
    /// Start date/time as ISO 8601 string (required).
    pub start: String,
    /// End date/time as ISO 8601 string (optional; defaults to start + 1h).
    pub end: Option<String>,
    /// Whether this is an all-day event.
    #[serde(default)]
    pub all_day: bool,
    /// Event location.
    pub location: Option<String>,
    /// Event description.
    pub description: Option<String>,
}

/// Fields that can be updated on an existing event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateEvent {
    pub summary: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_day: Option<bool>,
    pub location: Option<String>,
    pub description: Option<String>,
}

/// Result of a successful event creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedEvent {
    pub href: String,
    pub etag: Option<String>,
    pub uid: String,
}

/// Result of a successful event update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatedEvent {
    pub href: String,
    pub etag: Option<String>,
}

/// Time range filter for listing events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start of range as ISO 8601 string.
    pub start: String,
    /// End of range as ISO 8601 string.
    pub end: String,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_info_serde_roundtrip() {
        let info = CalendarInfo {
            href: "/dav/calendars/work".into(),
            display_name: Some("Work".into()),
            color: Some("#FF5733".into()),
            description: None,
            collection_etag: Some("\"collection-1\"".into()),
            supports_sync: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: CalendarInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.href, "/dav/calendars/work");
        assert_eq!(parsed.display_name.as_deref(), Some("Work"));
        assert_eq!(parsed.collection_etag.as_deref(), Some("\"collection-1\""));
        assert!(parsed.supports_sync);
    }

    #[test]
    fn calendar_info_new_fields_default_when_absent() {
        let parsed: CalendarInfo = serde_json::from_str(
            r#"{"href":"/dav/calendars/work","display_name":null,"color":null,"description":null}"#,
        )
        .unwrap();

        assert!(parsed.collection_etag.is_none());
        assert!(!parsed.supports_sync);
    }

    #[test]
    fn new_event_serde_roundtrip() {
        let event = NewEvent {
            summary: "Meeting".into(),
            start: "2025-06-15T10:00:00".into(),
            end: Some("2025-06-15T11:00:00".into()),
            all_day: false,
            location: Some("Room 42".into()),
            description: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: NewEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.summary, "Meeting");
        assert_eq!(parsed.start, "2025-06-15T10:00:00");
    }

    #[test]
    fn event_summary_serde_roundtrip() {
        let summary = EventSummary {
            href: "/cal/event1.ics".into(),
            etag: "\"abc123\"".into(),
            uid: Some("uid-1@example.com".into()),
            summary: Some("Lunch".into()),
            start: Some("2025-06-15T12:00:00".into()),
            end: Some("2025-06-15T13:00:00".into()),
            all_day: false,
            location: None,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: EventSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.etag, "\"abc123\"");
        assert_eq!(parsed.uid.as_deref(), Some("uid-1@example.com"));
    }

    #[test]
    fn update_event_defaults_all_none() {
        let update = UpdateEvent::default();
        assert!(update.summary.is_none());
        assert!(update.start.is_none());
        assert!(update.end.is_none());
        assert!(update.all_day.is_none());
        assert!(update.location.is_none());
        assert!(update.description.is_none());
    }
}
