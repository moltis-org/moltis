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
use {
    super::{containers::*, docker::*, host::*, paths::*, platform::*, router::*, types::*, *},
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
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

    fn ensure_ready_calls(&self) -> usize {
        self.ensure_ready_calls.load(Ordering::SeqCst)
    }

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

#[test]
fn test_sandbox_mode_display() {
    assert_eq!(SandboxMode::Off.to_string(), "off");
    assert_eq!(SandboxMode::NonMain.to_string(), "non-main");
    assert_eq!(SandboxMode::All.to_string(), "all");
}

#[test]
fn test_sandbox_scope_display() {
    assert_eq!(SandboxScope::Session.to_string(), "session");
    assert_eq!(SandboxScope::Agent.to_string(), "agent");
    assert_eq!(SandboxScope::Shared.to_string(), "shared");
}

#[test]
fn test_docker_hardening_args_prebuilt() {
    let args = DockerSandbox::hardening_args(true);
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    assert!(args.contains(&"--read-only".to_string()));
    // Verify tmpfs mounts are present
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    assert!(args.contains(&"/run:rw,nosuid,size=64m".to_string()));
}

#[test]
fn test_docker_hardening_args_not_prebuilt() {
    let args = DockerSandbox::hardening_args(false);
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    // --read-only must NOT be present for non-prebuilt (needs apt-get)
    assert!(!args.contains(&"--read-only".to_string()));
    // tmpfs mounts still present
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
}

#[test]
fn test_workspace_mount_display() {
    assert_eq!(WorkspaceMount::None.to_string(), "none");
    assert_eq!(WorkspaceMount::Ro.to_string(), "ro");
    assert_eq!(WorkspaceMount::Rw.to_string(), "rw");
}

#[test]
fn test_home_persistence_display() {
    assert_eq!(HomePersistence::Off.to_string(), "off");
    assert_eq!(HomePersistence::Session.to_string(), "session");
    assert_eq!(HomePersistence::Shared.to_string(), "shared");
}

#[test]
fn test_resource_limits_default() {
    let limits = ResourceLimits::default();
    assert!(limits.memory_limit.is_none());
    assert!(limits.cpu_quota.is_none());
    assert!(limits.pids_max.is_none());
}

#[test]
fn test_resource_limits_serde() {
    let json = r#"{"memory_limit":"512M","cpu_quota":1.5,"pids_max":100}"#;
    let limits: ResourceLimits = serde_json::from_str(json).unwrap();
    assert_eq!(limits.memory_limit.as_deref(), Some("512M"));
    assert_eq!(limits.cpu_quota, Some(1.5));
    assert_eq!(limits.pids_max, Some(100));
}

#[test]
fn test_sandbox_config_serde() {
    let json = r#"{
        "mode": "all",
        "scope": "session",
        "workspace_mount": "rw",
        "no_network": true,
        "resource_limits": {"memory_limit": "1G"}
    }"#;
    let config: SandboxConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.mode, SandboxMode::All);
    assert_eq!(config.workspace_mount, WorkspaceMount::Rw);
    assert!(config.no_network);
    assert_eq!(config.resource_limits.memory_limit.as_deref(), Some("1G"));
}

#[test]
fn test_docker_resource_args() {
    let config = SandboxConfig {
        resource_limits: ResourceLimits {
            memory_limit: Some("256M".into()),
            cpu_quota: Some(0.5),
            pids_max: Some(50),
        },
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.resource_args();
    assert_eq!(args, vec![
        "--memory",
        "256M",
        "--cpus",
        "0.5",
        "--pids-limit",
        "50"
    ]);
}

#[test]
fn test_docker_workspace_args_ro() {
    let config = SandboxConfig {
        workspace_mount: WorkspaceMount::Ro,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.workspace_args();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let workspace_dir = moltis_config::data_dir();
    let expected_volume = format!("{}:{}:ro", workspace_dir.display(), workspace_dir.display());
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_workspace_args_uses_host_data_dir_override() {
    let config = SandboxConfig {
        workspace_mount: WorkspaceMount::Ro,
        host_data_dir: Some(PathBuf::from("/host/moltis-data")),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.workspace_args();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let guest_workspace_dir = moltis_config::data_dir();
    let expected_volume = format!("/host/moltis-data:{}:ro", guest_workspace_dir.display());
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_workspace_args_none() {
    let config = SandboxConfig {
        workspace_mount: WorkspaceMount::None,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.workspace_args().is_empty());
}

#[test]
fn test_docker_home_persistence_args_off() {
    let config = SandboxConfig {
        home_persistence: HomePersistence::Off,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    assert!(docker.home_persistence_args(&id).unwrap().is_empty());
}

#[test]
fn test_docker_home_persistence_args_default_shared() {
    let config = SandboxConfig::default();
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_host_dir = moltis_config::data_dir()
        .join("sandbox")
        .join("home")
        .join("shared");
    let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_default_shared_uses_host_data_dir_override() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_volume = format!(
        "{}:/home/sandbox:rw",
        host_data_dir.join("sandbox/home/shared").display()
    );
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_custom_shared_absolute_path() {
    let config = SandboxConfig {
        shared_home_dir: Some(PathBuf::from("/tmp/moltis-shared-home")),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_volume = "/tmp/moltis-shared-home:/home/sandbox:rw".to_string();
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_custom_shared_relative_path() {
    let config = SandboxConfig {
        shared_home_dir: Some(PathBuf::from("sandbox/custom-shared")),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_host_dir = moltis_config::data_dir().join("sandbox/custom-shared");
    let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_custom_shared_guest_absolute_path_uses_host_override() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        host_data_dir: Some(host_data_dir.clone()),
        shared_home_dir: Some(moltis_config::data_dir().join("sandbox/custom-shared")),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_volume = format!(
        "{}:/home/sandbox:rw",
        host_data_dir.join("sandbox/custom-shared").display()
    );
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_session() {
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess:/weird key".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_host_dir = moltis_config::data_dir()
        .join("sandbox")
        .join("home")
        .join("session")
        .join("sess--weird-key");
    let expected_volume = format!("{}:/home/sandbox:rw", expected_host_dir.display());
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_docker_home_persistence_args_session_uses_host_data_dir_override() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess:/weird key".into(),
    };
    let args = docker.home_persistence_args(&id).unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    let expected_volume = format!(
        "{}:/home/sandbox:rw",
        host_data_dir
            .join("sandbox/home/session/sess--weird-key")
            .display()
    );
    assert_eq!(args[1], expected_volume);
}

#[test]
fn test_create_sandbox_off_uses_no_sandbox() {
    let config = SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    };
    let sandbox = create_sandbox(config);
    assert_eq!(sandbox.backend_name(), "none");
    assert!(!sandbox.is_real());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test".into(),
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sandbox.ensure_ready(&id, None).await.unwrap();
        sandbox.cleanup(&id).await.unwrap();
    });
}

#[tokio::test]
async fn test_no_sandbox_exec() {
    let sandbox = NoSandbox;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test".into(),
    };
    let opts = ExecOpts::default();
    let result = sandbox.exec(&id, "echo sandbox-test", &opts).await.unwrap();
    assert_eq!(result.stdout.trim(), "sandbox-test");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn test_docker_container_name() {
    let config = SandboxConfig {
        container_prefix: Some("my-prefix".into()),
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "abc123".into(),
    };
    assert_eq!(docker.container_name(&id), "my-prefix-abc123");
}

/// Helper: build a `SandboxRouter` with a deterministic backend so tests
/// don't depend on the host having Docker / Apple Container installed.
fn router_with_real_backend(config: SandboxConfig) -> SandboxRouter {
    let backend: Arc<dyn Sandbox> = Arc::new(TestSandbox::new("docker", None, None));
    SandboxRouter::with_backend(config, backend)
}

#[tokio::test]
async fn test_sandbox_router_default_all() {
    let config = SandboxConfig::default(); // mode = All
    let router = router_with_real_backend(config);
    assert!(router.is_sandboxed("main").await);
    assert!(router.is_sandboxed("session:abc").await);
}

#[tokio::test]
async fn test_sandbox_router_mode_off() {
    let config = SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    };
    let router = router_with_real_backend(config);
    assert!(!router.is_sandboxed("main").await);
    assert!(!router.is_sandboxed("session:abc").await);
}

#[tokio::test]
async fn test_sandbox_router_mode_all() {
    let config = SandboxConfig {
        mode: SandboxMode::All,
        ..Default::default()
    };
    let router = router_with_real_backend(config);
    assert!(router.is_sandboxed("main").await);
    assert!(router.is_sandboxed("session:abc").await);
}

#[tokio::test]
async fn test_sandbox_router_mode_non_main() {
    let config = SandboxConfig {
        mode: SandboxMode::NonMain,
        ..Default::default()
    };
    let router = router_with_real_backend(config);
    assert!(!router.is_sandboxed("main").await);
    assert!(router.is_sandboxed("session:abc").await);
}

#[tokio::test]
async fn test_sandbox_router_override() {
    let config = SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    };
    let router = router_with_real_backend(config);
    assert!(!router.is_sandboxed("session:abc").await);

    router.set_override("session:abc", true).await;
    assert!(router.is_sandboxed("session:abc").await);

    router.set_override("session:abc", false).await;
    assert!(!router.is_sandboxed("session:abc").await);

    router.remove_override("session:abc").await;
    assert!(!router.is_sandboxed("session:abc").await);
}

#[tokio::test]
async fn test_sandbox_router_override_overrides_mode() {
    let config = SandboxConfig {
        mode: SandboxMode::All,
        ..Default::default()
    };
    let router = router_with_real_backend(config);
    assert!(router.is_sandboxed("main").await);

    // Override to disable sandbox for main
    router.set_override("main", false).await;
    assert!(!router.is_sandboxed("main").await);
}

#[tokio::test]
async fn test_sandbox_router_no_runtime_returns_false() {
    let backend: Arc<dyn Sandbox> = Arc::new(NoSandbox);
    let config = SandboxConfig {
        mode: SandboxMode::All,
        ..Default::default()
    };
    let router = SandboxRouter::with_backend(config, backend);

    // Even with mode=All, no runtime means not sandboxed
    assert!(!router.is_sandboxed("main").await);
    assert!(!router.is_sandboxed("session:abc").await);

    // Overrides are also ignored when there's no runtime
    router.set_override("main", true).await;
    assert!(!router.is_sandboxed("main").await);
}

#[test]
fn test_backend_name_docker() {
    let sandbox = DockerSandbox::new(SandboxConfig::default());
    assert_eq!(sandbox.backend_name(), "docker");
}

#[test]
fn test_backend_name_podman() {
    let sandbox = DockerSandbox::podman(SandboxConfig::default());
    assert_eq!(sandbox.backend_name(), "podman");
}

#[test]
fn test_backend_name_none() {
    let sandbox = NoSandbox;
    assert_eq!(sandbox.backend_name(), "none");
}

#[test]
fn test_sandbox_router_backend_name() {
    // With "auto", the backend depends on what's available on the host.
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    let name = router.backend_name();
    assert!(
        name == "docker"
            || name == "podman"
            || name == "apple-container"
            || name == "restricted-host",
        "unexpected backend: {name}"
    );
}

#[test]
fn test_sandbox_router_explicit_docker_backend() {
    let config = SandboxConfig {
        backend: "docker".into(),
        ..Default::default()
    };
    let router = SandboxRouter::new(config);
    assert_eq!(router.backend_name(), "docker");
}

#[test]
fn test_sandbox_router_config_accessor() {
    let config = SandboxConfig {
        mode: SandboxMode::NonMain,
        scope: SandboxScope::Agent,
        image: Some("alpine:latest".into()),
        ..Default::default()
    };
    let router = SandboxRouter::new(config);
    assert_eq!(*router.mode(), SandboxMode::NonMain);
    assert_eq!(router.config().scope, SandboxScope::Agent);
    assert_eq!(router.config().image.as_deref(), Some("alpine:latest"));
}

#[test]
fn test_sandbox_router_sandbox_id_for() {
    let config = SandboxConfig {
        scope: SandboxScope::Session,
        ..Default::default()
    };
    let router = SandboxRouter::new(config);
    let id = router.sandbox_id_for("session:abc");
    assert_eq!(id.key, "session-abc");
    // Plain alphanumeric keys pass through unchanged.
    let id2 = router.sandbox_id_for("main");
    assert_eq!(id2.key, "main");
}

#[tokio::test]
async fn test_resolve_image_default() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    let img = router.resolve_image("main", None).await;
    assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
}

#[tokio::test]
async fn test_resolve_image_skill_override() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    let img = router
        .resolve_image("main", Some("moltis-cache/my-skill:abc123"))
        .await;
    assert_eq!(img, "moltis-cache/my-skill:abc123");
}

#[tokio::test]
async fn test_resolve_image_session_override() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    router
        .set_image_override("sess1", "custom:latest".into())
        .await;
    let img = router.resolve_image("sess1", None).await;
    assert_eq!(img, "custom:latest");
}

#[tokio::test]
async fn test_resolve_image_skill_beats_session() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    router
        .set_image_override("sess1", "custom:latest".into())
        .await;
    let img = router
        .resolve_image("sess1", Some("moltis-cache/skill:hash"))
        .await;
    assert_eq!(img, "moltis-cache/skill:hash");
}

#[tokio::test]
async fn test_resolve_image_config_override() {
    let config = SandboxConfig {
        image: Some("my-org/image:v1".into()),
        ..Default::default()
    };
    let router = SandboxRouter::new(config);
    let img = router.resolve_image("main", None).await;
    assert_eq!(img, "my-org/image:v1");
}

#[tokio::test]
async fn test_remove_image_override() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    router
        .set_image_override("sess1", "custom:latest".into())
        .await;
    router.remove_image_override("sess1").await;
    let img = router.resolve_image("sess1", None).await;
    assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
}

#[test]
fn test_docker_image_tag_deterministic() {
    let packages = vec!["curl".into(), "git".into(), "wget".into()];
    let tag1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
    let tag2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
    assert_eq!(tag1, tag2);
    assert!(tag1.starts_with("moltis-main-sandbox:"));
}

#[test]
fn test_docker_image_tag_order_independent() {
    let p1 = vec!["curl".into(), "git".into()];
    let p2 = vec!["git".into(), "curl".into()];
    assert_eq!(
        sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1),
        sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2),
    );
}

#[test]
fn test_docker_image_tag_normalizes_whitespace_and_duplicates() {
    let p1 = vec!["curl".into(), "git".into(), "curl".into()];
    let p2 = vec![" git ".into(), "curl".into()];
    assert_eq!(
        sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1),
        sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2),
    );
}

#[test]
fn test_sandbox_image_dockerfile_creates_home_in_install_layer() {
    let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into()]);
    assert!(dockerfile.contains(
        "RUN apt-get update -qq && apt-get install -y -qq curl && mkdir -p /home/sandbox"
    ));
    assert!(!dockerfile.contains("RUN mkdir -p /home/sandbox\n"));
}

#[test]
fn test_sandbox_image_dockerfile_installs_gogcli() {
    let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into()]);
    assert!(dockerfile.contains(&format!("go install {GOGCLI_MODULE_PATH}@{GOGCLI_VERSION}")));
    assert!(dockerfile.contains("ln -sf /usr/local/bin/gog /usr/local/bin/gogcli"));
}

#[test]
fn test_docker_image_tag_changes_with_base() {
    let packages = vec!["curl".into()];
    let t1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &packages);
    let t2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:24.04", &packages);
    assert_ne!(t1, t2);
}

#[test]
fn test_docker_image_tag_changes_with_packages() {
    let p1 = vec!["curl".into()];
    let p2 = vec!["curl".into(), "git".into()];
    let t1 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p1);
    let t2 = sandbox_image_tag("moltis-main-sandbox", "ubuntu:25.10", &p2);
    assert_ne!(t1, t2);
}

#[test]
fn test_rebuildable_sandbox_image_tag_requires_packages() {
    let tag = rebuildable_sandbox_image_tag(
        "moltis-main-sandbox:deadbeef",
        "moltis-main-sandbox",
        "ubuntu:25.10",
        &[],
    );
    assert!(tag.is_none());
}

#[test]
fn test_rebuildable_sandbox_image_tag_requires_local_repo_prefix() {
    let tag =
        rebuildable_sandbox_image_tag("ubuntu:25.10", "moltis-main-sandbox", "ubuntu:25.10", &[
            "curl".into(),
        ]);
    assert!(tag.is_none());
}

#[test]
fn test_rebuildable_sandbox_image_tag_returns_deterministic_tag() {
    let packages = vec!["curl".into(), "git".into()];
    let tag = rebuildable_sandbox_image_tag(
        "moltis-main-sandbox:oldtag",
        "moltis-main-sandbox",
        "ubuntu:25.10",
        &packages,
    );
    assert_eq!(
        tag,
        Some(sandbox_image_tag(
            "moltis-main-sandbox",
            "ubuntu:25.10",
            &packages
        ))
    );
}

#[tokio::test]
async fn test_no_sandbox_build_image_is_noop() {
    let sandbox = NoSandbox;
    let result = sandbox
        .build_image("ubuntu:25.10", &["curl".into()])
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_sandbox_router_events() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);
    let mut rx = router.subscribe_events();

    router.emit_event(SandboxEvent::Provisioning {
        container: "test".into(),
        packages: vec!["curl".into()],
    });

    let event = rx.try_recv().unwrap();
    match event {
        SandboxEvent::Provisioning {
            container,
            packages,
        } => {
            assert_eq!(container, "test");
            assert_eq!(packages, vec!["curl".to_string()]);
        },
        _ => panic!("unexpected event variant"),
    }

    assert!(router.mark_preparing_once("main").await);
    assert!(!router.mark_preparing_once("main").await);
    router.clear_prepared_session("main").await;
    assert!(router.mark_preparing_once("main").await);
}

#[tokio::test]
async fn test_sandbox_router_global_image_override() {
    let config = SandboxConfig::default();
    let router = SandboxRouter::new(config);

    // Default
    let img = router.default_image().await;
    assert_eq!(img, DEFAULT_SANDBOX_IMAGE);

    // Set global override
    router
        .set_global_image(Some("moltis-sandbox:abc123".into()))
        .await;
    let img = router.default_image().await;
    assert_eq!(img, "moltis-sandbox:abc123");

    // Global override flows through resolve_image
    let img = router.resolve_image("main", None).await;
    assert_eq!(img, "moltis-sandbox:abc123");

    // Session override still wins
    router.set_image_override("main", "custom:v1".into()).await;
    let img = router.resolve_image("main", None).await;
    assert_eq!(img, "custom:v1");

    // Clear and revert
    router.set_global_image(None).await;
    router.remove_image_override("main").await;
    let img = router.default_image().await;
    assert_eq!(img, DEFAULT_SANDBOX_IMAGE);
}

#[cfg(target_os = "macos")]
#[test]
fn test_backend_name_apple_container() {
    let sandbox = AppleContainerSandbox::new(SandboxConfig::default());
    assert_eq!(sandbox.backend_name(), "apple-container");
}

#[cfg(target_os = "macos")]
#[test]
fn test_sandbox_router_explicit_apple_container_backend() {
    let config = SandboxConfig {
        backend: "apple-container".into(),
        ..Default::default()
    };
    let router = SandboxRouter::new(config);
    assert_eq!(router.backend_name(), "apple-container");
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_apple_container_name_generation_rotation() {
    let sandbox = AppleContainerSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-abc".into(),
    };

    let first_name = sandbox.container_name(&id).await;
    assert_eq!(first_name, "moltis-sandbox-session-abc");

    let rotated_name = sandbox.bump_container_generation(&id).await;
    assert_eq!(rotated_name, "moltis-sandbox-session-abc-g1");

    let current_name = sandbox.container_name(&id).await;
    assert_eq!(current_name, "moltis-sandbox-session-abc-g1");
}

/// When both Docker and Apple Container are available, test that we can
/// explicitly select each one.
#[test]
fn test_select_backend_explicit_choices() {
    // Docker backend
    if is_cli_available("docker") {
        let config = SandboxConfig {
            backend: "docker".into(),
            ..Default::default()
        };
        let backend = select_backend(config);
        assert_eq!(backend.backend_name(), "docker");
    }

    // Podman backend
    if is_cli_available("podman") {
        let config = SandboxConfig {
            backend: "podman".into(),
            ..Default::default()
        };
        let backend = select_backend(config);
        assert_eq!(backend.backend_name(), "podman");
    }

    // Apple Container backend (macOS only)
    #[cfg(target_os = "macos")]
    if is_cli_available("container") {
        let config = SandboxConfig {
            backend: "apple-container".into(),
            ..Default::default()
        };
        let backend = select_backend(config);
        assert_eq!(backend.backend_name(), "apple-container");
    }
}

#[test]
fn test_is_apple_container_service_error() {
    assert!(is_apple_container_service_error(
        "Error: internalError: \"XPC connection error\""
    ));
    assert!(is_apple_container_service_error(
        "Error: Connection invalid while contacting service"
    ));
    assert!(!is_apple_container_service_error(
        "Error: something else happened"
    ));
}

#[test]
fn test_is_apple_container_exists_error() {
    assert!(is_apple_container_exists_error(
        "Error: exists: \"container with id moltis-sandbox-main already exists\""
    ));
    assert!(is_apple_container_exists_error(
        "Error: container already exists"
    ));
    assert!(!is_apple_container_exists_error("Error: no such container"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_is_apple_container_unavailable_error() {
    assert!(is_apple_container_unavailable_error(
        "cannot exec: container is not running"
    ));
    assert!(is_apple_container_unavailable_error(
        "invalidState: \"container xyz is not running\""
    ));
    assert!(is_apple_container_unavailable_error(
        "invalidState: \"no sandbox client exists: container is stopped\""
    ));
    // notFound errors from get/inspect failures
    assert!(is_apple_container_unavailable_error(
        "Error: notFound: \"get failed: container moltis-sandbox-main not found\""
    ));
    assert!(is_apple_container_unavailable_error(
        "container not found: moltis-sandbox-session-abc"
    ));
    assert!(!is_apple_container_unavailable_error("permission denied"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_should_restart_after_readiness_error() {
    assert!(should_restart_after_readiness_error(
        "cannot exec: container is not running",
        ContainerState::Stopped
    ));
    assert!(!should_restart_after_readiness_error(
        "cannot exec: container is not running",
        ContainerState::Running
    ));
    assert!(!should_restart_after_readiness_error(
        "permission denied",
        ContainerState::Stopped
    ));
}

#[test]
fn test_apple_container_bootstrap_command_uses_portable_sleep() {
    let command = apple_container_bootstrap_command();
    assert!(command.contains("mkdir -p /home/sandbox"));
    assert!(command.contains("command -v gnusleep >/dev/null 2>&1"));
    assert!(command.contains("exec gnusleep infinity"));
    assert!(command.contains("exec sleep 2147483647"));
    assert!(!command.contains("exec sleep infinity"));
}

#[test]
fn test_apple_container_run_args_pin_workdir_and_bootstrap_home() {
    let args = apple_container_run_args("moltis-sandbox-test", "ubuntu:25.10", Some("UTC"), None);
    let expected = vec![
        "run",
        "-d",
        "--name",
        "moltis-sandbox-test",
        "--workdir",
        "/tmp",
        "-e",
        "TZ=UTC",
        "ubuntu:25.10",
        "sh",
        "-c",
        "mkdir -p /home/sandbox && if command -v gnusleep >/dev/null 2>&1; then exec gnusleep infinity; else exec sleep 2147483647; fi",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    assert_eq!(args, expected);
}

#[test]
fn test_apple_container_run_args_with_home_volume() {
    let args = apple_container_run_args(
        "moltis-sandbox-test",
        "ubuntu:25.10",
        Some("UTC"),
        Some("/tmp/home:/home/sandbox"),
    );
    let expected = vec![
        "run",
        "-d",
        "--name",
        "moltis-sandbox-test",
        "--workdir",
        "/tmp",
        "-e",
        "TZ=UTC",
        "--volume",
        "/tmp/home:/home/sandbox",
        "ubuntu:25.10",
        "sh",
        "-c",
        "mkdir -p /home/sandbox && if command -v gnusleep >/dev/null 2>&1; then exec gnusleep infinity; else exec sleep 2147483647; fi",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    assert_eq!(args, expected);
}

#[test]
fn test_apple_container_exec_args_pin_workdir_and_bootstrap_home() {
    let args = apple_container_exec_args("moltis-sandbox-test", "true".to_string());
    let expected = vec![
        "exec",
        "--workdir",
        "/tmp",
        "moltis-sandbox-test",
        "sh",
        "-c",
        "mkdir -p /home/sandbox && true",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    assert_eq!(args, expected);
}

#[test]
fn test_container_exec_shell_args_apple_container_uses_safe_wrapper() {
    let args = container_exec_shell_args("container", "moltis-sandbox-test", "echo hi".into());
    let expected = vec![
        "exec",
        "--workdir",
        "/tmp",
        "moltis-sandbox-test",
        "sh",
        "-c",
        "mkdir -p /home/sandbox && echo hi",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    assert_eq!(args, expected);
}

#[test]
fn test_container_exec_shell_args_docker_keeps_standard_exec_shape() {
    let args = container_exec_shell_args("docker", "moltis-sandbox-test", "echo hi".into());
    let expected = vec!["exec", "moltis-sandbox-test", "sh", "-c", "echo hi"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(args, expected);
}

#[test]
fn test_apple_container_status_from_inspect() {
    assert_eq!(
        apple_container_status_from_inspect(
            r#"[{"id":"abc","status":"running","configuration":{}}]"#
        ),
        Some("running")
    );
    assert_eq!(
        apple_container_status_from_inspect(r#"[{"id":"abc","status":"stopped"}]"#),
        Some("stopped")
    );
    assert_eq!(apple_container_status_from_inspect("[]"), None);
    assert_eq!(apple_container_status_from_inspect(""), None);
}

#[test]
fn test_is_apple_container_daemon_stale_error() {
    // Full EINVAL pattern from container logs
    assert!(is_apple_container_daemon_stale_error(
        "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\""
    ));
    // Both patterns required — neither alone should match
    assert!(!is_apple_container_daemon_stale_error(
        "NSPOSIXErrorDomain Code=22"
    ));
    assert!(!is_apple_container_daemon_stale_error("Invalid argument"));
    // Log-fetching errors with NSPOSIXErrorDomain Code=2 must NOT match
    assert!(!is_apple_container_daemon_stale_error(
        "Error Domain=NSPOSIXErrorDomain Code=2 \"No such file or directory\""
    ));
    assert!(!is_apple_container_daemon_stale_error(
        "container is not running"
    ));
    assert!(!is_apple_container_daemon_stale_error("permission denied"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_is_apple_container_boot_failure() {
    // No logs at all — VM never booted
    assert!(is_apple_container_boot_failure(None));
    // Empty logs
    assert!(is_apple_container_boot_failure(Some("")));
    assert!(is_apple_container_boot_failure(Some("  \n  ")));
    // stdio.log doesn't exist — VM never produced output
    assert!(is_apple_container_boot_failure(Some(
        r#"Error: invalidArgument: "failed to fetch container logs: internalError: "failed to open container logs: Error Domain=NSCocoaErrorDomain Code=4 "The file "stdio.log" doesn't exist."""#
    )));
    // Real logs present — not a boot failure
    assert!(!is_apple_container_boot_failure(Some(
        "sleep: invalid time interval 'infinity'"
    )));
    // Daemon-stale EINVAL is NOT a boot failure (different handler)
    assert!(!is_apple_container_boot_failure(Some(
        "Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\""
    )));
}

#[test]
fn test_is_apple_container_corruption_error() {
    assert!(is_apple_container_corruption_error(
        "failed to bootstrap container because config.json is missing"
    ));
    // Daemon-stale errors should also trigger corruption/failover
    assert!(is_apple_container_corruption_error(
        "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\""
    ));
    assert!(!is_apple_container_corruption_error(
        "cannot exec: container is not running"
    ));
    assert!(!is_apple_container_corruption_error(
        "invalidState: \"no sandbox client exists: container is stopped\""
    ));
    assert!(!is_apple_container_corruption_error("permission denied"));
    // Boot failure "VM never booted" should trigger corruption/failover
    assert!(is_apple_container_corruption_error(
        "apple container test did not become exec-ready (VM never booted): timeout"
    ));
}

#[tokio::test]
async fn test_failover_sandbox_switches_from_apple_to_docker() {
    let primary = Arc::new(TestSandbox::new(
        "apple-container",
        Some("failed to bootstrap container: config.json missing"),
        None,
    ));
    let fallback = Arc::new(TestSandbox::new("docker", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-abc".into(),
    };

    sandbox.ensure_ready(&id, None).await.unwrap();
    sandbox.ensure_ready(&id, None).await.unwrap();

    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 2);
}

#[tokio::test]
async fn test_failover_sandbox_switches_on_boot_failure() {
    let primary = Arc::new(TestSandbox::new(
        "apple-container",
        Some("apple container test did not become exec-ready (VM never booted): timeout"),
        None,
    ));
    let fallback = Arc::new(TestSandbox::new("docker", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-boot".into(),
    };

    sandbox.ensure_ready(&id, None).await.unwrap();

    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 1);
}

#[tokio::test]
async fn test_failover_sandbox_does_not_switch_on_unrelated_error() {
    let primary = Arc::new(TestSandbox::new(
        "apple-container",
        Some("permission denied"),
        None,
    ));
    let fallback = Arc::new(TestSandbox::new("docker", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-abc".into(),
    };

    let error = sandbox.ensure_ready(&id, None).await.unwrap_err();
    assert!(format!("{error:#}").contains("permission denied"));
    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 0);
}

#[tokio::test]
async fn test_failover_sandbox_switches_exec_path() {
    let primary = Arc::new(TestSandbox::new(
        "apple-container",
        None,
        Some("failed to bootstrap container: config.json missing"),
    ));
    let fallback = Arc::new(TestSandbox::new("docker", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-abc".into(),
    };

    let result = sandbox
        .exec(&id, "uname -a", &ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(primary.exec_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 1);
    assert_eq!(fallback.exec_calls(), 1);
}

#[tokio::test]
async fn test_failover_sandbox_switches_on_daemon_stale_error() {
    let primary = Arc::new(TestSandbox::new(
        "apple-container",
        Some(
            "Error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=22 \"Invalid argument\"\"",
        ),
        None,
    ));
    let fallback = Arc::new(TestSandbox::new("docker", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-abc".into(),
    };

    sandbox.ensure_ready(&id, None).await.unwrap();
    sandbox.ensure_ready(&id, None).await.unwrap();

    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 2);
}

#[tokio::test]
async fn test_failover_sandbox_docker_to_wasm() {
    let primary = Arc::new(TestSandbox::new(
        "docker",
        Some("cannot connect to the docker daemon"),
        None,
    ));
    let fallback = Arc::new(TestSandbox::new("wasm", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-docker-wasm".into(),
    };

    sandbox.ensure_ready(&id, None).await.unwrap();

    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 1);
}

#[tokio::test]
async fn test_failover_docker_does_not_switch_on_unrelated_error() {
    let primary = Arc::new(TestSandbox::new("docker", Some("image not found"), None));
    let fallback = Arc::new(TestSandbox::new("wasm", None, None));
    let sandbox = FailoverSandbox::new(primary.clone(), fallback.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-docker-no-failover".into(),
    };

    let error = sandbox.ensure_ready(&id, None).await.unwrap_err();
    assert!(format!("{error:#}").contains("image not found"));
    assert_eq!(primary.ensure_ready_calls(), 1);
    assert_eq!(fallback.ensure_ready_calls(), 0);
}

#[test]
fn test_is_docker_failover_error() {
    assert!(is_docker_failover_error(
        "Cannot connect to the Docker daemon at unix:///var/run/docker.sock"
    ));
    assert!(is_docker_failover_error("Is the docker daemon running?"));
    assert!(is_docker_failover_error(
        "error during connect: connection refused"
    ));
    assert!(!is_docker_failover_error("image not found"));
    assert!(!is_docker_failover_error("permission denied"));
}

#[test]
fn test_is_podman_failover_error() {
    assert!(is_podman_failover_error(
        "Cannot connect to Podman: connection refused"
    ));
    assert!(is_podman_failover_error(
        "Error: podman: no such file or directory"
    ));
    assert!(is_podman_failover_error("OCI runtime not found: crun"));
    assert!(!is_podman_failover_error("image not found"));
    assert!(!is_podman_failover_error("permission denied"));
}

#[test]
fn test_select_backend_podman() {
    // This test always succeeds — select_backend("podman") unconditionally
    // creates a DockerSandbox::podman() regardless of CLI availability.
    let config = SandboxConfig {
        backend: "podman".into(),
        ..Default::default()
    };
    let backend = select_backend(config);
    assert_eq!(backend.backend_name(), "podman");
}

#[test]
fn test_select_backend_wasm() {
    let config = SandboxConfig {
        backend: "wasm".into(),
        ..Default::default()
    };
    let backend = select_backend(config);
    if is_wasm_sandbox_available() {
        assert_eq!(backend.backend_name(), "wasm");
    } else {
        // Falls back to restricted-host when wasm feature is disabled.
        assert_eq!(backend.backend_name(), "restricted-host");
    }
}

#[test]
fn test_select_backend_restricted_host() {
    let config = SandboxConfig {
        backend: "restricted-host".into(),
        ..Default::default()
    };
    let backend = select_backend(config);
    assert_eq!(backend.backend_name(), "restricted-host");
}

#[test]
fn test_is_debian_host() {
    let result = is_debian_host();
    // On macOS/Windows this should be false; on Debian/Ubuntu it should be true.
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        assert!(!result);
    }
    // On Linux, it depends on the distro — just verify it returns a bool without panic.
    let _ = result;
}

#[test]
fn test_host_package_name_candidates_t64_to_base() {
    assert_eq!(host_package_name_candidates("libgtk-3-0t64"), vec![
        "libgtk-3-0t64".to_string(),
        "libgtk-3-0".to_string()
    ]);
}

#[test]
fn test_host_package_name_candidates_base_to_t64_for_soname() {
    assert_eq!(host_package_name_candidates("libcups2"), vec![
        "libcups2".to_string(),
        "libcups2t64".to_string()
    ]);
}

#[test]
fn test_host_package_name_candidates_non_library_stays_single() {
    assert_eq!(host_package_name_candidates("curl"), vec![
        "curl".to_string()
    ]);
    assert_eq!(host_package_name_candidates("libreoffice-core"), vec![
        "libreoffice-core".to_string()
    ]);
}

#[tokio::test]
async fn test_provision_host_packages_empty() {
    let result = provision_host_packages(&[]).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_provision_host_packages_non_debian() {
    if is_debian_host() {
        // Can't test the non-debian path on a Debian host.
        return;
    }
    let result = provision_host_packages(&["curl".into()]).await.unwrap();
    assert!(result.is_none());
}

#[test]
fn test_is_running_as_root() {
    // In CI and dev, we typically don't run as root.
    let result = is_running_as_root();
    // Just verify it returns a bool without panic.
    let _ = result;
}

#[test]
fn test_should_use_docker_backend() {
    assert!(should_use_docker_backend(true, true));
    assert!(!should_use_docker_backend(true, false));
    assert!(!should_use_docker_backend(false, true));
    assert!(!should_use_docker_backend(false, false));
}

#[test]
fn container_run_state_serializes_lowercase() {
    assert_eq!(
        serde_json::to_value(ContainerRunState::Running)
            .unwrap()
            .as_str(),
        Some("running")
    );
    assert_eq!(
        serde_json::to_value(ContainerRunState::Stopped)
            .unwrap()
            .as_str(),
        Some("stopped")
    );
    assert_eq!(
        serde_json::to_value(ContainerRunState::Exited)
            .unwrap()
            .as_str(),
        Some("exited")
    );
    assert_eq!(
        serde_json::to_value(ContainerRunState::Unknown)
            .unwrap()
            .as_str(),
        Some("unknown")
    );
}

#[test]
fn container_backend_serializes_kebab_case() {
    assert_eq!(
        serde_json::to_value(ContainerBackend::AppleContainer)
            .unwrap()
            .as_str(),
        Some("apple-container")
    );
    assert_eq!(
        serde_json::to_value(ContainerBackend::Docker)
            .unwrap()
            .as_str(),
        Some("docker")
    );
    assert_eq!(
        serde_json::to_value(ContainerBackend::Podman)
            .unwrap()
            .as_str(),
        Some("podman")
    );
}

#[test]
fn running_container_serializes_to_json() {
    let c = RunningContainer {
        name: "moltis-sandbox-sess1".into(),
        image: "ubuntu:25.10".into(),
        state: ContainerRunState::Running,
        backend: ContainerBackend::Docker,
        cpus: Some(2),
        memory_mb: Some(512),
        started: Some("2025-01-01T00:00:00Z".into()),
        addr: None,
    };
    let json = serde_json::to_value(&c).unwrap();
    assert_eq!(json["name"], "moltis-sandbox-sess1");
    assert_eq!(json["state"], "running");
    assert_eq!(json["backend"], "docker");
    assert_eq!(json["cpus"], 2);
    assert_eq!(json["memory_mb"], 512);
    assert!(json["addr"].is_null());
}

#[test]
fn test_zombie_set_lifecycle() {
    // Fresh state: nothing is a zombie.
    assert!(!is_zombie("ghost-1"));

    // Mark as zombie.
    mark_zombie("ghost-1");
    assert!(is_zombie("ghost-1"));

    // Marking again is idempotent.
    mark_zombie("ghost-1");
    assert!(is_zombie("ghost-1"));

    // A different name is not a zombie.
    assert!(!is_zombie("ghost-2"));

    // Unmark clears the zombie.
    unmark_zombie("ghost-1");
    assert!(!is_zombie("ghost-1"));

    // Unmarking a non-zombie is a no-op.
    unmark_zombie("ghost-1");

    // Clear removes all zombies.
    mark_zombie("ghost-a");
    mark_zombie("ghost-b");
    assert!(is_zombie("ghost-a"));
    assert!(is_zombie("ghost-b"));
    clear_zombies();
    assert!(!is_zombie("ghost-a"));
    assert!(!is_zombie("ghost-b"));
}

// ── NetworkPolicy / proxy wiring tests ────────────────────────────────

#[test]
fn test_from_config_network_trusted_overrides_no_network() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: "trusted".into(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Trusted);
}

#[test]
fn test_from_config_network_bypass_overrides_no_network() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: "bypass".into(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Bypass);
}

#[test]
fn test_from_config_empty_network_defaults_to_trusted() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: false,
        network: String::new(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Trusted);
}

#[test]
fn test_from_config_no_network_true_empty_network_is_blocked() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: String::new(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Blocked);
}

#[test]
fn test_docker_network_run_args_blocked() {
    let config = SandboxConfig {
        network: NetworkPolicy::Blocked,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert_eq!(docker.network_run_args(), vec!["--network=none"]);
}

#[test]
fn test_docker_network_run_args_trusted() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.network_run_args();
    assert_eq!(args, vec!["--add-host=host.docker.internal:host-gateway"]);
}

#[test]
fn test_docker_network_run_args_bypass() {
    let config = SandboxConfig {
        network: NetworkPolicy::Bypass,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.network_run_args().is_empty());
}

#[test]
fn test_docker_proxy_exec_env_args_trusted() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.proxy_exec_env_args();
    let expected_url = format!(
        "http://host.docker.internal:{}",
        moltis_network_filter::DEFAULT_PROXY_PORT
    );
    // Should contain -e pairs for HTTP_PROXY, http_proxy, HTTPS_PROXY, https_proxy,
    // NO_PROXY, no_proxy (6 keys x 2 args each = 12 args).
    assert_eq!(args.len(), 12);
    assert!(args.contains(&format!("HTTP_PROXY={expected_url}")));
    assert!(args.contains(&format!("https_proxy={expected_url}")));
    assert!(args.contains(&"NO_PROXY=localhost,127.0.0.1,::1".to_string()));
    assert!(args.contains(&"no_proxy=localhost,127.0.0.1,::1".to_string()));
}

#[test]
fn test_docker_proxy_exec_env_args_blocked() {
    let config = SandboxConfig {
        network: NetworkPolicy::Blocked,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.proxy_exec_env_args().is_empty());
}

#[test]
fn test_docker_proxy_exec_env_args_bypass() {
    let config = SandboxConfig {
        network: NetworkPolicy::Bypass,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.proxy_exec_env_args().is_empty());
}

#[test]
fn test_docker_resolve_host_gateway_always_returns_host_gateway() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    // Docker always uses the host-gateway token regardless of version.
    assert_eq!(docker.resolve_host_gateway(), "host-gateway");
}

#[test]
fn test_podman_network_run_args_trusted_contains_add_host() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let podman = DockerSandbox::podman(config);
    let args = podman.network_run_args();
    // The exact IP depends on the host environment (Podman version and
    // rootless/rootful mode), but the flag must always start with
    // `--add-host=host.docker.internal:`.
    assert_eq!(args.len(), 1);
    assert!(
        args[0].starts_with("--add-host=host.docker.internal:"),
        "unexpected arg: {}",
        args[0],
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_proxy_prefix_trusted() {
    // Build the same prefix that exec() would build for Trusted mode,
    // but using the helper logic directly.
    let gateway = "192.168.64.1";
    let proxy_url = format!(
        "http://{}:{}",
        gateway,
        moltis_network_filter::DEFAULT_PROXY_PORT
    );
    let mut prefix = String::new();
    let escaped_proxy = proxy_url.replace('\'', "'\\''");
    for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
        prefix.push_str(&format!("export {key}='{escaped_proxy}'; "));
    }
    for key in ["NO_PROXY", "no_proxy"] {
        prefix.push_str(&format!("export {key}='localhost,127.0.0.1,::1'; "));
    }

    assert!(prefix.contains("export HTTP_PROXY="));
    assert!(prefix.contains("export https_proxy="));
    assert!(prefix.contains(&format!(":{}", moltis_network_filter::DEFAULT_PROXY_PORT)));
    assert!(prefix.contains("export NO_PROXY='localhost,127.0.0.1,::1'"));
}

mod restricted_host_tests {
    use super::*;

    #[test]
    fn test_restricted_host_sandbox_backend_name() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        assert_eq!(sandbox.backend_name(), "restricted-host");
    }

    #[test]
    fn test_restricted_host_sandbox_is_real() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        assert!(sandbox.is_real());
    }

    #[tokio::test]
    async fn test_restricted_host_sandbox_ensure_ready_noop() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-rh".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
    }

    #[tokio::test]
    async fn test_restricted_host_sandbox_exec_simple_echo() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-rh-echo".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "echo hello", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_restricted_host_sandbox_restricted_env() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-rh-env".into(),
        };
        let result = sandbox
            .exec(&id, "echo $HOME", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "/tmp");
    }

    #[tokio::test]
    async fn test_restricted_host_sandbox_build_image_returns_none() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        let result = sandbox
            .build_image("ubuntu:latest", &["curl".to_string()])
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_restricted_host_sandbox_cleanup_noop() {
        let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-rh-cleanup".into(),
        };
        sandbox.cleanup(&id).await.unwrap();
    }

    #[test]
    fn test_parse_memory_limit() {
        assert_eq!(parse_memory_limit("512M"), Some(512 * 1024 * 1024));
        assert_eq!(parse_memory_limit("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_limit("256k"), Some(256 * 1024));
        assert_eq!(parse_memory_limit("1024"), Some(1024));
        assert_eq!(parse_memory_limit("invalid"), None);
    }

    #[test]
    fn test_wasm_sandbox_available() {
        assert!(is_wasm_sandbox_available());
    }
}

#[cfg(feature = "wasm")]
mod wasm_sandbox_tests {
    use super::*;

    fn test_config() -> SandboxConfig {
        SandboxConfig {
            home_persistence: HomePersistence::Off,
            ..Default::default()
        }
    }

    #[test]
    fn test_wasm_sandbox_backend_name() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        assert_eq!(sandbox.backend_name(), "wasm");
    }

    #[test]
    fn test_wasm_sandbox_is_real() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        assert!(sandbox.is_real());
    }

    #[test]
    fn test_wasm_sandbox_fuel_limit_default() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        assert_eq!(sandbox.fuel_limit(), 1_000_000_000);
    }

    #[test]
    fn test_wasm_sandbox_fuel_limit_custom() {
        let mut config = test_config();
        config.wasm_fuel_limit = Some(500_000);
        let sandbox = WasmSandbox::new(config).unwrap();
        assert_eq!(sandbox.fuel_limit(), 500_000);
    }

    #[test]
    fn test_wasm_sandbox_epoch_interval_default() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        assert_eq!(sandbox.epoch_interval_ms(), 100);
    }

    #[tokio::test]
    async fn test_wasm_sandbox_ensure_ready_creates_dirs() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-ready".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        assert!(sandbox.home_dir(&id).exists());
        assert!(sandbox.tmp_dir(&id).exists());
        // Cleanup.
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_cleanup_removes_dirs() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-cleanup".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let root = sandbox.sandbox_root(&id);
        assert!(root.exists());
        sandbox.cleanup(&id).await.unwrap();
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_echo() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-echo".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "echo hello world", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello world");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_echo_no_newline() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-echo-n".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "echo -n hello", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_pwd() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-pwd".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "pwd", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "/home/sandbox");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_true_false() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-tf".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        let result = sandbox
            .exec(&id, "true", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        let result = sandbox
            .exec(&id, "false", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_mkdir_ls() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-mkdir-ls".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        let result = sandbox
            .exec(&id, "mkdir /home/sandbox/testdir", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        let result = sandbox
            .exec(&id, "ls /home/sandbox", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("testdir"));
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_touch_cat() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-touch-cat".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        // Write a file using echo with redirect.
        let result = sandbox
            .exec(
                &id,
                "echo hello > /home/sandbox/test.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        // Read it back.
        let result = sandbox
            .exec(&id, "cat /home/sandbox/test.txt", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_rm() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-rm".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        sandbox
            .exec(
                &id,
                "echo data > /home/sandbox/to_delete.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();

        let result = sandbox
            .exec(&id, "rm /home/sandbox/to_delete.txt", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        let result = sandbox
            .exec(&id, "cat /home/sandbox/to_delete.txt", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_unknown_command_127() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-unknown".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "nonexistent_cmd", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 127);
        assert!(result.stderr.contains("command not found"));
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_path_escape_blocked() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-escape".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        // Try to cat a file outside sandbox.
        let result = sandbox
            .exec(&id, "cat /etc/passwd", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        assert!(result.stderr.contains("outside sandbox"));
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_and_connector() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-and".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "true && echo yes", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "yes");

        let result = sandbox
            .exec(&id, "false && echo no", &ExecOpts::default())
            .await
            .unwrap();
        // The echo shouldn't run, so stdout should be empty.
        assert!(result.stdout.is_empty());
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_or_connector() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-or".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();
        let result = sandbox
            .exec(&id, "false || echo fallback", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "fallback");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_test_file() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-testcmd".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        sandbox
            .exec(
                &id,
                "echo x > /home/sandbox/exists.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();

        let result = sandbox
            .exec(
                &id,
                "test -f /home/sandbox/exists.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);

        let result = sandbox
            .exec(&id, "test -f /home/sandbox/nope.txt", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_basename_dirname() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-pathops".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        let result = sandbox
            .exec(
                &id,
                "basename /home/sandbox/foo/bar.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.stdout.trim(), "bar.txt");

        let result = sandbox
            .exec(
                &id,
                "dirname /home/sandbox/foo/bar.txt",
                &ExecOpts::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.stdout.trim(), "/home/sandbox/foo");
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_builtin_which() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "test-wasm-which".into(),
        };
        sandbox.ensure_ready(&id, None).await.unwrap();

        let result = sandbox
            .exec(&id, "which echo", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("built-in"));

        let result = sandbox
            .exec(&id, "which nonexistent", &ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1);
        sandbox.cleanup(&id).await.unwrap();
    }

    #[tokio::test]
    async fn test_wasm_sandbox_build_image_returns_none() {
        let sandbox = WasmSandbox::new(test_config()).unwrap();
        let result = sandbox
            .build_image("ubuntu:latest", &["curl".to_string()])
            .await
            .unwrap();
        assert!(result.is_none());
    }
}

#[cfg(target_os = "linux")]
mod linux_tests {
    use super::*;

    #[test]
    fn test_cgroup_scope_name() {
        let config = SandboxConfig::default();
        let cgroup = CgroupSandbox::new(config);
        let id = SandboxId {
            scope: SandboxScope::Session,
            key: "sess1".into(),
        };
        assert_eq!(cgroup.scope_name(&id), "moltis-sandbox-sess1");
    }

    #[test]
    fn test_cgroup_property_args() {
        let config = SandboxConfig {
            resource_limits: ResourceLimits {
                memory_limit: Some("1G".into()),
                cpu_quota: Some(2.0),
                pids_max: Some(200),
            },
            ..Default::default()
        };
        let cgroup = CgroupSandbox::new(config);
        let args = cgroup.property_args();
        assert!(args.contains(&"MemoryMax=1G".to_string()));
        assert!(args.contains(&"CPUQuota=200%".to_string()));
        assert!(args.contains(&"TasksMax=200".to_string()));
    }
}
