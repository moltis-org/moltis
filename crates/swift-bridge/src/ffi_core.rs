//! Core FFI exports: version, identity, chat, providers, models, registry,
//! free_string, callback registration, httpd lifecycle, abort/peek, and shutdown.

use std::ffi::{CString, c_char};

use {
    moltis_provider_setup::{
        KeyStore, detect_auto_provider_sources_with_overrides, known_providers,
    },
    secrecy::ExposeSecret,
};

use crate::{
    callbacks::{
        LOG_CALLBACK, LogCallback, NETWORK_AUDIT_CALLBACK, NetworkAuditCallback,
        SESSION_EVENT_CALLBACK, SessionEventCallback, emit_log, emit_session_event,
    },
    chat::build_chat_response,
    helpers::{
        config_dir_string, encode_error, encode_json, parse_ffi_request, read_c_string,
        record_call, trace_call, with_ffi_boundary,
    },
    state::{BRIDGE, HTTPD, HttpdHandle, build_registry, stop_httpd_handle},
    types::*,
};

// ── Version / Identity ───────────────────────────────────────────────────

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_version() -> *mut c_char {
    record_call("moltis_version");
    trace_call("moltis_version");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "moltis_version called");
        let response = VersionResponse {
            bridge_version: moltis_config::VERSION,
            moltis_version: moltis_config::VERSION,
            config_dir: config_dir_string(),
        };
        emit_log(
            "INFO",
            "bridge",
            &format!(
                "version: bridge={} config_dir={}",
                response.bridge_version, response.config_dir
            ),
        );
        encode_json(&response)
    })
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_get_identity() -> *mut c_char {
    record_call("moltis_get_identity");
    trace_call("moltis_get_identity");

    with_ffi_boundary(|| {
        let resolved = moltis_config::resolve_identity();
        emit_log("DEBUG", "bridge", "moltis_get_identity called");
        encode_json(&resolved)
    })
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_chat_json(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_chat_json");
    trace_call("moltis_chat_json");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<ChatRequest>("moltis_chat_json", request_json) {
            Ok(request) => request,
            Err(e) => return e,
        };

        build_chat_response(request)
    })
}

// ── Provider management ──────────────────────────────────────────────────

/// Returns JSON array of all known providers.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_known_providers() -> *mut c_char {
    record_call("moltis_known_providers");
    trace_call("moltis_known_providers");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Loading known providers");
        let providers: Vec<BridgeKnownProvider> = known_providers()
            .into_iter()
            .map(|p| BridgeKnownProvider {
                name: p.name,
                display_name: p.display_name,
                auth_type: p.auth_type.as_str(),
                env_key: p.env_key,
                default_base_url: p.default_base_url,
                requires_model: p.requires_model,
                key_optional: p.key_optional,
            })
            .collect();
        emit_log(
            "INFO",
            "bridge",
            &format!("Known providers: {}", providers.len()),
        );
        encode_json(&providers)
    })
}

/// Returns JSON array of auto-detected provider sources.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_detect_providers() -> *mut c_char {
    record_call("moltis_detect_providers");
    trace_call("moltis_detect_providers");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Detecting provider sources");
        let config = moltis_config::discover_and_load();
        let sources =
            detect_auto_provider_sources_with_overrides(&config.providers, None, &config.env);
        let bridge_sources: Vec<BridgeDetectedSource> = sources
            .into_iter()
            .map(|s| BridgeDetectedSource {
                provider: s.provider,
                source: s.source,
            })
            .collect();
        let names: Vec<&str> = bridge_sources.iter().map(|s| s.provider.as_str()).collect();
        emit_log(
            "INFO",
            "bridge",
            &format!("Detected {} sources: {:?}", bridge_sources.len(), names),
        );
        encode_json(&bridge_sources)
    })
}

/// Saves provider configuration (API key, base URL, models).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_save_provider_config(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_save_provider_config");
    trace_call("moltis_save_provider_config");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SaveProviderRequest>(
            "moltis_save_provider_config",
            request_json,
        ) {
            Ok(request) => request,
            Err(e) => return e,
        };

        emit_log(
            "INFO",
            "bridge.config",
            &format!("Saving config for provider={}", request.provider),
        );

        let key_store = KeyStore::new();
        let api_key = request.api_key.map(|s| s.expose_secret().clone());
        match key_store.save_config(&request.provider, api_key, request.base_url, request.models) {
            Ok(()) => {
                emit_log("INFO", "bridge.config", "Provider config saved");
                encode_json(&OkResponse { ok: true })
            },
            Err(error) => {
                emit_log("ERROR", "bridge.config", &format!("Save failed: {error}"));
                encode_error("save_failed", &error.to_string())
            },
        }
    })
}

/// Lists all discovered models from the current provider registry.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_list_models() -> *mut c_char {
    record_call("moltis_list_models");
    trace_call("moltis_list_models");

    with_ffi_boundary(|| {
        emit_log("DEBUG", "bridge", "Listing models from registry");
        let registry = BRIDGE.registry.read().unwrap_or_else(|e| e.into_inner());
        let models: Vec<BridgeModelInfo> = registry
            .list_models()
            .iter()
            .map(|m| BridgeModelInfo {
                id: m.id.clone(),
                provider: m.provider.clone(),
                display_name: m.display_name.clone(),
                created_at: m.created_at,
                recommended: m.recommended,
            })
            .collect();
        emit_log("INFO", "bridge", &format!("Listed {} models", models.len()));
        encode_json(&models)
    })
}

/// Rebuilds the global provider registry from saved config + env.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_refresh_registry() -> *mut c_char {
    record_call("moltis_refresh_registry");
    trace_call("moltis_refresh_registry");

    with_ffi_boundary(|| {
        emit_log("INFO", "bridge", "Refreshing provider registry");
        let new_registry = build_registry();
        let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
        *guard = new_registry;
        emit_log("INFO", "bridge", "Provider registry rebuilt");
        encode_json(&OkResponse { ok: true })
    })
}

// ── Free / Callbacks ─────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
/// # Safety
///
/// `ptr` must either be null or a pointer previously returned by one of the
/// `moltis_*` FFI functions from this crate. Passing any other pointer, or
/// freeing the same pointer more than once, is undefined behavior.
pub unsafe extern "C" fn moltis_free_string(ptr: *mut c_char) {
    record_call("moltis_free_string");

    if ptr.is_null() {
        return;
    }

    // SAFETY: pointer must originate from `CString::into_raw` in this crate.
    let _ = unsafe { CString::from_raw(ptr) };
}

/// Register a callback to receive log events from the Rust bridge.
/// Only the first call takes effect; subsequent calls are ignored.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_log_callback(callback: LogCallback) {
    let _ = LOG_CALLBACK.set(callback);
    emit_log("INFO", "bridge", "Log callback registered");
}

/// Register a callback for session events (created, deleted, patched).
///
/// The callback receives a JSON string: `{"kind":"created","sessionKey":"..."}`.
/// Rust owns the pointer -- the callback must copy the data before returning.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_session_event_callback(callback: SessionEventCallback) {
    if SESSION_EVENT_CALLBACK.set(callback).is_ok() {
        // Spawn a background task that subscribes to session events and
        // invokes the callback for each one.
        let bus = BRIDGE
            .session_metadata
            .event_bus()
            .expect("bridge session_metadata must have an event bus");
        let mut rx = bus.subscribe();
        BRIDGE.runtime.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => emit_session_event(&event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        emit_log(
                            "WARN",
                            "bridge.session_events",
                            &format!("Session event subscriber lagged, skipped {n} events"),
                        );
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        emit_log("INFO", "bridge", "Session event callback registered");
    }
}

/// Register a callback for network audit events (domain filter decisions).
///
/// The callback receives a JSON string with fields: `domain`, `port`,
/// `protocol`, `action`, `source`, and optionally `method` and `path`.
/// Rust owns the pointer -- the callback must copy the data before returning.
///
/// # Safety
///
/// `callback` must be a valid function pointer that remains valid for
/// the lifetime of the process.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moltis_set_network_audit_callback(callback: NetworkAuditCallback) {
    let _ = NETWORK_AUDIT_CALLBACK.set(callback);
    emit_log("INFO", "bridge", "Network audit callback registered");
}

// ── HTTPD lifecycle ──────────────────────────────────────────────────────

/// Starts the embedded HTTP server with the full Moltis gateway.
/// Returns JSON with `{"running": true, "addr": "..."}`.
/// If already running, returns the current status without restarting.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_start_httpd(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_start_httpd");
    trace_call("moltis_start_httpd");

    with_ffi_boundary(|| {
        let request: StartHttpdRequest = if request_json.is_null() {
            StartHttpdRequest {
                host: default_httpd_host(),
                port: default_httpd_port(),
                config_dir: None,
                data_dir: None,
            }
        } else {
            match read_c_string(request_json) {
                Ok(raw) => match serde_json::from_str(&raw) {
                    Ok(r) => r,
                    Err(e) => return encode_error("invalid_json", &e.to_string()),
                },
                Err(msg) => return encode_error("null_pointer_or_invalid_utf8", &msg),
            }
        };

        let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());

        // Already running -- return current status.
        if let Some(handle) = guard.as_ref() {
            emit_log(
                "INFO",
                "bridge.httpd",
                &format!("Server already running on {}", handle.addr),
            );
            return encode_json(&HttpdStatusResponse {
                running: true,
                addr: Some(handle.addr.to_string()),
            });
        }

        let bind_addr = format!("{}:{}", request.host, request.port);
        emit_log(
            "INFO",
            "bridge.httpd",
            &format!("Starting full gateway on {bind_addr}"),
        );

        // Prepare the full gateway (config, DB migrations, service wiring,
        // background tasks). This runs on the bridge runtime via block_on --
        // valid because this is an extern "C" fn, not async.
        let prepared = match BRIDGE
            .runtime
            .block_on(moltis_httpd::prepare_httpd_embedded(
                &request.host,
                request.port,
                true, // no_tls -- the macOS app manages its own TLS if needed
                None, // log_buffer
                request.config_dir.map(std::path::PathBuf::from),
                request.data_dir.map(std::path::PathBuf::from),
                Some(moltis_web::web_routes), // full web UI
                BRIDGE.session_metadata.event_bus().cloned(), // share bus with gateway
            )) {
            Ok(p) => p,
            Err(e) => {
                emit_log(
                    "ERROR",
                    "bridge.httpd",
                    &format!("Gateway init failed: {e}"),
                );
                return encode_error("gateway_init_failed", &e.to_string());
            },
        };

        let gateway_state = prepared.state;

        // Bind the TCP listener synchronously so we can report errors immediately.
        let listener = match BRIDGE
            .runtime
            .block_on(tokio::net::TcpListener::bind(&bind_addr))
        {
            Ok(l) => l,
            Err(e) => {
                emit_log("ERROR", "bridge.httpd", &format!("Bind failed: {e}"));
                return encode_error("bind_failed", &e.to_string());
            },
        };

        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(e) => return encode_error("addr_error", &e.to_string()),
        };

        // Subscribe to the network audit broadcast (if the proxy is active)
        // and forward entries to Swift via the registered callback.
        #[cfg(feature = "trusted-network")]
        if let Some(ref audit_buf) = prepared.audit_buffer {
            let mut audit_rx = audit_buf.subscribe();
            BRIDGE.runtime.spawn(async move {
                loop {
                    match audit_rx.recv().await {
                        Ok(entry) => crate::callbacks::emit_network_audit(&entry),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            emit_log("INFO", "bridge.httpd", "Network audit bridge subscribed");
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let app = prepared.app;
        // Keep the proxy shutdown sender alive for the server's full lifetime;
        // dropping it closes the watch channel and terminates the proxy.
        #[cfg(feature = "trusted-network")]
        let _proxy_shutdown_tx = prepared._proxy_shutdown_tx;

        let server_task = BRIDGE.runtime.spawn(async move {
            // Hold the proxy sender inside the spawn so it lives as long as the server.
            #[cfg(feature = "trusted-network")]
            let _keep_proxy = _proxy_shutdown_tx;
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(e) = server.await {
                emit_log("ERROR", "bridge.httpd", &format!("Server error: {e}"));
            }
            emit_log("INFO", "bridge.httpd", "Server stopped");
        });

        emit_log(
            "INFO",
            "bridge.httpd",
            &format!("Gateway listening on {addr}"),
        );
        *guard = Some(HttpdHandle {
            shutdown_tx,
            server_task,
            addr,
            state: gateway_state,
        });

        encode_json(&HttpdStatusResponse {
            running: true,
            addr: Some(addr.to_string()),
        })
    })
}

/// Stops the embedded HTTP server. Returns `{"running": false}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_stop_httpd() -> *mut c_char {
    record_call("moltis_stop_httpd");
    trace_call("moltis_stop_httpd");

    with_ffi_boundary(|| {
        let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let handle = guard.take();
        drop(guard);
        if let Some(handle) = handle {
            let message = format!("Stopping httpd on {}", handle.addr);
            stop_httpd_handle(handle, "bridge.httpd", &message);
        } else {
            emit_log(
                "DEBUG",
                "bridge.httpd",
                "Stop called but server not running",
            );
        }
        encode_json(&HttpdStatusResponse {
            running: false,
            addr: None,
        })
    })
}

/// Returns the current httpd server status.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_httpd_status() -> *mut c_char {
    record_call("moltis_httpd_status");
    trace_call("moltis_httpd_status");

    with_ffi_boundary(|| {
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(handle) => encode_json(&HttpdStatusResponse {
                running: true,
                addr: Some(handle.addr.to_string()),
            }),
            None => encode_json(&HttpdStatusResponse {
                running: false,
                addr: None,
            }),
        }
    })
}

// ── Abort / Peek FFI ─────────────────────────────────────────────────────

/// Abort the active generation for a session. Requires the gateway to be
/// running (via `moltis_start_httpd`). Returns JSON with `{"aborted": bool}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_abort_session(session_key: *const c_char) -> *mut c_char {
    record_call("moltis_abort_session");
    trace_call("moltis_abort_session");

    with_ffi_boundary(|| {
        let key = match read_c_string(session_key) {
            Ok(k) => k,
            Err(msg) => return encode_error("invalid_session_key", &msg),
        };
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = guard.as_ref() else {
            return encode_error("gateway_not_running", "start the gateway first");
        };
        let state = std::sync::Arc::clone(&handle.state);
        drop(guard);

        let params = serde_json::json!({ "sessionKey": key });
        match BRIDGE
            .runtime
            .block_on(async { state.chat().await.abort(params).await })
        {
            Ok(res) => encode_json(&res),
            Err(e) => encode_error("abort_failed", &e.to_string()),
        }
    })
}

/// Peek at the current activity for a session. Requires the gateway to be
/// running. Returns JSON with `{"active": bool, ...}`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_peek_session(session_key: *const c_char) -> *mut c_char {
    record_call("moltis_peek_session");
    trace_call("moltis_peek_session");

    with_ffi_boundary(|| {
        let key = match read_c_string(session_key) {
            Ok(k) => k,
            Err(msg) => return encode_error("invalid_session_key", &msg),
        };
        let guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = guard.as_ref() else {
            return encode_error("gateway_not_running", "start the gateway first");
        };
        let state = std::sync::Arc::clone(&handle.state);
        drop(guard);

        let params = serde_json::json!({ "sessionKey": key });
        match BRIDGE
            .runtime
            .block_on(async { state.chat().await.peek(params).await })
        {
            Ok(res) => encode_json(&res),
            Err(e) => encode_error("peek_failed", &e.to_string()),
        }
    })
}

// ── Shutdown ─────────────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn moltis_shutdown() {
    record_call("moltis_shutdown");
    trace_call("moltis_shutdown");
    emit_log("INFO", "bridge", "Shutdown requested");

    // Stop the HTTP server if it is running.
    let mut guard = HTTPD.lock().unwrap_or_else(|e| e.into_inner());
    let handle = guard.take();
    drop(guard);
    if let Some(handle) = handle {
        let message = format!("Stopping httpd on {} during shutdown", handle.addr);
        stop_httpd_handle(handle, "bridge", &message);
    }

    emit_log("INFO", "bridge", "Shutdown complete");
}

// ── Tests ────────────────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use std::{
        ffi::{CStr, CString, c_char, c_void},
        sync::{Arc, Mutex},
    };

    use serde_json::Value;

    use {super::*, crate::state::BRIDGE};

    fn text_from_ptr(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "ffi returned null pointer");

        // SAFETY: pointer returned by this crate, converted back exactly once.
        let owned = unsafe { CString::from_raw(ptr) };

        match owned.into_string() {
            Ok(text) => text,
            Err(error) => panic!("failed to decode UTF-8 from ffi pointer: {error}"),
        }
    }

    fn json_from_ptr(ptr: *mut c_char) -> Value {
        let text = text_from_ptr(ptr);
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(error) => panic!("failed to parse ffi json payload: {error}; payload={text}"),
        }
    }

    #[test]
    fn version_returns_expected_payload() {
        let payload = json_from_ptr(moltis_version());

        let version = payload
            .get("bridge_version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(version, moltis_config::VERSION);

        let config_dir = payload
            .get("config_dir")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(!config_dir.is_empty(), "config_dir should be populated");
    }

    #[test]
    fn chat_returns_error_for_null_pointer() {
        let payload = json_from_ptr(moltis_chat_json(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn chat_returns_validation_counts() {
        let request =
            r#"{"message":"hello from swift","config_toml":"[server]\nport = \"invalid\""}"#;
        let c_request = match CString::new(request) {
            Ok(value) => value,
            Err(error) => panic!("failed to build c string for test request: {error}"),
        };

        let payload = json_from_ptr(moltis_chat_json(c_request.as_ptr()));

        // Chat response should have a reply (either from LLM or fallback)
        assert!(
            payload.get("reply").and_then(Value::as_str).is_some(),
            "response should contain a reply field"
        );

        let has_errors = payload
            .get("validation")
            .and_then(|value| value.get("has_errors"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        assert!(has_errors, "validation should detect invalid config value");
    }

    #[test]
    fn known_providers_returns_array() {
        let payload = json_from_ptr(moltis_known_providers());

        let providers = payload.as_array();
        assert!(
            providers.is_some(),
            "known_providers should return a JSON array"
        );
        let providers = providers.unwrap_or_else(|| panic!("not an array"));
        assert!(!providers.is_empty(), "should have at least one provider");

        // Check first provider has expected fields
        let first = &providers[0];
        assert!(first.get("name").and_then(Value::as_str).is_some());
        assert!(first.get("display_name").and_then(Value::as_str).is_some());
        assert!(first.get("auth_type").and_then(Value::as_str).is_some());
    }

    #[test]
    fn detect_providers_returns_array() {
        let payload = json_from_ptr(moltis_detect_providers());

        // Should always return a JSON array (possibly empty)
        assert!(
            payload.as_array().is_some(),
            "detect_providers should return a JSON array"
        );
    }

    #[test]
    fn save_provider_config_returns_error_for_null() {
        let payload = json_from_ptr(moltis_save_provider_config(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }

    #[test]
    fn list_models_returns_array() {
        let payload = json_from_ptr(moltis_list_models());

        assert!(
            payload.as_array().is_some(),
            "list_models should return a JSON array"
        );
    }

    #[test]
    fn refresh_registry_returns_ok() {
        let payload = json_from_ptr(moltis_refresh_registry());

        let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
        assert!(ok, "refresh_registry should return ok: true");
    }

    #[test]
    fn free_string_tolerates_null_pointer() {
        // SAFETY: null pointers are explicitly accepted and treated as no-op.
        unsafe {
            moltis_free_string(std::ptr::null_mut());
        }
    }

    #[test]
    fn chat_stream_sends_error_for_null_pointer() {
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        // Leak the Arc into user_data so the callback can access it.
        let user_data = Arc::into_raw(events_clone) as *mut c_void;

        unsafe extern "C" fn test_callback(event_json: *const c_char, user_data: *mut c_void) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events.lock().unwrap_or_else(|e| e.into_inner()).push(json);
            }
        }

        // SAFETY: null request_json triggers synchronous error callback.
        unsafe {
            crate::chat::moltis_chat_stream(std::ptr::null(), test_callback, user_data);
        }

        // Reclaim the Arc.
        let events = unsafe { Arc::from_raw(user_data as *const Mutex<Vec<String>>) };
        let received = events.lock().unwrap_or_else(|e| e.into_inner());

        assert_eq!(received.len(), 1, "should receive exactly one error event");
        let parsed: Value =
            serde_json::from_str(&received[0]).unwrap_or_else(|e| panic!("bad json: {e}"));
        assert_eq!(
            parsed.get("type").and_then(Value::as_str),
            Some("error"),
            "event type should be 'error'"
        );
    }

    #[test]
    #[serial_test::serial]
    fn httpd_start_and_stop() {
        // Start on a random high port to avoid conflicts.
        let request = r#"{"host":"127.0.0.1","port":0}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));

        let payload = json_from_ptr(moltis_start_httpd(c_request.as_ptr()));
        assert_eq!(
            payload.get("running").and_then(Value::as_bool),
            Some(true),
            "server should be running after start"
        );
        assert!(
            payload.get("addr").and_then(Value::as_str).is_some(),
            "should report the bound address"
        );

        // Status should confirm running.
        let status = json_from_ptr(moltis_httpd_status());
        assert_eq!(status.get("running").and_then(Value::as_bool), Some(true),);

        // Stop.
        let stopped = json_from_ptr(moltis_stop_httpd());
        assert_eq!(stopped.get("running").and_then(Value::as_bool), Some(false),);

        // Status after stop.
        let status2 = json_from_ptr(moltis_httpd_status());
        assert_eq!(status2.get("running").and_then(Value::as_bool), Some(false),);
    }

    #[test]
    #[serial_test::serial]
    fn httpd_stop_when_not_running() {
        // Stop without start should still return running: false.
        let payload = json_from_ptr(moltis_stop_httpd());
        assert_eq!(payload.get("running").and_then(Value::as_bool), Some(false),);
    }

    #[test]
    #[serial_test::serial]
    fn chat_stream_sends_error_for_no_provider() {
        // Force a no-provider environment so this test exercises the
        // synchronous error callback path deterministically.
        let original_registry = {
            let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, moltis_providers::ProviderRegistry::empty())
        };
        struct RestoreRegistry(Option<moltis_providers::ProviderRegistry>);
        impl Drop for RestoreRegistry {
            fn drop(&mut self) {
                if let Some(registry) = self.0.take() {
                    let mut guard = BRIDGE.registry.write().unwrap_or_else(|e| e.into_inner());
                    *guard = registry;
                }
            }
        }
        let _restore_registry = RestoreRegistry(Some(original_registry));

        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let user_data = Arc::into_raw(events_clone) as *mut c_void;

        unsafe extern "C" fn test_callback(event_json: *const c_char, user_data: *mut c_void) {
            // SAFETY: event_json is a valid NUL-terminated C string from
            // send_stream_event; user_data is our Arc<Mutex<Vec<String>>>.
            unsafe {
                let json = CStr::from_ptr(event_json).to_string_lossy().to_string();
                let events = &*(user_data as *const Mutex<Vec<String>>);
                events.lock().unwrap_or_else(|e| e.into_inner()).push(json);
            }
        }

        // With an empty registry, this must error synchronously.
        let request = r#"{"message":"test","model":"nonexistent-model-xyz"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));

        // SAFETY: valid C string, valid callback, valid user_data.
        unsafe {
            crate::chat::moltis_chat_stream(c_request.as_ptr(), test_callback, user_data);
        }

        let events = unsafe { Arc::from_raw(user_data as *const Mutex<Vec<String>>) };
        let received = events.lock().unwrap_or_else(|e| e.into_inner());

        assert!(
            !received.is_empty(),
            "should receive at least one stream event"
        );
        let parsed: Value =
            serde_json::from_str(&received[0]).unwrap_or_else(|e| panic!("bad json: {e}"));
        assert_eq!(
            parsed.get("type").and_then(Value::as_str),
            Some("error"),
            "expected an error event when no provider is available"
        );
    }
}
