use axum::http::{HeaderMap, HeaderValue};

use super::telephony_webhook_url;

#[test]
fn telephony_webhook_url_builds_absolute_url_from_forwarded_headers() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-host",
        HeaderValue::from_static("calls.example.com"),
    );
    headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));

    let url = telephony_webhook_url(
        "default",
        "gather",
        &headers,
        None,
        &moltis_config::schema::MoltisConfig::default(),
    )
    .unwrap_or_default();

    assert_eq!(
        url,
        "https://calls.example.com/api/channels/telephony/default/gather"
    );
}

#[test]
fn telephony_webhook_url_prefers_account_webhook_base() {
    let account_config = serde_json::json!({
        "webhook_url": "https://phone.example.com/base/",
    });

    let url = telephony_webhook_url(
        "default",
        "answer",
        &HeaderMap::new(),
        Some(account_config),
        &moltis_config::schema::MoltisConfig::default(),
    )
    .unwrap_or_default();

    assert_eq!(
        url,
        "https://phone.example.com/base/api/channels/telephony/default/answer"
    );
}

#[test]
fn inbound_call_rejected_denies_empty_allowlist() {
    use moltis_channels::gating::DmPolicy;

    use super::inbound_call_rejected;

    // An empty allowlist with an explicit Allowlist policy denies everyone.
    assert!(inbound_call_rejected(
        DmPolicy::Allowlist,
        "+15551234567",
        &[]
    ));

    let allowlist = vec!["+15551234567".to_string()];
    assert!(!inbound_call_rejected(
        DmPolicy::Allowlist,
        "+15551234567",
        &allowlist
    ));
    assert!(inbound_call_rejected(
        DmPolicy::Allowlist,
        "+15559999999",
        &allowlist
    ));

    assert!(inbound_call_rejected(
        DmPolicy::Disabled,
        "+15551234567",
        &allowlist
    ));
    assert!(!inbound_call_rejected(DmPolicy::Open, "+15559999999", &[]));
}
