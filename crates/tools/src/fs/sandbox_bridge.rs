//! Compatibility wrappers for sandboxed filesystem tools.
//!
//! The first-class sandbox filesystem interface now lives under
//! [`crate::sandbox::file_system`]. This module remains as a thin adapter so
//! existing fs tool code and tests keep a stable import path while the shell
//! transport stays hidden behind a reusable Rust service.

pub use crate::sandbox::file_system::{
    MAX_SANDBOX_WRITE_BYTES, SandboxFileSystem, SandboxGrepMode, SandboxGrepOptions,
    SandboxListFilesResult, SandboxReadResult,
};

#[cfg(test)]
pub(crate) use crate::sandbox::file_system::test_helpers;

use {
    crate::{
        Result,
        sandbox::{
            Sandbox, SandboxId, SandboxRouter,
            file_system::{CommandSandboxFileSystem, sandbox_file_system_for_session},
        },
    },
    std::sync::Arc,
};

/// Prepare a session sandbox and return a session-scoped filesystem service.
pub async fn ensure_sandbox(
    router: &SandboxRouter,
    session_key: &str,
) -> Result<Arc<dyn SandboxFileSystem>> {
    sandbox_file_system_for_session(router, session_key).await
}

/// Compatibility wrapper for direct backend/id reads.
pub async fn sandbox_read(
    backend: &Arc<dyn Sandbox>,
    id: &SandboxId,
    file_path: &str,
    max_bytes: u64,
) -> Result<SandboxReadResult> {
    CommandSandboxFileSystem::new(Arc::clone(backend), id.clone())
        .read_file(file_path, max_bytes)
        .await
}

/// Compatibility wrapper for direct backend/id writes.
pub async fn sandbox_write(
    backend: &Arc<dyn Sandbox>,
    id: &SandboxId,
    file_path: &str,
    content: &[u8],
) -> Result<Option<serde_json::Value>> {
    CommandSandboxFileSystem::new(Arc::clone(backend), id.clone())
        .write_file(file_path, content)
        .await
}

/// Compatibility wrapper for direct backend/id file listing.
pub async fn sandbox_list_files(
    backend: &Arc<dyn Sandbox>,
    id: &SandboxId,
    root: &str,
) -> Result<SandboxListFilesResult> {
    CommandSandboxFileSystem::new(Arc::clone(backend), id.clone())
        .list_files(root)
        .await
}

/// Compatibility wrapper for direct backend/id grep calls.
pub async fn sandbox_grep(
    backend: &Arc<dyn Sandbox>,
    id: &SandboxId,
    opts: SandboxGrepOptions,
) -> Result<serde_json::Value> {
    CommandSandboxFileSystem::new(Arc::clone(backend), id.clone())
        .grep(opts)
        .await
}
