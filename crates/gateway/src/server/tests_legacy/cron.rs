use std::sync::Arc;

use super::common::{DeliveredMessage, RecordingChannelOutbound, cron_delivery_request};

#[tokio::test]
async fn maybe_deliver_cron_output_sends_to_configured_channel() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Daily digest ready",
    )
    .await;

    let delivered = outbound.delivered.lock().await.clone();
    assert_eq!(delivered, vec![DeliveredMessage {
        account_id: "bot-main".to_string(),
        to: "123456".to_string(),
        text: "Daily digest ready".to_string(),
        reply_to: None,
    }]);
}

#[tokio::test]
async fn maybe_deliver_cron_output_skips_blank_messages() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "   ",
    )
    .await;

    assert!(outbound.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn maybe_deliver_cron_output_skips_when_deliver_is_false() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let mut req = cron_delivery_request();
    req.deliver = false;

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "should not be sent",
    )
    .await;

    assert!(outbound.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn maybe_deliver_cron_output_skips_when_no_outbound_configured() {
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(None, &req, "Daily digest ready").await;
}
