//! In-memory ring buffer for network audit logs.
//!
//! Bounded ring buffer with a `tokio::sync::broadcast` channel for real-time
//! streaming, plus optional JSONL file persistence.

use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

use {serde_json, tokio::sync::broadcast};

use crate::{FilterOutcome, NetworkAuditEntry, NetworkProtocol};

// ── Filter ──────────────────────────────────────────────────────────────────

/// Filter criteria for audit log queries.
pub struct NetworkAuditFilter {
    pub domain: Option<String>,
    pub protocol: Option<NetworkProtocol>,
    pub action: Option<FilterOutcome>,
    pub search: Option<String>,
}

impl NetworkAuditFilter {
    fn matches(&self, entry: &NetworkAuditEntry) -> bool {
        if let Some(ref d) = self.domain
            && !d.is_empty()
            && !entry.domain.to_lowercase().contains(&d.to_lowercase())
        {
            return false;
        }
        if let Some(ref proto) = self.protocol
            && entry.protocol != *proto
        {
            return false;
        }
        if let Some(ref act) = self.action
            && entry.action != *act
        {
            return false;
        }
        if let Some(ref q) = self.search
            && !q.is_empty()
        {
            let q_lower = q.to_lowercase();
            let in_domain = entry.domain.to_lowercase().contains(&q_lower);
            let in_url = entry
                .url
                .as_deref()
                .is_some_and(|u| u.to_lowercase().contains(&q_lower));
            let in_method = entry
                .method
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains(&q_lower));
            let in_session = entry.session.to_lowercase().contains(&q_lower);
            if !in_domain && !in_url && !in_method && !in_session {
                return false;
            }
        }
        true
    }
}

// ── Buffer ──────────────────────────────────────────────────────────────────

const DEFAULT_CAPACITY: usize = 5000;
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct NetworkAuditBuffer {
    buf: Arc<RwLock<VecDeque<NetworkAuditEntry>>>,
    capacity: usize,
    tx: broadcast::Sender<NetworkAuditEntry>,
    writer: Arc<Mutex<Option<File>>>,
    file_path: Arc<RwLock<Option<PathBuf>>>,
}

impl NetworkAuditBuffer {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            buf: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
            tx,
            writer: Arc::new(Mutex::new(None)),
            file_path: Arc::new(RwLock::new(None)),
        }
    }

    /// Enable JSONL file persistence at `path`.
    pub fn enable_persistence(&self, path: PathBuf) {
        if let Ok(mut fp) = self.file_path.write() {
            *fp = Some(path.clone());
        }
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path)
            && let Ok(mut w) = self.writer.lock()
        {
            *w = Some(file);
        }
    }

    /// Push an entry: broadcast, persist, ring-buffer.
    pub fn push(&self, entry: NetworkAuditEntry) {
        let _ = self.tx.send(entry.clone());

        // Persist to file.
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

    pub fn subscribe(&self) -> broadcast::Receiver<NetworkAuditEntry> {
        self.tx.subscribe()
    }

    /// Return the last `limit` matching entries from the in-memory ring buffer.
    pub fn list(&self, filter: &NetworkAuditFilter, limit: usize) -> Vec<NetworkAuditEntry> {
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
    pub fn list_from_file(
        &self,
        filter: &NetworkAuditFilter,
        limit: usize,
    ) -> Vec<NetworkAuditEntry> {
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
        let reader = BufReader::new(file);
        let mut ring = VecDeque::with_capacity(limit);
        for line in reader.lines() {
            let Ok(line) = line else {
                continue;
            };
            if line.is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<NetworkAuditEntry>(&line) else {
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

    /// Compute aggregate stats from the in-memory buffer.
    pub fn stats(&self) -> NetworkAuditStats {
        let buf = match self.buf.read() {
            Ok(b) => b,
            Err(_) => {
                return NetworkAuditStats {
                    total: 0,
                    allowed: 0,
                    denied: 0,
                    by_domain: vec![],
                };
            },
        };
        let mut total = 0u64;
        let mut allowed = 0u64;
        let mut denied = 0u64;
        let mut domain_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for entry in buf.iter() {
            total += 1;
            match entry.action {
                FilterOutcome::Allowed | FilterOutcome::ApprovedByUser => allowed += 1,
                FilterOutcome::Denied | FilterOutcome::Timeout => denied += 1,
            }
            *domain_counts.entry(entry.domain.clone()).or_default() += 1;
        }

        let mut by_domain: Vec<(String, u64)> = domain_counts.into_iter().collect();
        by_domain.sort_by(|a, b| b.1.cmp(&a.1));
        by_domain.truncate(20);

        NetworkAuditStats {
            total,
            allowed,
            denied,
            by_domain,
        }
    }

    pub fn file_path(&self) -> Option<PathBuf> {
        self.file_path.read().ok().and_then(|fp| fp.clone())
    }
}

impl Default for NetworkAuditBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct NetworkAuditStats {
    pub total: u64,
    pub allowed: u64,
    pub denied: u64,
    pub by_domain: Vec<(String, u64)>,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {super::*, crate::ApprovalSource, time::OffsetDateTime};

    fn make_entry(
        domain: &str,
        action: FilterOutcome,
        protocol: NetworkProtocol,
    ) -> NetworkAuditEntry {
        NetworkAuditEntry {
            timestamp: OffsetDateTime::now_utc(),
            session: "127.0.0.1:1234".into(),
            domain: domain.into(),
            port: 443,
            protocol,
            action,
            method: None,
            url: None,
            status: None,
            bytes_sent: 100,
            bytes_received: 200,
            duration_ms: 50,
            error: None,
            approval_source: Some(ApprovalSource::Config),
        }
    }

    #[test]
    fn buffer_ring_evicts_oldest() {
        let buf = NetworkAuditBuffer::new(3);
        for i in 0..5 {
            buf.push(make_entry(
                &format!("domain{i}.com"),
                FilterOutcome::Allowed,
                NetworkProtocol::HttpConnect,
            ));
        }
        let all = buf.list(
            &NetworkAuditFilter {
                domain: None,
                protocol: None,
                action: None,
                search: None,
            },
            100,
        );
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].domain, "domain2.com");
        assert_eq!(all[2].domain, "domain4.com");
    }

    #[test]
    fn filter_by_domain() {
        let buf = NetworkAuditBuffer::default();
        buf.push(make_entry(
            "github.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "evil.com",
            FilterOutcome::Denied,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "api.github.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));

        let result = buf.list(
            &NetworkAuditFilter {
                domain: Some("github".into()),
                protocol: None,
                action: None,
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_by_protocol() {
        let buf = NetworkAuditBuffer::default();
        buf.push(make_entry(
            "a.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "b.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpForward,
        ));

        let result = buf.list(
            &NetworkAuditFilter {
                domain: None,
                protocol: Some(NetworkProtocol::HttpForward),
                action: None,
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].domain, "b.com");
    }

    #[test]
    fn filter_by_action() {
        let buf = NetworkAuditBuffer::default();
        buf.push(make_entry(
            "a.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "b.com",
            FilterOutcome::Denied,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "c.com",
            FilterOutcome::Timeout,
            NetworkProtocol::HttpConnect,
        ));

        let result = buf.list(
            &NetworkAuditFilter {
                domain: None,
                protocol: None,
                action: Some(FilterOutcome::Denied),
                search: None,
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].domain, "b.com");
    }

    #[test]
    fn filter_by_search() {
        let buf = NetworkAuditBuffer::default();
        let mut entry = make_entry(
            "npmjs.org",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpForward,
        );
        entry.method = Some("GET".into());
        entry.url = Some("http://npmjs.org/package/express".into());
        buf.push(entry);
        buf.push(make_entry(
            "github.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));

        let result = buf.list(
            &NetworkAuditFilter {
                domain: None,
                protocol: None,
                action: None,
                search: Some("express".into()),
            },
            100,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].domain, "npmjs.org");
    }

    #[test]
    fn broadcast_receiver_gets_entries() {
        let buf = NetworkAuditBuffer::default();
        let mut rx = buf.subscribe();
        buf.push(make_entry(
            "test.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        let entry = rx.try_recv().unwrap();
        assert_eq!(entry.domain, "test.com");
    }

    #[test]
    fn persistence_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");

        let buf = NetworkAuditBuffer::default();
        buf.enable_persistence(path);
        buf.push(make_entry(
            "a.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "b.com",
            FilterOutcome::Denied,
            NetworkProtocol::HttpConnect,
        ));

        let entries = buf.list_from_file(
            &NetworkAuditFilter {
                domain: None,
                protocol: None,
                action: None,
                search: None,
            },
            100,
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].domain, "a.com");
        assert_eq!(entries[1].domain, "b.com");
    }

    #[test]
    fn stats_calculation() {
        let buf = NetworkAuditBuffer::default();
        buf.push(make_entry(
            "github.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "github.com",
            FilterOutcome::Allowed,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "evil.com",
            FilterOutcome::Denied,
            NetworkProtocol::HttpConnect,
        ));
        buf.push(make_entry(
            "npmjs.org",
            FilterOutcome::ApprovedByUser,
            NetworkProtocol::HttpForward,
        ));

        let stats = buf.stats();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.allowed, 3); // 2 allowed + 1 approved_by_user
        assert_eq!(stats.denied, 1);
        assert_eq!(stats.by_domain[0], ("github.com".into(), 2));
    }

    #[test]
    fn list_respects_limit() {
        let buf = NetworkAuditBuffer::default();
        for i in 0..10 {
            buf.push(make_entry(
                &format!("d{i}.com"),
                FilterOutcome::Allowed,
                NetworkProtocol::HttpConnect,
            ));
        }
        let result = buf.list(
            &NetworkAuditFilter {
                domain: None,
                protocol: None,
                action: None,
                search: None,
            },
            3,
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].domain, "d7.com");
        assert_eq!(result[2].domain, "d9.com");
    }
}
