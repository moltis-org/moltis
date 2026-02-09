//! Parsing utilities for durations and absolute timestamps.

use anyhow::{Result, bail};

/// Parse a human-friendly duration string into milliseconds.
///
/// Supported suffixes: `s` (seconds), `m` (minutes), `h` (hours), `d` (days).
/// Examples: `"30s"`, `"5m"`, `"2h"`, `"1d"`.
pub fn parse_duration_ms(input: &str) -> Result<u64> {
    let input = input.trim();
    if input.is_empty() {
        bail!("empty duration string");
    }

    let (num_str, suffix) = match input.find(|c: char| c.is_alphabetic()) {
        Some(i) => (&input[..i], &input[i..]),
        None => bail!("duration missing unit suffix (s/m/h/d): {input}"),
    };

    let value: u64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in duration: {num_str}"))?;

    if value == 0 {
        bail!("duration must be > 0");
    }

    let ms = match suffix {
        "s" => value * 1_000,
        "m" => value * 60_000,
        "h" => value * 3_600_000,
        "d" => value * 86_400_000,
        _ => bail!("unknown duration suffix: {suffix} (expected s/m/h/d)"),
    };

    Ok(ms)
}

/// Parse an ISO 8601 timestamp string into epoch milliseconds.
///
/// Accepts formats like `"2026-01-12T18:00:00Z"` or with timezone offset.
pub fn parse_absolute_time_ms(input: &str) -> Result<u64> {
    use chrono::{DateTime, Utc};

    let dt: DateTime<Utc> = input
        .parse::<DateTime<Utc>>()
        .map_err(|e| anyhow::anyhow!("invalid ISO 8601 timestamp: {e}"))?;

    let ms = dt.timestamp_millis();
    if ms < 0 {
        bail!("timestamp is before epoch");
    }
    Ok(ms as u64)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_seconds() {
        assert_eq!(parse_duration_ms("30s").unwrap(), 30_000);
    }

    #[test]
    fn test_parse_minutes() {
        assert_eq!(parse_duration_ms("5m").unwrap(), 300_000);
    }

    #[test]
    fn test_parse_hours() {
        assert_eq!(parse_duration_ms("2h").unwrap(), 7_200_000);
    }

    #[test]
    fn test_parse_days() {
        assert_eq!(parse_duration_ms("1d").unwrap(), 86_400_000);
    }

    #[test]
    fn test_parse_with_whitespace() {
        assert_eq!(parse_duration_ms("  10m  ").unwrap(), 600_000);
    }

    #[test]
    fn test_parse_empty() {
        assert!(parse_duration_ms("").is_err());
    }

    #[test]
    fn test_parse_no_suffix() {
        assert!(parse_duration_ms("100").is_err());
    }

    #[test]
    fn test_parse_zero() {
        assert!(parse_duration_ms("0s").is_err());
    }

    #[test]
    fn test_parse_bad_suffix() {
        assert!(parse_duration_ms("10x").is_err());
    }

    #[test]
    fn test_parse_iso_utc() {
        let ms = parse_absolute_time_ms("2026-01-12T18:00:00Z").unwrap();
        assert!(ms > 0);
        // Verify round-trip via chrono.
        let dt = chrono::DateTime::from_timestamp_millis(ms as i64).unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2026-01-12T18:00:00Z"
        );
    }

    #[test]
    fn test_parse_iso_with_offset() {
        let ms_utc = parse_absolute_time_ms("2026-01-12T18:00:00Z").unwrap();
        let ms_offset = parse_absolute_time_ms("2026-01-12T19:00:00+01:00").unwrap();
        // Same instant.
        assert_eq!(ms_utc, ms_offset);
    }

    #[test]
    fn test_parse_iso_invalid() {
        assert!(parse_absolute_time_ms("not a date").is_err());
    }
}
