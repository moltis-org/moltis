use std::path::PathBuf;

use anyhow::{Context, Result};

/// Pre-compile WASM components to `.cwasm` for AOT deserialization at runtime.
///
/// Uses the same `wasmtime::Config` as `WasmComponentEngine::new()` in
/// `crates/tools/src/wasm_engine.rs` so the serialized bytes are compatible.
fn main() -> Result<()> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.consume_fuel(true);
    config.epoch_interruption(true);
    let engine = wasmtime::Engine::new(&config)?;

    let wasm_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wasm32-wasip2/release");

    let components = [
        "moltis_wasm_calc",
        "moltis_wasm_web_fetch",
        "moltis_wasm_web_search",
    ];

    for name in &components {
        let wasm_path = wasm_dir.join(format!("{name}.wasm"));
        let cwasm_path = wasm_dir.join(format!("{name}.cwasm"));

        if !wasm_path.exists() {
            eprintln!("skip: {wasm_path:?} not found");
            continue;
        }

        let wasm_bytes = std::fs::read(&wasm_path)
            .with_context(|| format!("failed to read {}", wasm_path.display()))?;

        let precompiled = engine
            .precompile_component(&wasm_bytes)
            .with_context(|| format!("failed to precompile {}", wasm_path.display()))?;

        std::fs::write(&cwasm_path, &precompiled)
            .with_context(|| format!("failed to write {}", cwasm_path.display()))?;

        eprintln!(
            "{name}: {wasm_kb}KB .wasm -> {cwasm_kb}KB .cwasm",
            wasm_kb = wasm_bytes.len() / 1024,
            cwasm_kb = precompiled.len() / 1024,
        );
    }

    Ok(())
}
