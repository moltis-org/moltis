#[cfg(feature = "wasm")]
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(feature = "wasm")]
use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::Value,
    sha2::{Digest, Sha256},
};

#[cfg(feature = "wasm")]
use crate::{
    wasm_component::{PureToolError, PureToolResult, marshal_tool_result, pure_tool},
    wasm_engine::WasmComponentEngine,
    wasm_limits::WasmResourceLimiter,
};

#[cfg(feature = "wasm")]
#[derive(Debug)]
struct WasmRunnerStoreState {
    limiter: WasmResourceLimiter,
}

#[cfg(feature = "wasm")]
struct EpochTicker {
    should_stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "wasm")]
impl EpochTicker {
    fn start(engine: wasmtime::Engine, timeout: Duration, interval_ms: u64) -> Self {
        let should_stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&should_stop);
        let handle = std::thread::spawn(move || {
            let interval = Duration::from_millis(interval_ms);
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline && !stop_flag.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                engine.increment_epoch();
            }
        });
        Self {
            should_stop,
            handle: Some(handle),
        }
    }
}

#[cfg(feature = "wasm")]
impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Host adapter that executes a `pure-tool` component as an `AgentTool`.
#[cfg(feature = "wasm")]
pub struct WasmToolRunner {
    engine: Arc<WasmComponentEngine>,
    component: wasmtime::component::Component,
    component_hash: [u8; 32],
    name: String,
    description: String,
    parameters_schema: Value,
    fuel_limit: u64,
    memory_limit_bytes: usize,
    timeout: Duration,
    epoch_interval_ms: u64,
}

#[cfg(feature = "wasm")]
impl WasmToolRunner {
    pub fn new(
        engine: Arc<WasmComponentEngine>,
        wasm_bytes: &[u8],
        fuel_limit: u64,
        memory_limit_bytes: usize,
        timeout: Duration,
        epoch_interval_ms: u64,
    ) -> Result<Self> {
        let component = engine
            .compile_component(wasm_bytes)
            .context("failed to compile wasm component")?;
        let component_hash = hash_component_bytes(wasm_bytes);
        let metadata = Self::load_metadata(&engine, &component, memory_limit_bytes)?;
        Ok(Self {
            engine,
            component,
            component_hash,
            name: metadata.name,
            description: metadata.description,
            parameters_schema: metadata.parameters_schema,
            fuel_limit,
            memory_limit_bytes,
            timeout,
            epoch_interval_ms,
        })
    }

    #[must_use]
    pub fn component_hash(&self) -> [u8; 32] {
        self.component_hash
    }

    fn load_metadata(
        engine: &WasmComponentEngine,
        component: &wasmtime::component::Component,
        memory_limit_bytes: usize,
    ) -> Result<WasmToolMetadata> {
        let mut store = new_store(engine.engine(), memory_limit_bytes);
        let linker = wasmtime::component::Linker::new(engine.engine());
        let tool = pure_tool::PureTool::instantiate(&mut store, component, &linker)
            .context("failed to instantiate pure-tool component for metadata")?;

        let name = tool.call_name(&mut store)?;
        let description = tool.call_description(&mut store)?;
        let parameters_schema_raw = tool.call_parameters_schema(&mut store)?;
        let parameters_schema =
            serde_json::from_str(&parameters_schema_raw).with_context(|| {
                format!("component `{name}` returned invalid parameters-schema JSON")
            })?;

        Ok(WasmToolMetadata {
            name,
            description,
            parameters_schema,
        })
    }

    fn execute_blocking(&self, params: Value) -> Result<Value> {
        let params_json = serde_json::to_string(&params)?;
        let engine = self.engine.engine().clone();
        let mut store = new_store(self.engine.engine(), self.memory_limit_bytes);
        store.set_fuel(self.fuel_limit)?;
        store.set_epoch_deadline(1);

        let _ticker = EpochTicker::start(engine, self.timeout, self.epoch_interval_ms);
        let linker = wasmtime::component::Linker::new(self.engine.engine());
        let tool = pure_tool::PureTool::instantiate(&mut store, &self.component, &linker)
            .context("failed to instantiate pure-tool component")?;
        let result = tool.call_execute(&mut store, &params_json)?;
        decode_result(&self.name, result)
    }
}

#[cfg(feature = "wasm")]
#[async_trait]
impl AgentTool for WasmToolRunner {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters_schema.clone()
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        tokio::task::spawn_blocking({
            let this = self.clone_for_blocking();
            move || this.execute_blocking(params)
        })
        .await
        .context("wasm tool runner task join error")?
    }
}

#[cfg(feature = "wasm")]
impl WasmToolRunner {
    fn clone_for_blocking(&self) -> Self {
        Self {
            engine: Arc::clone(&self.engine),
            component: self.component.clone(),
            component_hash: self.component_hash,
            name: self.name.clone(),
            description: self.description.clone(),
            parameters_schema: self.parameters_schema.clone(),
            fuel_limit: self.fuel_limit,
            memory_limit_bytes: self.memory_limit_bytes,
            timeout: self.timeout,
            epoch_interval_ms: self.epoch_interval_ms,
        }
    }
}

#[cfg(feature = "wasm")]
struct WasmToolMetadata {
    name: String,
    description: String,
    parameters_schema: Value,
}

#[cfg(feature = "wasm")]
fn new_store(
    engine: &wasmtime::Engine,
    memory_limit_bytes: usize,
) -> wasmtime::Store<WasmRunnerStoreState> {
    let limiter = WasmResourceLimiter::new(memory_limit_bytes);
    let mut store = wasmtime::Store::new(engine, WasmRunnerStoreState { limiter });
    store.limiter(|state| &mut state.limiter);
    store
}

#[cfg(feature = "wasm")]
fn hash_component_bytes(wasm_bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(wasm_bytes);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

#[cfg(feature = "wasm")]
fn decode_result(tool_name: &str, result: PureToolResult) -> Result<Value> {
    match result {
        PureToolResult::Ok(value) => Ok(marshal_tool_result(value)),
        PureToolResult::Err(error) => tool_error(tool_name, error),
    }
}

#[cfg(feature = "wasm")]
fn tool_error(tool_name: &str, error: PureToolError) -> Result<Value> {
    bail!(
        "wasm tool `{tool_name}` failed [{}]: {}",
        error.code,
        error.message
    )
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod tests {
    use {
        super::decode_result,
        crate::wasm_component::{PureToolError, PureToolResult, PureToolValue},
    };

    #[test]
    fn decode_result_maps_ok_value() {
        let value = decode_result("calc", PureToolResult::Ok(PureToolValue::Integer(7))).unwrap();
        assert_eq!(value, serde_json::json!(7));
    }

    #[test]
    fn decode_result_maps_err_variant() {
        let err = decode_result(
            "calc",
            PureToolResult::Err(PureToolError {
                code: "bad_input".to_string(),
                message: "not parseable".to_string(),
            }),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("wasm tool `calc` failed [bad_input]")
        );
    }
}
