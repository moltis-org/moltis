//! Encoding helpers, utility functions, and metrics/tracing wrappers.

use std::ffi::{CStr, CString, c_char};

use {moltis_config::validate::Severity, serde::Serialize};

use crate::{
    state::BRIDGE,
    types::{
        ErrorEnvelope, ErrorPayload, SandboxSharedHomeConfigResponse, SandboxStatusResponse,
        ValidationSummary,
    },
};

// ── Encoding helpers ─────────────────────────────────────────────────────

pub(crate) fn encode_json<T: Serialize>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(json) => json,
        Err(_) => {
            "{\"error\":{\"code\":\"serialization_error\",\"message\":\"failed to serialize response\"}}"
                .to_owned()
        }
    }
}

pub(crate) fn encode_error(code: &str, message: &str) -> String {
    encode_json(&ErrorEnvelope {
        error: ErrorPayload { code, message },
    })
}

pub(crate) fn into_c_ptr(payload: String) -> *mut c_char {
    match CString::new(payload) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

pub(crate) fn with_ffi_boundary<F>(work: F) -> *mut c_char
where
    F: FnOnce() -> String,
{
    use std::panic::{AssertUnwindSafe, catch_unwind};

    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(payload) => into_c_ptr(payload),
        Err(_) => into_c_ptr(encode_error(
            "panic",
            "unexpected panic occurred in Rust FFI boundary",
        )),
    }
}

/// Parses a C string JSON pointer into a typed request, recording errors
/// against `function` for metrics. Returns `Err(encoded_error_json)` on
/// failure so callers can early-return from `with_ffi_boundary`.
pub(crate) fn parse_ffi_request<T: serde::de::DeserializeOwned>(
    function: &'static str,
    ptr: *const c_char,
) -> Result<T, String> {
    let raw = read_c_string(ptr).map_err(|message| {
        record_error(function, "null_pointer_or_invalid_utf8");
        encode_error("null_pointer_or_invalid_utf8", &message)
    })?;
    serde_json::from_str::<T>(&raw).map_err(|error| {
        record_error(function, "invalid_json");
        encode_error("invalid_json", &error.to_string())
    })
}

#[allow(unsafe_code)]
pub(crate) fn read_c_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("request_json pointer was null".to_owned());
    }

    // SAFETY: pointer nullability is checked above, and callers guarantee a
    // valid NUL-terminated C string for the duration of the call.
    let c_str = unsafe { CStr::from_ptr(ptr) };
    match c_str.to_str() {
        Ok(text) => Ok(text.to_owned()),
        Err(_) => Err("request_json was not valid UTF-8".to_owned()),
    }
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    match bytes {
        b if b >= GB => format!("{:.1} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

pub(crate) fn build_validation_summary(config_toml: Option<&str>) -> Option<ValidationSummary> {
    let config_toml = config_toml?;
    let result = moltis_config::validate::validate_toml_str(config_toml);

    Some(ValidationSummary {
        errors: result.count(Severity::Error),
        warnings: result.count(Severity::Warning),
        info: result.count(Severity::Info),
        has_errors: result.has_errors(),
    })
}

pub(crate) fn config_dir_string() -> String {
    match moltis_config::config_dir() {
        Some(path) => path.display().to_string(),
        None => "unavailable".to_owned(),
    }
}

pub(crate) fn data_dir_string() -> String {
    moltis_config::data_dir().display().to_string()
}

pub(crate) fn vault_status_string() -> String {
    let Some(vault) = BRIDGE.credential_store.vault() else {
        return "disabled".to_owned();
    };
    match BRIDGE.runtime.block_on(async { vault.status().await }) {
        Ok(status) => format!("{status:?}").to_lowercase(),
        Err(_) => "error".to_owned(),
    }
}

pub(crate) fn sandbox_effective_default_image(config: &moltis_config::MoltisConfig) -> String {
    if let Some(value) = BRIDGE
        .sandbox_default_image_override
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return value;
    }
    config
        .tools
        .exec
        .sandbox
        .image
        .clone()
        .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_owned())
}

pub(crate) fn sandbox_backend_name(config: &moltis_config::MoltisConfig) -> String {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let backend = moltis_tools::sandbox::create_sandbox(runtime_cfg);
    backend.backend_name().to_owned()
}

pub(crate) fn sandbox_status_from_config(
    config: &moltis_config::MoltisConfig,
) -> SandboxStatusResponse {
    SandboxStatusResponse {
        backend: sandbox_backend_name(config),
        os: std::env::consts::OS.to_owned(),
        default_image: sandbox_effective_default_image(config),
    }
}

pub(crate) fn sandbox_container_prefix(config: &moltis_config::MoltisConfig) -> String {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    runtime_cfg
        .container_prefix
        .unwrap_or_else(|| "moltis-sandbox".to_owned())
}

pub(crate) fn sandbox_shared_home_config_from_config(
    config: &moltis_config::MoltisConfig,
) -> SandboxSharedHomeConfigResponse {
    let runtime_cfg = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    let mode = match config.tools.exec.sandbox.home_persistence {
        moltis_config::schema::HomePersistenceConfig::Off => "off",
        moltis_config::schema::HomePersistenceConfig::Session => "session",
        moltis_config::schema::HomePersistenceConfig::Shared => "shared",
    };

    SandboxSharedHomeConfigResponse {
        enabled: matches!(
            config.tools.exec.sandbox.home_persistence,
            moltis_config::schema::HomePersistenceConfig::Shared
        ),
        mode: mode.to_owned(),
        path: moltis_tools::sandbox::shared_home_dir_path(&runtime_cfg)
            .display()
            .to_string(),
        configured_path: config.tools.exec.sandbox.shared_home_dir.clone(),
    }
}

// ── Metrics / tracing helpers ────────────────────────────────────────────

#[cfg(feature = "metrics")]
pub(crate) fn record_call(function: &'static str) {
    metrics::counter!("moltis_swift_bridge_calls_total", "function" => function).increment(1);
}

#[cfg(not(feature = "metrics"))]
pub(crate) fn record_call(_function: &'static str) {}

#[cfg(feature = "metrics")]
pub(crate) fn record_error(function: &'static str, code: &'static str) {
    metrics::counter!(
        "moltis_swift_bridge_errors_total",
        "function" => function,
        "code" => code
    )
    .increment(1);
}

#[cfg(not(feature = "metrics"))]
pub(crate) fn record_error(_function: &'static str, _code: &'static str) {}

#[cfg(feature = "tracing")]
pub(crate) fn trace_call(function: &'static str) {
    tracing::debug!(target: "moltis_swift_bridge", function, "ffi call");
}

#[cfg(not(feature = "tracing"))]
pub(crate) fn trace_call(_function: &'static str) {}
