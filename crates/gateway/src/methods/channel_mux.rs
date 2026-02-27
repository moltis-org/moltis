use moltis_protocol::{ErrorShape, error_codes};

use super::MethodRegistry;

pub(super) fn register(reg: &mut MethodRegistry) {
    // channel.join: add channels to client's joined set
    reg.register(
        "channel.join",
        Box::new(|ctx| {
            Box::pin(async move {
                let channels: Vec<String> = ctx
                    .params
                    .get("channels")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing or invalid 'channels' array",
                        )
                    })?;

                let mut inner = ctx.state.inner.write().await;
                if let Some(client) = inner.clients.get_mut(&ctx.client_conn_id) {
                    for ch in &channels {
                        client.joined_channels.insert(ch.clone());
                    }
                }

                Ok(serde_json::json!({ "joined": channels }))
            })
        }),
    );

    // channel.leave: remove channels from client's joined set
    reg.register(
        "channel.leave",
        Box::new(|ctx| {
            Box::pin(async move {
                let channels: Vec<String> = ctx
                    .params
                    .get("channels")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing or invalid 'channels' array",
                        )
                    })?;

                let mut inner = ctx.state.inner.write().await;
                if let Some(client) = inner.clients.get_mut(&ctx.client_conn_id) {
                    for ch in &channels {
                        client.joined_channels.remove(ch);
                    }
                }

                Ok(serde_json::json!({ "left": channels }))
            })
        }),
    );
}
