use moltis_protocol::{ErrorShape, error_codes};

use super::MethodRegistry;

pub(super) fn register(reg: &mut MethodRegistry) {
    // subscribe: add events to client's subscription set
    reg.register(
        "subscribe",
        Box::new(|ctx| {
            Box::pin(async move {
                let events: Vec<String> = ctx
                    .params
                    .get("events")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing or invalid 'events' array",
                        )
                    })?;

                let subscription_id = uuid::Uuid::new_v4().to_string();

                let mut inner = ctx.state.inner.write().await;
                if let Some(client) = inner.clients.get_mut(&ctx.client_conn_id) {
                    let subs = client.subscriptions.get_or_insert_with(Default::default);
                    for event in &events {
                        subs.insert(event.clone());
                    }
                }

                Ok(serde_json::json!({
                    "subscriptionId": subscription_id,
                    "events": events,
                }))
            })
        }),
    );

    // unsubscribe: remove events from client's subscription set
    reg.register(
        "unsubscribe",
        Box::new(|ctx| {
            Box::pin(async move {
                let events: Option<Vec<String>> = ctx
                    .params
                    .get("events")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());

                let subscription_id: Option<String> = ctx
                    .params
                    .get("subscriptionId")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                if events.is_none() && subscription_id.is_none() {
                    return Err(ErrorShape::new(
                        error_codes::INVALID_REQUEST,
                        "provide 'events' or 'subscriptionId'",
                    ));
                }

                let mut inner = ctx.state.inner.write().await;
                if let Some(client) = inner.clients.get_mut(&ctx.client_conn_id) {
                    if let Some(ref events) = events
                        && let Some(ref mut subs) = client.subscriptions
                    {
                        for event in events {
                            subs.remove(event);
                        }
                    }
                    // If subscriptionId is provided, clear all subscriptions
                    // (each subscribe call returns a unique ID — unsubscribing by ID
                    // resets to empty set).
                    if subscription_id.is_some() && events.is_none() {
                        client.subscriptions = Some(Default::default());
                    }
                }

                Ok(serde_json::json!({}))
            })
        }),
    );
}
