//! GraphQL subscription resolvers.
//!
//! Subscriptions bridge from the gateway's broadcast channel. Each subscription
//! filters events by name and deserializes the payload into typed events.

use std::sync::Arc;

use {
    async_graphql::{Context, Result, Subscription},
    tokio_stream::Stream,
};

use crate::{
    context::GqlContext,
    types::{GenericEvent, TickEvent},
};

/// Root subscription type.
#[derive(Default)]
pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    /// Chat events (streaming tokens, completion, abort).
    async fn chat_event(
        &self,
        ctx: &Context<'_>,
        session_key: Option<String>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let mut rx = c.subscribe();
        Ok(async_stream::stream! {
            while let Ok((event_name, payload)) = rx.recv().await {
                if event_name == "chat" {
                    if let Some(ref sk) = session_key
                        && let Some(event_sk) = payload.get("sessionKey").and_then(|v| v.as_str())
                        && event_sk != sk
                    {
                        continue;
                    }
                    yield GenericEvent::from(payload);
                }
            }
        })
    }

    /// Session change events (patch, switch, delete).
    async fn session_changed(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "session").await
    }

    /// Cron job notifications (created, updated, removed, run complete).
    async fn cron_notification(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "cron").await
    }

    /// Channel events.
    async fn channel_event(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "channel").await
    }

    /// Node connect/disconnect events.
    async fn node_event(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "node").await
    }

    /// System tick events (periodic heartbeat with stats).
    async fn tick(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = TickEvent>> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let mut rx = c.subscribe();
        Ok(async_stream::stream! {
            while let Ok((name, payload)) = rx.recv().await {
                if name == "tick"
                    && let Ok(evt) = serde_json::from_value::<TickEvent>(payload)
                {
                    yield evt;
                }
            }
        })
    }

    /// Log entry events.
    async fn log_entry(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "logs").await
    }

    /// MCP server status change events.
    async fn mcp_status_changed(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "mcp.status").await
    }

    /// Execution approval events.
    async fn approval_event(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let mut rx = c.subscribe();
        Ok(async_stream::stream! {
            while let Ok((event_name, payload)) = rx.recv().await {
                if event_name.starts_with("exec.approval.") {
                    yield GenericEvent::from(payload);
                }
            }
        })
    }

    /// Config change events.
    async fn config_changed(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "config").await
    }

    /// System presence change events.
    async fn presence_changed(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "presence").await
    }

    /// Metrics update events.
    async fn metrics_update(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "metrics.update").await
    }

    /// Update availability events.
    async fn update_available(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "update.available").await
    }

    /// Voice config changed events.
    async fn voice_config_changed(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "voice.config.changed").await
    }

    /// Skills install progress events.
    async fn skills_install_progress(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = GenericEvent>> {
        event_stream(ctx, "skills.install.progress").await
    }

    /// All events (unfiltered, for debugging).
    async fn all_events(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = GenericEvent>> {
        let c = ctx.data::<Arc<GqlContext>>()?;
        let mut rx = c.subscribe();
        Ok(async_stream::stream! {
            while let Ok((_event_name, payload)) = rx.recv().await {
                yield GenericEvent::from(payload);
            }
        })
    }
}

/// Helper to create a filtered event stream for a specific event name.
async fn event_stream(
    ctx: &Context<'_>,
    event_name: &'static str,
) -> Result<impl Stream<Item = GenericEvent>> {
    let c = ctx.data::<Arc<GqlContext>>()?;
    let mut rx = c.subscribe();
    Ok(async_stream::stream! {
        while let Ok((name, payload)) = rx.recv().await {
            if name == event_name {
                yield GenericEvent::from(payload);
            }
        }
    })
}
