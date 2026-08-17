//! Process-local coordination for Apple Container CLI operations.
//!
//! The Apple Container service can drop XPC connections when multiple heavy
//! operations (image pulls/builds) run concurrently. Keep those operations
//! serialized across Moltis crates.

#[cfg(target_os = "macos")]
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

#[cfg(target_os = "macos")]
pub struct AppleContainerOperationState {
    _serialized_operations: (),
}

#[cfg(target_os = "macos")]
static APPLE_CONTAINER_OPERATION_LOCK: Mutex<AppleContainerOperationState> =
    Mutex::new(AppleContainerOperationState {
        _serialized_operations: (),
    });
#[cfg(target_os = "macos")]
static APPLE_CONTAINER_UNHEALTHY: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
pub fn apple_container_operation_lock() -> MutexGuard<'static, AppleContainerOperationState> {
    APPLE_CONTAINER_OPERATION_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn apple_container_marked_unhealthy() -> bool {
    APPLE_CONTAINER_UNHEALTHY.load(Ordering::Acquire)
}

#[cfg(target_os = "macos")]
pub fn mark_apple_container_unhealthy() {
    APPLE_CONTAINER_UNHEALTHY.store(true, Ordering::Release);
}
