use {
    super::*,
    crate::socket_reconnect::{RECONNECT_MAX_BACKOFF, jitter_factor, jittered, next_backoff},
};

#[test]
fn backoff_doubles_and_caps() {
    use std::time::Duration;
    assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
    assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
    // Caps at RECONNECT_MAX_BACKOFF (30s).
    assert_eq!(next_backoff(Duration::from_secs(20)), RECONNECT_MAX_BACKOFF);
    assert_eq!(next_backoff(RECONNECT_MAX_BACKOFF), RECONNECT_MAX_BACKOFF);
}

#[test]
fn jitter_stays_within_bounds() {
    use std::time::Duration;
    let base = Duration::from_secs(10);
    for _ in 0..200 {
        let j = jittered(base);
        // Exactly the documented +/-25%. Looser bounds would also accept a
        // one-sided distribution, which is the bug this guards against.
        assert!(j >= base.mul_f64(0.75), "jitter {j:?} below lower bound");
        assert!(j <= base.mul_f64(1.25), "jitter {j:?} above upper bound");
    }
}

#[test]
fn jitter_is_two_sided() {
    // The factor must be able to land on both sides of the base delay;
    // dividing nanoseconds by the wrong constant silently made it
    // always-negative, which no range assertion alone would catch.
    assert!(jitter_factor(0) < 1.0, "minimum should shorten the delay");
    assert!(
        jitter_factor(999_999_999) > 1.0,
        "maximum should lengthen the delay"
    );
}

#[test]
fn ack_target_dm_is_acknowledged() {
    assert_eq!(
        ack_reaction_target(true, true, false, Some("111.222".into())),
        Some("111.222".into())
    );
}

#[test]
fn ack_target_mention_is_acknowledged() {
    assert_eq!(
        ack_reaction_target(true, false, true, Some("111.222".into())),
        Some("111.222".into())
    );
}

#[test]
fn ack_target_unaddressed_channel_message_is_not_acknowledged() {
    assert_eq!(
        ack_reaction_target(true, false, false, Some("111.222".into())),
        None
    );
}

#[test]
fn ack_target_disabled_config_never_acknowledges() {
    assert_eq!(
        ack_reaction_target(false, true, true, Some("111.222".into())),
        None
    );
}

#[test]
fn reaction_trigger_requires_enabled_added_and_not_self() {
    // Disabled → never.
    assert!(!reaction_should_trigger(false, true, false, true, "x", &[]));
    // Removal → never.
    assert!(!reaction_should_trigger(true, false, false, true, "x", &[]));
    // Bot's own reaction → never.
    assert!(!reaction_should_trigger(true, true, true, true, "eyes", &[]));
    // Enabled add by another user with no allowlist → trigger.
    assert!(reaction_should_trigger(true, true, false, true, "x", &[]));
}

#[test]
fn reaction_trigger_respects_emoji_allowlist() {
    let allow = vec!["white_check_mark".to_string()];
    assert!(reaction_should_trigger(
        true,
        true,
        false,
        true,
        "white_check_mark",
        &allow
    ));
    assert!(!reaction_should_trigger(
        true, true, false, true, "x", &allow
    ));
}

#[test]
fn dm_open_allows_anyone() {
    assert!(check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Open,
        &GroupPolicy::Open,
        &[],
        &[],
    ));
}

#[test]
fn dm_allowlist_requires_user() {
    assert!(!check_access(
        true,
        "U999",
        "D456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &["U123".to_string()],
        &[],
    ));
    assert!(check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &["U123".to_string()],
        &[],
    ));
}

#[test]
fn empty_dm_allowlist_denies_all() {
    assert!(!check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &[],
        &[],
    ));
}

#[test]
fn dm_disabled_denies_all() {
    assert!(!check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Disabled,
        &GroupPolicy::Open,
        &[],
        &[],
    ));
}

#[test]
fn channel_open_allows_any() {
    assert!(check_access(
        false,
        "U123",
        "C456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &[],
        &[],
    ));
}

#[test]
fn channel_allowlist_requires_channel() {
    assert!(!check_access(
        false,
        "U123",
        "C999",
        &DmPolicy::Allowlist,
        &GroupPolicy::Allowlist,
        &[],
        &["C456".to_string()],
    ));
    assert!(check_access(
        false,
        "U123",
        "C456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Allowlist,
        &[],
        &["C456".to_string()],
    ));
}

#[test]
fn empty_channel_allowlist_denies_all() {
    assert!(!check_access(
        false,
        "U123",
        "C456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Allowlist,
        &[],
        &[],
    ));
}

#[test]
fn removing_last_allowlist_entry_denies_access() {
    let mut allowlist = vec!["U123".to_string()];
    assert!(check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &allowlist,
        &[],
    ));
    allowlist.clear();
    assert!(!check_access(
        true,
        "U123",
        "D456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Open,
        &allowlist,
        &[],
    ));
}

#[test]
fn channel_disabled_denies_all() {
    assert!(!check_access(
        false,
        "U123",
        "C456",
        &DmPolicy::Allowlist,
        &GroupPolicy::Disabled,
        &[],
        &[],
    ));
}
