#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use {
    async_trait::async_trait, moltis_agents::tool_registry::AgentTool,
    moltis_skills::discover::SkillDiscoverer, serde_json::json,
};

use {super::*, crate::checkpoints::CheckpointManager};

/// Re-export for the read test that checks the constant matches the skills crate.
const SIDECAR_SUBDIRS: &[&str] = moltis_skills::SIDECAR_SUBDIRS;

#[path = "crud_write.rs"]
mod crud_write;
#[path = "read.rs"]
mod read;
