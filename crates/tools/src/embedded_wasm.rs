//! WASM component loading with three-tier resolution:
//!
//! 1. **Debug filesystem** — reads from `target/wasm32-wasip2/release/` for
//!    iterative development without relinking the host.
//! 2. **External share dir** — `share_dir()/wasm/` for packaged deployments
//!    where WASM components live outside the binary.
//! 3. **Embedded fallback** — `include_bytes!` compiled into the binary (only
//!    when both `wasm` and `embedded-wasm` features are enabled).

#[cfg(feature = "wasm")]
use std::borrow::Cow;
#[cfg(all(feature = "wasm", debug_assertions))]
use std::path::PathBuf;

#[cfg(feature = "wasm")]
use crate::Result;

// ── Embedded constants (release + embedded-wasm) ────────────────────────────

#[cfg(all(feature = "wasm", feature = "embedded-wasm", not(debug_assertions)))]
const CALC_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_calc.wasm"
));
#[cfg(all(feature = "wasm", feature = "embedded-wasm", not(debug_assertions)))]
const WEB_FETCH_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_web_fetch.wasm"
));
#[cfg(all(feature = "wasm", feature = "embedded-wasm", not(debug_assertions)))]
const WEB_SEARCH_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_web_search.wasm"
));

// ── Debug path helper ───────────────────────────────────────────────────────
#[cfg(all(feature = "wasm", debug_assertions))]
fn component_debug_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/wasm32-wasip2/release/{file_name}"))
}

// ── Shared resolution logic ─────────────────────────────────────────────────

/// Try to load a WASM component from the external share directory.
#[cfg(feature = "wasm")]
fn load_from_share_dir(file_name: &str) -> Option<Cow<'static, [u8]>> {
    let share = moltis_config::share_dir()?;
    let path = share.join("wasm").join(file_name);
    std::fs::read(&path).ok().map(Cow::Owned)
}

/// Load a WASM component with three-tier resolution:
/// 1. Debug filesystem (debug builds only)
/// 2. External share dir
/// 3. Embedded fallback
#[cfg(all(feature = "wasm", debug_assertions))]
fn load_component(file_name: &str, tool_name: &str) -> Result<Cow<'static, [u8]>> {
    // 1. Debug filesystem
    let path = component_debug_path(file_name);
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(Cow::Owned(bytes));
    }

    // 2. External share dir
    if let Some(bytes) = load_from_share_dir(file_name) {
        return Ok(bytes);
    }

    // 3. File not found
    Err(crate::error::Error::message(format!(
        "missing {tool_name} wasm artifact at {}; run `just wasm-tools` first",
        path.display()
    )))
}

/// Load the embedded calc component bytes.
///
/// In debug builds this reads the guest artifact from `target/` so iterative
/// development can rebuild the component without relinking the host.
/// In release builds the resolution order is: external share dir → embedded.
#[cfg(feature = "wasm")]
pub fn calc_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component("moltis_wasm_calc.wasm", "calc")
    }

    #[cfg(not(debug_assertions))]
    {
        // External share dir first
        if let Some(bytes) = load_from_share_dir("moltis_wasm_calc.wasm") {
            return Ok(bytes);
        }
        // Embedded fallback
        #[cfg(feature = "embedded-wasm")]
        {
            Ok(Cow::Borrowed(CALC_COMPONENT_RELEASE_BYTES))
        }
        #[cfg(not(feature = "embedded-wasm"))]
        {
            Err(crate::error::Error::message(
                "calc WASM component not found: install to share_dir/wasm/ or enable embedded-wasm feature",
            ))
        }
    }
}

/// Load the embedded web_fetch component bytes.
///
/// In release builds the resolution order is: external share dir → embedded.
#[cfg(feature = "wasm")]
pub fn web_fetch_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component("moltis_wasm_web_fetch.wasm", "web_fetch")
    }

    #[cfg(not(debug_assertions))]
    {
        if let Some(bytes) = load_from_share_dir("moltis_wasm_web_fetch.wasm") {
            return Ok(bytes);
        }
        #[cfg(feature = "embedded-wasm")]
        {
            Ok(Cow::Borrowed(WEB_FETCH_COMPONENT_RELEASE_BYTES))
        }
        #[cfg(not(feature = "embedded-wasm"))]
        {
            Err(crate::error::Error::message(
                "web_fetch WASM component not found: install to share_dir/wasm/ or enable embedded-wasm feature",
            ))
        }
    }
}

/// Load the embedded web_search component bytes.
///
/// In release builds the resolution order is: external share dir → embedded.
#[cfg(feature = "wasm")]
pub fn web_search_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component("moltis_wasm_web_search.wasm", "web_search")
    }

    #[cfg(not(debug_assertions))]
    {
        if let Some(bytes) = load_from_share_dir("moltis_wasm_web_search.wasm") {
            return Ok(bytes);
        }
        #[cfg(feature = "embedded-wasm")]
        {
            Ok(Cow::Borrowed(WEB_SEARCH_COMPONENT_RELEASE_BYTES))
        }
        #[cfg(not(feature = "embedded-wasm"))]
        {
            Err(crate::error::Error::message(
                "web_search WASM component not found: install to share_dir/wasm/ or enable embedded-wasm feature",
            ))
        }
    }
}

/// Whether the release bytes are pre-compiled (`.cwasm`) or raw (`.wasm`).
///
/// Used by `register_wasm_tools()` to choose between `deserialize_component()`
/// and `compile_component()`.
#[cfg(feature = "wasm")]
pub fn is_precompiled() -> bool {
    false
}
