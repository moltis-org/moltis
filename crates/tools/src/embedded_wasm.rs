#[cfg(feature = "wasm")]
use std::borrow::Cow;
#[cfg(all(feature = "wasm", debug_assertions))]
use std::path::PathBuf;

#[cfg(feature = "wasm")]
use crate::Result;
#[cfg(all(feature = "wasm", debug_assertions))]
use crate::error::Context;

#[cfg(all(feature = "wasm", not(debug_assertions)))]
const CALC_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_calc.wasm"
));
#[cfg(all(feature = "wasm", not(debug_assertions)))]
const WEB_FETCH_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_web_fetch.wasm"
));
#[cfg(all(feature = "wasm", not(debug_assertions)))]
const WEB_SEARCH_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_web_search.wasm"
));

#[cfg(all(feature = "wasm", debug_assertions))]
fn component_debug_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/wasm32-wasip2/release/{file_name}"))
}

#[cfg(all(feature = "wasm", debug_assertions))]
fn load_component_debug_bytes(file_name: &str, tool_name: &str) -> Result<Cow<'static, [u8]>> {
    let path = component_debug_path(file_name);
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "missing {tool_name} wasm artifact at {}; run `just wasm-tools` first",
            path.display()
        )
    })?;
    Ok(Cow::Owned(bytes))
}

/// Load the embedded calc component bytes.
///
/// In debug builds this reads the guest artifact from `target/` so iterative
/// development can rebuild the component without relinking the host.
/// In release builds this uses `include_bytes!` for deterministic embedding.
#[cfg(feature = "wasm")]
pub fn calc_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component_debug_bytes("moltis_wasm_calc.wasm", "calc")
    }

    #[cfg(not(debug_assertions))]
    {
        Ok(Cow::Borrowed(CALC_COMPONENT_RELEASE_BYTES))
    }
}

/// Load the embedded web_fetch component bytes.
#[cfg(feature = "wasm")]
pub fn web_fetch_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component_debug_bytes("moltis_wasm_web_fetch.wasm", "web_fetch")
    }

    #[cfg(not(debug_assertions))]
    {
        Ok(Cow::Borrowed(WEB_FETCH_COMPONENT_RELEASE_BYTES))
    }
}

/// Load the embedded web_search component bytes.
#[cfg(feature = "wasm")]
pub fn web_search_component_bytes() -> Result<Cow<'static, [u8]>> {
    #[cfg(debug_assertions)]
    {
        load_component_debug_bytes("moltis_wasm_web_search.wasm", "web_search")
    }

    #[cfg(not(debug_assertions))]
    {
        Ok(Cow::Borrowed(WEB_SEARCH_COMPONENT_RELEASE_BYTES))
    }
}
