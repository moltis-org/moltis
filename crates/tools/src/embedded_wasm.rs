#[cfg(feature = "wasm")]
use std::{borrow::Cow, path::PathBuf};

#[cfg(feature = "wasm")]
use anyhow::{Context, Result};

#[cfg(all(feature = "wasm", not(debug_assertions)))]
const CALC_COMPONENT_RELEASE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-wasip2/release/moltis_wasm_calc.wasm"
));

#[cfg(feature = "wasm")]
fn calc_component_debug_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32-wasip2/release/moltis_wasm_calc.wasm")
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
        let path = calc_component_debug_path();
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "missing calc wasm artifact at {}; run `just wasm-tools` first",
                path.display()
            )
        })?;
        Ok(Cow::Owned(bytes))
    }

    #[cfg(not(debug_assertions))]
    {
        Ok(Cow::Borrowed(CALC_COMPONENT_RELEASE_BYTES))
    }
}
