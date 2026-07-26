//! The instrumentation API used by the agent runtime.
//!
//! Callers hold a [`TurnRecorder`] for the duration of an agent run and open a
//! [`StepGuard`] around each LLM call, tool invocation or retrieval. The
//! recorder owns id generation, parent/child nesting, trace-scope propagation
//! and redaction, so instrumentation at the call site stays to a few lines.
//!
//! Every entry point is a no-op when no sink is installed, and the constructor
//! returns `None` in that case so callers pay nothing beyond one atomic read.

use std::sync::{Arc, Mutex};

use time::OffsetDateTime;

use crate::{
    model::{
        Event, Level, ObservationId, ObservationKind, ObservationRecord, ScoreRecord, ScoreValue,
        TokenUsage, TraceId, TraceRecord, TraceScope,
    },
    redact::RedactionPolicy,
    sink::{self, ObservationSink},
};

/// Settings that shape what a recorder emits.
#[derive(Debug, Clone)]
pub struct RecorderSettings {
    /// Redaction applied to every payload before it is handed to a sink.
    pub redaction: RedactionPolicy,
    /// Whether to attach turn and step inputs.
    pub capture_input: bool,
    /// Whether to attach turn and step outputs.
    pub capture_output: bool,
    /// Whether to attach tool arguments and results.
    pub capture_tool_io: bool,
    /// Fraction of turns to trace, in `0.0..=1.0`.
    pub sample_rate: f64,
}

impl Default for RecorderSettings {
    fn default() -> Self {
        Self {
            redaction: RedactionPolicy::default(),
            capture_input: true,
            capture_output: true,
            capture_tool_io: true,
            sample_rate: 1.0,
        }
    }
}

impl RecorderSettings {
    /// Whether this turn should be traced, given the sample rate.
    #[must_use]
    fn sampled(&self) -> bool {
        if self.sample_rate >= 1.0 {
            return true;
        }
        if self.sample_rate <= 0.0 {
            return false;
        }
        rand::random::<f64>() < self.sample_rate
    }
}

/// Records one agent run.
pub struct TurnRecorder {
    sink: Arc<dyn ObservationSink>,
    settings: RecorderSettings,
    trace_id: TraceId,
    scope: TraceScope,
    /// The run's root observation, parent of every step.
    root_id: ObservationId,
    /// Trace record, retained so the closing update carries the final output.
    trace: Mutex<TraceRecord>,
}

impl TurnRecorder {
    /// Begin recording a turn, or return `None` when instrumentation is off or
    /// the turn was not sampled.
    #[must_use]
    pub fn begin(
        name: impl Into<String>,
        scope: TraceScope,
        settings: RecorderSettings,
    ) -> Option<Self> {
        let sink = sink::global_sink()?;
        if !settings.sampled() {
            return None;
        }

        let mut trace = TraceRecord::new(name);
        trace.scope = scope.clone();
        let trace_id = trace.id.clone();

        // The root observation shares the trace id so exporters can parent
        // orphan steps onto it without extra bookkeeping.
        let root_id = ObservationId(trace_id.0.clone());

        let recorder = Self {
            sink,
            settings,
            trace_id: trace_id.clone(),
            scope,
            root_id,
            trace: Mutex::new(trace.clone()),
        };
        recorder.sink.record(Event::Trace(Box::new(trace)));
        Some(recorder)
    }

    /// The trace being recorded, for correlating scores later.
    #[must_use]
    pub fn trace_id(&self) -> &TraceId {
        &self.trace_id
    }

    /// Attach the turn's input.
    pub fn set_input(&self, input: serde_json::Value) {
        if !self.settings.capture_input {
            return;
        }
        let redacted = self.settings.redaction.redact(&input);
        self.with_trace(|trace| trace.input = Some(redacted));
    }

    /// Attach the turn's output.
    pub fn set_output(&self, output: serde_json::Value) {
        if !self.settings.capture_output {
            return;
        }
        let redacted = self.settings.redaction.redact(&output);
        self.with_trace(|trace| trace.output = Some(redacted));
    }

    /// Attach a metadata entry to the trace.
    pub fn set_metadata(&self, key: impl Into<String>, value: serde_json::Value) {
        self.with_trace(|trace| {
            trace.metadata.insert(key.into(), value);
        });
    }

    /// Open a step nested under the run's root.
    #[must_use]
    pub fn step(&self, kind: ObservationKind, name: impl Into<String>) -> StepGuard {
        self.step_under(kind, name, Some(self.root_id.clone()))
    }

    /// Open a step nested under an explicit parent, for sub-agents and tools
    /// invoked from within another step.
    #[must_use]
    pub fn step_under(
        &self,
        kind: ObservationKind,
        name: impl Into<String>,
        parent: Option<ObservationId>,
    ) -> StepGuard {
        let record = ObservationRecord::start(self.trace_id.clone(), kind, name)
            .with_parent(parent.or_else(|| Some(self.root_id.clone())))
            .with_scope(self.scope.clone());

        // Emit the start eagerly so a long-running turn is visible in the
        // backend while it is still executing, rather than only at the end.
        self.sink
            .record(Event::ObservationStart(Box::new(record.clone())));

        StepGuard {
            sink: Arc::clone(&self.sink),
            settings: self.settings.clone(),
            record: Some(record),
        }
    }

    /// Record a score against this turn.
    pub fn score(&self, name: impl Into<String>, value: ScoreValue, comment: Option<String>) {
        let mut score = ScoreRecord::new(self.trace_id.clone(), name, value);
        score.comment = comment;
        score.environment = self.scope.environment.clone();
        self.sink.record(Event::Score(Box::new(score)));
    }

    /// Close the turn, emitting the final trace state.
    ///
    /// Takes `&self` rather than `self` so the recorder can be shared across
    /// the concurrently-executing tool futures in the agent loop.
    pub fn finish(&self) {
        let trace = self.snapshot_trace();
        self.sink.record(Event::Trace(Box::new(trace)));
    }

    /// Close the turn as failed.
    pub fn finish_with_error(&self, message: impl Into<String>) {
        self.set_metadata("error", serde_json::Value::String(message.into()));
        self.finish();
    }

    fn with_trace(&self, f: impl FnOnce(&mut TraceRecord)) {
        let mut guard = self
            .trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut guard);
    }

    fn snapshot_trace(&self) -> TraceRecord {
        self.trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// An open observation. Emits its completion on drop, so an early `?` return
/// still produces a closed span rather than one that hangs open forever.
pub struct StepGuard {
    sink: Arc<dyn ObservationSink>,
    settings: RecorderSettings,
    /// `None` once the guard has emitted, so drop does not emit twice.
    record: Option<ObservationRecord>,
}

impl StepGuard {
    /// This step's id, for nesting children beneath it.
    #[must_use]
    pub fn id(&self) -> Option<ObservationId> {
        self.record.as_ref().map(|r| r.id.clone())
    }

    /// Attach the step's input.
    pub fn set_input(&mut self, input: serde_json::Value) {
        if !self.capture_input_allowed() {
            return;
        }
        let redacted = self.settings.redaction.redact(&input);
        if let Some(record) = self.record.as_mut() {
            record.input = Some(redacted);
        }
    }

    /// Attach the step's output.
    pub fn set_output(&mut self, output: serde_json::Value) {
        if !self.capture_output_allowed() {
            return;
        }
        let redacted = self.settings.redaction.redact(&output);
        if let Some(record) = self.record.as_mut() {
            record.output = Some(redacted);
        }
    }

    /// Set the model this step used.
    pub fn set_model(&mut self, model: impl Into<String>) {
        if let Some(record) = self.record.as_mut() {
            record.model = Some(model.into());
        }
    }

    /// Set a sampling parameter.
    pub fn set_model_parameter(&mut self, key: impl Into<String>, value: serde_json::Value) {
        if let Some(record) = self.record.as_mut() {
            record.model_parameters.insert(key.into(), value);
        }
    }

    /// Set token usage.
    pub fn set_usage(&mut self, usage: TokenUsage) {
        if let Some(record) = self.record.as_mut() {
            record.usage = Some(usage);
        }
    }

    /// Mark the instant the first output token arrived.
    ///
    /// Ignored if called twice: only the first token defines time-to-first-token.
    pub fn mark_first_token(&mut self) {
        if let Some(record) = self.record.as_mut()
            && record.completion_start_time.is_none()
        {
            record.completion_start_time = Some(OffsetDateTime::now_utc());
        }
    }

    /// Link this generation to a managed prompt version.
    pub fn set_prompt(&mut self, name: impl Into<String>, version: i32) {
        if let Some(record) = self.record.as_mut() {
            record.prompt_name = Some(name.into());
            record.prompt_version = Some(version);
        }
    }

    /// Attach a metadata entry.
    pub fn set_metadata(&mut self, key: impl Into<String>, value: serde_json::Value) {
        if let Some(record) = self.record.as_mut() {
            record.metadata.insert(key.into(), value);
        }
    }

    /// Raise the severity of this step.
    pub fn set_level(&mut self, level: Level, message: Option<String>) {
        if let Some(record) = self.record.as_mut() {
            record.level = level;
            record.status_message = message;
        }
    }

    /// Mark the step failed. The span is still emitted on drop.
    pub fn fail(&mut self, message: impl Into<String>) {
        if let Some(record) = self.record.as_mut() {
            record.fail(message);
        }
    }

    /// Emit the completed observation now rather than at drop.
    pub fn finish(mut self) {
        self.emit();
    }

    fn emit(&mut self) {
        let Some(mut record) = self.record.take() else {
            return;
        };
        record.finish();
        self.sink.record(Event::ObservationEnd(Box::new(record)));
    }

    /// Tool arguments are gated by their own switch as well as the input
    /// switch, since they are the most likely place for credentials to appear.
    fn capture_input_allowed(&self) -> bool {
        match self.record.as_ref().map(|r| r.kind) {
            Some(ObservationKind::Tool) => self.settings.capture_tool_io,
            _ => self.settings.capture_input,
        }
    }

    fn capture_output_allowed(&self) -> bool {
        match self.record.as_ref().map(|r| r.kind) {
            Some(ObservationKind::Tool) => self.settings.capture_tool_io,
            _ => self.settings.capture_output,
        }
    }
}

impl Drop for StepGuard {
    fn drop(&mut self) {
        // An early return through `?` must still close the span; otherwise the
        // backend shows a step that never ended.
        self.emit();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use {async_trait::async_trait, serde_json::json};

    use super::*;

    struct CollectingSink {
        events: StdMutex<Vec<Event>>,
    }

    impl CollectingSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: StdMutex::new(Vec::new()),
            })
        }

        fn events(&self) -> Vec<Event> {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn observations(&self) -> Vec<ObservationRecord> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::ObservationEnd(o) => Some(*o),
                    _ => None,
                })
                .collect()
        }

        fn traces(&self) -> Vec<TraceRecord> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    Event::Trace(t) => Some(*t),
                    _ => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl ObservationSink for CollectingSink {
        fn name(&self) -> &str {
            "collecting"
        }

        fn record(&self, event: Event) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }

        async fn flush(&self, _timeout: std::time::Duration) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// Serialises tests that touch the process-wide sink registry.
    static GLOBAL_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_sink<R>(f: impl FnOnce(Arc<CollectingSink>) -> R) -> R {
        let _guard = GLOBAL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let collected = CollectingSink::new();
        sink::set_global_sink(collected.clone());
        let out = f(collected);
        sink::clear_global_sink();
        out
    }

    fn scope() -> TraceScope {
        TraceScope {
            session_id: Some("agent:main:main".into()),
            user_id: Some("telegram:42".into()),
            tags: vec!["telegram".into()],
            environment: Some("production".into()),
            release: None,
            version: None,
        }
    }

    #[test]
    fn returns_none_when_no_sink_is_installed() {
        let _guard = GLOBAL_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sink::clear_global_sink();

        // Instrumentation must cost nothing when switched off.
        assert!(TurnRecorder::begin("turn", scope(), RecorderSettings::default()).is_none());
    }

    #[test]
    fn zero_sample_rate_skips_the_turn_entirely() {
        with_sink(|collected| {
            let settings = RecorderSettings {
                sample_rate: 0.0,
                ..Default::default()
            };
            assert!(TurnRecorder::begin("turn", scope(), settings).is_none());
            assert!(collected.events().is_empty());
        });
    }

    #[test]
    fn steps_are_parented_to_the_run_root() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            let root = ObservationId(recorder.trace_id().0.clone());

            recorder.step(ObservationKind::Generation, "llm").finish();
            recorder.finish();

            let observations = collected.observations();
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].parent_id, Some(root));
        });
    }

    #[test]
    fn nested_steps_are_parented_to_their_enclosing_step() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");

            let outer = recorder.step(ObservationKind::Agent, "sub-agent");
            let outer_id = outer.id().expect("open step has an id");
            recorder
                .step_under(ObservationKind::Tool, "exec", Some(outer_id.clone()))
                .finish();
            outer.finish();
            recorder.finish();

            let tool = collected
                .observations()
                .into_iter()
                .find(|o| o.kind == ObservationKind::Tool)
                .expect("tool observation emitted");
            assert_eq!(tool.parent_id, Some(outer_id));
        });
    }

    #[test]
    fn steps_carry_the_trace_scope() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            recorder.step(ObservationKind::Generation, "llm").finish();
            recorder.finish();

            let observed = &collected.observations()[0];
            assert_eq!(
                observed.scope.session_id.as_deref(),
                Some("agent:main:main")
            );
            assert_eq!(observed.scope.user_id.as_deref(), Some("telegram:42"));
        });
    }

    #[test]
    fn a_start_event_is_emitted_before_the_step_completes() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            let step = recorder.step(ObservationKind::Generation, "llm");

            // A multi-minute turn should be visible while it is still running.
            let starts = collected
                .events()
                .into_iter()
                .filter(|e| matches!(e, Event::ObservationStart(_)))
                .count();
            assert_eq!(starts, 1);

            step.finish();
            recorder.finish();
        });
    }

    #[test]
    fn dropping_a_step_still_closes_it() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");

            // Simulates an early `?` return out of the agent loop.
            drop(recorder.step(ObservationKind::Tool, "exec"));
            recorder.finish();

            let observations = collected.observations();
            assert_eq!(observations.len(), 1, "dropped step must still be emitted");
            assert!(observations[0].end_time.is_some());
        });
    }

    #[test]
    fn finishing_a_step_does_not_emit_it_twice() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            recorder.step(ObservationKind::Tool, "exec").finish();
            recorder.finish();

            assert_eq!(collected.observations().len(), 1);
        });
    }

    #[test]
    fn payloads_are_redacted_before_reaching_the_sink() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");

            let mut step = recorder.step(ObservationKind::Tool, "exec");
            step.set_input(json!({ "api_key": "abc123", "cmd": "ls" }));
            step.finish();
            recorder.set_input(json!({ "password": "hunter2" }));
            recorder.finish();

            let tool_input = collected.observations()[0]
                .input
                .clone()
                .expect("input captured");
            assert_eq!(tool_input["api_key"], json!("[REDACTED]"));
            assert_eq!(tool_input["cmd"], json!("ls"));

            let trace = collected.traces().last().cloned().expect("trace emitted");
            assert_eq!(
                trace.input.expect("trace input")["password"],
                json!("[REDACTED]")
            );
        });
    }

    #[test]
    fn capture_switches_suppress_payloads() {
        with_sink(|collected| {
            let settings = RecorderSettings {
                capture_input: false,
                capture_output: false,
                capture_tool_io: false,
                ..Default::default()
            };
            let recorder = TurnRecorder::begin("turn", scope(), settings).expect("sink installed");

            let mut step = recorder.step(ObservationKind::Tool, "exec");
            step.set_input(json!({ "cmd": "ls" }));
            step.set_output(json!("file list"));
            step.finish();
            recorder.set_input(json!("hello"));
            recorder.finish();

            let observed = &collected.observations()[0];
            assert!(observed.input.is_none());
            assert!(observed.output.is_none());
            assert!(collected.traces().last().expect("trace").input.is_none());
        });
    }

    #[test]
    fn tool_io_switch_is_independent_of_the_generation_switches() {
        with_sink(|collected| {
            // Tool arguments are the likeliest place for credentials, so they
            // must be suppressible without losing LLM inputs.
            let settings = RecorderSettings {
                capture_input: true,
                capture_tool_io: false,
                ..Default::default()
            };
            let recorder = TurnRecorder::begin("turn", scope(), settings).expect("sink installed");

            let mut tool = recorder.step(ObservationKind::Tool, "exec");
            tool.set_input(json!({ "cmd": "ls" }));
            tool.finish();

            let mut generation = recorder.step(ObservationKind::Generation, "llm");
            generation.set_input(json!("hello"));
            generation.finish();
            recorder.finish();

            let observations = collected.observations();
            let tool = observations
                .iter()
                .find(|o| o.kind == ObservationKind::Tool)
                .expect("tool emitted");
            let generation = observations
                .iter()
                .find(|o| o.kind == ObservationKind::Generation)
                .expect("generation emitted");

            assert!(tool.input.is_none());
            assert!(generation.input.is_some());
        });
    }

    #[test]
    fn first_token_marker_records_only_the_first_call() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");

            let mut step = recorder.step(ObservationKind::Generation, "llm");
            step.mark_first_token();
            let first = step
                .record
                .as_ref()
                .and_then(|r| r.completion_start_time)
                .expect("first token recorded");
            step.mark_first_token();
            let second = step
                .record
                .as_ref()
                .and_then(|r| r.completion_start_time)
                .expect("still recorded");
            step.finish();
            recorder.finish();

            assert_eq!(first, second, "time-to-first-token must not drift");
            assert!(collected.observations()[0].completion_start_time.is_some());
        });
    }

    #[test]
    fn failed_steps_carry_error_level_and_message() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");

            let mut step = recorder.step(ObservationKind::Generation, "llm");
            step.fail("provider returned 500");
            step.finish();
            recorder.finish();

            let observed = &collected.observations()[0];
            assert_eq!(observed.level, Level::Error);
            assert_eq!(
                observed.status_message.as_deref(),
                Some("provider returned 500")
            );
        });
    }

    #[test]
    fn scores_are_emitted_against_the_turn() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            let trace_id = recorder.trace_id().clone();

            recorder.score(
                "user-feedback",
                ScoreValue::Numeric(1.0),
                Some("helpful".into()),
            );
            recorder.finish();

            let score = collected
                .events()
                .into_iter()
                .find_map(|e| match e {
                    Event::Score(s) => Some(*s),
                    _ => None,
                })
                .expect("score emitted");

            assert_eq!(score.trace_id, trace_id);
            assert_eq!(score.environment.as_deref(), Some("production"));
        });
    }

    #[test]
    fn the_closing_trace_carries_output_set_during_the_turn() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            recorder.set_output(json!("final answer"));
            recorder.finish();

            let last = collected.traces().last().cloned().expect("trace emitted");
            assert_eq!(last.output, Some(json!("final answer")));
        });
    }

    #[test]
    fn finish_with_error_records_the_failure_on_the_trace() {
        with_sink(|collected| {
            let recorder = TurnRecorder::begin("turn", scope(), RecorderSettings::default())
                .expect("sink installed");
            recorder.finish_with_error("context window exceeded");

            let last = collected.traces().last().cloned().expect("trace emitted");
            assert_eq!(
                last.metadata.get("error"),
                Some(&json!("context window exceeded"))
            );
        });
    }
}
