//! Import report types — aggregated results of an import operation.

use serde::{Deserialize, Serialize};

use crate::{agents::ImportedAgents, channels::ImportedChannels, identity::ImportedIdentity};

/// A single import category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportCategory {
    Identity,
    Providers,
    Skills,
    Memory,
    Channels,
    Sessions,
    McpServers,
    WorkspaceFiles,
}

impl std::fmt::Display for ImportCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity => write!(f, "Identity"),
            Self::Providers => write!(f, "Providers"),
            Self::Skills => write!(f, "Skills"),
            Self::Memory => write!(f, "Memory"),
            Self::Channels => write!(f, "Channels"),
            Self::Sessions => write!(f, "Sessions"),
            Self::McpServers => write!(f, "MCP Servers"),
            Self::WorkspaceFiles => write!(f, "Workspace Files"),
        }
    }
}

/// Outcome status of a single category import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Success,
    Partial,
    Skipped,
    Failed,
}

/// Report for a single import category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryReport {
    pub category: ImportCategory,
    pub status: ImportStatus,
    pub items_imported: usize,
    #[serde(default)]
    pub items_updated: usize,
    pub items_skipped: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl CategoryReport {
    /// Create a successful report with no warnings.
    pub fn success(category: ImportCategory, items_imported: usize) -> Self {
        Self {
            category,
            status: ImportStatus::Success,
            items_imported,
            items_updated: 0,
            items_skipped: 0,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Create a skipped report (nothing to import for this category).
    pub fn skipped(category: ImportCategory) -> Self {
        Self {
            category,
            status: ImportStatus::Skipped,
            items_imported: 0,
            items_updated: 0,
            items_skipped: 0,
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Create a failed report.
    pub fn failed(category: ImportCategory, error: String) -> Self {
        Self {
            category,
            status: ImportStatus::Failed,
            items_imported: 0,
            items_updated: 0,
            items_skipped: 0,
            warnings: Vec::new(),
            errors: vec![error],
        }
    }
}

/// A deferred/unsupported feature noted during import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub feature: String,
    pub description: String,
}

/// Full import report with all categories and deferred items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub categories: Vec<CategoryReport>,
    pub todos: Vec<TodoItem>,
    /// Identity data extracted during import (if selected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_identity: Option<ImportedIdentity>,
    /// Channel data extracted during import (if selected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_channels: Option<ImportedChannels>,
    /// Agent data extracted during import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_agents: Option<ImportedAgents>,
}

impl ImportReport {
    /// Create an empty report.
    pub fn new() -> Self {
        Self {
            categories: Vec::new(),
            todos: Vec::new(),
            imported_identity: None,
            imported_channels: None,
            imported_agents: None,
        }
    }

    /// Add a category report.
    pub fn add_category(&mut self, report: CategoryReport) {
        self.categories.push(report);
    }

    /// Add a TODO item for an unsupported feature.
    pub fn add_todo(&mut self, feature: impl Into<String>, description: impl Into<String>) {
        self.todos.push(TodoItem {
            feature: feature.into(),
            description: description.into(),
        });
    }

    /// Total items successfully imported across all categories.
    pub fn total_imported(&self) -> usize {
        self.categories.iter().map(|c| c.items_imported).sum()
    }

    /// Whether any category failed.
    pub fn has_failures(&self) -> bool {
        self.categories
            .iter()
            .any(|c| c.status == ImportStatus::Failed)
    }
}

impl Default for ImportReport {
    fn default() -> Self {
        Self::new()
    }
}
