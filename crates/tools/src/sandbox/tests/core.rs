#![allow(clippy::unwrap_used, clippy::expect_used)]
use {super::*, crate::sandbox::types::tail_lines};

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
    let args = DockerSandbox::hardening_args(true, BackendKind::Docker);
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    assert!(args.contains(&"--read-only".to_string()));
    // Verify tmpfs mounts are present
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    assert!(args.contains(&"/run:rw,nosuid,size=64m".to_string()));
    // Host metadata isolation — assert flag-value adjacency for --hostname
    let hostname_pos = args
        .iter()
        .position(|a| a == "--hostname")
        .expect("--hostname flag missing");
    assert_eq!(
        args[hostname_pos + 1],
        "sandbox",
        "--hostname value should be 'sandbox'"
    );
    assert!(args.contains(&"/sys/firmware:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/class/dmi:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/devices/virtual/dmi:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/class/block:ro,nosuid".to_string()));
}

#[test]
fn test_docker_hardening_args_not_prebuilt() {
    let args = DockerSandbox::hardening_args(false, BackendKind::Docker);
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    // --read-only must NOT be present for non-prebuilt (needs apt-get)
    assert!(!args.contains(&"--read-only".to_string()));
    // tmpfs mounts still present
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    // Host metadata isolation still present — all 4 sysfs masks + hostname
    let hostname_pos = args
        .iter()
        .position(|a| a == "--hostname")
        .expect("--hostname flag missing");
    assert_eq!(
        args[hostname_pos + 1],
        "sandbox",
        "--hostname value should be 'sandbox'"
    );
    assert!(args.contains(&"/sys/firmware:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/class/dmi:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/devices/virtual/dmi:ro,nosuid".to_string()));
    assert!(args.contains(&"/sys/class/block:ro,nosuid".to_string()));
}

#[test]
fn test_docker_hardening_args_podman() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Podman);
    // Core hardening flags must still be present
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    assert!(args.contains(&"--read-only".to_string()));
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    assert!(args.contains(&"/run:rw,nosuid,size=64m".to_string()));
    let hostname_pos = args
        .iter()
        .position(|a| a == "--hostname")
        .expect("--hostname flag missing");
    assert_eq!(
        args[hostname_pos + 1],
        "sandbox",
        "--hostname value should be 'sandbox'"
    );
    // Sysfs tmpfs overlays must NOT be present — Podman's tmpcopyup breaks
    // these under --cap-drop ALL.
    assert!(!args.contains(&"/sys/firmware:ro,nosuid".to_string()));
    assert!(!args.contains(&"/sys/class/dmi:ro,nosuid".to_string()));
    assert!(!args.contains(&"/sys/devices/virtual/dmi:ro,nosuid".to_string()));
    assert!(!args.contains(&"/sys/class/block:ro,nosuid".to_string()));
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
fn test_workspace_mount_points_sandbox_at_moltis_data_dir_memory_files() {
    let config = SandboxConfig {
        workspace_mount: WorkspaceMount::Ro,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.workspace_args();
    let workspace_dir = moltis_config::data_dir();
    let guest_memory_file = workspace_dir.join("MEMORY.md");
    let guest_memory_dir = workspace_dir.join("memory");

    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "-v");
    assert!(
        args[1].contains(&format!(":{}:ro", workspace_dir.display())),
        "workspace mount should expose the Moltis data dir inside the sandbox"
    );
    assert_eq!(guest_memory_file, workspace_dir.join("MEMORY.md"));
    assert_eq!(guest_memory_dir, workspace_dir.join("memory"));
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
fn test_sandbox_home_persistence_is_separate_from_memory_workspace() {
    let config = SandboxConfig::default();
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };

    let home_dir =
        guest_visible_sandbox_home_persistence_host_dir(&config, &id).expect("shared home path");
    let data_dir = moltis_config::data_dir();

    assert_eq!(
        home_dir,
        data_dir.join("sandbox").join("home").join("shared")
    );
    assert_ne!(home_dir, data_dir);
    assert_eq!(
        home_dir.parent(),
        Some(data_dir.join("sandbox").join("home").as_path())
    );
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
fn test_resolve_workspace_guest_path_on_host_uses_host_override() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        workspace_mount: WorkspaceMount::Rw,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let guest_file = moltis_config::data_dir().join("notes/todo.txt");

    let resolved =
        resolve_workspace_guest_path_on_host(&config, Some("docker"), &guest_file).unwrap();

    assert_eq!(resolved, host_data_dir.join("notes/todo.txt"));
}

#[test]
fn test_resolve_home_persistence_guest_path_on_host_uses_session_mount() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let guest_file = guest_visible_sandbox_home_persistence_host_dir(&config, &id)
        .unwrap()
        .join("history.txt");

    let resolved =
        resolve_home_persistence_guest_path_on_host(&config, Some("docker"), &id, &guest_file)
            .unwrap();

    assert_eq!(
        resolved,
        host_data_dir.join("sandbox/home/session/sess-1/history.txt")
    );
}

#[tokio::test]
async fn test_docker_read_file_uses_mounted_workspace_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let host_file = host_data_dir.join("notes/todo.txt");
    std::fs::create_dir_all(host_file.parent().unwrap()).unwrap();
    std::fs::write(&host_file, "docker mounted read").unwrap();

    let docker = DockerSandbox::new(SandboxConfig {
        workspace_mount: WorkspaceMount::Rw,
        host_data_dir: Some(host_data_dir),
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-read".into(),
    };
    let guest_file = moltis_config::data_dir().join("notes/todo.txt");

    let result = docker
        .read_file(&id, &guest_file.display().to_string(), 1024)
        .await
        .unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"docker mounted read"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test]
async fn test_docker_write_file_uses_mounted_workspace_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let docker = DockerSandbox::new(SandboxConfig {
        workspace_mount: WorkspaceMount::Rw,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-write".into(),
    };
    let guest_file = moltis_config::data_dir().join("notes/todo.txt");
    std::fs::create_dir_all(host_data_dir.join("notes")).unwrap();

    let result = docker
        .write_file(
            &id,
            &guest_file.display().to_string(),
            b"docker mounted write",
        )
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(
        std::fs::read_to_string(host_data_dir.join("notes/todo.txt")).unwrap(),
        "docker mounted write"
    );
}

#[tokio::test]
async fn test_docker_list_files_remaps_mounted_workspace_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let host_root = host_data_dir.join("notes");
    std::fs::create_dir_all(host_root.join("nested")).unwrap();
    std::fs::write(host_root.join("todo.txt"), "a").unwrap();
    std::fs::write(host_root.join("nested/done.txt"), "b").unwrap();

    let docker = DockerSandbox::new(SandboxConfig {
        workspace_mount: WorkspaceMount::Rw,
        host_data_dir: Some(host_data_dir),
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-list".into(),
    };
    let guest_root = moltis_config::data_dir().join("notes");

    let files = docker
        .list_files(&id, &guest_root.display().to_string())
        .await
        .unwrap();
    assert_eq!(files.files, vec![
        guest_root.join("nested/done.txt").display().to_string(),
        guest_root.join("todo.txt").display().to_string(),
    ]);
    assert!(!files.truncated);
}

#[tokio::test]
async fn test_provisioning_guard_skips_second_call() {
    let docker = DockerSandbox::new(SandboxConfig::default());
    let name = "moltis-sandbox-test-guard";

    // First insertion succeeds.
    {
        let mut guard = docker.provisioned.lock().await;
        assert!(!guard.contains(name));
        guard.insert(name.to_string());
    }

    // Second check shows already provisioned.
    {
        let guard = docker.provisioned.lock().await;
        assert!(guard.contains(name));
    }
}

#[tokio::test]
async fn test_provisioning_guard_cleared_on_cleanup_entry() {
    let docker = DockerSandbox::new(SandboxConfig::default());
    let name = "moltis-sandbox-test-cleanup";

    // Mark as provisioned.
    docker.provisioned.lock().await.insert(name.to_string());
    assert!(docker.provisioned.lock().await.contains(name));

    // Simulate cleanup clearing the entry.
    docker.provisioned.lock().await.remove(name);
    assert!(!docker.provisioned.lock().await.contains(name));
}

#[tokio::test]
async fn test_provisioning_guard_independent_containers() {
    let docker = DockerSandbox::new(SandboxConfig::default());

    docker
        .provisioned
        .lock()
        .await
        .insert("container-a".to_string());

    let guard = docker.provisioned.lock().await;
    assert!(guard.contains("container-a"));
    assert!(!guard.contains("container-b"));
}

#[test]
fn test_podman_build_verifies_image_in_store() {
    // The Podman constructor must set `kind = BackendKind::Podman` so the
    // post-build verification branch in `build_image` activates.
    let sandbox = DockerSandbox::podman(SandboxConfig::default());
    assert_eq!(sandbox.kind, BackendKind::Podman);
    assert_eq!(sandbox.backend_name(), "podman");

    // Docker constructor must NOT be Podman.
    let docker = DockerSandbox::new(SandboxConfig::default());
    assert_eq!(docker.kind, BackendKind::Docker);
    assert_ne!(docker.kind, BackendKind::Podman);
}

#[test]
fn test_tail_lines_fewer_than_n() {
    let text = "line1\nline2\nline3";
    assert_eq!(tail_lines(text, 5), text);
}

#[test]
fn test_tail_lines_exact_n() {
    let text = "line1\nline2\nline3";
    assert_eq!(tail_lines(text, 3), text);
}

#[test]
fn test_tail_lines_more_than_n() {
    let text = "line1\nline2\nline3\nline4\nline5";
    let result = tail_lines(text, 2);
    assert!(result.starts_with("... [3 lines truncated]"));
    assert!(result.contains("line4\nline5"));
    assert!(!result.contains("line3"));
}

#[test]
fn test_tail_lines_empty() {
    assert_eq!(tail_lines("", 5), "");
}
