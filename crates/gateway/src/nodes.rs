use std::{collections::HashMap, time::Instant};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("node not found")]
    NodeNotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A provider discovered on a remote node.
#[derive(Debug, Clone)]
pub struct NodeProviderEntry {
    pub provider: String,
    pub models: Vec<String>,
}

/// A connected device node (macOS, iOS, Android).
#[derive(Debug, Clone)]
pub struct NodeSession {
    pub node_id: String,
    pub conn_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub commands: Vec<String>,
    pub permissions: HashMap<String, bool>,
    pub path_env: Option<String>,
    pub remote_ip: Option<String>,
    pub connected_at: Instant,
    // ── Telemetry fields (updated by node.telemetry events) ──────────
    pub mem_total: Option<u64>,
    pub mem_available: Option<u64>,
    pub cpu_count: Option<u32>,
    pub cpu_usage: Option<f32>,
    pub uptime_secs: Option<u64>,
    pub services: Vec<String>,
    pub last_telemetry: Option<Instant>,
    // ── Extended telemetry (P1) ──────────────────────────────────────
    pub disk_total: Option<u64>,
    pub disk_available: Option<u64>,
    pub runtimes: Vec<String>,
    // ── Provider discovery (P1) ─────────────────────────────────────
    pub providers: Vec<NodeProviderEntry>,
}

/// Registry of connected device nodes and their capabilities.
pub struct NodeRegistry {
    /// node_id → NodeSession
    nodes: HashMap<String, NodeSession>,
    /// conn_id → node_id (reverse lookup for cleanup on disconnect)
    by_conn: HashMap<String, String>,
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            by_conn: HashMap::new(),
        }
    }

    pub fn register(&mut self, session: NodeSession) {
        self.by_conn
            .insert(session.conn_id.clone(), session.node_id.clone());
        self.nodes.insert(session.node_id.clone(), session);
    }

    pub fn unregister_by_conn(&mut self, conn_id: &str) -> Option<NodeSession> {
        let node_id = self.by_conn.remove(conn_id)?;
        self.nodes.remove(&node_id)
    }

    pub fn get(&self, node_id: &str) -> Option<&NodeSession> {
        self.nodes.get(node_id)
    }

    pub fn get_mut(&mut self, node_id: &str) -> Option<&mut NodeSession> {
        self.nodes.get_mut(node_id)
    }

    pub fn list(&self) -> Vec<&NodeSession> {
        self.nodes.values().collect()
    }

    pub fn has_mobile_node(&self) -> bool {
        self.nodes
            .values()
            .any(|n| n.platform == "ios" || n.platform == "android")
    }

    pub fn rename(&mut self, node_id: &str, display_name: &str) -> Result<()> {
        let node = self.nodes.get_mut(node_id).ok_or(Error::NodeNotFound)?;
        node.display_name = Some(display_name.to_string());
        Ok(())
    }

    /// Update telemetry data for a node.
    pub fn update_telemetry(
        &mut self,
        node_id: &str,
        mem_total: Option<u64>,
        mem_available: Option<u64>,
        cpu_count: Option<u32>,
        cpu_usage: Option<f32>,
        uptime_secs: Option<u64>,
        services: Vec<String>,
        disk_total: Option<u64>,
        disk_available: Option<u64>,
        runtimes: Vec<String>,
    ) -> Result<()> {
        let node = self.nodes.get_mut(node_id).ok_or(Error::NodeNotFound)?;
        node.mem_total = mem_total;
        node.mem_available = mem_available;
        node.cpu_count = cpu_count;
        node.cpu_usage = cpu_usage;
        node.uptime_secs = uptime_secs;
        node.services = services;
        node.disk_total = disk_total;
        node.disk_available = disk_available;
        node.runtimes = runtimes;
        node.last_telemetry = Some(Instant::now());
        Ok(())
    }

    /// Remove all nodes (used when disconnecting all clients).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.by_conn.clear();
    }

    pub fn count(&self) -> usize {
        self.nodes.len()
    }
}
