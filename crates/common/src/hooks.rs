//! Core hook types shared across crates.
//!
//! These types define the hook event system. The full registry and shell handler
//! live in `moltis-plugins`; this module provides the trait and types needed by
//! crates like `moltis-agents` that cannot depend on plugins.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use {
    anyhow::Result,
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tracing::{debug, info, warn},
};

// ── HookEvent ───────────────────────────────────────────────────────────────

/// Lifecycle events that hooks can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    BeforeAgentStart,
    AgentEnd,
    BeforeLLMCall,
    AfterLLMCall,
    BeforeCompaction,
    AfterCompaction,
    MessageReceived,
    MessageSending,
    MessageSent,
    BeforeToolCall,
    AfterToolCall,
    ToolResultPersist,
    SessionStart,
    SessionEnd,
    GatewayStart,
    GatewayStop,
    Command,
}

impl fmt::Display for HookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl HookEvent {
    /// All variants, for iteration.
    pub const ALL: &'static [HookEvent] = &[
        Self::BeforeAgentStart,
        Self::AgentEnd,
        Self::BeforeLLMCall,
        Self::AfterLLMCall,
        Self::BeforeCompaction,
        Self::AfterCompaction,
        Self::MessageReceived,
        Self::MessageSending,
        Self::MessageSent,
        Self::BeforeToolCall,
        Self::AfterToolCall,
        Self::ToolResultPersist,
        Self::SessionStart,
        Self::SessionEnd,
        Self::GatewayStart,
        Self::GatewayStop,
        Self::Command,
    ];

    /// Returns true if this event is read-only and handlers can run in parallel.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::AgentEnd
                | Self::AfterToolCall
                | Self::MessageReceived
                | Self::MessageSent
                | Self::AfterCompaction
                | Self::SessionStart
                | Self::SessionEnd
                | Self::GatewayStart
                | Self::GatewayStop
                | Self::Command
        )
    }
}

// ── HookPayload ─────────────────────────────────────────────────────────────

/// Typed payload carried with each hook event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum HookPayload {
    BeforeAgentStart {
        session_key: String,
        model: String,
    },
    AgentEnd {
        session_key: String,
        text: String,
        iterations: usize,
        tool_calls: usize,
    },
    BeforeLLMCall {
        session_key: String,
        provider: String,
        model: String,
        messages: Value,
        tool_count: usize,
        iteration: usize,
    },
    AfterLLMCall {
        session_key: String,
        provider: String,
        model: String,
        text: Option<String>,
        tool_calls: Vec<Value>,
        input_tokens: u32,
        output_tokens: u32,
        iteration: usize,
    },
    BeforeCompaction {
        session_key: String,
        message_count: usize,
    },
    AfterCompaction {
        session_key: String,
        summary_len: usize,
    },
    MessageReceived {
        session_key: String,
        content: String,
        channel: Option<String>,
    },
    MessageSending {
        session_key: String,
        content: String,
    },
    MessageSent {
        session_key: String,
        content: String,
    },
    BeforeToolCall {
        session_key: String,
        tool_name: String,
        arguments: Value,
    },
    AfterToolCall {
        session_key: String,
        tool_name: String,
        success: bool,
        result: Option<Value>,
    },
    ToolResultPersist {
        session_key: String,
        tool_name: String,
        result: Value,
    },
    SessionStart {
        session_key: String,
    },
    SessionEnd {
        session_key: String,
    },
    GatewayStart {
        address: String,
    },
    GatewayStop,
    Command {
        session_key: String,
        action: String,
        sender_id: Option<String>,
    },
}

impl HookPayload {
    /// Returns the [`HookEvent`] variant that matches this payload.
    pub fn event(&self) -> HookEvent {
        match self {
            Self::BeforeAgentStart { .. } => HookEvent::BeforeAgentStart,
            Self::AgentEnd { .. } => HookEvent::AgentEnd,
            Self::BeforeLLMCall { .. } => HookEvent::BeforeLLMCall,
            Self::AfterLLMCall { .. } => HookEvent::AfterLLMCall,
            Self::BeforeCompaction { .. } => HookEvent::BeforeCompaction,
            Self::AfterCompaction { .. } => HookEvent::AfterCompaction,
            Self::MessageReceived { .. } => HookEvent::MessageReceived,
            Self::MessageSending { .. } => HookEvent::MessageSending,
            Self::MessageSent { .. } => HookEvent::MessageSent,
            Self::BeforeToolCall { .. } => HookEvent::BeforeToolCall,
            Self::AfterToolCall { .. } => HookEvent::AfterToolCall,
            Self::ToolResultPersist { .. } => HookEvent::ToolResultPersist,
            Self::SessionStart { .. } => HookEvent::SessionStart,
            Self::SessionEnd { .. } => HookEvent::SessionEnd,
            Self::GatewayStart { .. } => HookEvent::GatewayStart,
            Self::GatewayStop => HookEvent::GatewayStop,
            Self::Command { .. } => HookEvent::Command,
        }
    }
}

// ── HookAction ──────────────────────────────────────────────────────────────

/// The outcome a hook handler returns.
#[derive(Debug, Default)]
pub enum HookAction {
    /// Let the event proceed normally.
    #[default]
    Continue,
    /// Replace part of the payload data (e.g. modify tool arguments or results).
    ModifyPayload(Value),
    /// Block the action entirely, with a reason string.
    Block(String),
}

// ── HookHandler trait ───────────────────────────────────────────────────────

/// Trait implemented by both native and shell hook handlers.
#[async_trait]
pub trait HookHandler: Send + Sync {
    /// A human-readable name for this handler.
    fn name(&self) -> &str;

    /// Which events this handler subscribes to.
    fn events(&self) -> &[HookEvent];

    /// Priority for ordering. Higher values run first. Default is 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Handle the event, returning an action that may modify or block the flow.
    async fn handle(&self, event: HookEvent, payload: &HookPayload) -> Result<HookAction>;

    /// Synchronous handle for hot-path use (e.g. `ToolResultPersist`).
    /// Default implementation blocks on the async `handle`.
    fn handle_sync(&self, event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        // Default: spawn a blocking task. Native hooks can override for zero-overhead.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // Inside a tokio runtime — use block_in_place to avoid nested runtime panic.
                tokio::task::block_in_place(|| handle.block_on(self.handle(event, payload)))
            },
            Err(_) => {
                // No runtime available; create a temporary one.
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(self.handle(event, payload))
            },
        }
    }
}

// ── HookStats ───────────────────────────────────────────────────────────────

/// Per-handler health statistics for circuit breaker logic.
pub struct HookStats {
    pub call_count: AtomicU64,
    pub failure_count: AtomicU64,
    pub consecutive_failures: AtomicU64,
    pub total_latency_us: AtomicU64,
    pub disabled: AtomicBool,
    pub disabled_at: std::sync::Mutex<Option<Instant>>,
}

impl HookStats {
    pub fn new() -> Self {
        Self {
            call_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            consecutive_failures: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            disabled: AtomicBool::new(false),
            disabled_at: std::sync::Mutex::new(None),
        }
    }

    pub fn record_success(&self, latency: Duration) {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.total_latency_us
            .fetch_add(latency.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn record_failure(&self, latency: Duration) {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        self.total_latency_us
            .fetch_add(latency.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn avg_latency(&self) -> Duration {
        let calls = self.call_count.load(Ordering::Relaxed);
        if calls == 0 {
            return Duration::ZERO;
        }
        let total = self.total_latency_us.load(Ordering::Relaxed);
        Duration::from_micros(total / calls)
    }
}

impl Default for HookStats {
    fn default() -> Self {
        Self::new()
    }
}

// ── Handler entry (with stats) ──────────────────────────────────────────────

struct HandlerEntry {
    handler: Arc<dyn HookHandler>,
    stats: Arc<HookStats>,
}

// ── HookRegistry ────────────────────────────────────────────────────────────

/// Manages registered hook handlers and dispatches events to them.
pub struct HookRegistry {
    handlers: HashMap<HookEvent, Vec<HandlerEntry>>,
    /// Maximum consecutive failures before auto-disabling a handler.
    circuit_breaker_threshold: u64,
    /// Cooldown period before re-enabling a circuit-broken handler.
    circuit_breaker_cooldown: Duration,
    /// When true, Block/Modify results are logged but not applied.
    pub dry_run: bool,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            circuit_breaker_threshold: 3,
            circuit_breaker_cooldown: Duration::from_secs(60),
            dry_run: false,
        }
    }

    /// Set circuit breaker parameters.
    pub fn with_circuit_breaker(mut self, threshold: u64, cooldown: Duration) -> Self {
        self.circuit_breaker_threshold = threshold;
        self.circuit_breaker_cooldown = cooldown;
        self
    }

    /// Enable dry-run mode.
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Register a handler for all events it subscribes to.
    /// Handlers are sorted by priority (descending) within each event.
    pub fn register(&mut self, handler: Arc<dyn HookHandler>) {
        let stats = Arc::new(HookStats::new());
        for &event in handler.events() {
            let entry = HandlerEntry {
                handler: Arc::clone(&handler),
                stats: Arc::clone(&stats),
            };
            let handlers = self.handlers.entry(event).or_default();
            handlers.push(entry);
            // Sort by priority descending (higher priority first).
            handlers.sort_by_key(|h| std::cmp::Reverse(h.handler.priority()));
        }
        info!(handler = handler.name(), "hook handler registered");
    }

    /// Returns true if any handlers are registered for the given event.
    pub fn has_handlers(&self, event: HookEvent) -> bool {
        self.handlers.get(&event).is_some_and(|v| !v.is_empty())
    }

    /// Get stats for a named handler. Returns None if not found.
    pub fn handler_stats(&self, name: &str) -> Option<Arc<HookStats>> {
        for entries in self.handlers.values() {
            for entry in entries {
                if entry.handler.name() == name {
                    return Some(Arc::clone(&entry.stats));
                }
            }
        }
        None
    }

    /// List all registered handler names (deduplicated).
    pub fn handler_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .handlers
            .values()
            .flatten()
            .map(|e| e.handler.name().to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Check if a handler is circuit-broken and potentially re-enable it.
    fn check_circuit_breaker(&self, entry: &HandlerEntry) -> bool {
        let is_disabled = entry.stats.disabled.load(Ordering::Relaxed);

        if !is_disabled {
            // Check if we should trip the breaker.
            let consecutive_failures = entry.stats.consecutive_failures.load(Ordering::Relaxed);
            if consecutive_failures >= self.circuit_breaker_threshold {
                entry.stats.disabled.store(true, Ordering::Relaxed);
                *entry
                    .stats
                    .disabled_at
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                warn!(
                    handler = entry.handler.name(),
                    "hook circuit breaker tripped after {} consecutive failures",
                    self.circuit_breaker_threshold
                );
                return true;
            }
            return false;
        }

        // Already disabled — check if cooldown period has elapsed.
        let disabled_at = entry
            .stats
            .disabled_at
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(at) = *disabled_at
            && at.elapsed() >= self.circuit_breaker_cooldown
        {
            drop(disabled_at);
            entry.stats.disabled.store(false, Ordering::Relaxed);
            entry.stats.consecutive_failures.store(0, Ordering::Relaxed);
            info!(
                handler = entry.handler.name(),
                "hook circuit breaker reset after cooldown"
            );
            return false;
        }
        true // still disabled
    }

    /// Dispatch an event to all registered handlers.
    ///
    /// Read-only events dispatch handlers in parallel (results are collected
    /// but Block/Modify are ignored since the event is informational).
    /// Modifying events dispatch sequentially:
    /// - Returns the first [`HookAction::Block`] encountered (short-circuits).
    /// - Returns the last [`HookAction::ModifyPayload`] if any.
    /// - Otherwise returns [`HookAction::Continue`].
    pub async fn dispatch(&self, payload: &HookPayload) -> Result<HookAction> {
        let event = payload.event();
        let handlers = match self.handlers.get(&event) {
            Some(h) if !h.is_empty() => h,
            _ => return Ok(HookAction::Continue),
        };

        debug!(event = %event, count = handlers.len(), "dispatching hook event");

        if event.is_read_only() {
            self.dispatch_parallel(event, payload, handlers).await
        } else {
            self.dispatch_sequential(event, payload, handlers).await
        }
    }

    /// Dispatch handlers in parallel. Block/Modify actions are logged but
    /// don't affect the event flow (read-only events are informational).
    async fn dispatch_parallel(
        &self,
        event: HookEvent,
        payload: &HookPayload,
        handlers: &[HandlerEntry],
    ) -> Result<HookAction> {
        let mut futures = Vec::new();
        for entry in handlers {
            if self.check_circuit_breaker(entry) {
                continue;
            }
            let handler = Arc::clone(&entry.handler);
            let stats = Arc::clone(&entry.stats);
            let payload = payload.clone();
            futures.push(async move {
                let start = Instant::now();
                let result = handler.handle(event, &payload).await;
                let latency = start.elapsed();
                match &result {
                    Ok(_) => stats.record_success(latency),
                    Err(_) => stats.record_failure(latency),
                }
                (handler.name().to_string(), result)
            });
        }

        let results = futures::future::join_all(futures).await;
        for (name, result) in results {
            match result {
                Ok(HookAction::Continue) => {},
                Ok(HookAction::ModifyPayload(_)) => {
                    debug!(handler = %name, event = %event, "hook modify on read-only event (ignored)");
                },
                Ok(HookAction::Block(reason)) => {
                    debug!(handler = %name, event = %event, reason = %reason, "hook block on read-only event (ignored)");
                },
                Err(e) => {
                    warn!(handler = %name, event = %event, error = %e, "hook handler failed");
                },
            }
        }

        Ok(HookAction::Continue)
    }

    /// Dispatch handlers sequentially (for modifying events).
    async fn dispatch_sequential(
        &self,
        event: HookEvent,
        payload: &HookPayload,
        handlers: &[HandlerEntry],
    ) -> Result<HookAction> {
        let mut last_modify: Option<Value> = None;

        for entry in handlers {
            if self.check_circuit_breaker(entry) {
                continue;
            }

            let start = Instant::now();
            let result = entry.handler.handle(event, payload).await;
            let latency = start.elapsed();

            match result {
                Ok(HookAction::Continue) => {
                    entry.stats.record_success(latency);
                },
                Ok(HookAction::ModifyPayload(v)) => {
                    entry.stats.record_success(latency);
                    if self.dry_run {
                        info!(handler = entry.handler.name(), event = %event, "hook modify (dry-run, not applied)");
                    } else {
                        debug!(handler = entry.handler.name(), event = %event, "hook modified payload");
                        last_modify = Some(v);
                    }
                },
                Ok(HookAction::Block(reason)) => {
                    entry.stats.record_success(latency);
                    if self.dry_run {
                        info!(handler = entry.handler.name(), event = %event, reason = %reason, "hook block (dry-run, not applied)");
                    } else {
                        info!(handler = entry.handler.name(), event = %event, reason = %reason, "hook blocked event");
                        return Ok(HookAction::Block(reason));
                    }
                },
                Err(e) => {
                    entry.stats.record_failure(latency);
                    warn!(handler = entry.handler.name(), event = %event, error = %e, "hook handler failed");
                },
            }
        }

        match last_modify {
            Some(v) => Ok(HookAction::ModifyPayload(v)),
            None => Ok(HookAction::Continue),
        }
    }

    /// Synchronous dispatch for hot-path events like `ToolResultPersist`.
    pub fn dispatch_sync(&self, payload: &HookPayload) -> Result<HookAction> {
        let event = payload.event();
        let handlers = match self.handlers.get(&event) {
            Some(h) if !h.is_empty() => h,
            _ => return Ok(HookAction::Continue),
        };

        debug!(event = %event, count = handlers.len(), "dispatching hook event (sync)");

        let mut last_modify: Option<Value> = None;

        for entry in handlers {
            if self.check_circuit_breaker(entry) {
                continue;
            }

            let start = Instant::now();
            let result = entry.handler.handle_sync(event, payload);
            let latency = start.elapsed();

            match result {
                Ok(HookAction::Continue) => {
                    entry.stats.record_success(latency);
                },
                Ok(HookAction::ModifyPayload(v)) => {
                    entry.stats.record_success(latency);
                    if self.dry_run {
                        info!(handler = entry.handler.name(), event = %event, "hook modify (dry-run, not applied)");
                    } else {
                        debug!(handler = entry.handler.name(), event = %event, "hook modified payload (sync)");
                        last_modify = Some(v);
                    }
                },
                Ok(HookAction::Block(reason)) => {
                    entry.stats.record_success(latency);
                    if self.dry_run {
                        info!(handler = entry.handler.name(), event = %event, reason = %reason, "hook block (dry-run, not applied)");
                    } else {
                        info!(handler = entry.handler.name(), event = %event, reason = %reason, "hook blocked event (sync)");
                        return Ok(HookAction::Block(reason));
                    }
                },
                Err(e) => {
                    entry.stats.record_failure(latency);
                    warn!(handler = entry.handler.name(), event = %event, error = %e, "hook handler failed (sync)");
                },
            }
        }

        match last_modify {
            Some(v) => Ok(HookAction::ModifyPayload(v)),
            None => Ok(HookAction::Continue),
        }
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct PriorityHandler {
        handler_name: String,
        handler_priority: i32,
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for PriorityHandler {
        fn name(&self) -> &str {
            &self.handler_name
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        fn priority(&self) -> i32 {
            self.handler_priority
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            Ok(HookAction::Continue)
        }
    }

    struct BlockingPriorityHandler {
        handler_name: String,
        handler_priority: i32,
        subscribed: Vec<HookEvent>,
    }

    #[async_trait]
    impl HookHandler for BlockingPriorityHandler {
        fn name(&self) -> &str {
            &self.handler_name
        }

        fn events(&self) -> &[HookEvent] {
            &self.subscribed
        }

        fn priority(&self) -> i32 {
            self.handler_priority
        }

        async fn handle(&self, _event: HookEvent, _payload: &HookPayload) -> Result<HookAction> {
            Ok(HookAction::Block(self.handler_name.clone()))
        }
    }

    fn modifying_payload() -> HookPayload {
        HookPayload::BeforeToolCall {
            session_key: "test".into(),
            tool_name: "exec".into(),
            arguments: serde_json::json!({}),
        }
    }

    fn read_only_payload() -> HookPayload {
        HookPayload::SessionStart {
            session_key: "test".into(),
        }
    }

    #[test]
    fn priority_ordering() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(PriorityHandler {
            handler_name: "low".into(),
            handler_priority: -10,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(PriorityHandler {
            handler_name: "high".into(),
            handler_priority: 10,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(PriorityHandler {
            handler_name: "default".into(),
            handler_priority: 0,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));

        let handlers = registry.handlers.get(&HookEvent::BeforeToolCall).unwrap();
        assert_eq!(handlers[0].handler.name(), "high");
        assert_eq!(handlers[1].handler.name(), "default");
        assert_eq!(handlers[2].handler.name(), "low");
    }

    #[tokio::test]
    async fn higher_priority_block_wins() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(BlockingPriorityHandler {
            handler_name: "low-blocker".into(),
            handler_priority: 0,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));
        registry.register(Arc::new(BlockingPriorityHandler {
            handler_name: "high-blocker".into(),
            handler_priority: 10,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));

        let result = registry.dispatch(&modifying_payload()).await.unwrap();
        match result {
            HookAction::Block(name) => assert_eq!(name, "high-blocker"),
            _ => panic!("expected Block from high-priority handler"),
        }
    }

    #[tokio::test]
    async fn read_only_events_ignore_block() {
        let mut registry = HookRegistry::new();
        registry.register(Arc::new(BlockingPriorityHandler {
            handler_name: "blocker".into(),
            handler_priority: 0,
            subscribed: vec![HookEvent::SessionStart],
        }));

        let result = registry.dispatch(&read_only_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn circuit_breaker_trips_after_failures() {
        struct FailingHandler;

        #[async_trait]
        impl HookHandler for FailingHandler {
            fn name(&self) -> &str {
                "failer"
            }

            fn events(&self) -> &[HookEvent] {
                &[HookEvent::BeforeToolCall]
            }

            async fn handle(
                &self,
                _event: HookEvent,
                _payload: &HookPayload,
            ) -> Result<HookAction> {
                anyhow::bail!("always fails")
            }
        }

        let mut registry = HookRegistry::new().with_circuit_breaker(2, Duration::from_millis(100));
        registry.register(Arc::new(FailingHandler));

        let payload = modifying_payload();

        // First two calls fail, accumulating consecutive_failures = 2.
        registry.dispatch(&payload).await.unwrap();
        registry.dispatch(&payload).await.unwrap();

        // Third call: check_circuit_breaker sees 2 >= 2 and trips the breaker.
        registry.dispatch(&payload).await.unwrap();
        let stats = registry.handler_stats("failer").unwrap();
        assert!(stats.disabled.load(Ordering::Relaxed));

        // Wait for cooldown and check re-enable.
        tokio::time::sleep(Duration::from_millis(150)).await;
        registry.dispatch(&payload).await.unwrap();
        assert!(!stats.disabled.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn dry_run_does_not_block() {
        let mut registry = HookRegistry::new().with_dry_run(true);
        registry.register(Arc::new(BlockingPriorityHandler {
            handler_name: "blocker".into(),
            handler_priority: 0,
            subscribed: vec![HookEvent::BeforeToolCall],
        }));

        let result = registry.dispatch(&modifying_payload()).await.unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[test]
    fn command_event_and_payload() {
        let payload = HookPayload::Command {
            session_key: "test".into(),
            action: "new".into(),
            sender_id: None,
        };
        assert_eq!(payload.event(), HookEvent::Command);
    }

    #[test]
    fn hook_stats_tracking() {
        let stats = HookStats::new();
        stats.record_success(Duration::from_millis(10));
        stats.record_success(Duration::from_millis(20));
        stats.record_failure(Duration::from_millis(30));
        assert_eq!(stats.call_count.load(Ordering::Relaxed), 3);
        assert_eq!(stats.failure_count.load(Ordering::Relaxed), 1);
        assert_eq!(stats.consecutive_failures.load(Ordering::Relaxed), 1);
        assert_eq!(stats.avg_latency(), Duration::from_millis(20));
    }

    #[test]
    fn read_only_classification() {
        assert!(HookEvent::AgentEnd.is_read_only());
        assert!(HookEvent::SessionStart.is_read_only());
        assert!(HookEvent::GatewayStart.is_read_only());
        assert!(HookEvent::Command.is_read_only());
        assert!(!HookEvent::BeforeAgentStart.is_read_only());
        assert!(!HookEvent::BeforeLLMCall.is_read_only());
        assert!(!HookEvent::AfterLLMCall.is_read_only());
        assert!(!HookEvent::BeforeToolCall.is_read_only());
        assert!(!HookEvent::MessageSending.is_read_only());
        assert!(!HookEvent::ToolResultPersist.is_read_only());
    }

    #[test]
    fn before_llm_call_payload_event() {
        let payload = HookPayload::BeforeLLMCall {
            session_key: "sess-1".into(),
            provider: "openai".into(),
            model: "gpt-4o".into(),
            messages: serde_json::json!([{"role": "user", "content": "hello"}]),
            tool_count: 3,
            iteration: 1,
        };
        assert_eq!(payload.event(), HookEvent::BeforeLLMCall);
    }

    #[test]
    fn after_llm_call_payload_event() {
        let payload = HookPayload::AfterLLMCall {
            session_key: "sess-1".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-20250514".into(),
            text: Some("Hello!".into()),
            tool_calls: vec![serde_json::json!({"name": "exec"})],
            input_tokens: 100,
            output_tokens: 50,
            iteration: 2,
        };
        assert_eq!(payload.event(), HookEvent::AfterLLMCall);
    }

    #[test]
    fn all_events_array_includes_llm_hooks() {
        assert!(HookEvent::ALL.contains(&HookEvent::BeforeLLMCall));
        assert!(HookEvent::ALL.contains(&HookEvent::AfterLLMCall));
        assert_eq!(HookEvent::ALL.len(), 17);
    }

    #[test]
    fn llm_hook_payloads_serialize_round_trip() {
        let before = HookPayload::BeforeLLMCall {
            session_key: "s".into(),
            provider: "p".into(),
            model: "m".into(),
            messages: serde_json::json!([]),
            tool_count: 0,
            iteration: 1,
        };
        let json = serde_json::to_string(&before).unwrap();
        let deser: HookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.event(), HookEvent::BeforeLLMCall);

        let after = HookPayload::AfterLLMCall {
            session_key: "s".into(),
            provider: "p".into(),
            model: "m".into(),
            text: None,
            tool_calls: vec![],
            input_tokens: 0,
            output_tokens: 0,
            iteration: 1,
        };
        let json = serde_json::to_string(&after).unwrap();
        let deser: HookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.event(), HookEvent::AfterLLMCall);
    }
}
