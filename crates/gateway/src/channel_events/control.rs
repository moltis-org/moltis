use super::*;

pub(super) async fn request_disable_account(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    channel_type: &str,
    account_id: &str,
    reason: &str,
) {
    warn!(
        channel_type,
        account_id,
        reason,
        "stopping local polling: detected bot already running on another instance"
    );

    if let Some(state) = state.get() {
        // Note: We intentionally do NOT remove the channel from the database.
        // The channel config should remain persisted so other moltis instances
        // sharing the same database can still use it. The polling loop will
        // cancel itself after this call returns.

        // Broadcast an event so the UI can update.
        let channel_type: moltis_channels::ChannelType = match channel_type.parse() {
            Ok(ct) => ct,
            Err(e) => {
                warn!("request_disable_account: {e}");
                return;
            },
        };
        let event = ChannelEvent::AccountDisabled {
            channel_type,
            account_id: account_id.to_string(),
            reason: reason.to_string(),
        };
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                warn!("failed to serialize AccountDisabled event: {e}");
                return;
            },
        };
        broadcast(state, "channel", payload, BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        })
        .await;
    } else {
        warn!("request_disable_account: gateway not ready");
    }
}
