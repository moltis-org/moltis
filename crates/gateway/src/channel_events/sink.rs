use super::*;

/// Broadcasts channel events over the gateway WebSocket.
///
/// Uses a deferred `OnceCell` reference so the sink can be created before
/// `GatewayState` exists (same pattern as cron callbacks).
pub struct GatewayChannelEventSink {
    pub(super) state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
}

impl GatewayChannelEventSink {
    pub fn new(state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>) -> Self {
        Self { state }
    }
}

pub(super) async fn emit(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    event: ChannelEvent,
) {
    if let Some(state) = state.get() {
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                warn!("failed to serialize channel event: {e}");
                return;
            },
        };

        // Render QR data as an SVG so the frontend can display it directly.
        #[cfg(feature = "whatsapp")]
        let payload = {
            let mut payload = payload;
            if let ChannelEvent::PairingQrCode { ref qr_data, .. } = event
                && let Ok(code) = qrcode::QrCode::new(qr_data)
            {
                let svg = code
                    .render::<qrcode::render::svg::Color>()
                    .min_dimensions(200, 200)
                    .quiet_zone(true)
                    .build();
                if let serde_json::Value::Object(ref mut map) = payload {
                    map.insert("qr_svg".into(), serde_json::Value::String(svg));
                }
            }
            payload
        };

        // QR code events are large and frequent — drop them if the client
        // is slow.  Pairing result events (complete/failed) are critical
        // one-shots that must not be lost.
        let droppable = matches!(event, ChannelEvent::PairingQrCode { .. });
        broadcast(state, "channel", payload, BroadcastOpts {
            drop_if_slow: droppable,
            ..Default::default()
        })
        .await;
    }
}
