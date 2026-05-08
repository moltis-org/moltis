//! Tool implementations and policy enforcement — sandbox subsystem.
//!
//! Split into submodules by domain for maintainability.

#[cfg(target_os = "macos")]
pub(crate) mod apple;
pub(crate) mod containers;
pub(crate) mod daytona;
pub(crate) mod docker;
pub(crate) mod file_system;
#[cfg(target_os = "linux")]
pub(crate) mod firecracker;
pub(crate) mod host;
pub(crate) mod paths;
pub(crate) mod platform;
pub mod router;
pub(crate) mod sync;
pub(crate) mod types;
#[cfg(feature = "vercel-sandbox")]
pub(crate) mod vercel;
pub(crate) mod wasm;

#[cfg(test)]
mod tests;

// ── Re-exports (preserves the existing public API) ───────────────────────────

#[cfg(target_os = "macos")]
pub use apple::{AppleContainerSandbox, ensure_apple_container_service};
#[cfg(target_os = "linux")]
pub use firecracker::{
    FirecrackerSandbox, FirecrackerSandboxConfig, firecracker_bin_available,
    resolve_firecracker_bin,
};
#[cfg(target_os = "linux")]
pub use platform::CgroupSandbox;
#[cfg(feature = "vercel-sandbox")]
pub use vercel::{VercelSandbox, VercelSandboxConfig};
#[cfg(feature = "wasm")]
pub use wasm::WasmSandbox;
pub use {
    containers::{
        ContainerBackend, ContainerDiskUsage, ContainerRunState, RunningContainer, SandboxImage,
        clean_all_containers, clean_sandbox_images, container_cli, container_disk_usage,
        is_cli_available, list_running_containers, list_sandbox_images, remove_container,
        remove_sandbox_image, restart_container_daemon, sandbox_image_tag, stop_container,
    },
    daytona::{DaytonaSandbox, DaytonaSandboxConfig},
    docker::{DockerSandbox, NoSandbox},
    host::{HostProvisionResult, is_debian_host, provision_host_packages},
    paths::shared_home_dir_path,
    platform::{RestrictedHostSandbox, is_wasm_sandbox_available},
    router::{FailoverSandbox, SandboxEvent, SandboxRouter, auto_detect_backend, create_sandbox},
    types::{
        BuildImageResult, DEFAULT_SANDBOX_IMAGE, HomePersistence, NetworkPolicy, ResourceLimits,
        Sandbox, SandboxBackendId, SandboxConfig, SandboxId, SandboxMode, SandboxScope,
        WorkspaceMount,
    },
};
