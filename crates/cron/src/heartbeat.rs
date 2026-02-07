//! Heartbeat logic: token stripping, empty-content detection, active-hours check.

use chrono::{Local, NaiveTime, Timelike, Utc};

/// The sentinel token an LLM returns when nothing noteworthy is happening.
pub const HEARTBEAT_OK: &str = "HEARTBEAT_OK";

/// Default heartbeat interval in milliseconds (30 minutes).
pub const DEFAULT_INTERVAL_MS: u64 = 30 * 60 * 1000;

/// Default maximum characters for an acknowledgment reply.
pub const DEFAULT_ACK_MAX_CHARS: usize = 300;

/// Default heartbeat prompt sent to the LLM.
pub const DEFAULT_PROMPT: &str = "\
You are performing a periodic heartbeat check. Review any pending items \
(inbox, calendar, reminders, scheduled tasks) and determine if anything \
needs the user's attention right now.\n\n\
- If nothing requires attention, reply with exactly: HEARTBEAT_OK\n\
- If something needs attention, describe it concisely (under 300 characters).\n\
Do NOT wrap HEARTBEAT_OK in markdown formatting.";

/// Result of stripping the `HEARTBEAT_OK` token from an LLM reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripResult {
    /// Whether the reply should be suppressed (not delivered to the user).
    pub should_skip: bool,
    /// The remaining text after stripping.
    pub text: String,
    /// Whether the token was found and removed.
    pub did_strip: bool,
}

/// How aggressively to strip the heartbeat token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripMode {
    /// Only strip if the entire reply is the token (possibly wrapped in bold).
    Exact,
    /// Strip the token from edges and check what remains.
    Trim,
}

/// Source of the effective heartbeat prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatPromptSource {
    Config,
    HeartbeatMd,
    Default,
}

impl HeartbeatPromptSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::HeartbeatMd => "heartbeat_md",
            Self::Default => "default",
        }
    }
}

/// Strip `HEARTBEAT_OK` from `text`, handling common LLM formatting wrappers
/// like `**HEARTBEAT_OK**` and `<b>HEARTBEAT_OK</b>`.
///
/// Returns a [`StripResult`] indicating whether the reply should be suppressed.
pub fn strip_heartbeat_token(text: &str, mode: StripMode, max_ack_chars: usize) -> StripResult {
    let trimmed = text.trim();

    // Unwrap common bold wrappers.
    let unwrapped = unwrap_bold(trimmed);

    if unwrapped == HEARTBEAT_OK {
        return StripResult {
            should_skip: true,
            text: String::new(),
            did_strip: true,
        };
    }

    if mode == StripMode::Exact {
        return StripResult {
            should_skip: false,
            text: trimmed.to_string(),
            did_strip: false,
        };
    }

    // Trim mode: remove the token from edges.
    let mut result = trimmed.to_string();
    let mut did_strip = false;

    let patterns = [
        HEARTBEAT_OK.to_string(),
        format!("**{HEARTBEAT_OK}**"),
        format!("<b>{HEARTBEAT_OK}</b>"),
    ];
    for pattern in &patterns {
        if result.contains(pattern.as_str()) {
            result = result.replace(pattern.as_str(), "");
            did_strip = true;
        }
    }

    let result = result.trim().to_string();
    let should_skip = result.is_empty() || result.len() <= max_ack_chars && result.is_empty();

    StripResult {
        should_skip,
        text: result,
        did_strip,
    }
}

/// Returns `true` if a HEARTBEAT.md file's content is effectively empty
/// (only headers, blank lines, and empty list items).
pub fn is_heartbeat_content_empty(content: &str) -> bool {
    content.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed == "-"
            || trimmed == "*"
            || trimmed == "- "
            || trimmed == "* "
    })
}

/// Check whether the current time falls within the active hours window.
///
/// Handles overnight windows (e.g. start=22:00, end=06:00).
/// If timezone is "local" or empty, uses the system local time.
pub fn is_within_active_hours(start: &str, end: &str, timezone: &str) -> bool {
    let start_time = match parse_hhmm(start) {
        Some(t) => t,
        None => return true, // invalid config → always active
    };
    let end_time = match parse_hhmm(end) {
        Some(t) => t,
        None => return true,
    };

    // "24:00" means end-of-day.
    let end_minutes = if end == "24:00" {
        24 * 60
    } else {
        end_time.hour() as u32 * 60 + end_time.minute() as u32
    };
    let start_minutes = start_time.hour() as u32 * 60 + start_time.minute() as u32;

    let now_minutes = current_minutes(timezone);

    if start_minutes <= end_minutes {
        // Normal window: 08:00–24:00
        now_minutes >= start_minutes && now_minutes < end_minutes
    } else {
        // Overnight window: 22:00–06:00
        now_minutes >= start_minutes || now_minutes < end_minutes
    }
}

/// Resolve the heartbeat prompt with precedence:
///
/// 1. Explicit config prompt (`custom`)
/// 2. `HEARTBEAT.md` content (`heartbeat_md`)
/// 3. Built-in default prompt
pub fn resolve_heartbeat_prompt(
    custom: Option<&str>,
    heartbeat_md: Option<&str>,
) -> (String, HeartbeatPromptSource) {
    if let Some(p) = custom.map(str::trim)
        && !p.is_empty()
    {
        return (p.to_string(), HeartbeatPromptSource::Config);
    }
    if let Some(md) = heartbeat_md.map(str::trim)
        && !md.is_empty()
        && !is_heartbeat_content_empty(md)
    {
        return (md.to_string(), HeartbeatPromptSource::HeartbeatMd);
    }
    (DEFAULT_PROMPT.to_string(), HeartbeatPromptSource::Default)
}

/// Parse a human-friendly interval string like "30m", "1h", "90s" into milliseconds.
/// Returns `None` for unparseable input.
pub fn parse_interval_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1_000u64)
    } else {
        // Assume milliseconds if no suffix.
        (s, 1u64)
    };

    num_str.trim().parse::<u64>().ok().map(|n| n * multiplier)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn unwrap_bold(s: &str) -> &str {
    // **HEARTBEAT_OK**
    if let Some(inner) = s.strip_prefix("**").and_then(|s| s.strip_suffix("**")) {
        return inner;
    }
    // <b>HEARTBEAT_OK</b>
    if let Some(inner) = s.strip_prefix("<b>").and_then(|s| s.strip_suffix("</b>")) {
        return inner;
    }
    s
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

fn current_minutes(timezone: &str) -> u32 {
    if timezone.is_empty() || timezone == "local" {
        let local = Local::now();
        local.hour() * 60 + local.minute()
    } else if let Ok(tz) = timezone.parse::<chrono_tz::Tz>() {
        let dt = Utc::now().with_timezone(&tz);
        dt.hour() * 60 + dt.minute()
    } else {
        // Fallback to local on invalid tz.
        let local = Local::now();
        local.hour() * 60 + local.minute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_heartbeat_token ────────────────────────────────────────────

    #[test]
    fn strip_exact_heartbeat_ok() {
        let r = strip_heartbeat_token("HEARTBEAT_OK", StripMode::Exact, 300);
        assert!(r.should_skip);
        assert!(r.did_strip);
        assert!(r.text.is_empty());
    }

    #[test]
    fn strip_bold_wrapped() {
        let r = strip_heartbeat_token("**HEARTBEAT_OK**", StripMode::Exact, 300);
        assert!(r.should_skip);
        assert!(r.did_strip);
    }

    #[test]
    fn strip_html_bold_wrapped() {
        let r = strip_heartbeat_token("<b>HEARTBEAT_OK</b>", StripMode::Exact, 300);
        assert!(r.should_skip);
        assert!(r.did_strip);
    }

    #[test]
    fn strip_with_whitespace() {
        let r = strip_heartbeat_token("  HEARTBEAT_OK  \n", StripMode::Exact, 300);
        assert!(r.should_skip);
        assert!(r.did_strip);
    }

    #[test]
    fn strip_exact_with_extra_text() {
        let r = strip_heartbeat_token("HEARTBEAT_OK but also check email", StripMode::Exact, 300);
        assert!(!r.should_skip);
        assert!(!r.did_strip);
    }

    #[test]
    fn strip_trim_removes_token() {
        let r = strip_heartbeat_token(
            "HEARTBEAT_OK\nYou have a meeting at 3pm",
            StripMode::Trim,
            300,
        );
        assert!(!r.should_skip);
        assert!(r.did_strip);
        assert!(r.text.contains("meeting"));
        assert!(!r.text.contains("HEARTBEAT_OK"));
    }

    #[test]
    fn strip_trim_only_token() {
        let r = strip_heartbeat_token("**HEARTBEAT_OK**\n", StripMode::Trim, 300);
        assert!(r.should_skip);
        assert!(r.did_strip);
    }

    // ── is_heartbeat_content_empty ───────────────────────────────────────

    #[test]
    fn empty_content() {
        assert!(is_heartbeat_content_empty(""));
        assert!(is_heartbeat_content_empty("  \n\n  "));
    }

    #[test]
    fn headers_only() {
        assert!(is_heartbeat_content_empty("# Heartbeat\n## Inbox\n- \n"));
    }

    #[test]
    fn has_content() {
        assert!(!is_heartbeat_content_empty(
            "# Heartbeat\n- Check email from Bob"
        ));
    }

    // ── is_within_active_hours ───────────────────────────────────────────

    #[test]
    fn invalid_time_always_active() {
        assert!(is_within_active_hours("invalid", "24:00", "local"));
    }

    #[test]
    fn active_hours_normal_window() {
        // We can't assert exact behavior without controlling time,
        // but we can verify it doesn't panic.
        let _ = is_within_active_hours("08:00", "24:00", "local");
        let _ = is_within_active_hours("09:00", "17:00", "UTC");
    }

    #[test]
    fn active_hours_overnight_window() {
        let _ = is_within_active_hours("22:00", "06:00", "local");
    }

    // ── resolve_heartbeat_prompt ─────────────────────────────────────────

    #[test]
    fn default_prompt_when_none() {
        let (p, source) = resolve_heartbeat_prompt(None, None);
        assert_eq!(p, DEFAULT_PROMPT);
        assert_eq!(source, HeartbeatPromptSource::Default);
    }

    #[test]
    fn default_prompt_when_empty() {
        let (p, source) = resolve_heartbeat_prompt(Some("  "), None);
        assert_eq!(p, DEFAULT_PROMPT);
        assert_eq!(source, HeartbeatPromptSource::Default);
    }

    #[test]
    fn custom_prompt() {
        let (p, source) = resolve_heartbeat_prompt(Some("Check my inbox"), None);
        assert_eq!(p, "Check my inbox");
        assert_eq!(source, HeartbeatPromptSource::Config);
    }

    #[test]
    fn heartbeat_md_used_when_config_missing() {
        let (p, source) = resolve_heartbeat_prompt(None, Some("# Heartbeat\n- Check inbox"));
        assert_eq!(p, "# Heartbeat\n- Check inbox");
        assert_eq!(source, HeartbeatPromptSource::HeartbeatMd);
    }

    #[test]
    fn config_overrides_heartbeat_md() {
        let (p, source) =
            resolve_heartbeat_prompt(Some("Use config prompt"), Some("# Heartbeat\n- Check"));
        assert_eq!(p, "Use config prompt");
        assert_eq!(source, HeartbeatPromptSource::Config);
    }

    #[test]
    fn prompt_source_as_str_values() {
        assert_eq!(HeartbeatPromptSource::Config.as_str(), "config");
        assert_eq!(HeartbeatPromptSource::HeartbeatMd.as_str(), "heartbeat_md");
        assert_eq!(HeartbeatPromptSource::Default.as_str(), "default");
    }

    // ── parse_interval_ms ────────────────────────────────────────────────

    #[test]
    fn parse_minutes() {
        assert_eq!(parse_interval_ms("30m"), Some(1_800_000));
    }

    #[test]
    fn parse_hours() {
        assert_eq!(parse_interval_ms("1h"), Some(3_600_000));
    }

    #[test]
    fn parse_seconds() {
        assert_eq!(parse_interval_ms("90s"), Some(90_000));
    }

    #[test]
    fn parse_raw_ms() {
        assert_eq!(parse_interval_ms("5000"), Some(5000));
    }

    #[test]
    fn parse_empty() {
        assert_eq!(parse_interval_ms(""), None);
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_interval_ms("abc"), None);
    }
}
