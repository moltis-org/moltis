//! Shared contract tests for the [`ChannelPlugin`] trait.
//!
//! These functions validate that any `ChannelPlugin` implementation satisfies
//! the lifecycle and error-handling semantics required by the registry and
//! gateway. Run against `TestPlugin` in registry tests; real channel plugins
//! only need per-channel descriptor-coherence tests.

use crate::{Result, plugin::ChannelPlugin};

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
