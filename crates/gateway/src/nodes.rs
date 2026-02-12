use std::{collections::HashMap, time::Instant};

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

    pub fn list(&self) -> Vec<&NodeSession> {
        self.nodes.values().collect()
    }

    pub fn has_mobile_node(&self) -> bool {
        self.nodes
            .values()
            .any(|n| n.platform == "ios" || n.platform == "android")
    }

    pub fn rename(&mut self, node_id: &str, display_name: &str) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| "node not found".to_string())?;
        node.display_name = Some(display_name.to_string());
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
