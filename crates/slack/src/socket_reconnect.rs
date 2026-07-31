use std::time::Duration;

pub(crate) const RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub(crate) const RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(30);
pub(crate) const RECONNECT_STABLE_AFTER: Duration = Duration::from_secs(60);

/// Sleep for the current jittered backoff and double it up to the cap.
pub(crate) async fn backoff_sleep(
    cancel: &tokio_util::sync::CancellationToken,
    backoff: &mut Duration,
) -> bool {
    tokio::select! {
        () = cancel.cancelled() => return false,
        () = tokio::time::sleep(jittered(*backoff)) => {},
    }
    *backoff = next_backoff(*backoff);
    true
}

pub(crate) fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(RECONNECT_MAX_BACKOFF)
}

/// Apply +/-25% jitter using wall-clock nanoseconds as non-security entropy.
pub(crate) fn jittered(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    base.mul_f64(jitter_factor(nanos))
}

pub(crate) fn jitter_factor(nanos: u32) -> f64 {
    const NANOS_PER_SEC: f64 = 1_000_000_000.0;
    let unit = f64::from(nanos) / NANOS_PER_SEC;
    (0.75 + unit * 0.5).clamp(0.75, 1.25)
}
