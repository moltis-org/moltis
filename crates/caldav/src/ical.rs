//! iCalendar build/parse helpers using the `icalendar` crate.

use std::collections::BTreeMap;

use icalendar::{Calendar, Component, Event, EventLike};

use crate::{
    error::{Error, Result},
    types::{EventSummary, NewEvent, UpdateEvent},
};

/// A property value and its RFC 5545 parameters.
///
/// Values remain escaped exactly as they appear after content-line unfolding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexedProperty {
    pub value: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, Vec<String>>,
}

/// Read-side index of recurrence-relevant properties from one VEVENT.
///
/// The index does not expand recurrence and is not a replacement for the raw
/// iCalendar resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventComponentIndex {
    pub uid: Option<IndexedProperty>,
    pub summary: Option<IndexedProperty>,
    pub description: Option<IndexedProperty>,
    pub location: Option<IndexedProperty>,
    pub organizer: Option<IndexedProperty>,
    #[serde(default)]
    pub attendees: Vec<IndexedProperty>,
    pub dtstart: Option<IndexedProperty>,
    pub dtend: Option<IndexedProperty>,
    pub recurrence_id: Option<IndexedProperty>,
    pub status: Option<IndexedProperty>,
    #[serde(default)]
    pub rrule: Vec<IndexedProperty>,
    #[serde(default)]
    pub rdate: Vec<IndexedProperty>,
    #[serde(default)]
    pub exdate: Vec<IndexedProperty>,
}

/// Index VEVENT properties from a raw VCALENDAR without expanding recurrence.
///
/// Content lines are unfolded before parsing. Property values are retained as
/// written, while property and parameter names are normalised to uppercase.
pub fn index_event_components(ical_data: &str) -> Result<Vec<EventComponentIndex>> {
    let lines = unfold_content_lines(ical_data)?;
    let mut indexes = Vec::new();
    let mut current_event: Option<EventComponentIndex> = None;
    let mut nested_component_depth = 0_u32;
    let mut saw_calendar = false;

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, property) = parse_content_line(&line)?;
        let value_upper = property.value.to_ascii_uppercase();

        if name == "BEGIN" {
            if value_upper == "VCALENDAR" {
                saw_calendar = true;
            } else if value_upper == "VEVENT" && current_event.is_none() {
                if !saw_calendar {
                    return Err(Error::IcalParse(
                        "VEVENT appears outside VCALENDAR".to_string(),
                    ));
                }
                current_event = Some(EventComponentIndex::default());
            } else if current_event.is_some() {
                nested_component_depth = nested_component_depth.saturating_add(1);
            }
            continue;
        }

        if name == "END" {
            if current_event.is_some() && nested_component_depth > 0 {
                nested_component_depth -= 1;
            } else if value_upper == "VEVENT" {
                let event = current_event.take().ok_or_else(|| {
                    Error::IcalParse("VEVENT end appears without a matching begin".to_string())
                })?;
                indexes.push(event);
            } else if value_upper == "VCALENDAR" && current_event.is_some() {
                return Err(Error::IcalParse(
                    "VCALENDAR ended before VEVENT".to_string(),
                ));
            }
            continue;
        }

        if nested_component_depth == 0
            && let Some(event) = current_event.as_mut()
        {
            match name.as_str() {
                "UID" if event.uid.is_none() => event.uid = Some(property),
                "SUMMARY" if event.summary.is_none() => event.summary = Some(property),
                "DESCRIPTION" if event.description.is_none() => event.description = Some(property),
                "LOCATION" if event.location.is_none() => event.location = Some(property),
                "ORGANIZER" if event.organizer.is_none() => event.organizer = Some(property),
                "ATTENDEE" => event.attendees.push(property),
                "DTSTART" if event.dtstart.is_none() => event.dtstart = Some(property),
                "DTEND" if event.dtend.is_none() => event.dtend = Some(property),
                "RECURRENCE-ID" if event.recurrence_id.is_none() => {
                    event.recurrence_id = Some(property);
                },
                "STATUS" if event.status.is_none() => event.status = Some(property),
                "RRULE" => event.rrule.push(property),
                "RDATE" => event.rdate.push(property),
                "EXDATE" => event.exdate.push(property),
                _ => {},
            }
        }
    }

    if current_event.is_some() {
        return Err(Error::IcalParse("unterminated VEVENT".to_string()));
    }
    if !saw_calendar {
        return Err(Error::IcalParse(
            "iCalendar data does not contain VCALENDAR".to_string(),
        ));
    }

    Ok(indexes)
}

fn unfold_content_lines(ical_data: &str) -> Result<Vec<String>> {
    let mut unfolded: Vec<String> = Vec::new();
    for raw_line in ical_data.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(continuation) = line.strip_prefix([' ', '\t']) {
            let previous = unfolded.last_mut().ok_or_else(|| {
                Error::IcalParse("content starts with a folded continuation".to_string())
            })?;
            previous.push_str(continuation);
        } else {
            unfolded.push(line.to_string());
        }
    }
    Ok(unfolded)
}

fn parse_content_line(line: &str) -> Result<(String, IndexedProperty)> {
    let colon = first_unquoted_delimiter(line, ':')?
        .ok_or_else(|| Error::IcalParse("content line is missing ':'".to_string()))?;
    let header = &line[..colon];
    let value = &line[colon + 1..];
    let header_parts = split_quoted(header, ';')?;
    let raw_name = header_parts
        .first()
        .copied()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::IcalParse("content line has no property name".to_string()))?;
    let name = raw_name
        .rsplit_once('.')
        .map_or(raw_name, |(_, property_name)| property_name)
        .to_ascii_uppercase();
    let mut parameters = BTreeMap::<String, Vec<String>>::new();

    for raw_parameter in header_parts.iter().skip(1) {
        let (parameter_name, raw_values) = raw_parameter.split_once('=').ok_or_else(|| {
            Error::IcalParse(format!("property {name} has a malformed parameter"))
        })?;
        if parameter_name.is_empty() {
            return Err(Error::IcalParse(format!(
                "property {name} has an unnamed parameter"
            )));
        }
        let values = split_quoted(raw_values, ',')?
            .into_iter()
            .map(unquote_parameter_value)
            .map(String::from)
            .collect::<Vec<_>>();
        parameters
            .entry(parameter_name.to_ascii_uppercase())
            .or_default()
            .extend(values);
    }

    Ok((name, IndexedProperty {
        value: value.to_string(),
        parameters,
    }))
}

fn first_unquoted_delimiter(value: &str, delimiter: char) -> Result<Option<usize>> {
    let mut quoted = false;
    let mut escaped = false;
    for (position, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if character == delimiter && !quoted {
            return Ok(Some(position));
        }
    }
    if quoted {
        return Err(Error::IcalParse(
            "content line contains an unterminated quoted parameter".to_string(),
        ));
    }
    Ok(None)
}

fn split_quoted(value: &str, delimiter: char) -> Result<Vec<&str>> {
    let positions = delimiter_positions(value, delimiter)?;
    let mut parts = Vec::with_capacity(positions.len() + 1);
    let mut start = 0;
    for position in positions {
        parts.push(&value[start..position]);
        start = position + delimiter.len_utf8();
    }
    parts.push(&value[start..]);
    Ok(parts)
}

fn delimiter_positions(value: &str, delimiter: char) -> Result<Vec<usize>> {
    let mut positions = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (position, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if character == delimiter && !quoted {
            positions.push(position);
        }
    }
    if quoted {
        return Err(Error::IcalParse(
            "content line contains an unterminated quoted parameter".to_string(),
        ));
    }
    Ok(positions)
}

fn unquote_parameter_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|unquoted| unquoted.strip_suffix('"'))
        .unwrap_or(value)
}

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
        .map_err(|e| Error::IcalParse(format!("failed to parse iCalendar data: {e}")))?;

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
        .map_err(|e| Error::IcalParse(format!("failed to parse existing iCalendar data: {e}")))?;

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
    } else if raw.len() >= 15 && raw.is_ascii() && raw.contains('T') {
        // DATETIME value: YYYYMMDDTHHMMSS -> YYYY-MM-DDTHH:MM:SS
        // (require ASCII, like the DATE branch above, so the fixed byte-index
        // slicing below cannot land inside a multi-byte UTF-8 char and panic)
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
    fn indexes_recurrence_timezone_and_cancelled_override() {
        let ical = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:series-1@example.com\r\n",
            "SUMMARY:Weekly \"planning meeting that is\r\n",
            " deliberately folded\r\n",
            "DESCRIPTION:Discuss milestones\r\n",
            "LOCATION:Room 7\r\n",
            "ORGANIZER;CN=Alex:mailto:alex@example.com\r\n",
            "ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.com\r\n",
            "ATTENDEE;PARTSTAT=DECLINED:mailto:lee@example.com\r\n",
            "DTSTART;TZID=America/New_York:20260105T090000\r\n",
            "DTEND;TZID=America/New_York:20260105T100000\r\n",
            "RRULE:FREQ=WEEKLY;COUNT=4\r\n",
            "RDATE;TZID=America/New_York:20260202T090000,20260209T090000\r\n",
            "EXDATE;TZID=America/New_York:20260119T090000\r\n",
            "END:VEVENT\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:series-1@example.com\r\n",
            "RECURRENCE-ID;TZID=America/New_York;RANGE=THISANDFUTURE:20260126T090000\r\n",
            "DTSTART;TZID=America/New_York:20260126T110000\r\n",
            "STATUS:CANCELLED\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );

        let indexes = index_event_components(ical).unwrap();

        assert_eq!(indexes.len(), 2);
        let master = &indexes[0];
        assert_eq!(
            master
                .summary
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("Weekly \"planning meeting that isdeliberately folded")
        );
        assert_eq!(
            master
                .dtstart
                .as_ref()
                .and_then(|property| property.parameters.get("TZID")),
            Some(&vec!["America/New_York".to_string()])
        );
        assert_eq!(master.rrule[0].value, "FREQ=WEEKLY;COUNT=4");
        assert_eq!(master.rdate[0].value, "20260202T090000,20260209T090000");
        assert_eq!(master.exdate[0].value, "20260119T090000");
        assert_eq!(
            master
                .description
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("Discuss milestones")
        );
        assert_eq!(
            master
                .location
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("Room 7")
        );
        assert_eq!(
            master
                .organizer
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("mailto:alex@example.com")
        );
        assert_eq!(master.attendees.len(), 2);
        assert_eq!(master.attendees[0].value, "mailto:sam@example.com");
        assert_eq!(
            master.attendees[0].parameters.get("PARTSTAT"),
            Some(&vec!["ACCEPTED".to_string()])
        );

        let override_event = &indexes[1];
        assert_eq!(
            override_event
                .status
                .as_ref()
                .map(|property| property.value.as_str()),
            Some("CANCELLED")
        );
        let recurrence_id = override_event.recurrence_id.as_ref().unwrap();
        assert_eq!(recurrence_id.value, "20260126T090000");
        assert_eq!(
            recurrence_id.parameters.get("RANGE"),
            Some(&vec!["THISANDFUTURE".to_string()])
        );
        assert_eq!(
            recurrence_id.parameters.get("TZID"),
            Some(&vec!["America/New_York".to_string()])
        );
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
    fn normalise_datetime_non_ascii_does_not_panic() {
        // A malformed DATETIME (>=15 bytes, contains 'T') with a multi-byte
        // char straddling a fixed byte-slice boundary must not panic; it is
        // returned as-is like any other unrecognized value.
        let raw = "2025061\u{e9}T00:00:00"; // 'é' straddles byte index 8
        assert_eq!(normalise_datetime(raw), raw);
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
