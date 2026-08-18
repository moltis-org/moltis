//! Time-range helpers for filtering calendar events.
//!
//! Converts ISO 8601 range bounds to the iCalendar UTC basic format required
//! by CalDAV `time-range` filters (RFC 4791 §9.9), and provides client-side
//! overlap filtering shared by the mock client used in tests.
//!
//! Naive date-times (no UTC offset) are treated as UTC. Date-only values
//! resolve to midnight UTC.

use moltis_agents::time::{
    Date, OffsetDateTime, PrimitiveDateTime, UtcOffset, format_description::well_known::Iso8601,
};

use crate::error::{Error, Result};

#[cfg(test)]
use {
    crate::types::{EventSummary, TimeRange},
    moltis_agents::time::Duration,
};

/// Parse an ISO 8601 date or date-time string into a UTC instant.
fn parse_iso_utc(value: &str) -> Option<OffsetDateTime> {
    if let Ok(dt) = OffsetDateTime::parse(value, &Iso8601::DEFAULT) {
        return Some(dt);
    }
    if let Ok(dt) = PrimitiveDateTime::parse(value, &Iso8601::DEFAULT) {
        return Some(dt.assume_utc());
    }
    if let Ok(date) = Date::parse(value, &Iso8601::DEFAULT) {
        return Some(date.midnight().assume_utc());
    }
    None
}

/// Convert an ISO 8601 date/time string to iCalendar UTC basic format
/// (`YYYYMMDDTHHMMSSZ`, e.g. `20260101T000000Z`).
pub(crate) fn to_ical_utc(value: &str) -> Result<String> {
    let dt = parse_iso_utc(value)
        .ok_or_else(|| Error::Validation(format!("invalid ISO 8601 date/time: '{value}'")))?
        .to_offset(UtcOffset::UTC);
    Ok(format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    ))
}

/// Whether an event overlaps the given range, following the `time-range`
/// overlap semantics of RFC 4791 §9.9 — the same rules a compliant CalDAV
/// server applies server-side — so this mirror stays faithful to it:
///
/// - Events with an explicit DTEND, and all-day events (treated as spanning
///   one full day), overlap when `range.start < event_end` **and**
///   `event_start < range.end`. Both bounds are strict, so an event that
///   merely touches a boundary — ending exactly at `range.start` or starting
///   exactly at `range.end` — does not match.
/// - Instantaneous events (a DATE-TIME DTSTART with no DTEND) overlap when
///   `range.start <= event_start < range.end`: inclusive at the lower bound,
///   per the RFC's zero-duration rule.
///
/// Events whose start is missing or unparseable, or a range that cannot be
/// parsed, are kept — silently hiding them would be worse than over-reporting.
#[cfg(test)]
pub(crate) fn event_overlaps(event: &EventSummary, range: &TimeRange) -> bool {
    let (Some(range_start), Some(range_end)) =
        (parse_iso_utc(&range.start), parse_iso_utc(&range.end))
    else {
        return true;
    };
    let Some(start) = event.start.as_deref().and_then(parse_iso_utc) else {
        return true;
    };
    match event.end.as_deref().and_then(parse_iso_utc) {
        // Explicit DTEND: half-open [start, end), strict on both bounds.
        Some(end) => range_start < end.max(start) && start < range_end,
        // All-day without DTEND: spans one full day.
        None if event.all_day => range_start < start + Duration::days(1) && start < range_end,
        // Instantaneous DATE-TIME: lower bound inclusive, upper bound strict.
        None => range_start <= start && start < range_end,
    }
}

/// Drop events that fall entirely outside the range.
#[cfg(test)]
pub(crate) fn filter_events(events: Vec<EventSummary>, range: &TimeRange) -> Vec<EventSummary> {
    events
        .into_iter()
        .filter(|event| event_overlaps(event, range))
        .collect()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn event(start: Option<&str>, end: Option<&str>, all_day: bool) -> EventSummary {
        EventSummary {
            href: "/cal/test.ics".into(),
            etag: "\"etag\"".into(),
            uid: Some("uid@test".into()),
            summary: Some("Test".into()),
            start: start.map(String::from),
            end: end.map(String::from),
            all_day,
            location: None,
        }
    }

    fn range(start: &str, end: &str) -> TimeRange {
        TimeRange {
            start: start.into(),
            end: end.into(),
        }
    }

    #[test]
    fn to_ical_utc_naive_datetime() {
        assert_eq!(
            to_ical_utc("2026-01-01T00:00:00").unwrap(),
            "20260101T000000Z"
        );
    }

    #[test]
    fn to_ical_utc_zulu_datetime() {
        assert_eq!(
            to_ical_utc("2026-06-15T09:30:00Z").unwrap(),
            "20260615T093000Z"
        );
    }

    #[test]
    fn to_ical_utc_converts_offset_to_utc() {
        assert_eq!(
            to_ical_utc("2026-01-01T02:00:00+02:00").unwrap(),
            "20260101T000000Z"
        );
    }

    #[test]
    fn to_ical_utc_date_only_is_midnight_utc() {
        assert_eq!(to_ical_utc("2026-03-15").unwrap(), "20260315T000000Z");
    }

    #[test]
    fn to_ical_utc_rejects_garbage() {
        assert!(to_ical_utc("not-a-date").is_err());
        assert!(to_ical_utc("").is_err());
    }

    #[test]
    fn event_before_range_is_excluded() {
        let ev = event(
            Some("2026-01-01T10:00:00"),
            Some("2026-01-01T11:00:00"),
            false,
        );
        assert!(!event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_after_range_is_excluded() {
        let ev = event(
            Some("2026-03-01T10:00:00"),
            Some("2026-03-01T11:00:00"),
            false,
        );
        assert!(!event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_overlapping_range_start_is_included() {
        // Starts before the range, ends inside it.
        let ev = event(
            Some("2026-01-31T23:00:00"),
            Some("2026-02-01T01:00:00"),
            false,
        );
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_overlapping_range_end_is_included() {
        // Starts inside the range, ends after it.
        let ev = event(
            Some("2026-02-27T23:00:00"),
            Some("2026-02-28T01:00:00"),
            false,
        );
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_ending_exactly_at_range_start_is_excluded() {
        // RFC 4791 §9.9 uses a strict `range.start < DTEND`, so an event that
        // ends the instant the range begins does not overlap.
        let ev = event(
            Some("2026-01-31T23:00:00"),
            Some("2026-02-01T00:00:00"),
            false,
        );
        assert!(!event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_starting_exactly_at_range_end_is_excluded() {
        // Strict `DTSTART < range.end`: an event that starts the instant the
        // range ends does not overlap.
        let ev = event(
            Some("2026-02-28T00:00:00"),
            Some("2026-02-28T01:00:00"),
            false,
        );
        assert!(!event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn instantaneous_event_at_range_start_is_included() {
        // Instantaneous events use the RFC's inclusive lower bound
        // (`range.start <= DTSTART`), unlike events with an explicit DTEND.
        let ev = event(Some("2026-02-01T00:00:00"), None, false);
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn event_spanning_entire_range_is_included() {
        let ev = event(
            Some("2026-01-01T00:00:00"),
            Some("2026-12-31T00:00:00"),
            false,
        );
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn instantaneous_event_without_dtend_uses_start() {
        let inside = event(Some("2026-02-15T12:00:00"), None, false);
        let outside = event(Some("2026-03-15T12:00:00"), None, false);
        let r = range("2026-02-01T00:00:00", "2026-02-28T00:00:00");
        assert!(event_overlaps(&inside, &r));
        assert!(!event_overlaps(&outside, &r));
    }

    #[test]
    fn all_day_event_without_dtend_spans_one_day() {
        let ev = event(Some("2026-02-01"), None, true);
        // Range starts in the evening of the event's day: still overlaps.
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T20:00:00", "2026-02-02T00:00:00")
        ));
        // Range entirely after that day: no overlap.
        assert!(!event_overlaps(
            &ev,
            &range("2026-02-03T00:00:00", "2026-02-04T00:00:00")
        ));
    }

    #[test]
    fn event_with_missing_start_is_kept() {
        let ev = event(None, None, false);
        assert!(event_overlaps(
            &ev,
            &range("2026-02-01T00:00:00", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn unparseable_range_keeps_everything() {
        let ev = event(Some("2026-01-01T10:00:00"), None, false);
        assert!(event_overlaps(
            &ev,
            &range("garbage", "2026-02-28T00:00:00")
        ));
    }

    #[test]
    fn filter_events_drops_only_out_of_range() {
        let events = vec![
            event(
                Some("2026-01-01T10:00:00"),
                Some("2026-01-01T11:00:00"),
                false,
            ),
            event(
                Some("2026-02-15T10:00:00"),
                Some("2026-02-15T11:00:00"),
                false,
            ),
        ];
        let kept = filter_events(events, &range("2026-02-01T00:00:00", "2026-02-28T00:00:00"));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].start.as_deref(), Some("2026-02-15T10:00:00"));
    }
}
