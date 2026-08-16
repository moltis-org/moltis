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
                    "delivery": status.delivery,
                    "config": {
                        "enabled": config.enabled,
                        "environment": config.environment,
                        "sample_rate": config.sample_rate,
                        "queue_capacity": config.queue_capacity,
                        "flush_interval_ms": config.flush_interval_ms,
                        "max_batch_bytes": config.max_batch_bytes,
                        "langfuse": {
                            "enabled": config.langfuse.enabled,
                            "host": config.langfuse.host,
                            "public_key": config.langfuse.public_key,
                            "secret_key_set": config.langfuse.secret_key.is_some(),
                            "capture_input": config.langfuse.capture_input,
                            "capture_output": config.langfuse.capture_output,
                            "capture_tool_io": config.langfuse.capture_tool_io,
                            "timeout_secs": config.langfuse.timeout_secs,
                        },
                        "otlp": {
                            "enabled": config.otlp.enabled,
                            "endpoint": config.otlp.endpoint,
                            "content": config.otlp.content,
                            "emit_user_id": config.otlp.emit_user_id,
                            "timeout_secs": config.otlp.timeout_secs,
                        },
                        "datadog": {
                            "enabled": config.datadog.enabled,
                            "endpoint": config.datadog.endpoint,
                            "service": config.datadog.service,
                            "api_key_set": config.datadog.api_key.is_some(),
                            "content": config.datadog.content,
                            "timeout_secs": config.datadog.timeout_secs,
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
                    #[cfg(feature = "langfuse")]
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
                    // Say so rather than falling through to the generic "no test
                    // available" answer, which would read as a Langfuse quirk.
                    #[cfg(not(feature = "langfuse"))]
                    "langfuse" => Ok(serde_json::json!({
                        "ok": false,
                        "error": "this build was compiled without the `langfuse` feature",
                    })),
                    // OTLP collectors have no standard health endpoint, and
                    // POSTing a probe span would pollute the operator's traces
                    // with fake data. Live delivery counters are the honest
                    // signal once the exporter has received real events.
                    other => Ok(serde_json::json!({
                        "ok": false,
                        "error": format!(
                            "no connection test available for `{other}`; check its \
                             delivery status after the next agent run"
                        ),
                    })),
                }
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            auth::{AuthMode, ResolvedAuth},
            methods::MethodContext,
            services::GatewayServices,
            state::GatewayState,
        },
        moltis_config::{InstrumentationConfig, LangfuseSettings},
    };

    async fn dispatch(
        state: std::sync::Arc<GatewayState>,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let mut registry = MethodRegistry::default();
        register(&mut registry);
        let response = registry
            .dispatch(MethodContext {
                request_id: "test".into(),
                method: method.into(),
                params,
                client_conn_id: "conn-1".into(),
                client_role: "operator".into(),
                client_scopes: vec!["operator.read".into(), "operator.write".into()],
                state,
                channel: None,
            })
            .await;

        assert!(response.ok, "method failed: {:?}", response.error);
        response
            .payload
            .unwrap_or_else(|| panic!("{method} returned no payload"))
    }

    fn gateway_state() -> std::sync::Arc<GatewayState> {
        GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            GatewayServices::noop(),
        )
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn status_preserves_the_latest_skipped_backend_failure() {
        let state = gateway_state();
        let config = InstrumentationConfig {
            enabled: true,
            langfuse: LangfuseSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        state.instrumentation.apply(&config, "test");

        let payload = dispatch(state, "instrumentation.status", serde_json::json!({})).await;

        assert_eq!(payload["active"], false);
        assert_eq!(payload["skipped"][0]["name"], "langfuse");
        assert_eq!(payload["config"]["flush_interval_ms"], 5_000);
        assert_eq!(payload["config"]["max_batch_bytes"], 3_000_000);
        assert_eq!(payload["config"]["langfuse"]["timeout_secs"], 10);
        assert!(
            payload["skipped"][0]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("public_key"))
        );
        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn unsupported_connection_test_points_to_live_delivery_health() {
        let payload = dispatch(
            gateway_state(),
            "instrumentation.test",
            serde_json::json!({ "backend": "otlp" }),
        )
        .await;

        assert_eq!(payload["ok"], false);
        let error = payload["error"].as_str().unwrap_or_default();
        assert!(error.contains("delivery status"));
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn status_surfaces_each_exporter_delivery_path() {
        let state = gateway_state();
        let config = InstrumentationConfig {
            enabled: true,
            langfuse: LangfuseSettings {
                enabled: true,
                public_key: "pk-test".into(),
                secret_key: Some(secrecy::Secret::new("sk-test".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };
        state.instrumentation.apply(&config, "test");

        let payload = dispatch(state, "instrumentation.status", serde_json::json!({})).await;

        assert_eq!(payload["delivery"][0]["name"], "langfuse");
        assert_eq!(payload["delivery"][0]["accepted"], 0);
        assert_eq!(payload["delivery"][0]["delivered"], 0);
        assert_eq!(payload["delivery"][0]["dropped_queue_full"], 0);
        assert_eq!(payload["delivery"][0]["dropped_failed"], 0);
        assert_eq!(payload["delivery"][0]["retries"], 0);
        assert_eq!(payload["delivery"][1]["name"], "langfuse-scores");
        moltis_observability::clear_global_sink();
    }
}
