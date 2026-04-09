//! Error classification and retry logic for Teams Bot Framework API calls.
//!
//! Classifies HTTP response status codes into retry categories following the
//! same model as OpenClaw's `errors.ts`.

use std::time::Duration;

/// Classification of a Teams send error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendErrorKind {
    /// 401/403 — authentication failure, do not retry.
    Auth,
    /// 429 — rate limited, retry after backoff.
    Throttled,
    /// 408 or 5xx — transient failure, retry with exponential backoff.
    Transient,
    /// Other 4xx — permanent failure, do not retry.
    Permanent,
    /// No status code available (network error, etc.).
    Unknown,
}

/// Full classification of a send error including retry hints.
#[derive(Debug, Clone)]
pub struct SendErrorClassification {
    pub kind: SendErrorKind,
    pub status_code: Option<u16>,
    /// Server-suggested retry delay from the `Retry-After` header.
    pub retry_after: Option<Duration>,
}

/// Classify an HTTP response status and extract retry hints.
pub fn classify_send_error(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> SendErrorClassification {
    let retry_after = extract_retry_after(headers);
    let code = status.as_u16();
    let kind = match code {
        401 | 403 => SendErrorKind::Auth,
        429 => SendErrorKind::Throttled,
        408 | 500..=599 => SendErrorKind::Transient,
        400..=499 => SendErrorKind::Permanent,
        _ => SendErrorKind::Unknown,
    };
    SendErrorClassification {
        kind,
        status_code: Some(code),
        retry_after,
    }
}

/// Whether the error kind is worth retrying.
pub fn should_retry(kind: SendErrorKind) -> bool {
    matches!(kind, SendErrorKind::Throttled | SendErrorKind::Transient)
}

/// Compute delay for the next retry attempt.
///
/// Uses exponential backoff: `base_delay * 2^(attempt-1)`, clamped to
/// `max_delay`. Respects server-provided `Retry-After` if it is longer.
pub fn compute_retry_delay(
    attempt: u32,
    classification: &SendErrorClassification,
    base_delay: Duration,
    max_delay: Duration,
) -> Duration {
    let exp_delay = base_delay.saturating_mul(1u32.wrapping_shl(attempt.saturating_sub(1)));
    let backoff = exp_delay.min(max_delay);
    match classification.retry_after {
        Some(server_delay) if server_delay > backoff => server_delay.min(max_delay),
        _ => backoff,
    }
}

fn extract_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let val = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())?;
    // Try parsing as integer seconds first.
    if let Ok(secs) = val.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn classify_401_as_auth() {
        let c = classify_send_error(
            reqwest::StatusCode::UNAUTHORIZED,
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(c.kind, SendErrorKind::Auth);
        assert!(!should_retry(c.kind));
    }

    #[test]
    fn classify_403_as_auth() {
        let c = classify_send_error(
            reqwest::StatusCode::FORBIDDEN,
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(c.kind, SendErrorKind::Auth);
    }

    #[test]
    fn classify_429_as_throttled() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "5".parse().unwrap());
        let c = classify_send_error(reqwest::StatusCode::TOO_MANY_REQUESTS, &headers);
        assert_eq!(c.kind, SendErrorKind::Throttled);
        assert!(should_retry(c.kind));
        assert_eq!(c.retry_after, Some(Duration::from_secs(5)));
    }

    #[test]
    fn classify_500_as_transient() {
        let c = classify_send_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(c.kind, SendErrorKind::Transient);
        assert!(should_retry(c.kind));
    }

    #[test]
    fn classify_408_as_transient() {
        let c = classify_send_error(
            reqwest::StatusCode::REQUEST_TIMEOUT,
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(c.kind, SendErrorKind::Transient);
    }

    #[test]
    fn classify_400_as_permanent() {
        let c = classify_send_error(
            reqwest::StatusCode::BAD_REQUEST,
            &reqwest::header::HeaderMap::new(),
        );
        assert_eq!(c.kind, SendErrorKind::Permanent);
        assert!(!should_retry(c.kind));
    }

    #[test]
    fn compute_delay_exponential_backoff() {
        let c = SendErrorClassification {
            kind: SendErrorKind::Transient,
            status_code: Some(500),
            retry_after: None,
        };
        let base = Duration::from_millis(250);
        let max = Duration::from_secs(10);
        assert_eq!(
            compute_retry_delay(1, &c, base, max),
            Duration::from_millis(250)
        );
        assert_eq!(
            compute_retry_delay(2, &c, base, max),
            Duration::from_millis(500)
        );
        assert_eq!(
            compute_retry_delay(3, &c, base, max),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn compute_delay_respects_retry_after() {
        let c = SendErrorClassification {
            kind: SendErrorKind::Throttled,
            status_code: Some(429),
            retry_after: Some(Duration::from_secs(8)),
        };
        let base = Duration::from_millis(250);
        let max = Duration::from_secs(10);
        // Server says 8s, which is larger than 250ms backoff.
        assert_eq!(
            compute_retry_delay(1, &c, base, max),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn compute_delay_clamps_to_max() {
        let c = SendErrorClassification {
            kind: SendErrorKind::Transient,
            status_code: Some(500),
            retry_after: None,
        };
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(5);
        // 2^6 = 64s, clamped to 5s
        assert_eq!(
            compute_retry_delay(7, &c, base, max),
            Duration::from_secs(5)
        );
    }
}
