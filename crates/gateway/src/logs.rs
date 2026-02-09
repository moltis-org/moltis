//! In-memory log capture layer and live logs service.
//!
//! [`LogBroadcastLayer`] is a `tracing_subscriber::Layer` that captures every
//! tracing event into a bounded ring buffer and broadcasts new entries to a
//! `tokio::sync::broadcast` channel for real-time streaming to WebSocket
//! clients.
//!
//! When persistence is enabled via [`LogBuffer::enable_persistence`], entries
//! are appended to a JSONL file. Historical entries are read lazily from the
//! file only when the UI requests them — nothing is loaded into memory at
//! startup.

use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::sync::broadcast,
    tracing::field::{Field, Visit},
    tracing_subscriber::{Layer, layer::Context},
};

use crate::services::{LogsService, ServiceResult};

// ── LogEntry ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts: u64,
    pub level: String,
    pub target: String,
    pub message: String,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
}

// ── LogBuffer ───────────────────────────────────────────────────────────────

const DEFAULT_CAPACITY: usize = 10_000;
const DEFAULT_BROADCAST_CAPACITY: usize = 512;

#[derive(Clone)]
pub struct LogBuffer {
    /// In-memory ring buffer for current-session entries.
    buf: Arc<RwLock<VecDeque<LogEntry>>>,
    capacity: usize,
    tx: broadcast::Sender<LogEntry>,
    /// Append-only file writer (set after `enable_persistence`).
    writer: Arc<Mutex<Option<File>>>,
    /// Path to the JSONL file for on-demand reads.
    file_path: Arc<RwLock<Option<PathBuf>>>,
    /// Path to a small file persisting visit state.
    visited_path: Arc<RwLock<Option<PathBuf>>>,
    /// Running totals of warn/error entries (all time, including file).
    total_warns: Arc<AtomicU64>,
    total_errors: Arc<AtomicU64>,
    /// Snapshot of totals at last ack (visit).
    acked_warns: Arc<AtomicU64>,
    acked_errors: Arc<AtomicU64>,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            buf: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
            tx,
            writer: Arc::new(Mutex::new(None)),
            file_path: Arc::new(RwLock::new(None)),
            visited_path: Arc::new(RwLock::new(None)),
            total_warns: Arc::new(AtomicU64::new(0)),
            total_errors: Arc::new(AtomicU64::new(0)),
            acked_warns: Arc::new(AtomicU64::new(0)),
            acked_errors: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Enable file-backed persistence. New entries will be appended to `path`.
    /// Historical entries are **not** loaded into memory — they are read
    /// on-demand when [`Self::list_from_file`] is called.
    ///
    /// Scans the JSONL file once to seed warn/error counters, and loads acked
    /// counters from a sibling `.visited` file.
    pub fn enable_persistence(&self, path: PathBuf) {
        // Store path for on-demand reads.
        if let Ok(mut fp) = self.file_path.write() {
            *fp = Some(path.clone());
        }

        // Seed running totals by scanning existing log file.
        if let Ok(file) = File::open(&path) {
            let reader = BufReader::new(file);
            let (mut w, mut e) = (0u64, 0u64);
            for line in reader.lines() {
                let Ok(line) = line else {
                    continue;
                };
                // Fast path: check level without full deserialization.
                if line.contains("\"WARN\"") || line.contains("\"warn\"") {
                    w += 1;
                } else if line.contains("\"ERROR\"") || line.contains("\"error\"") {
                    e += 1;
                }
            }
            self.total_warns.store(w, Ordering::Relaxed);
            self.total_errors.store(e, Ordering::Relaxed);
        }

        // Open the file for appending.
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path)
            && let Ok(mut w) = self.writer.lock()
        {
            *w = Some(file);
        }

        // Load acked counters from sibling file.
        // Format: "acked_warns acked_errors"
        let vpath = path.with_extension("visited");
        if let Ok(contents) = std::fs::read_to_string(&vpath) {
            let parts: Vec<&str> = contents.split_whitespace().collect();
            if parts.len() == 2
                && let (Ok(aw), Ok(ae)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>())
            {
                self.acked_warns.store(aw, Ordering::Relaxed);
                self.acked_errors.store(ae, Ordering::Relaxed);
            }
        }
        if let Ok(mut vp) = self.visited_path.write() {
            *vp = Some(vpath);
        }
    }

    pub fn push(&self, entry: LogEntry) {
        // Update running counters.
        match entry.level.as_str() {
            "WARN" | "warn" => {
                self.total_warns.fetch_add(1, Ordering::Relaxed);
            },
            "ERROR" | "error" => {
                self.total_errors.fetch_add(1, Ordering::Relaxed);
            },
            _ => {},
        }

        // Best-effort broadcast — receivers may be behind.
        let _ = self.tx.send(entry.clone());

        // Persist to file if enabled.
        if let Ok(mut w) = self.writer.lock()
            && let Some(ref mut file) = *w
            && let Ok(json) = serde_json::to_string(&entry)
        {
            let _ = writeln!(file, "{json}");
        }

        if let Ok(mut buf) = self.buf.write() {
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.tx.subscribe()
    }

    /// Return the last `limit` entries from the in-memory ring buffer only.
    pub fn list(&self, filter: &LogFilter, limit: usize) -> Vec<LogEntry> {
        let buf = match self.buf.read() {
            Ok(b) => b,
            Err(_) => return vec![],
        };
        buf.iter()
            .rev()
            .filter(|e| filter.matches(e))
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Read the last `limit` matching entries from the persisted JSONL file.
    /// This scans the entire file but does not keep entries in memory beyond
    /// the returned vector.
    pub fn list_from_file(&self, filter: &LogFilter, limit: usize) -> Vec<LogEntry> {
        let path = match self.file_path.read() {
            Ok(fp) => match fp.as_ref() {
                Some(p) => p.clone(),
                None => return vec![],
            },
            Err(_) => return vec![],
        };

        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => return vec![],
        };

        // Collect the last `limit` matching entries using a ring.
        let reader = BufReader::new(file);
        let mut ring = VecDeque::with_capacity(limit);
        for line in reader.lines() {
            let Ok(line) = line else {
                continue;
            };
            if line.is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<LogEntry>(&line) else {
                continue;
            };
            if !filter.matches(&entry) {
                continue;
            }
            if ring.len() >= limit {
                ring.pop_front();
            }
            ring.push_back(entry);
        }
        ring.into()
    }

    /// Return the path to the persisted JSONL file, if persistence is enabled.
    pub fn file_path(&self) -> Option<PathBuf> {
        self.file_path.read().ok().and_then(|fp| fp.clone())
    }

    /// O(1) count of unseen warn/error entries since the last ack.
    pub fn count_unseen(&self) -> (u64, u64) {
        let tw = self.total_warns.load(Ordering::Relaxed);
        let te = self.total_errors.load(Ordering::Relaxed);
        let aw = self.acked_warns.load(Ordering::Relaxed);
        let ae = self.acked_errors.load(Ordering::Relaxed);
        (tw.saturating_sub(aw), te.saturating_sub(ae))
    }

    /// Snapshot current totals as "seen" and persist to disk.
    pub fn ack_visit(&self) {
        let tw = self.total_warns.load(Ordering::Relaxed);
        let te = self.total_errors.load(Ordering::Relaxed);
        self.acked_warns.store(tw, Ordering::Relaxed);
        self.acked_errors.store(te, Ordering::Relaxed);

        if let Ok(vp) = self.visited_path.read()
            && let Some(ref path) = *vp
        {
            let _ = std::fs::write(path, format!("{tw} {te}"));
        }
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

// ── LogFilter ───────────────────────────────────────────────────────────────

pub struct LogFilter {
    pub level: Option<String>,
    pub target: Option<String>,
    pub search: Option<String>,
}

impl LogFilter {
    fn level_ord(l: &str) -> u8 {
        match l {
            "TRACE" | "trace" => 0,
            "DEBUG" | "debug" => 1,
            "INFO" | "info" => 2,
            "WARN" | "warn" => 3,
            "ERROR" | "error" => 4,
            _ => 2,
        }
    }

    fn matches(&self, entry: &LogEntry) -> bool {
        if let Some(ref lvl) = self.level
            && Self::level_ord(&entry.level) < Self::level_ord(lvl)
        {
            return false;
        }
        if let Some(ref tgt) = self.target
            && !tgt.is_empty()
            && !entry.target.contains(tgt.as_str())
        {
            return false;
        }
        if let Some(ref q) = self.search
            && !q.is_empty()
        {
            let q_lower = q.to_lowercase();
            if !entry.message.to_lowercase().contains(&q_lower)
                && !entry.target.to_lowercase().contains(&q_lower)
            {
                return false;
            }
        }
        true
    }
}

// ── Visitor (extracts fields from tracing events) ───────────────────────────

struct FieldVisitor {
    message: String,
    fields: serde_json::Map<String, Value>,
}

impl FieldVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: serde_json::Map::new(),
        }
    }
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().into(), Value::String(format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.into();
        } else {
            self.fields
                .insert(field.name().into(), Value::String(value.into()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().into(), Value::Bool(value));
    }
}

// ── LogBroadcastLayer ───────────────────────────────────────────────────────

pub struct LogBroadcastLayer {
    buffer: LogBuffer,
}

impl LogBroadcastLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> Layer<S> for LogBroadcastLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = FieldVisitor::new();
        event.record(&mut visitor);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let entry = LogEntry {
            ts,
            level: meta.level().to_string(),
            target: meta.target().into(),
            message: visitor.message,
            fields: visitor.fields,
        };

        self.buffer.push(entry);
    }
}

// ── LiveLogsService ─────────────────────────────────────────────────────────

pub struct LiveLogsService {
    buffer: LogBuffer,
}

impl LiveLogsService {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

#[async_trait]
impl LogsService for LiveLogsService {
    async fn tail(&self, params: Value) -> ServiceResult {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
        let filter = LogFilter {
            level: None,
            target: None,
            search: None,
        };
        let entries = self.buffer.list(&filter, limit);
        Ok(serde_json::json!({ "entries": entries, "subscribed": true }))
    }

    async fn list(&self, params: Value) -> ServiceResult {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
        let filter = LogFilter {
            level: params
                .get("level")
                .and_then(|v| v.as_str())
                .map(String::from),
            target: params
                .get("target")
                .and_then(|v| v.as_str())
                .map(String::from),
            search: params
                .get("search")
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        // Fast path: try in-memory ring buffer first (instant, covers current session).
        let entries = self.buffer.list(&filter, limit);
        if !entries.is_empty() {
            return Ok(serde_json::json!({ "entries": entries }));
        }
        // Slow path: fall back to file after restart when memory is empty.
        // Run on a blocking thread so we don't stall the async runtime.
        let buffer = self.buffer.clone();
        let entries = tokio::task::spawn_blocking(move || buffer.list_from_file(&filter, limit))
            .await
            .unwrap_or_default();
        Ok(serde_json::json!({ "entries": entries }))
    }

    async fn status(&self) -> ServiceResult {
        let (warns, errors) = self.buffer.count_unseen();
        Ok(serde_json::json!({ "unseen_warns": warns, "unseen_errors": errors }))
    }

    async fn ack(&self) -> ServiceResult {
        self.buffer.ack_visit();
        Ok(serde_json::json!({}))
    }

    fn log_file_path(&self) -> Option<PathBuf> {
        self.buffer.file_path()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry_ts(level: &str, target: &str, message: &str, ts: u64) -> LogEntry {
        LogEntry {
            ts,
            level: level.into(),
            target: target.into(),
            message: message.into(),
            fields: serde_json::Map::new(),
        }
    }

    fn make_entry(level: &str, target: &str, message: &str) -> LogEntry {
        LogEntry {
            ts: 1000,
            level: level.into(),
            target: target.into(),
            message: message.into(),
            fields: serde_json::Map::new(),
        }
    }

    #[test]
    fn buffer_ring_evicts_oldest() {
        let buf = LogBuffer::new(3);
        for i in 0..5 {
            buf.push(make_entry("INFO", "test", &format!("msg{i}")));
        }
        let all = buf.list(
            &LogFilter {
                level: None,
                target: None,
                search: None,
            },
            100,
        );
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "msg2");
        assert_eq!(all[2].message, "msg4");
    }

    #[test]
    fn filter_by_level() {
        let buf = LogBuffer::default();
        buf.push(make_entry("DEBUG", "a", "debug msg"));
        buf.push(make_entry("INFO", "a", "info msg"));
        buf.push(make_entry("ERROR", "a", "error msg"));

        let result = buf.list(
            &LogFilter {
                level: Some("WARN".into()),
                target: None,
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, "ERROR");
    }

    #[test]
    fn filter_by_target() {
        let buf = LogBuffer::default();
        buf.push(make_entry("INFO", "moltis::gateway", "a"));
        buf.push(make_entry("INFO", "moltis::chat", "b"));

        let result = buf.list(
            &LogFilter {
                level: None,
                target: Some("gateway".into()),
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message, "a");
    }

    #[test]
    fn filter_by_search() {
        let buf = LogBuffer::default();
        buf.push(make_entry("INFO", "a", "hello world"));
        buf.push(make_entry("INFO", "a", "goodbye world"));

        let result = buf.list(
            &LogFilter {
                level: None,
                target: None,
                search: Some("hello".into()),
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message, "hello world");
    }

    #[test]
    fn list_respects_limit() {
        let buf = LogBuffer::default();
        for i in 0..10 {
            buf.push(make_entry("INFO", "a", &format!("msg{i}")));
        }
        let result = buf.list(
            &LogFilter {
                level: None,
                target: None,
                search: None,
            },
            3,
        );
        assert_eq!(result.len(), 3);
        // Should return the latest 3
        assert_eq!(result[0].message, "msg7");
        assert_eq!(result[2].message, "msg9");
    }

    #[test]
    fn broadcast_receiver_gets_entries() {
        let buf = LogBuffer::default();
        let mut rx = buf.subscribe();
        buf.push(make_entry("INFO", "test", "hello"));
        let entry = rx.try_recv().unwrap();
        assert_eq!(entry.message, "hello");
    }

    #[test]
    fn persistence_write_and_read_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        let buf = LogBuffer::default();
        buf.enable_persistence(path.clone());
        buf.push(make_entry("INFO", "test", "persisted1"));
        buf.push(make_entry("WARN", "test", "persisted2"));

        // list_from_file reads directly from disk.
        let entries = buf.list_from_file(
            &LogFilter {
                level: None,
                target: None,
                search: None,
            },
            100,
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "persisted1");
        assert_eq!(entries[1].message, "persisted2");
    }

    #[test]
    fn persistence_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        // Session 1: write entries.
        {
            let buf = LogBuffer::default();
            buf.enable_persistence(path.clone());
            buf.push(make_entry("INFO", "test", "old1"));
            buf.push(make_entry("INFO", "test", "old2"));
        }

        // Session 2: new buffer, same file. In-memory buffer is empty.
        {
            let buf = LogBuffer::default();
            buf.enable_persistence(path.clone());
            buf.push(make_entry("INFO", "test", "new1"));

            // In-memory only sees current session.
            let mem = buf.list(
                &LogFilter {
                    level: None,
                    target: None,
                    search: None,
                },
                100,
            );
            assert_eq!(mem.len(), 1);
            assert_eq!(mem[0].message, "new1");

            // File sees all entries across sessions.
            let file = buf.list_from_file(
                &LogFilter {
                    level: None,
                    target: None,
                    search: None,
                },
                100,
            );
            assert_eq!(file.len(), 3);
            assert_eq!(file[0].message, "old1");
            assert_eq!(file[2].message, "new1");
        }
    }

    #[test]
    fn list_from_file_respects_filter_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        let buf = LogBuffer::default();
        buf.enable_persistence(path);
        for i in 0..10 {
            buf.push(make_entry("INFO", "a", &format!("msg{i}")));
        }
        buf.push(make_entry("ERROR", "a", "err"));

        // Filter by level=ERROR
        let result = buf.list_from_file(
            &LogFilter {
                level: Some("ERROR".into()),
                target: None,
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].message, "err");

        // Limit
        let result = buf.list_from_file(
            &LogFilter {
                level: None,
                target: None,
                search: None,
            },
            3,
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].message, "msg8");
    }

    #[tokio::test]
    async fn live_logs_service_tail() {
        let buf = LogBuffer::default();
        for i in 0..5 {
            buf.push(make_entry("INFO", "test", &format!("msg{i}")));
        }
        let svc = LiveLogsService::new(buf);
        let result = svc.tail(serde_json::json!({ "limit": 3 })).await.unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(result["subscribed"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn live_logs_service_list_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        let buf = LogBuffer::default();
        buf.enable_persistence(path);
        buf.push(make_entry("DEBUG", "gateway", "debug line"));
        buf.push(make_entry("ERROR", "chat", "error line"));
        buf.push(make_entry("INFO", "gateway", "info line"));

        let svc = LiveLogsService::new(buf);
        let result = svc
            .list(serde_json::json!({ "level": "info", "target": "gateway" }))
            .await
            .unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["message"], "info line");
    }

    #[tokio::test]
    async fn live_logs_service_list_falls_back_to_memory() {
        // No persistence — should fall back to in-memory buffer.
        let buf = LogBuffer::default();
        buf.push(make_entry("INFO", "a", "mem only"));

        let svc = LiveLogsService::new(buf);
        let result = svc.list(serde_json::json!({})).await.unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["message"], "mem only");
    }

    #[test]
    fn count_unseen_in_memory() {
        let buf = LogBuffer::default();
        buf.push(make_entry_ts("INFO", "a", "ok", 400));
        buf.push(make_entry_ts("WARN", "a", "w1", 600));
        buf.push(make_entry_ts("ERROR", "a", "e1", 700));

        // Nothing acked yet — all warn/error are unseen.
        let (warns, errors) = buf.count_unseen();
        assert_eq!(warns, 1);
        assert_eq!(errors, 1);

        // Ack, then push more.
        buf.ack_visit();
        buf.push(make_entry_ts("WARN", "a", "w2", 800));
        buf.push(make_entry_ts("WARN", "a", "w3", 900));
        buf.push(make_entry_ts("INFO", "a", "ok2", 1000));

        let (warns, errors) = buf.count_unseen();
        assert_eq!(warns, 2);
        assert_eq!(errors, 0);
    }

    #[test]
    fn count_unseen_seeded_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        // Session 1: push entries and ack.
        {
            let buf = LogBuffer::default();
            buf.enable_persistence(path.clone());
            buf.push(make_entry_ts("WARN", "a", "w1", 100));
            buf.push(make_entry_ts("ERROR", "a", "e1", 200));
            buf.push(make_entry_ts("WARN", "a", "w2", 300));
            buf.ack_visit(); // acked_warns=2, acked_errors=1
        }

        // Session 2: new buffer loads counters from file.
        {
            let buf = LogBuffer::default();
            buf.enable_persistence(path.clone());

            // File has 2 warns, 1 error — all acked.
            let (warns, errors) = buf.count_unseen();
            assert_eq!(warns, 0);
            assert_eq!(errors, 0);

            // Push a new warn.
            buf.push(make_entry_ts("WARN", "a", "w3", 400));
            let (warns, errors) = buf.count_unseen();
            assert_eq!(warns, 1);
            assert_eq!(errors, 0);
        }
    }

    #[test]
    fn ack_visit_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        let buf = LogBuffer::default();
        buf.enable_persistence(path.clone());
        buf.push(make_entry("WARN", "a", "w"));
        buf.push(make_entry("ERROR", "a", "e"));
        buf.ack_visit();

        // Check the file was written.
        let vpath = path.with_extension("visited");
        let contents = std::fs::read_to_string(&vpath).unwrap();
        assert_eq!(contents, "1 1"); // 1 warn, 1 error

        // New buffer loading from same path should pick up the acked counters.
        let buf2 = LogBuffer::default();
        buf2.enable_persistence(path);
        assert_eq!(buf2.acked_warns.load(Ordering::Relaxed), 1);
        assert_eq!(buf2.acked_errors.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn live_logs_service_status_and_ack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");

        let buf = LogBuffer::default();
        buf.enable_persistence(path);

        buf.push(make_entry_ts("ERROR", "a", "e", 100));
        buf.push(make_entry_ts("WARN", "a", "w", 200));

        let svc = LiveLogsService::new(buf);

        // All entries are unseen (nothing acked).
        let status = svc.status().await.unwrap();
        assert_eq!(status["unseen_errors"], 1);
        assert_eq!(status["unseen_warns"], 1);

        // Ack clears them.
        svc.ack().await.unwrap();
        let status = svc.status().await.unwrap();
        assert_eq!(status["unseen_errors"], 0);
        assert_eq!(status["unseen_warns"], 0);
    }

    #[test]
    fn file_path_none_without_persistence() {
        let buf = LogBuffer::default();
        assert!(buf.file_path().is_none());
    }

    #[test]
    fn file_path_returns_path_with_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");
        let buf = LogBuffer::default();
        buf.enable_persistence(path.clone());
        assert_eq!(buf.file_path().unwrap(), path);
    }

    #[test]
    fn log_file_path_via_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.jsonl");
        let buf = LogBuffer::default();
        buf.enable_persistence(path.clone());
        let svc = LiveLogsService::new(buf);
        assert_eq!(svc.log_file_path().unwrap(), path);
    }

    #[test]
    fn log_file_path_none_without_persistence() {
        let buf = LogBuffer::default();
        let svc = LiveLogsService::new(buf);
        assert!(svc.log_file_path().is_none());
    }
}
