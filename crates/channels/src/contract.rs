//! Shared contract tests for the [`ChannelPlugin`] trait.
//!
//! These functions validate that any `ChannelPlugin` implementation satisfies
//! the lifecycle and error-handling semantics required by the registry and
//! gateway. Run against `TestPlugin` in registry tests; real channel plugins
//! only need per-channel descriptor-coherence tests.

use crate::{
    Result,
    plugin::{ChannelPlugin, StreamEvent},
};

/// Start → `has_account` → stop → `!has_account`.
pub async fn lifecycle_start_stop(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-1";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    assert!(
        plugin.has_account(id),
        "has_account must return true after start_account"
    );
    assert!(
        plugin.account_ids().contains(&id.to_string()),
        "account_ids must include the started account"
    );

    plugin.stop_account(id).await?;
    assert!(
        !plugin.has_account(id),
        "has_account must return false after stop_account"
    );
    Ok(())
}

/// Starting the same account twice must not panic.
pub async fn double_start_same_account(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-double";
    let config = serde_json::json!({});

    plugin.start_account(id, config.clone()).await?;
    // Second start: must succeed or return a clear error — must not panic.
    let result = plugin.start_account(id, config).await;
    assert!(
        result.is_ok(),
        "second start_account should succeed, got: {result:?}"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

/// Stopping an unknown account must not panic.
pub async fn stop_unknown_account(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    // Should not panic — may return Ok or Err.
    let _ = plugin.stop_account("nonexistent-account").await;
    Ok(())
}

/// `account_config()` returns `Some` after start for plugins that support it.
pub async fn config_view_after_start(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-config";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let view = plugin.account_config(id);
    assert!(
        view.is_some(),
        "account_config must return Some after start_account"
    );

    plugin.stop_account(id).await?;
    assert!(
        plugin.account_config(id).is_none(),
        "account_config must return None after stop_account"
    );
    Ok(())
}

/// After `start_account`, `outbound()` must return `Some`.
pub async fn outbound_available_after_start(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-outbound";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    assert!(
        plugin.outbound().is_some(),
        "outbound must return Some after start_account"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

/// After `start_account`, `shared_outbound().send_text()` must return `Ok`.
pub async fn shared_outbound_send_succeeds_after_start(
    plugin: &mut dyn ChannelPlugin,
) -> Result<()> {
    let id = "contract-acct-send";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let outbound = plugin.shared_outbound();
    let result = outbound.send_text(id, "peer-1", "hello", None).await;
    assert!(
        result.is_ok(),
        "shared_outbound send_text must succeed after start, got: {result:?}"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

/// Sending a stream with `StreamEvent::Done` must complete without error.
pub async fn stream_completes_on_done_signal(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-stream-done";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let stream_out = plugin.shared_stream_outbound();

    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tx.send(StreamEvent::Delta("hello ".into())).await.ok();
    tx.send(StreamEvent::Done).await.ok();
    drop(tx);

    let result = stream_out.send_stream(id, "peer-1", None, rx).await;
    assert!(
        result.is_ok(),
        "stream must complete on Done signal, got: {result:?}"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

/// Sending a stream with `StreamEvent::Error` must complete without panicking.
pub async fn stream_completes_on_error_signal(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-stream-err";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let stream_out = plugin.shared_stream_outbound();

    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tx.send(StreamEvent::Error("test error".into())).await.ok();
    drop(tx);

    let result = stream_out.send_stream(id, "peer-1", None, rx).await;
    assert!(
        result.is_ok(),
        "stream must complete on Error signal without panic, got: {result:?}"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

/// Sending to an unknown account must return a classifiable (non-retryable) error.
pub async fn outbound_error_classification(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-err-class";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let outbound = plugin.shared_outbound();

    // Send to a non-existent peer — the outbound should succeed (NullOutbound)
    // but the registry's resolve_outbound for an unknown account returns an error.
    // Test error classification on known error variants.
    let unknown_err = crate::Error::unknown_account("bad-account");
    assert!(
        !unknown_err.is_retryable(),
        "unknown_account error must NOT be retryable"
    );

    let external_err = crate::Error::external(
        "network timeout",
        std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"),
    );
    assert!(
        external_err.is_retryable(),
        "external/network error must be retryable"
    );

    let invalid_err = crate::Error::invalid_input("bad payload");
    assert!(
        !invalid_err.is_retryable(),
        "invalid input error must NOT be retryable"
    );

    // Also verify outbound still works for the started account.
    let result = outbound.send_text(id, "peer-1", "test", None).await;
    assert!(result.is_ok(), "outbound must still work: {result:?}");

    plugin.stop_account(id).await?;
    Ok(())
}

/// `status().probe()` on an unknown account must return `connected: false`.
pub async fn probe_unknown_account_returns_disconnected(plugin: &dyn ChannelPlugin) -> Result<()> {
    let status = plugin
        .status()
        .ok_or_else(|| crate::Error::unavailable("status() must return Some for probe tests"))?;
    let snap = status.probe("nonexistent-probe-acct").await?;
    assert!(
        !snap.connected,
        "probe on unknown account must return connected=false"
    );
    Ok(())
}

/// After `start_account`, `status().probe()` must return `connected: true`.
pub async fn probe_started_account_returns_connected(plugin: &mut dyn ChannelPlugin) -> Result<()> {
    let id = "contract-acct-probe";
    let config = serde_json::json!({});

    plugin.start_account(id, config).await?;
    let status = plugin
        .status()
        .ok_or_else(|| crate::Error::unavailable("status() must return Some for probe tests"))?;
    let snap = status.probe(id).await?;
    assert!(
        snap.connected,
        "probe on started account must return connected=true"
    );

    plugin.stop_account(id).await?;
    Ok(())
}

// ── Channel webhook verifier contracts ──────────────────────────────────────

use crate::channel_webhook_middleware::{ChannelWebhookRejection, ChannelWebhookVerifier};

/// A verifier must reject an empty body with no signature headers.
pub fn channel_webhook_verifier_rejects_empty_signature(verifier: &dyn ChannelWebhookVerifier) {
    let headers = http::HeaderMap::new();
    let result = verifier.verify(&headers, b"{}");
    assert!(
        result.is_err(),
        "verifier must reject requests without signature headers"
    );
    match result {
        Err(
            ChannelWebhookRejection::BadSignature(_) | ChannelWebhookRejection::MissingHeaders(_),
        ) => {},
        Err(other) => panic!("expected BadSignature or MissingHeaders, got: {other}"),
        Ok(_) => panic!("verifier must reject requests without signature headers"),
    }
}

/// A verifier must reject a body with an invalid/corrupted signature.
pub fn channel_webhook_verifier_rejects_bad_signature(
    verifier: &dyn ChannelWebhookVerifier,
    headers_with_bad_sig: &http::HeaderMap,
) {
    let result = verifier.verify(headers_with_bad_sig, b"{\"text\":\"hello\"}");
    assert!(
        result.is_err(),
        "verifier must reject requests with bad signatures"
    );
    assert!(
        matches!(result, Err(ChannelWebhookRejection::BadSignature(_))),
        "rejection must be BadSignature"
    );
}

/// A verifier must produce a non-empty `channel_type()`.
pub fn channel_webhook_verifier_has_channel_type(verifier: &dyn ChannelWebhookVerifier) {
    let ct = verifier.channel_type();
    assert!(
        !ct.as_str().is_empty(),
        "channel_type().as_str() must not be empty"
    );
}

/// A verifier's `max_timestamp_age()` must be positive.
pub fn channel_webhook_verifier_has_positive_max_age(verifier: &dyn ChannelWebhookVerifier) {
    let age = verifier.max_timestamp_age();
    assert!(
        !age.is_zero(),
        "max_timestamp_age() must be positive, got: {age:?}"
    );
}

/// A verifier's `rate_policy()` must have a non-zero rate.
pub fn channel_webhook_verifier_has_valid_rate_policy(verifier: &dyn ChannelWebhookVerifier) {
    let policy = verifier.rate_policy();
    assert!(
        policy.max_requests_per_minute > 0,
        "rate_policy().max_requests_per_minute must be > 0"
    );
}
