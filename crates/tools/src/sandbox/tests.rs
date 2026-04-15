#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[cfg(target_os = "macos")]
use super::apple::*;
#[cfg(target_os = "macos")]
use crate::sandbox::file_system::{
    oci_container_list_files, oci_container_read_file, oci_container_write_file,
};
#[cfg(target_os = "macos")]
use std::env;
use {
    super::{containers::*, docker::*, host::*, paths::*, platform::*, router::*, types::*, *},
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
        sandbox::file_system::SandboxReadResult,
    },
};

fn clear_host_data_dir_test_state() {
    host_data_dir_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    let overrides = TEST_CONTAINER_MOUNT_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    overrides
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

fn set_test_container_mount_override(cli: &str, reference: &str, mounts: Vec<ContainerMount>) {
    let overrides = TEST_CONTAINER_MOUNT_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()));
    overrides
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(test_container_mount_override_key(cli, reference), mounts);
}

#[cfg(target_os = "macos")]
const OCI_RUNTIME_E2E_ENV: &str = "MOLTIS_SANDBOX_RUNTIME_E2E";
#[cfg(target_os = "macos")]
const OCI_RUNTIME_E2E_IMAGE: &str = "alpine:3.21";

#[cfg(target_os = "macos")]
fn runtime_container_e2e_enabled(cli: &str) -> bool {
    let requested = env::var(OCI_RUNTIME_E2E_ENV)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes")
        })
        .unwrap_or(false);
    if !requested || !is_cli_available(cli) {
        return false;
    }
    std::process::Command::new(cli)
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
struct RuntimeContainerGuard {
    cli: String,
    name: String,
}

#[cfg(target_os = "macos")]
impl RuntimeContainerGuard {
    async fn start(cli: &str) -> Result<Self> {
        let name = format!("moltis-runtime-e2e-{}", uuid::Uuid::new_v4().simple());
        let output = tokio::process::Command::new(cli)
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &name,
                OCI_RUNTIME_E2E_IMAGE,
                "sleep",
                "600",
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::message(format!(
                "{cli} run failed for runtime e2e container '{name}': {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(Self {
            cli: cli.to_string(),
            name,
        })
    }

    async fn exec(&self, command: &str) -> Result<String> {
        let output = tokio::process::Command::new(&self.cli)
            .args(["exec", &self.name, "sh", "-c", command])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::message(format!(
                "{} exec failed in runtime e2e container '{}': {}",
                self.cli,
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(target_os = "macos")]
impl Drop for RuntimeContainerGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.cli)
            .args(["rm", "-f", &self.name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(target_os = "macos")]
async fn assert_runtime_oci_file_transfers(cli: &str) -> Result<()> {
    let container = RuntimeContainerGuard::start(cli).await?;
    container
        .exec(
            "mkdir -p /tmp/moltis-e2e/list && \
             printf 'hello runtime\\n' > /tmp/moltis-e2e/read.txt && \
             printf 'alpha\\n' > /tmp/moltis-e2e/list/a.txt && \
             printf 'beta\\n' > /tmp/moltis-e2e/list/b.txt",
        )
        .await?;

    let read_result =
        oci_container_read_file(cli, &container.name, "/tmp/moltis-e2e/read.txt", 1024).await?;
    match read_result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"hello runtime\n"),
        other => panic!("expected Ok from runtime OCI read, got {other:?}"),
    }

    assert!(
        oci_container_write_file(
            cli,
            &container.name,
            "/tmp/moltis-e2e/write.txt",
            b"written from host"
        )
        .await?
        .is_none()
    );
    let written = container.exec("cat /tmp/moltis-e2e/write.txt").await?;
    assert_eq!(written, "written from host");

    let files = oci_container_list_files(cli, &container.name, "/tmp/moltis-e2e/list").await?;
    assert_eq!(files.files, vec![
        "/tmp/moltis-e2e/list/a.txt".to_string(),
        "/tmp/moltis-e2e/list/b.txt".to_string(),
    ]);
    assert!(!files.truncated);

    Ok(())
}

#[test]
fn test_normalize_cgroup_container_ref() {
    assert_eq!(
        normalize_cgroup_container_ref(
            "docker-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.scope"
        ),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into())
    );
    assert_eq!(
        normalize_cgroup_container_ref(
            "libpod-abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdef.scope"
        ),
        Some("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdef".into())
    );
    assert!(normalize_cgroup_container_ref("user.slice").is_none());
}

#[test]
fn test_parse_container_mounts_from_inspect() {
    let mounts = parse_container_mounts_from_inspect(
        r#"[{
            "Mounts": [
                {
                    "Source": "/host/data",
                    "Destination": "/home/moltis/.moltis"
                },
                {
                    "Source": "/host/config",
                    "Destination": "/home/moltis/.config/moltis"
                }
            ]
        }]"#,
    );
    assert_eq!(mounts, vec![
        ContainerMount {
            source: PathBuf::from("/host/data"),
            destination: PathBuf::from("/home/moltis/.moltis"),
        },
        ContainerMount {
            source: PathBuf::from("/host/config"),
            destination: PathBuf::from("/home/moltis/.config/moltis"),
        },
    ]);
}

#[test]
fn test_resolve_host_path_from_mounts_prefers_longest_prefix() {
    let mounts = vec![
        ContainerMount {
            source: PathBuf::from("/host"),
            destination: PathBuf::from("/home"),
        },
        ContainerMount {
            source: PathBuf::from("/host/data"),
            destination: PathBuf::from("/home/moltis/.moltis"),
        },
    ];
    let resolved = resolve_host_path_from_mounts(
        &PathBuf::from("/home/moltis/.moltis/sandbox/home/shared"),
        &mounts,
    );
    assert_eq!(
        resolved,
        Some(PathBuf::from("/host/data/sandbox/home/shared"))
    );
}

#[test]
fn test_detect_host_data_dir_with_references_uses_mount_overrides() {
    clear_host_data_dir_test_state();
    let guest_data_dir = PathBuf::from("/home/moltis/.moltis");
    set_test_container_mount_override("docker", "parent-container", vec![ContainerMount {
        source: PathBuf::from("/srv/moltis/data"),
        destination: guest_data_dir.clone(),
    }]);

    let detected =
        detect_host_data_dir_with_references("docker", &guest_data_dir, &[String::from(
            "parent-container",
        )]);

    assert_eq!(detected, Some(PathBuf::from("/srv/moltis/data")));
}

#[test]
fn test_detect_host_data_dir_does_not_cache_missing_result() {
    clear_host_data_dir_test_state();
    let guest_data_dir = PathBuf::from("/home/moltis/.moltis");
    assert_eq!(detect_host_data_dir("docker", &guest_data_dir), None);
    let cache_key = format!("docker:{}", guest_data_dir.display());
    assert!(
        !host_data_dir_cache()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&cache_key)
    );

    let reference = String::from("retry-container");

    set_test_container_mount_override("docker", &reference, vec![ContainerMount {
        source: PathBuf::from("/srv/moltis/data"),
        destination: guest_data_dir.clone(),
    }]);

    let detected = detect_host_data_dir_with_references("docker", &guest_data_dir, &[reference]);
    assert_eq!(detected, Some(PathBuf::from("/srv/moltis/data")));
}

#[test]
fn test_ensure_sandbox_home_persistence_host_dir_propagates_guest_visible_create_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let blocking_file = temp_dir.path().join("blocking-file");
    std::fs::write(&blocking_file, "x").unwrap();
    let config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        shared_home_dir: Some(blocking_file.join("nested")),
        ..Default::default()
    };
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };

    let result = ensure_sandbox_home_persistence_host_dir(&config, None, &id);
    assert!(result.is_err());
}

#[test]
fn test_ensure_sandbox_home_persistence_host_dir_allows_translated_create_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let blocking_file = temp_dir.path().join("blocking-file");
    std::fs::write(&blocking_file, "x").unwrap();
    let config = SandboxConfig {
        host_data_dir: Some(blocking_file.join("host")),
        ..Default::default()
    };
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };

    let result = ensure_sandbox_home_persistence_host_dir(&config, Some("docker"), &id)
        .unwrap()
        .unwrap();
    assert_eq!(result, blocking_file.join("host/sandbox/home/shared"));
}

struct TestSandbox {
    name: &'static str,
    ensure_ready_error: Option<String>,
    exec_error: Option<String>,
    ensure_ready_calls: AtomicUsize,
    exec_calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
}

impl TestSandbox {
    fn new(name: &'static str, ensure_ready_error: Option<&str>, exec_error: Option<&str>) -> Self {
        Self {
            name,
            ensure_ready_error: ensure_ready_error.map(ToOwned::to_owned),
            exec_error: exec_error.map(ToOwned::to_owned),
            ensure_ready_calls: AtomicUsize::new(0),
            exec_calls: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
        }
    }

    #[cfg(target_os = "macos")]
    fn ensure_ready_calls(&self) -> usize {
        self.ensure_ready_calls.load(Ordering::SeqCst)
    }

    #[cfg(target_os = "macos")]
    fn exec_calls(&self) -> usize {
        self.exec_calls.load(Ordering::SeqCst)
    }
}

#[test]
fn truncate_output_for_display_handles_multibyte_boundary() {
    let mut output = format!("{}л{}", "a".repeat(1999), "z".repeat(10));
    truncate_output_for_display(&mut output, 2000);
    assert!(output.contains("[output truncated]"));
    assert!(!output.contains('л'));
}

#[async_trait::async_trait]
impl Sandbox for TestSandbox {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        self.ensure_ready_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref msg) = self.ensure_ready_error {
            return Err(Error::message(msg));
        }
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref msg) = self.exec_error {
            return Err(Error::message(msg));
        }
        Ok(ExecResult {
            stdout: "ok".into(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod apple;
mod core;
mod docker_router;
#[cfg(target_os = "linux")]
mod linux;
mod network;
mod platform;
mod restricted_host;
#[cfg(feature = "wasm")]
mod wasm;
