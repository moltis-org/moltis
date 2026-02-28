#[cfg(feature = "wasm")]
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
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
};

#[cfg(feature = "wasm")]
use crate::{
    wasm_component::{HttpHostImpl, HttpToolResult, PureToolResult, http_tool, pure_tool},
    wasm_engine::{WasmComponentEngine, hash_component_bytes},
    wasm_limits::WasmResourceLimiter,
};

// ---------------------------------------------------------------------------
// Unified store state
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm")]
struct WasmStoreState {
    limiter: WasmResourceLimiter,
    table: wasmtime::component::ResourceTable,
    wasi: wasmtime_wasi::WasiCtx,
    http_host: Option<HttpHostImpl>,
}

#[cfg(feature = "wasm")]
impl WasmStoreState {
    fn new(memory_limit_bytes: usize, http_host: Option<HttpHostImpl>) -> Self {
        Self {
            limiter: WasmResourceLimiter::new(memory_limit_bytes),
            table: wasmtime::component::ResourceTable::new(),
            wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
            http_host,
        }
    }
}

#[cfg(feature = "wasm")]
impl wasmtime_wasi::IoView for WasmStoreState {
    fn table(&mut self) -> &mut wasmtime::component::ResourceTable {
        &mut self.table
    }
}

#[cfg(feature = "wasm")]
impl wasmtime_wasi::WasiView for WasmStoreState {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        &mut self.wasi
    }
}

// ---------------------------------------------------------------------------
// Epoch ticker (unchanged)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Unified WasmToolRunner
// ---------------------------------------------------------------------------

/// Host adapter that executes a WASM component as an `AgentTool`.
///
/// When `http_host` is `None` the component is instantiated as a `pure-tool`;
/// when `Some` it is instantiated as an `http-tool` with host-side HTTP.
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
    http_host: Option<HttpHostImpl>,
}

#[cfg(feature = "wasm")]
struct CachedResult {
    value: Value,
    expires_at: Instant,
}

#[cfg(feature = "wasm")]
pub struct CachingWasmToolRunner {
    inner: Arc<dyn AgentTool>,
    component_hash: [u8; 32],
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CachedResult>>,
}

#[cfg(feature = "wasm")]
impl WasmToolRunner {
    /// Create a pure-tool runner (no HTTP capability).
    pub fn new(
        engine: Arc<WasmComponentEngine>,
        wasm_bytes: &[u8],
        fuel_limit: u64,
        memory_limit_bytes: usize,
        timeout: Duration,
        epoch_interval_ms: u64,
    ) -> Result<Self> {
        Self::new_inner(
            engine,
            wasm_bytes,
            fuel_limit,
            memory_limit_bytes,
            timeout,
            epoch_interval_ms,
            None,
        )
    }

    /// Create an HTTP-capable tool runner.
    pub fn new_http(
        engine: Arc<WasmComponentEngine>,
        wasm_bytes: &[u8],
        fuel_limit: u64,
        memory_limit_bytes: usize,
        timeout: Duration,
        epoch_interval_ms: u64,
        http_host: HttpHostImpl,
    ) -> Result<Self> {
        Self::new_inner(
            engine,
            wasm_bytes,
            fuel_limit,
            memory_limit_bytes,
            timeout,
            epoch_interval_ms,
            Some(http_host),
        )
    }

    fn new_inner(
        engine: Arc<WasmComponentEngine>,
        wasm_bytes: &[u8],
        fuel_limit: u64,
        memory_limit_bytes: usize,
        timeout: Duration,
        epoch_interval_ms: u64,
        http_host: Option<HttpHostImpl>,
    ) -> Result<Self> {
        let component = engine
            .compile_component(wasm_bytes)
            .context("failed to compile wasm component")?;
        let component_hash = hash_component_bytes(wasm_bytes);
        let metadata =
            Self::load_metadata(&engine, &component, memory_limit_bytes, http_host.as_ref())?;
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
            http_host,
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
        http_host: Option<&HttpHostImpl>,
    ) -> Result<WasmToolMetadata> {
        let mut store = new_store(engine.engine(), memory_limit_bytes, http_host.cloned());
        store
            .set_fuel(METADATA_FUEL_BUDGET)
            .context("failed to set metadata fuel budget")?;
        store.set_epoch_deadline(METADATA_EPOCH_DEADLINE_TICKS);

        let instance = WasmToolInstance::instantiate(engine, &mut store, component, "metadata")?;
        let name = instance.call_name(&mut store)?;
        let description = instance.call_description(&mut store)?;
        let parameters_schema_raw = instance.call_parameters_schema(&mut store)?;
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
        let mut store = new_store(
            self.engine.engine(),
            self.memory_limit_bytes,
            self.http_host.clone(),
        );
        store.set_fuel(self.fuel_limit)?;
        store.set_epoch_deadline(1);

        let _ticker = EpochTicker::start(engine, self.timeout, self.epoch_interval_ms);
        let instance =
            WasmToolInstance::instantiate(&self.engine, &mut store, &self.component, &self.name)?;
        let result = instance.call_execute(&mut store, &params_json)?;
        decode_tool_result(&self.name, result)
    }

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
            http_host: self.http_host.clone(),
        }
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

// ---------------------------------------------------------------------------
// CachingWasmToolRunner
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm")]
impl CachingWasmToolRunner {
    pub fn new(inner: Arc<dyn AgentTool>, component_hash: [u8; 32], cache_ttl: Duration) -> Self {
        Self {
            inner,
            component_hash,
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn component_hash(&self) -> [u8; 32] {
        self.component_hash
    }

    fn cache_get(&self, key: &str) -> Option<Value> {
        let cache = self.cache.lock().ok()?;
        let entry = cache.get(key)?;
        if Instant::now() < entry.expires_at {
            Some(entry.value.clone())
        } else {
            None
        }
    }

    fn cache_set(&self, key: String, value: Value) {
        if self.cache_ttl.is_zero() {
            return;
        }
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() > MAX_CACHE_ENTRIES_BEFORE_EVICTION {
                let now = Instant::now();
                cache.retain(|_, entry| entry.expires_at > now);
            }
            cache.insert(key, CachedResult {
                value,
                expires_at: Instant::now() + self.cache_ttl,
            });
        }
    }
}

#[cfg(feature = "wasm")]
#[async_trait]
impl AgentTool for CachingWasmToolRunner {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let cache_key = serde_json::to_string(&params)?;
        if let Some(cached) = self.cache_get(&cache_key) {
            return Ok(cached);
        }
        let result = self.inner.execute(params).await?;
        self.cache_set(cache_key, result.clone());
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Registration helper — called from server.rs
// ---------------------------------------------------------------------------

/// Register all embedded WASM tools into `registry`.
///
/// Individual tool failures are logged but do not abort the entire registration.
#[cfg(feature = "wasm")]
pub fn register_wasm_tools(
    registry: &mut moltis_agents::tool_registry::ToolRegistry,
    wasm_limits: &crate::wasm_limits::WasmToolLimits,
    epoch_interval_ms: u64,
    fetch_timeout_secs: u64,
    fetch_cache_ttl_minutes: u64,
    search_timeout_secs: u64,
    search_cache_ttl_minutes: u64,
    search_api_key: Option<&str>,
) -> Result<()> {
    use std::collections::HashMap;

    use crate::wasm_component::SecretHeaders;

    let wasm_engine =
        Arc::new(WasmComponentEngine::new(None).context("failed to create wasm component engine")?);

    // --- calc (pure tool) ---
    match crate::embedded_wasm::calc_component_bytes() {
        Ok(calc_bytes) => {
            let (fuel, memory) = wasm_limits.resolve_store_limits("calc");
            match WasmToolRunner::new(
                Arc::clone(&wasm_engine),
                calc_bytes.as_ref(),
                fuel,
                memory,
                Duration::from_secs(2),
                epoch_interval_ms,
            ) {
                Ok(runner) => {
                    let hash = runner.component_hash();
                    registry.register_wasm(Box::new(runner), hash);
                },
                Err(e) => tracing::warn!(%e, "failed to initialize calc_wasm tool"),
            }
        },
        Err(e) => tracing::warn!(
            %e,
            "calc_wasm component unavailable; run `just wasm-tools` to build guest components"
        ),
    }

    // --- web_fetch (http tool, no secret headers) ---
    match crate::embedded_wasm::web_fetch_component_bytes() {
        Ok(fetch_bytes) => {
            let (fuel, memory) = wasm_limits.resolve_store_limits("web_fetch");
            match HttpHostImpl::new(
                Duration::from_secs(fetch_timeout_secs),
                2_000_000,
                Vec::new(),
                None,
                HashMap::new(),
            ) {
                Ok(http_host) => {
                    match WasmToolRunner::new_http(
                        Arc::clone(&wasm_engine),
                        fetch_bytes.as_ref(),
                        fuel,
                        memory,
                        Duration::from_secs(5),
                        epoch_interval_ms,
                        http_host,
                    ) {
                        Ok(runner) => {
                            let hash = runner.component_hash();
                            let cache_ttl =
                                Duration::from_secs(fetch_cache_ttl_minutes.saturating_mul(60));
                            let cached =
                                CachingWasmToolRunner::new(Arc::new(runner), hash, cache_ttl);
                            registry.register_wasm(Box::new(cached), hash);
                        },
                        Err(e) => tracing::warn!(%e, "failed to initialize web_fetch_wasm tool"),
                    }
                },
                Err(e) => {
                    tracing::warn!(%e, "failed to initialize HTTP host for web_fetch_wasm");
                },
            }
        },
        Err(e) => tracing::warn!(
            %e,
            "web_fetch_wasm component unavailable; run `just wasm-tools` to build guest components"
        ),
    }

    // --- web_search (http tool, secret headers for Brave API key) ---
    match crate::embedded_wasm::web_search_component_bytes() {
        Ok(search_bytes) => {
            let (fuel, memory) = wasm_limits.resolve_store_limits("web_search");
            let mut secret_headers: SecretHeaders = HashMap::new();
            if let Some(key) = search_api_key.filter(|k| !k.trim().is_empty()) {
                secret_headers.insert("api.search.brave.com".to_string(), vec![(
                    "X-Subscription-Token".to_string(),
                    key.to_string(),
                )]);
            }
            let domain_allowlist = Some(vec!["api.search.brave.com".to_string()]);
            match HttpHostImpl::new(
                Duration::from_secs(search_timeout_secs),
                2_000_000,
                Vec::new(),
                domain_allowlist,
                secret_headers,
            ) {
                Ok(http_host) => {
                    match WasmToolRunner::new_http(
                        Arc::clone(&wasm_engine),
                        search_bytes.as_ref(),
                        fuel,
                        memory,
                        Duration::from_secs(5),
                        epoch_interval_ms,
                        http_host,
                    ) {
                        Ok(runner) => {
                            let hash = runner.component_hash();
                            let cache_ttl =
                                Duration::from_secs(search_cache_ttl_minutes.saturating_mul(60));
                            let cached =
                                CachingWasmToolRunner::new(Arc::new(runner), hash, cache_ttl);
                            registry.register_wasm(Box::new(cached), hash);
                        },
                        Err(e) => tracing::warn!(%e, "failed to initialize web_search_wasm tool"),
                    }
                },
                Err(e) => {
                    tracing::warn!(%e, "failed to initialize HTTP host for web_search_wasm");
                },
            }
        },
        Err(e) => tracing::warn!(
            %e,
            "web_search_wasm component unavailable; run `just wasm-tools` to build guest components"
        ),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internals: instance dispatch, store/linker helpers, result marshaling
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm")]
struct WasmToolMetadata {
    name: String,
    description: String,
    parameters_schema: Value,
}

#[cfg(feature = "wasm")]
const METADATA_FUEL_BUDGET: u64 = 50_000_000;
#[cfg(feature = "wasm")]
const METADATA_EPOCH_DEADLINE_TICKS: u64 = 1_000_000;
#[cfg(feature = "wasm")]
const MAX_CACHE_ENTRIES_BEFORE_EVICTION: usize = 200;

/// Dispatches between pure-tool and http-tool component instances.
#[cfg(feature = "wasm")]
enum WasmToolInstance {
    Pure(pure_tool::PureTool),
    Http(http_tool::HttpTool),
}

#[cfg(feature = "wasm")]
impl WasmToolInstance {
    fn instantiate(
        engine: &WasmComponentEngine,
        store: &mut wasmtime::Store<WasmStoreState>,
        component: &wasmtime::component::Component,
        label: &str,
    ) -> Result<Self> {
        if store.data().http_host.is_some() {
            let linker = engine
                .create_http_linker(|state: &mut WasmStoreState| {
                    // SAFETY: instantiate() only takes this branch when http_host.is_some(),
                    // and the linker getter is only invoked for stores that carry the host.
                    state.http_host.as_mut().unwrap_or_else(|| unreachable!())
                })
                .context("failed to create http-tool linker")?;
            let tool = http_tool::HttpTool::instantiate(store, component, &linker)
                .with_context(|| format!("failed to instantiate http-tool component `{label}`"))?;
            Ok(Self::Http(tool))
        } else {
            let mut linker = wasmtime::component::Linker::new(engine.engine());
            wasmtime_wasi::add_to_linker_sync(&mut linker)
                .context("failed to link wasi preview2")?;
            let tool = pure_tool::PureTool::instantiate(store, component, &linker)
                .with_context(|| format!("failed to instantiate pure-tool component `{label}`"))?;
            Ok(Self::Pure(tool))
        }
    }

    fn call_name(&self, store: &mut wasmtime::Store<WasmStoreState>) -> Result<String> {
        match self {
            Self::Pure(t) => t.call_name(store).context("call_name failed"),
            Self::Http(t) => t.call_name(store).context("call_name failed"),
        }
    }

    fn call_description(&self, store: &mut wasmtime::Store<WasmStoreState>) -> Result<String> {
        match self {
            Self::Pure(t) => t.call_description(store).context("call_description failed"),
            Self::Http(t) => t.call_description(store).context("call_description failed"),
        }
    }

    fn call_parameters_schema(
        &self,
        store: &mut wasmtime::Store<WasmStoreState>,
    ) -> Result<String> {
        match self {
            Self::Pure(t) => t
                .call_parameters_schema(store)
                .context("call_parameters_schema failed"),
            Self::Http(t) => t
                .call_parameters_schema(store)
                .context("call_parameters_schema failed"),
        }
    }

    fn call_execute(
        &self,
        store: &mut wasmtime::Store<WasmStoreState>,
        params_json: &str,
    ) -> Result<ToolResult> {
        match self {
            Self::Pure(t) => t
                .call_execute(store, params_json)
                .map(ToolResult::from_pure)
                .context("pure-tool execute failed"),
            Self::Http(t) => t
                .call_execute(store, params_json)
                .map(ToolResult::from_http)
                .context("http-tool execute failed"),
        }
    }
}

/// Unified result type for both pure-tool and http-tool components.
#[cfg(feature = "wasm")]
enum ToolResult {
    Ok(Value),
    Err { code: String, message: String },
}

#[cfg(feature = "wasm")]
impl ToolResult {
    fn from_pure(result: PureToolResult) -> Self {
        match result {
            PureToolResult::Ok(value) => Self::Ok(marshal_pure_value(value)),
            PureToolResult::Err(error) => Self::Err {
                code: error.code,
                message: error.message,
            },
        }
    }

    fn from_http(result: HttpToolResult) -> Self {
        match result {
            HttpToolResult::Ok(value) => Self::Ok(marshal_http_value(value)),
            HttpToolResult::Err(error) => Self::Err {
                code: error.code,
                message: error.message,
            },
        }
    }
}

/// Marshal a `PureToolValue` into `serde_json::Value`.
#[cfg(feature = "wasm")]
fn marshal_pure_value(value: crate::wasm_component::PureToolValue) -> Value {
    use crate::wasm_component::PureToolValue;
    match value {
        PureToolValue::Text(text) => Value::String(text),
        PureToolValue::Number(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        PureToolValue::Integer(integer) => Value::Number(integer.into()),
        PureToolValue::Boolean(boolean) => Value::Bool(boolean),
        PureToolValue::Json(json) => {
            serde_json::from_str::<Value>(&json).unwrap_or(Value::String(json))
        },
    }
}

/// Marshal an `HttpToolValue` into `serde_json::Value`.
#[cfg(feature = "wasm")]
fn marshal_http_value(value: crate::wasm_component::HttpToolValue) -> Value {
    use crate::wasm_component::HttpToolValue;
    match value {
        HttpToolValue::Text(text) => Value::String(text),
        HttpToolValue::Number(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        HttpToolValue::Integer(integer) => Value::Number(integer.into()),
        HttpToolValue::Boolean(boolean) => Value::Bool(boolean),
        HttpToolValue::Json(json) => {
            serde_json::from_str::<Value>(&json).unwrap_or(Value::String(json))
        },
    }
}

#[cfg(feature = "wasm")]
fn decode_tool_result(tool_name: &str, result: ToolResult) -> Result<Value> {
    match result {
        ToolResult::Ok(value) => Ok(value),
        ToolResult::Err { code, message } => {
            bail!("wasm tool `{tool_name}` failed [{code}]: {message}")
        },
    }
}

#[cfg(feature = "wasm")]
fn new_store(
    engine: &wasmtime::Engine,
    memory_limit_bytes: usize,
    http_host: Option<HttpHostImpl>,
) -> wasmtime::Store<WasmStoreState> {
    let mut store =
        wasmtime::Store::new(engine, WasmStoreState::new(memory_limit_bytes, http_host));
    store.limiter(|state| &mut state.limiter);
    store
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "wasm"))]
mod tests {
    use {
        super::{
            CachingWasmToolRunner, ToolResult, WasmToolRunner, marshal_http_value,
            marshal_pure_value,
        },
        crate::{
            calc::CalcTool,
            wasm_component::{
                HttpToolError, HttpToolResult, HttpToolValue, PureToolError, PureToolResult,
                PureToolValue,
            },
            wasm_engine::WasmComponentEngine,
            wasm_limits::WasmToolLimits,
        },
        moltis_agents::tool_registry::{AgentTool, ToolRegistry},
        std::{
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            time::Duration,
        },
    };

    fn maybe_calc_runner(fuel_limit: u64) -> Option<WasmToolRunner> {
        let calc_component_bytes = match crate::embedded_wasm::calc_component_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("skipping calc wasm runner tests: {err}");
                return None;
            },
        };
        let engine = Arc::new(WasmComponentEngine::new(None).unwrap());
        let wasm_limits = WasmToolLimits::default();
        let (_, memory_limit_bytes) = wasm_limits.resolve_store_limits("calc");

        Some(
            WasmToolRunner::new(
                engine,
                calc_component_bytes.as_ref(),
                fuel_limit,
                memory_limit_bytes,
                Duration::from_secs(2),
                100,
            )
            .unwrap(),
        )
    }

    #[test]
    fn decode_result_maps_ok_value() {
        let result = ToolResult::from_pure(PureToolResult::Ok(PureToolValue::Integer(7)));
        let value = super::decode_tool_result("calc", result).unwrap();
        assert_eq!(value, serde_json::json!(7));
    }

    #[test]
    fn decode_result_maps_err_variant() {
        let result = ToolResult::from_pure(PureToolResult::Err(PureToolError {
            code: "bad_input".to_string(),
            message: "not parseable".to_string(),
        }));
        let err = super::decode_tool_result("calc", result).unwrap_err();
        assert!(
            err.to_string()
                .contains("wasm tool `calc` failed [bad_input]")
        );
    }

    #[tokio::test]
    async fn calc_wasm_matches_native_for_expressions() {
        let Some(calc_wasm_tool) = maybe_calc_runner(100_000) else {
            return;
        };

        let native = CalcTool::new();
        let expressions = [
            "1+1",
            "2+3*4",
            "(10+2)/3",
            "2^8",
            "2^3^2",
            "15%4",
            "-5+3",
            "-(2+3)*4",
            "1/2+3%2",
            "(3+4)*(5-2)",
            "0.1+0.2",
            "5--2",
            "+7",
            "2*(3+(4*5))",
            "9/3/3",
            "(2+3)^(1+1)",
            "6%4+2*3",
            "2.5*4",
            "10-3-2",
            "((1+2)+(3+4))*2",
            "1e3+2",
        ];

        for expression in expressions {
            let params = serde_json::json!({ "expression": expression });
            let native_result = native.execute(params.clone()).await.unwrap();
            let wasm_result = calc_wasm_tool.execute(params).await.unwrap();
            assert_eq!(wasm_result, native_result, "expression `{expression}`");
        }

        assert_eq!(
            calc_wasm_tool.parameters_schema(),
            native.parameters_schema()
        );
    }

    #[tokio::test]
    async fn calc_wasm_low_fuel_fails() {
        let Some(calc_wasm_tool) = maybe_calc_runner(1) else {
            return;
        };

        let mut expression = "1+".repeat(200);
        expression.push('1');
        let result = calc_wasm_tool
            .execute(serde_json::json!({ "expression": expression }))
            .await;
        assert!(result.is_err());
    }

    struct EchoTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl AgentTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "echo"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(params)
        }
    }

    #[tokio::test]
    async fn caching_runner_returns_cached_value() {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner: Arc<dyn AgentTool> = Arc::new(EchoTool {
            calls: Arc::clone(&calls),
        });
        let cached = CachingWasmToolRunner::new(inner, [7_u8; 32], Duration::from_secs(60));
        let params = serde_json::json!({"query":"rust"});

        let first = cached.execute(params.clone()).await.unwrap();
        let second = cached.execute(params).await.unwrap();

        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(cached.component_hash(), [7_u8; 32]);
    }

    // --- marshal_http_value tests ---

    #[test]
    fn marshal_http_value_text() {
        let value = marshal_http_value(HttpToolValue::Text("hello".to_string()));
        assert_eq!(value, serde_json::json!("hello"));
    }

    #[test]
    fn marshal_http_value_number() {
        let value = marshal_http_value(HttpToolValue::Number(12.5));
        assert_eq!(value, serde_json::json!(12.5));
    }

    #[test]
    fn marshal_http_value_integer() {
        let value = marshal_http_value(HttpToolValue::Integer(-99));
        assert_eq!(value, serde_json::json!(-99));
    }

    #[test]
    fn marshal_http_value_boolean() {
        let value = marshal_http_value(HttpToolValue::Boolean(false));
        assert_eq!(value, serde_json::json!(false));
    }

    #[test]
    fn marshal_http_value_json() {
        let value = marshal_http_value(HttpToolValue::Json(r#"{"a":1}"#.to_string()));
        assert_eq!(value, serde_json::json!({"a": 1}));
    }

    #[test]
    fn marshal_http_value_nan_becomes_null() {
        let value = marshal_http_value(HttpToolValue::Number(f64::NAN));
        assert_eq!(value, serde_json::Value::Null);
    }

    #[test]
    fn marshal_http_value_invalid_json_becomes_string() {
        let value = marshal_http_value(HttpToolValue::Json("not json".to_string()));
        assert_eq!(value, serde_json::json!("not json"));
    }

    // --- marshal_pure_value edge-case tests ---

    #[test]
    fn marshal_pure_value_nan_becomes_null() {
        let value = marshal_pure_value(PureToolValue::Number(f64::NAN));
        assert_eq!(value, serde_json::Value::Null);
    }

    #[test]
    fn marshal_pure_value_invalid_json_becomes_string() {
        let value = marshal_pure_value(PureToolValue::Json("{bad".to_string()));
        assert_eq!(value, serde_json::json!("{bad"));
    }

    // --- ToolResult::from_http tests ---

    #[test]
    fn decode_http_result_maps_ok() {
        let result = ToolResult::from_http(HttpToolResult::Ok(HttpToolValue::Text(
            "fetched".to_string(),
        )));
        let value = super::decode_tool_result("web_fetch", result).unwrap();
        assert_eq!(value, serde_json::json!("fetched"));
    }

    #[test]
    fn decode_http_result_maps_err() {
        let result = ToolResult::from_http(HttpToolResult::Err(HttpToolError {
            code: "network".to_string(),
            message: "connection refused".to_string(),
        }));
        let err = super::decode_tool_result("web_fetch", result).unwrap_err();
        assert!(
            err.to_string()
                .contains("wasm tool `web_fetch` failed [network]")
        );
    }

    // --- register_wasm_tools test ---

    #[test]
    fn register_wasm_tools_succeeds_with_available_components() {
        let mut registry = ToolRegistry::new();
        let limits = WasmToolLimits::default();
        let result = super::register_wasm_tools(&mut registry, &limits, 100, 30, 5, 15, 5, None);
        // Should succeed regardless of whether wasm binaries are present.
        // When binaries are missing, individual tools log warnings but the
        // function itself returns Ok.
        assert!(result.is_ok(), "register_wasm_tools failed: {result:?}");
    }

    // --- http runner integration test ---

    #[tokio::test]
    async fn web_fetch_http_runner_fetches_from_local_server() {
        use {
            crate::wasm_component::HttpHostImpl,
            std::{
                io::{Read, Write},
                net::TcpListener,
                thread,
            },
        };

        let fetch_bytes = match crate::embedded_wasm::web_fetch_component_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("skipping web_fetch http runner test: {err}");
                return;
            },
        };

        // Spin up a tiny HTTP server that returns a known body.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 2048];
                let _ = stream.read(&mut buf);
                let body = b"web_fetch_test_ok";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let ssrf_allowlist = vec!["127.0.0.1/32".parse().unwrap()];
        let http_host = HttpHostImpl::new(
            Duration::from_secs(5),
            2_000_000,
            ssrf_allowlist,
            None,
            std::collections::HashMap::new(),
        )
        .unwrap();

        let engine = Arc::new(WasmComponentEngine::new(None).unwrap());
        let limits = WasmToolLimits::default();
        let (fuel, memory) = limits.resolve_store_limits("web_fetch");
        let runner = WasmToolRunner::new_http(
            engine,
            fetch_bytes.as_ref(),
            fuel,
            memory,
            Duration::from_secs(5),
            100,
            http_host,
        )
        .unwrap();

        let url = format!("http://{addr}/test");
        let result = runner.execute(serde_json::json!({ "url": url })).await;

        // The runner should succeed and the result should contain our test body.
        let value = result.unwrap();
        let as_str = value.to_string();
        assert!(
            as_str.contains("web_fetch_test_ok"),
            "expected response body in result, got: {as_str}"
        );

        server.join().unwrap();
    }
}
