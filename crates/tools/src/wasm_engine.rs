#[cfg(feature = "wasm")]
use std::{
    collections::HashMap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

#[cfg(feature = "wasm")]
use anyhow::Result;
#[cfg(feature = "wasm")]
use sha2::{Digest, Sha256};
#[cfg(feature = "wasm")]
use wasmtime_wasi::WasiView;

#[cfg(feature = "wasm")]
use crate::wasm_component::{HttpHostImpl, add_http_outgoing_handler_to_linker};

#[cfg(feature = "wasm")]
type ComponentCache = HashMap<[u8; 32], wasmtime::component::Component>;

/// Shared Wasmtime engine for component-model and core module compilation.
///
/// Components are cached in-memory by SHA-256 of the original bytes.
#[cfg(feature = "wasm")]
pub struct WasmComponentEngine {
    engine: wasmtime::Engine,
    component_cache: RwLock<ComponentCache>,
}

#[cfg(feature = "wasm")]
impl WasmComponentEngine {
    pub fn new(memory_reservation: Option<u64>) -> Result<Self> {
        let mut wasm_config = wasmtime::Config::new();
        wasm_config.wasm_component_model(true);
        wasm_config.consume_fuel(true);
        wasm_config.epoch_interruption(true);

        if let Some(bytes) = memory_reservation {
            wasm_config.memory_reservation(bytes);
        }

        let engine = wasmtime::Engine::new(&wasm_config)?;
        Ok(Self {
            engine,
            component_cache: RwLock::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }

    pub fn compile_component(&self, wasm_bytes: &[u8]) -> Result<wasmtime::component::Component> {
        let hash = hash_component_bytes(wasm_bytes);

        if let Some(component) = self.read_cache().get(&hash) {
            return Ok(component.clone());
        }

        let compiled = wasmtime::component::Component::new(&self.engine, wasm_bytes)?;
        let mut cache = self.write_cache();
        if let Some(existing) = cache.get(&hash) {
            return Ok(existing.clone());
        }
        cache.insert(hash, compiled.clone());
        Ok(compiled)
    }

    pub fn compile_module(&self, wasm_bytes: &[u8]) -> Result<wasmtime::Module> {
        wasmtime::Module::new(&self.engine, wasm_bytes)
    }

    pub fn create_http_linker<T>(
        &self,
        host_getter: impl Fn(&mut T) -> &mut HttpHostImpl + Copy + Send + Sync + 'static,
    ) -> Result<wasmtime::component::Linker<T>>
    where
        T: WasiView + 'static,
    {
        let mut linker = wasmtime::component::Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)?;
        add_http_outgoing_handler_to_linker(&mut linker, host_getter)?;
        Ok(linker)
    }

    fn read_cache(&self) -> RwLockReadGuard<'_, ComponentCache> {
        self.component_cache
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_cache(&self) -> RwLockWriteGuard<'_, ComponentCache> {
        self.component_cache
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn component_cache_len(&self) -> usize {
        self.read_cache().len()
    }
}

#[cfg(feature = "wasm")]
pub fn hash_component_bytes(wasm_bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(wasm_bytes);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod tests {
    use std::sync::Arc;

    use super::WasmComponentEngine;

    fn component_a() -> &'static [u8] {
        b"(component (core module))"
    }

    fn component_b() -> &'static [u8] {
        b"(component (core module) (core module))"
    }

    fn module_a() -> &'static [u8] {
        b"(module (func (export \"_start\")))"
    }

    #[test]
    fn compile_component_round_trip_and_cache_hit() {
        let engine = WasmComponentEngine::new(None).unwrap();
        let first = engine.compile_component(component_a()).unwrap();
        let second = engine.compile_component(component_a()).unwrap();

        assert_eq!(first.image_range(), second.image_range());
        assert_eq!(engine.component_cache_len(), 1);
    }

    #[test]
    fn compile_component_different_bytes_different_entries() {
        let engine = WasmComponentEngine::new(None).unwrap();
        engine.compile_component(component_a()).unwrap();
        engine.compile_component(component_b()).unwrap();

        assert_eq!(engine.component_cache_len(), 2);
    }

    #[test]
    fn compile_module_core_wasm() {
        let engine = WasmComponentEngine::new(None).unwrap();
        let module = engine.compile_module(module_a()).unwrap();
        assert!(!module.exports().collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn compile_component_concurrent_access() {
        let engine = Arc::new(WasmComponentEngine::new(None).unwrap());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let engine = Arc::clone(&engine);
                std::thread::spawn(move || {
                    let component = engine.compile_component(component_a()).unwrap();
                    assert!(!component.image_range().is_empty());
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(engine.component_cache_len(), 1);
    }
}
