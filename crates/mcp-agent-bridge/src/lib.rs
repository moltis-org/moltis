mod adapter;
pub mod error;

pub use {
    adapter::{McpToolAdapter, sync_mcp_tools},
    error::Error,
};
