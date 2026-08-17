use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "openclaw.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.scan",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_scan()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "claude.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .claude_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "claude.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .claude_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "codex.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .codex_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "codex.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .codex_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "hermes.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .hermes_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "hermes.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .hermes_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}
