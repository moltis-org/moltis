use std::collections::HashMap;
#[cfg(feature = "wasm")]
use std::mem::size_of;

use serde::{Deserialize, Serialize};

const MB: u64 = 1024 * 1024;
const DEFAULT_MEMORY_BYTES: u64 = 16 * MB;
const DEFAULT_FUEL: u64 = 1_000_000;

/// Optional per-tool overrides for fuel and memory limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolLimitOverride {
    pub fuel: Option<u64>,
    pub memory: Option<u64>,
}

/// Runtime fuel/memory limits for WASM tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WasmToolLimits {
    pub default_memory: u64,
    pub default_fuel: u64,
    pub overrides: HashMap<String, ToolLimitOverride>,
}

impl Default for WasmToolLimits {
    fn default() -> Self {
        let mut overrides = HashMap::new();
        overrides.insert("calc".to_string(), ToolLimitOverride {
            fuel: Some(100_000),
            memory: Some(2 * MB),
        });
        overrides.insert("web_fetch".to_string(), ToolLimitOverride {
            fuel: Some(10_000_000),
            memory: Some(32 * MB),
        });
        overrides.insert("web_search".to_string(), ToolLimitOverride {
            fuel: Some(10_000_000),
            memory: Some(32 * MB),
        });
        overrides.insert("show_map".to_string(), ToolLimitOverride {
            fuel: Some(10_000_000),
            memory: Some(64 * MB),
        });
        overrides.insert("location".to_string(), ToolLimitOverride {
            fuel: Some(5_000_000),
            memory: Some(16 * MB),
        });

        Self {
            default_memory: DEFAULT_MEMORY_BYTES,
            default_fuel: DEFAULT_FUEL,
            overrides,
        }
    }
}

impl WasmToolLimits {
    /// Resolve `(fuel, memory_bytes)` for a tool name.
    #[must_use]
    pub fn resolve(&self, tool_name: &str) -> (u64, u64) {
        if let Some(tool_override) = self.overrides.get(tool_name) {
            let fuel = tool_override.fuel.unwrap_or(self.default_fuel);
            let memory = tool_override.memory.unwrap_or(self.default_memory);
            return (fuel, memory);
        }

        (self.default_fuel, self.default_memory)
    }

    /// Resolve `(fuel, memory_bytes_as_usize)` for store-level limits.
    #[must_use]
    pub fn resolve_store_limits(&self, tool_name: &str) -> (u64, usize) {
        let (fuel, memory_bytes) = self.resolve(tool_name);
        (fuel, usize::try_from(memory_bytes).unwrap_or(usize::MAX))
    }
}

impl From<&moltis_config::schema::ToolLimitOverrideConfig> for ToolLimitOverride {
    fn from(value: &moltis_config::schema::ToolLimitOverrideConfig) -> Self {
        Self {
            fuel: value.fuel,
            memory: value.memory,
        }
    }
}

impl From<&moltis_config::schema::WasmToolLimitsConfig> for WasmToolLimits {
    fn from(value: &moltis_config::schema::WasmToolLimitsConfig) -> Self {
        let overrides = value
            .tool_overrides
            .iter()
            .map(|(name, limits)| (name.clone(), ToolLimitOverride::from(limits)))
            .collect();
        Self {
            default_memory: value.default_memory,
            default_fuel: value.default_fuel,
            overrides,
        }
    }
}

/// Store resource limiter for WASM execution.
///
/// This limiter guards linear memory and table growth per store/invocation.
#[cfg(feature = "wasm")]
#[derive(Debug, Clone)]
pub struct WasmResourceLimiter {
    max_memory_bytes: usize,
    max_table_elements: usize,
}

#[cfg(feature = "wasm")]
impl WasmResourceLimiter {
    #[must_use]
    pub fn new(max_memory_bytes: usize) -> Self {
        let pointer_width = size_of::<usize>().max(1);
        let max_table_elements = max_memory_bytes / pointer_width;
        Self {
            max_memory_bytes,
            max_table_elements,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_table_limit(max_memory_bytes: usize, max_table_elements: usize) -> Self {
        Self {
            max_memory_bytes,
            max_table_elements,
        }
    }
}

#[cfg(feature = "wasm")]
impl wasmtime::ResourceLimiter for WasmResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let within_limit = desired <= self.max_memory_bytes;
        let within_wasm_maximum = maximum.is_none_or(|max| desired <= max);
        Ok(within_limit && within_wasm_maximum)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let within_limit = desired <= self.max_table_elements;
        let within_wasm_maximum = maximum.is_none_or(|max| desired <= max);
        Ok(within_limit && within_wasm_maximum)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::WasmToolLimits;

    #[test]
    fn resolve_prefers_tool_override() {
        let limits = WasmToolLimits::default();
        assert_eq!(limits.resolve("calc"), (100_000, 2_097_152));
        assert_eq!(limits.resolve("unknown"), (1_000_000, 16_777_216));
    }

    #[test]
    fn config_deser_and_conversion() {
        let config: moltis_config::schema::WasmToolLimitsConfig =
            serde_json::from_value(serde_json::json!({
                "default_memory": 2048,
                "default_fuel": 999,
                "tool_overrides": {
                    "foo": { "fuel": 11, "memory": 22 }
                }
            }))
            .unwrap();

        let runtime = WasmToolLimits::from(&config);
        assert_eq!(runtime.resolve("foo"), (11, 22));
        assert_eq!(runtime.resolve("bar"), (999, 2048));
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod wasm_tests {
    use super::WasmResourceLimiter;

    #[test]
    fn memory_growth_beyond_limit_rejected() {
        let mut limiter = WasmResourceLimiter::new(1024);
        let allowed =
            wasmtime::ResourceLimiter::memory_growing(&mut limiter, 0, 2048, None).unwrap();
        assert!(!allowed);
    }

    #[test]
    fn table_growth_beyond_limit_rejected() {
        let mut limiter = WasmResourceLimiter::with_table_limit(1024, 4);
        let allowed = wasmtime::ResourceLimiter::table_growing(&mut limiter, 0, 10, None).unwrap();
        assert!(!allowed);
    }

    #[test]
    fn fuel_exhaustion_returns_error() {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let module =
            wasmtime::Module::new(&engine, "(module (func (export \"_start\") (loop br 0)))")
                .unwrap();

        let mut store = wasmtime::Store::new(&engine, ());
        store.set_fuel(10).unwrap();

        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .unwrap();
        assert!(start.call(&mut store, ()).is_err());
    }
}
