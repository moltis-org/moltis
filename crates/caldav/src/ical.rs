//! iCalendar build/parse helpers using the `icalendar` crate.

use {
    anyhow::{Result, anyhow},
    icalendar::{Calendar, Component, Event, EventLike},
};

use crate::types::{EventSummary, NewEvent, UpdateEvent};

/// Build a VCALENDAR string containing a single VEVENT from the given parameters.
#[must_use]
pub fn build_vevent(event: &NewEvent, uid: &str) -> String {
    let mut vevent = Event::new();
    vevent.uid(uid);
    vevent.summary(&event.summary);

    if event.all_day {
        // All-day events use DATE values (YYYY-MM-DD)
        vevent.add_property("DTSTART;VALUE=DATE", event.start.replace('-', ""));
        if let Some(ref end) = event.end {
            vevent.add_property("DTEND;VALUE=DATE", end.replace('-', ""));
        }
    } else {
        vevent.add_property("DTSTART", format_datetime(&event.start));
        if let Some(ref end) = event.end {
            vevent.add_property("DTEND", format_datetime(end));
        }
    }

    if let Some(ref loc) = event.location {
        vevent.location(loc);
    }
    if let Some(ref desc) = event.description {
        vevent.description(desc);
    }

    let cal = Calendar::new().push(vevent).done();
    cal.to_string()
}

/// Parse raw iCalendar data and extract event summaries.
pub fn parse_events(ical_data: &str, href: &str, etag: &str) -> Result<Vec<EventSummary>> {
    let calendar: Calendar = ical_data
        .parse()
        .map_err(|e| anyhow!("failed to parse iCalendar data: {e}"))?;

    let mut events = Vec::new();
    for component in &calendar.components {
        if let icalendar::CalendarComponent::Event(vevent) = component {
            let uid = vevent.property_value("UID").map(String::from);
            let summary = vevent.property_value("SUMMARY").map(String::from);
            let start = extract_datetime(vevent, "DTSTART");
            let end = extract_datetime(vevent, "DTEND");
            let all_day = vevent
                .property_value("DTSTART")
                .is_some_and(|v| v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()));
            let location = vevent.property_value("LOCATION").map(String::from);

            events.push(EventSummary {
                href: href.to_string(),
                etag: etag.to_string(),
                uid,
                summary,
                start,
                end,
                all_day,
                location,
            });
        }
    }

    Ok(events)
}

/// Merge updates into an existing iCalendar string, preserving unmodified properties.
pub fn merge_updates(existing: &str, updates: &UpdateEvent) -> Result<String> {
    let mut calendar: Calendar = existing
        .parse()
        .map_err(|e| anyhow!("failed to parse existing iCalendar data: {e}"))?;

    let mut new_components = Vec::new();
    for component in calendar.components.drain(..) {
        if let icalendar::CalendarComponent::Event(mut vevent) = component {
            if let Some(ref summary) = updates.summary {
                vevent.summary(summary);
            }
            if let Some(ref start) = updates.start {
                let all_day = updates.all_day.unwrap_or(false);
                vevent.remove_property("DTSTART");
                if all_day {
                    vevent.add_property("DTSTART;VALUE=DATE", start.replace('-', ""));
                } else {
                    vevent.add_property("DTSTART", format_datetime(start));
                }
            }
            if let Some(ref end) = updates.end {
                let all_day = updates.all_day.unwrap_or(false);
                vevent.remove_property("DTEND");
                if all_day {
                    vevent.add_property("DTEND;VALUE=DATE", end.replace('-', ""));
                } else {
                    vevent.add_property("DTEND", format_datetime(end));
                }
            }
            if let Some(ref loc) = updates.location {
                vevent.location(loc);
            }
            if let Some(ref desc) = updates.description {
                vevent.description(desc);
            }
            new_components.push(icalendar::CalendarComponent::Event(vevent));
        } else {
            new_components.push(component);
        }
    }
    calendar.components = new_components;
    Ok(calendar.to_string())
}

/// Format a date/time string as iCalendar DATETIME (basic format).
/// Accepts ISO 8601 (`2025-06-15T10:00:00`) and converts to `20250615T100000`.
fn format_datetime(dt: &str) -> String {
    // If already in basic format, return as-is
    if !dt.contains('-') {
        return dt.to_string();
    }
    // Strip dashes and colons for basic iCalendar format
    dt.replace(['-', ':'], "")
}

/// Extract a date/time property and normalise to ISO 8601 for display.
fn extract_datetime(vevent: &Event, prop_name: &str) -> Option<String> {
    let raw = vevent.property_value(prop_name)?;
    Some(normalise_datetime(raw))
}

/// Convert basic iCalendar datetime (20250615T100000) to ISO 8601 (2025-06-15T10:00:00).
fn normalise_datetime(raw: &str) -> String {
    let raw = raw.trim_end_matches('Z');
    if raw.len() == 8 && raw.chars().all(|c| c.is_ascii_digit()) {
        // DATE value: YYYYMMDD -> YYYY-MM-DD
        format!("{}-{}-{}", &raw[..4], &raw[4..6], &raw[6..8])
    } else if raw.len() >= 15 && raw.contains('T') {
        // DATETIME value: YYYYMMDDTHHMMSS -> YYYY-MM-DDTHH:MM:SS
        let date_part = &raw[..8];
        let time_part = &raw[9..];
        let date = format!(
            "{}-{}-{}",
            &date_part[..4],
            &date_part[4..6],
            &date_part[6..8]
        );
        if time_part.len() >= 6 {
            let time = format!(
                "{}:{}:{}",
                &time_part[..2],
                &time_part[2..4],
                &time_part[4..6]
            );
            format!("{date}T{time}")
        } else {
            date
        }
    } else {
        // Already in a reasonable format or unknown — return as-is
        raw.to_string()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_vevent_basic() {
        let event = NewEvent {
            summary: "Team meeting".into(),
            start: "2025-06-15T10:00:00".into(),
            end: Some("2025-06-15T11:00:00".into()),
            all_day: false,
            location: Some("Room A".into()),
            description: Some("Weekly sync".into()),
        };
        let ical = build_vevent(&event, "test-uid-123@moltis");
        assert!(ical.contains("BEGIN:VCALENDAR"));
        assert!(ical.contains("BEGIN:VEVENT"));
        assert!(ical.contains("SUMMARY:Team meeting"));
        assert!(ical.contains("test-uid-123@moltis"));
        assert!(ical.contains("LOCATION:Room A"));
        assert!(ical.contains("DESCRIPTION:Weekly sync"));
        assert!(ical.contains("DTSTART:20250615T100000"));
        assert!(ical.contains("DTEND:20250615T110000"));
    }

    #[test]
    fn build_vevent_all_day() {
        let event = NewEvent {
            summary: "Holiday".into(),
            start: "2025-12-25".into(),
            end: Some("2025-12-26".into()),
            all_day: true,
            location: None,
            description: None,
        };
        let ical = build_vevent(&event, "holiday-uid@moltis");
        assert!(ical.contains("DTSTART;VALUE=DATE:20251225"));
        assert!(ical.contains("DTEND;VALUE=DATE:20251226"));
    }

    #[test]
    fn parse_events_roundtrip() {
        let event = NewEvent {
            summary: "Test event".into(),
            start: "2025-06-15T14:00:00".into(),
            end: Some("2025-06-15T15:00:00".into()),
            all_day: false,
            location: None,
            description: None,
        };
        let ical = build_vevent(&event, "roundtrip-uid@test");
        let parsed = parse_events(&ical, "/cal/test.ics", "\"etag1\"").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].summary.as_deref(), Some("Test event"));
        assert_eq!(parsed[0].uid.as_deref(), Some("roundtrip-uid@test"));
        assert_eq!(parsed[0].href, "/cal/test.ics");
        assert_eq!(parsed[0].etag, "\"etag1\"");
    }

    #[test]
    fn normalise_datetime_basic_to_iso() {
        assert_eq!(normalise_datetime("20250615T100000"), "2025-06-15T10:00:00");
        assert_eq!(
            normalise_datetime("20250615T100000Z"),
            "2025-06-15T10:00:00"
        );
    }

    #[test]
    fn normalise_datetime_date_only() {
        assert_eq!(normalise_datetime("20251225"), "2025-12-25");
    }

    #[test]
    fn format_datetime_iso_to_basic() {
        assert_eq!(format_datetime("2025-06-15T10:00:00"), "20250615T100000");
    }

    #[test]
    fn format_datetime_already_basic() {
        assert_eq!(format_datetime("20250615T100000"), "20250615T100000");
    }

    #[test]
    fn merge_updates_changes_summary() {
        let event = NewEvent {
            summary: "Original".into(),
            start: "2025-06-15T10:00:00".into(),
            end: None,
            all_day: false,
            location: None,
            description: None,
        };
        let ical = build_vevent(&event, "merge-uid@test");
        let updates = UpdateEvent {
            summary: Some("Updated title".into()),
            ..Default::default()
        };
        let merged = merge_updates(&ical, &updates).unwrap();
        assert!(merged.contains("SUMMARY:Updated title"));
    }
}
