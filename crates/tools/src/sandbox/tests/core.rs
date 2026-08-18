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
    let args = DockerSandbox::hardening_args(true, BackendKind::Docker, false);
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
    // Sysfs masks are present (actual set depends on host — macOS includes
    // all because /sys doesn't exist; Linux includes only existing paths).
    // On macOS CI all four are present.
    #[cfg(not(target_os = "linux"))]
    {
        assert!(args.contains(&"/sys/firmware:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/class/dmi:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/devices/virtual/dmi:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/class/block:ro,nosuid".to_string()));
    }
}

#[test]
fn test_docker_hardening_args_not_prebuilt() {
    let args = DockerSandbox::hardening_args(false, BackendKind::Docker, false);
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"ALL".to_string()));
    assert!(args.contains(&"--security-opt".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    // --read-only must NOT be present for non-prebuilt (needs apt-get)
    assert!(!args.contains(&"--read-only".to_string()));
    // tmpfs mounts still present
    assert!(args.contains(&"/tmp:rw,nosuid,size=256m".to_string()));
    // Host metadata isolation still present — hostname
    let hostname_pos = args
        .iter()
        .position(|a| a == "--hostname")
        .expect("--hostname flag missing");
    assert_eq!(
        args[hostname_pos + 1],
        "sandbox",
        "--hostname value should be 'sandbox'"
    );
    #[cfg(not(target_os = "linux"))]
    {
        assert!(args.contains(&"/sys/firmware:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/class/dmi:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/devices/virtual/dmi:ro,nosuid".to_string()));
        assert!(args.contains(&"/sys/class/block:ro,nosuid".to_string()));
    }
}

#[test]
fn test_docker_hardening_args_podman() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Podman, false);
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
fn test_podman_nested_hardening_args_are_privileged() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Podman, true);

    assert!(args.contains(&"--privileged".to_string()));
    assert!(!args.contains(&"--cap-drop".to_string()));
    assert!(!args.contains(&"no-new-privileges".to_string()));
    assert!(!args.contains(&"--read-only".to_string()));
}

#[test]
fn test_docker_ignores_nested_podman_hardening_flag() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Docker, true);

    assert!(!args.contains(&"--privileged".to_string()));
    assert!(args.contains(&"--cap-drop".to_string()));
    assert!(args.contains(&"no-new-privileges".to_string()));
    assert!(args.contains(&"--read-only".to_string()));
}

#[tokio::test]
async fn test_podman_socket_args_disabled_by_default() {
    let sandbox = DockerSandbox::podman(SandboxConfig::default());

    assert!(sandbox.podman_socket_path().await.unwrap().is_none());
    assert!(DockerSandbox::podman_socket_run_args_for_path(None).is_err());
}

#[test]
fn test_podman_socket_run_args_include_matching_env() {
    let args = DockerSandbox::podman_socket_run_args_for_path(Some(std::path::Path::new(
        "/run/user/1000/podman/podman.sock",
    )))
    .unwrap();

    assert_eq!(args, vec![
        "-v".to_string(),
        "/run/user/1000/podman/podman.sock:/tmp/moltis-host-podman.sock:rw".to_string(),
        "--security-opt".to_string(),
        "label=disable".to_string(),
        "-e".to_string(),
        "CONTAINER_HOST=unix:///tmp/moltis-host-podman.sock".to_string(),
        "-e".to_string(),
        "DOCKER_HOST=unix:///tmp/moltis-host-podman.sock".to_string(),
    ]);
}

#[test]
fn test_podman_mode_labels_detect_escape_hatch_transitions() {
    let hardened = DockerSandbox::podman(SandboxConfig::default())
        .podman_mode_label_value(None)
        .unwrap();
    let nested = DockerSandbox::podman(SandboxConfig {
        allow_nested_podman: true,
        ..Default::default()
    })
    .podman_mode_label_value(None)
    .unwrap();

    assert_eq!(hardened, "hardened");
    assert_eq!(nested, "nested");
    assert!(DockerSandbox::podman_mode_matches(
        Some(&hardened),
        &hardened
    ));
    assert!(!DockerSandbox::podman_mode_matches(
        Some(&nested),
        &hardened
    ));
    assert!(!DockerSandbox::podman_mode_matches(None, &hardened));
}

#[cfg(unix)]
#[test]
fn test_podman_host_mode_changes_when_socket_is_recreated() {
    use std::os::unix::net::UnixListener;

    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("podman.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    let sandbox = DockerSandbox::podman(SandboxConfig {
        allow_host_podman: true,
        ..Default::default()
    });
    let first = sandbox.podman_mode_label_value(Some(&socket_path)).unwrap();

    drop(listener);
    std::fs::remove_file(&socket_path).unwrap();
    let _replacement = UnixListener::bind(&socket_path).unwrap();
    let second = sandbox.podman_mode_label_value(Some(&socket_path)).unwrap();

    assert_ne!(first, second);
    assert!(!DockerSandbox::podman_mode_matches(Some(&first), &second));
    assert_eq!(DockerSandbox::podman_mode_label_args(&second), vec![
        "--label".to_string(),
        format!("org.moltis.podman-mode={second}"),
    ]);
}

#[cfg(unix)]
#[tokio::test]
async fn test_podman_mode_mismatch_recreates_running_container() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp_dir = tempfile::tempdir().unwrap();
    let fake_cli = temp_dir.path().join("fake-podman");
    let state = temp_dir.path().join("state");
    let log = temp_dir.path().join("calls");
    std::fs::write(&state, "running").unwrap();
    std::fs::write(
        &fake_cli,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> '{}'\n\
             if [ \"$1\" = \"inspect\" ]; then\n\
               case \"$3\" in\n\
                 *State.Running*) [ \"$(cat '{}')\" = \"running\" ] && printf 'true\\n' || printf 'false\\n' ;;\n\
                 *Config.Labels*) printf 'nested\\n' ;;\n\
               esac\n\
               exit 0\n\
             fi\n\
             if [ \"$1\" = \"rm\" ]; then printf 'stopped' > '{}'; exit 0; fi\n\
             if [ \"$1\" = \"image\" ]; then exit 0; fi\n\
             if [ \"$1\" = \"run\" ]; then printf 'running' > '{}'; exit 0; fi\n\
             exit 0\n",
            log.display(),
            state.display(),
            state.display(),
            state.display(),
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&fake_cli).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_cli, permissions).unwrap();

    let cli: &'static str = Box::leak(fake_cli.display().to_string().into_boxed_str());
    let sandbox = DockerSandbox::podman_with_cli(
        SandboxConfig {
            image: Some("moltis-sandbox:test".to_string()),
            network: NetworkPolicy::Blocked,
            workspace_mount: WorkspaceMount::None,
            home_persistence: HomePersistence::Off,
            ..Default::default()
        },
        cli,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "mode-transition".to_string(),
    };

    sandbox.ensure_ready(&id, None).await.unwrap();

    let calls = std::fs::read_to_string(log).unwrap();
    assert!(calls.contains("rm -f moltis-sandbox-mode-transition"));
    assert!(calls.contains("--label org.moltis.podman-mode=hardened"));
}

#[cfg(unix)]
#[tokio::test]
async fn test_podman_socket_validation_requires_live_unix_socket() {
    use std::os::unix::net::UnixListener;

    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("podman.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    assert!(DockerSandbox::is_usable_podman_socket(&socket_path).await);
    assert!(!DockerSandbox::is_usable_podman_socket(temp_dir.path()).await);
    let regular_file = temp_dir.path().join("regular");
    std::fs::write(&regular_file, "not a socket").unwrap();
    assert!(!DockerSandbox::is_usable_podman_socket(&regular_file).await);
    assert!(!DockerSandbox::is_usable_podman_socket(std::path::Path::new("relative.sock")).await);

    drop(listener);
    let stale_probe = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        DockerSandbox::is_usable_podman_socket(&socket_path),
    )
    .await
    .unwrap();
    assert!(!stale_probe);
}

#[test]
fn test_podman_rootless_reexec_restricted_detection() {
    assert!(is_podman_rootless_reexec_restricted(
        "cannot clone: Operation not permitted\nError: cannot re-exec process"
    ));
    assert!(!is_podman_rootless_reexec_restricted(
        "Error: image not found"
    ));
}

#[test]
fn test_podman_rootless_reexec_error_gets_actionable_context() {
    let formatted = format_container_run_stderr(
        BackendKind::Podman,
        "cannot clone: Operation not permitted\nError: cannot re-exec process",
    );

    assert!(formatted.contains("NoNewPrivileges=true"));
    assert!(formatted.contains("Rootless Podman"));
    assert!(formatted.contains("moltis-podman.conf"));
    assert!(formatted.contains("allow_host_podman=true"));
    assert!(formatted.contains("allow_nested_podman=true"));
}

#[test]
fn test_docker_run_error_does_not_get_podman_context() {
    let formatted = format_container_run_stderr(
        BackendKind::Docker,
        "cannot clone: Operation not permitted\nError: cannot re-exec process",
    );

    assert!(!formatted.contains("NoNewPrivileges=true"));
}

#[test]
fn test_sysfs_paths_to_mask_no_sysfs_root_returns_all() {
    // When /sys doesn't exist (macOS), all paths should be returned because
    // Docker Desktop runs in a Linux VM with full sysfs.
    let paths = sysfs_paths_to_mask_from("/nonexistent/sysfs/root");
    assert_eq!(paths, vec![
        "/sys/firmware",
        "/sys/class/dmi",
        "/sys/devices/virtual/dmi",
        "/sys/class/block",
    ]);
}

#[test]
fn test_sysfs_paths_to_mask_filters_missing_paths() {
    // Simulate a Linux host where the sysfs root exists but specific
    // subtrees are missing (e.g. ARM without DMI, or WSL2).
    let dir = tempfile::tempdir().unwrap();
    let sysfs_root = dir.path().join("sys");
    // Create only /sys/firmware and /sys/class/block, skip DMI paths
    // (simulates Raspberry Pi / ARM which lacks DMI).
    std::fs::create_dir_all(sysfs_root.join("firmware")).unwrap();
    std::fs::create_dir_all(sysfs_root.join("class/block")).unwrap();

    let paths = sysfs_paths_to_mask_from(sysfs_root.to_str().unwrap());
    // Only the two paths that exist under the tempdir sysfs root are returned.
    assert_eq!(paths, vec!["/sys/firmware", "/sys/class/block"]);
}

#[test]
fn test_sysfs_mask_paths_constant_contains_expected_entries() {
    // Guard against accidentally removing paths from the constant.
    assert!(SYSFS_MASK_PATHS.contains(&"/sys/firmware"));
    assert!(SYSFS_MASK_PATHS.contains(&"/sys/class/dmi"));
    assert!(SYSFS_MASK_PATHS.contains(&"/sys/devices/virtual/dmi"));
    assert!(SYSFS_MASK_PATHS.contains(&"/sys/class/block"));
    assert_eq!(SYSFS_MASK_PATHS.len(), 4);
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
    assert_eq!(config.managed_files_mount, ManagedFilesMount::Ro);
    assert!(config.no_network);
    assert_eq!(config.resource_limits.memory_limit.as_deref(), Some("1G"));
}

#[test]
fn test_runtime_sandbox_config_converts_managed_files_mount() {
    let config = moltis_config::schema::SandboxConfig {
        managed_files_mount: moltis_config::schema::ManagedFilesMountConfig::Rw,
        ..Default::default()
    };

    let runtime = SandboxConfig::from(&config);

    assert_eq!(runtime.managed_files_mount, ManagedFilesMount::Rw);
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
fn test_docker_hardening_args_enable_init_reaper() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Docker, false);
    assert!(
        args.contains(&"--init".to_string()),
        "Docker sandboxes must run with an init process so orphaned children are reaped"
    );
}

#[test]
fn test_podman_hardening_args_do_not_require_host_init_binary() {
    let args = DockerSandbox::hardening_args(true, BackendKind::Podman, false);
    assert!(!args.contains(&"--init".to_string()));
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
fn test_docker_managed_files_args_ro_with_legacy_override() {
    let docker = DockerSandbox::new(SandboxConfig::default());
    let args = docker.managed_files_args().unwrap();
    let files_dir = moltis_config::managed_files_dir();

    assert_eq!(args, vec![
        "-v",
        &format!("{}:{SANDBOX_FILES_DIR}:ro", files_dir.display()),
        "-v",
        &format!("{}:{}:ro", files_dir.display(), files_dir.display()),
    ]);
}

#[test]
fn test_docker_managed_files_args_rw_uses_host_data_dir_override() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("host-moltis-data");
    let docker = DockerSandbox::new(SandboxConfig {
        managed_files_mount: ManagedFilesMount::Rw,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    });
    let args = docker.managed_files_args().unwrap();
    let host_files = host_data_dir.join("files");

    assert_eq!(args, vec![
        "-v",
        &format!("{}:{SANDBOX_FILES_DIR}:rw", host_files.display()),
        "-v",
        &format!(
            "{}:{}:rw",
            host_files.display(),
            moltis_config::managed_files_dir().display()
        ),
    ]);
}

#[test]
fn test_docker_managed_files_policy_changes_with_source_or_mode() {
    let first = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Ro,
        ..Default::default()
    });
    let changed_source = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/two")),
        managed_files_mount: ManagedFilesMount::Ro,
        ..Default::default()
    });
    let changed_mode = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Rw,
        ..Default::default()
    });
    let changed_workspace_mount = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Ro,
        workspace_mount: WorkspaceMount::None,
        ..Default::default()
    });

    assert_ne!(
        first.managed_files_policy_fingerprint(),
        changed_source.managed_files_policy_fingerprint()
    );
    assert_ne!(
        first.managed_files_policy_fingerprint(),
        changed_mode.managed_files_policy_fingerprint()
    );
    assert_ne!(
        first.managed_files_policy_fingerprint(),
        changed_workspace_mount.managed_files_policy_fingerprint()
    );
    assert!(first.exposes_managed_files());
    assert!(
        !DockerSandbox::new(SandboxConfig {
            managed_files_mount: ManagedFilesMount::None,
            ..Default::default()
        })
        .exposes_managed_files()
    );
}

#[test]
fn test_docker_managed_files_args_none_masks_canonical_and_legacy_paths() {
    let docker = DockerSandbox::new(SandboxConfig {
        managed_files_mount: ManagedFilesMount::None,
        workspace_mount: WorkspaceMount::Rw,
        ..Default::default()
    });

    assert_eq!(docker.managed_files_args().unwrap(), vec![
        "--tmpfs",
        &format!("{SANDBOX_FILES_DIR}:ro,nosuid,nodev,noexec,size=64k"),
        "--tmpfs",
        &format!(
            "{}:ro,nosuid,nodev,noexec,size=64k",
            moltis_config::managed_files_dir().display()
        ),
    ]);
}

#[test]
fn test_docker_managed_files_args_omits_legacy_path_without_workspace_mount() {
    let docker = DockerSandbox::new(SandboxConfig {
        managed_files_mount: ManagedFilesMount::Ro,
        workspace_mount: WorkspaceMount::None,
        ..Default::default()
    });

    assert_eq!(docker.managed_files_args().unwrap().len(), 2);
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
fn test_resolve_managed_files_path_precedes_canonical_and_legacy_mappings() {
    let host_data_dir = PathBuf::from("/host/moltis-data");
    let config = SandboxConfig {
        managed_files_mount: ManagedFilesMount::Ro,
        workspace_mount: WorkspaceMount::Rw,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };

    for guest_file in [
        PathBuf::from(SANDBOX_FILES_DIR).join("nested/note.txt"),
        moltis_config::managed_files_dir().join("nested/note.txt"),
        PathBuf::from("/home/sandbox/other/../files/nested/note.txt"),
    ] {
        assert_eq!(
            resolve_managed_files_guest_path_on_host(&config, Some("docker"), &guest_file),
            ManagedFilesPath::ReadOnly(host_data_dir.join("files/nested/note.txt"))
        );
    }
}

#[test]
fn test_resolve_managed_files_path_none_is_unavailable_for_legacy_alias() {
    let config = SandboxConfig {
        managed_files_mount: ManagedFilesMount::None,
        workspace_mount: WorkspaceMount::Rw,
        ..Default::default()
    };

    assert_eq!(
        resolve_managed_files_guest_path_on_host(
            &config,
            Some("docker"),
            &moltis_config::managed_files_dir().join("private.txt"),
        ),
        ManagedFilesPath::Unavailable
    );
}

#[tokio::test]
async fn test_docker_managed_files_ro_rejects_canonical_and_legacy_writes() {
    let docker = DockerSandbox::new(SandboxConfig {
        managed_files_mount: ManagedFilesMount::Ro,
        workspace_mount: WorkspaceMount::Rw,
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "managed-ro".into(),
    };

    for guest_file in [
        PathBuf::from(SANDBOX_FILES_DIR).join("note.txt"),
        moltis_config::managed_files_dir().join("note.txt"),
    ] {
        let payload = docker
            .write_file(&id, &guest_file.display().to_string(), b"blocked")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(payload["kind"], "permission_denied");
    }
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
    let guest_file = PathBuf::from("/home/sandbox/history.txt");

    let resolved =
        resolve_home_persistence_guest_path_on_host(&config, Some("docker"), &id, &guest_file)
            .unwrap();

    assert_eq!(
        resolved,
        host_data_dir.join("sandbox/home/session/sess-1/history.txt")
    );
}

#[test]
fn test_resolve_home_persistence_guest_path_on_host_uses_shared_mount() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };

    let resolved = resolve_home_persistence_guest_path_on_host(
        &config,
        Some("docker"),
        &id,
        &PathBuf::from("/home/sandbox/history.txt"),
    )
    .unwrap();

    assert_eq!(
        resolved,
        host_data_dir.join("sandbox/home/shared/history.txt")
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

#[cfg(unix)]
#[tokio::test]
async fn test_docker_write_file_falls_back_to_container_copy_when_host_mount_is_inaccessible() {
    let temp_dir = tempfile::tempdir().unwrap();
    let fake_cli = temp_dir.path().join("fake-docker");
    std::fs::write(
        &fake_cli,
        "#!/bin/sh\n\
         if [ \"$1\" = \"exec\" ]; then printf 'missing-file\\n'; exit 0; fi\n\
         if [ \"$1\" = \"cp\" ]; then cat >/dev/null; exit 0; fi\n\
         exit 2\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&fake_cli).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_cli, permissions).unwrap();
    }

    let cli: &'static str = Box::leak(fake_cli.display().to_string().into_boxed_str());
    let host_data_dir = temp_dir.path().join("host-only-data");
    let docker = DockerSandbox::with_cli(
        SandboxConfig {
            workspace_mount: WorkspaceMount::Rw,
            host_data_dir: Some(host_data_dir),
            ..Default::default()
        },
        cli,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-write-fallback".into(),
    };
    let guest_file = moltis_config::data_dir().join("notes/todo.txt");

    let result = docker
        .write_file(
            &id,
            &guest_file.display().to_string(),
            b"docker fallback write",
        )
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_docker_write_file_uses_mounted_home_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let docker = DockerSandbox::new(SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-home-write".into(),
    };
    let host_home = host_data_dir.join("sandbox/home/session/test-docker-home-write");
    std::fs::create_dir_all(&host_home).unwrap();

    let result = docker
        .write_file(&id, "/home/sandbox/todo.txt", b"docker home write")
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(
        std::fs::read_to_string(host_home.join("todo.txt")).unwrap(),
        "docker home write"
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

#[cfg(unix)]
#[tokio::test]
async fn test_docker_list_files_falls_back_when_host_mount_is_inaccessible() {
    let temp_dir = tempfile::tempdir().unwrap();
    let fake_cli = temp_dir.path().join("fake-docker");
    std::fs::write(
        &fake_cli,
        "#!/bin/sh\n\
         if [ \"$1\" != \"exec\" ]; then exit 2; fi\n\
         case \"$*\" in\n\
           *\"find \"*) printf '/container/listed.txt\\n' ;;\n\
           *) printf 'dir\\n' ;;\n\
         esac\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&fake_cli).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_cli, permissions).unwrap();
    }

    let host_data_dir = temp_dir.path().join("missing-host-data");
    assert!(!host_data_dir.exists());
    let cli: &'static str = Box::leak(fake_cli.display().to_string().into_boxed_str());
    let docker = DockerSandbox::with_cli(
        SandboxConfig {
            workspace_mount: WorkspaceMount::Rw,
            host_data_dir: Some(host_data_dir.clone()),
            ..Default::default()
        },
        cli,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-docker-list-fallback".into(),
    };
    let guest_root = moltis_config::data_dir().join("notes");

    let files = docker
        .list_files(&id, &guest_root.display().to_string())
        .await
        .unwrap();

    assert_eq!(files.files, vec!["/container/listed.txt"]);
    assert!(!files.truncated);

    std::fs::create_dir_all(&host_data_dir).unwrap();
    std::os::unix::fs::symlink(
        temp_dir.path().join("missing-target"),
        host_data_dir.join("notes"),
    )
    .unwrap();

    let files = docker
        .list_files(&id, &guest_root.display().to_string())
        .await
        .unwrap();

    assert_eq!(files.files, vec!["/container/listed.txt"]);
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
