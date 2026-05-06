use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Channels
    reg.register(
        "channels.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    // channels.list is an alias for channels.status (used by the UI)
    reg.register(
        "channels.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.add",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .add(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.retry_ownership",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .retry_ownership(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.logout",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .logout(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .senders_list(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.approve",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .sender_approve(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.deny",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .sender_deny(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "send",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .send(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.oauth_start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .oauth_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.oauth_complete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .oauth_complete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}
