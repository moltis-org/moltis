//! `instrumentation.*` RPC methods backing the Instrumentation settings page.

use moltis_protocol::{ErrorShape, error_codes};

use super::MethodRegistry;

/// Register the `instrumentation.*` namespace.
pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register(
        "instrumentation.status",
        Box::new(|ctx| {
            Box::pin(async move {
                let status = ctx.state.instrumentation.status();
                let config = &ctx.state.config.instrumentation;

                // The secret key is deliberately absent: the UI shows whether
                // one is configured, never its value.
                serde_json::to_value(serde_json::json!({
                    "active": status.active,
                    "backends": status.backends,
                    "skipped": status.skipped,
                    "config": {
                        "enabled": config.enabled,
                        "environment": config.environment,
                        "sample_rate": config.sample_rate,
                        "queue_capacity": config.queue_capacity,
                        "langfuse": {
                            "enabled": config.langfuse.enabled,
                            "host": config.langfuse.host,
                            "public_key": config.langfuse.public_key,
                            "secret_key_set": config.langfuse.secret_key.is_some(),
                            "capture_input": config.langfuse.capture_input,
                            "capture_output": config.langfuse.capture_output,
                            "capture_tool_io": config.langfuse.capture_tool_io,
                        },
                        "otlp": {
                            "enabled": config.otlp.enabled,
                            "endpoint": config.otlp.endpoint,
                            "content": config.otlp.content,
                            "emit_user_id": config.otlp.emit_user_id,
                        },
                        "datadog": {
                            "enabled": config.datadog.enabled,
                            "endpoint": config.datadog.endpoint,
                            "service": config.datadog.service,
                            "api_key_set": config.datadog.api_key.is_some(),
                            "content": config.datadog.content,
                        },
                    },
                }))
                .map_err(|e| ErrorShape::new(error_codes::INTERNAL, e.to_string()))
            })
        }),
    );

    reg.register(
        "instrumentation.test",
        Box::new(|ctx| {
            Box::pin(async move {
                let backend = ctx
                    .params
                    .get("backend")
                    .and_then(|v| v.as_str())
                    .unwrap_or("langfuse");

                match backend {
                    "langfuse" => {
                        let Some(client) = ctx.state.instrumentation.langfuse() else {
                            return Ok(serde_json::json!({
                                "ok": false,
                                "error": "Langfuse is not enabled, or it failed to start. \
                                          Save valid credentials first.",
                            }));
                        };
                        match client.test_connection().await {
                            Ok(()) => Ok(serde_json::json!({ "ok": true })),
                            Err(error) => Ok(serde_json::json!({
                                "ok": false,
                                "error": error.to_string(),
                            })),
                        }
                    },
                    // OTLP collectors have no standard health endpoint, and
                    // POSTing a probe span would pollute the operator's traces
                    // with fake data. Sink counters are the honest signal.
                    other => Ok(serde_json::json!({
                        "ok": false,
                        "error": format!(
                            "no connection test available for `{other}`; check the \
                             delivery counters after the next agent run instead"
                        ),
                    })),
                }
            })
        }),
    );
}
