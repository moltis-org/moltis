use std::sync::Arc;

use {
    super::common::{DeliveredMessage, RecordingChannelOutbound, cron_delivery_request},
    async_trait::async_trait,
    moltis_common::types::ReplyPayload,
};

struct FailingChannelOutbound;

#[async_trait]
impl moltis_channels::ChannelOutbound for FailingChannelOutbound {
    async fn send_text(
        &self,
        _account_id: &str,
        _to: &str,
        _text: &str,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Err(moltis_channels::Error::unavailable("test delivery failure"))
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn maybe_deliver_cron_output_sends_to_configured_channel() {
    let outbound = Arc::new(RecordingChannelOutbound::default());
    let req = cron_delivery_request();

    crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound.clone() as Arc<dyn moltis_channels::ChannelOutbound>),
        &req,
        "Daily digest ready",
    )
    .await
    .unwrap();

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
    .await
    .unwrap();

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
    .await
    .unwrap();

    assert!(outbound.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn maybe_deliver_cron_output_fails_when_no_outbound_is_configured() {
    let req = cron_delivery_request();

    let error = crate::server::helpers::maybe_deliver_cron_output(None, &req, "Daily digest ready")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outbound is unavailable"));
}

#[tokio::test]
async fn maybe_deliver_cron_output_propagates_channel_failure() {
    let req = cron_delivery_request();
    let outbound: Arc<dyn moltis_channels::ChannelOutbound> = Arc::new(FailingChannelOutbound);

    let error = crate::server::helpers::maybe_deliver_cron_output(
        Some(outbound),
        &req,
        "Daily digest ready",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("test delivery failure"));
}
