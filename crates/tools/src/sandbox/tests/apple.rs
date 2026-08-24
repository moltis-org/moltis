#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn apple_container_names_fit_runtime_limit() {
    let sandbox = AppleContainerSandbox::new(SandboxConfig {
        container_prefix: Some("moltis-moltis-sandbox".into()),
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "session-4fc5159e-0cb6-49f2-8eba-553ba3ec897b".into(),
    };

    let initial = sandbox.container_name(&id).await;
    assert!(initial.len() <= container_name::APPLE_CONTAINER_ID_MAX_LEN);
    assert_eq!(sandbox.runtime_name(&id).await, Some(initial.clone()));

    let rotated = sandbox.bump_container_generation(&id).await;
    assert!(rotated.len() <= container_name::APPLE_CONTAINER_ID_MAX_LEN);
    assert!(rotated.ends_with("-g1"));

    let failover = FailoverSandbox::new(
        Arc::new(AppleContainerSandbox::new(SandboxConfig {
            container_prefix: Some("moltis-moltis-sandbox".into()),
            ..Default::default()
        })),
        Arc::new(DockerSandbox::new(SandboxConfig::default())),
    );
    assert_eq!(failover.runtime_name(&id).await, Some(initial));
    let runtime_info = failover.runtime_info(&id).await;
    assert_eq!(runtime_info.backend_name, "apple-container");
    assert_eq!(runtime_info.runtime_name, failover.runtime_name(&id).await);
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

#[tokio::test]
async fn test_runtime_oci_file_transfers_with_docker() {
    if !runtime_container_e2e_enabled("docker") {
        eprintln!(
            "skipping Docker OCI runtime e2e test, set {}=1 and ensure docker is available",
            OCI_RUNTIME_E2E_ENV
        );
        return;
    }

    assert_runtime_oci_file_transfers("docker").await.unwrap();
}

#[tokio::test]
async fn test_runtime_oci_file_transfers_with_podman() {
    if !runtime_container_e2e_enabled("podman") {
        eprintln!(
            "skipping Podman OCI runtime e2e test, set {}=1 and ensure podman is available",
            OCI_RUNTIME_E2E_ENV
        );
        return;
    }

    assert_runtime_oci_file_transfers("podman").await.unwrap();
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

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_start_progress_logger_stops_on_done_signal() {
    let done = spawn_apple_container_start_progress_logger_with_interval(
        std::time::Duration::from_millis(1),
    );

    done.send(()).unwrap();
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
    let args = apple_container_run_args(
        "moltis-sandbox-test",
        "ubuntu:25.10",
        Some("UTC"),
        &[],
        &ResourceLimits::default(),
    )
    .unwrap();
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
        &["/tmp/home:/home/sandbox".to_string()],
        &ResourceLimits::default(),
    )
    .unwrap();
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
fn test_apple_container_run_args_with_multiple_volumes() {
    let volumes = vec![
        "/tmp/home:/home/sandbox".to_string(),
        "/tmp/files:/home/sandbox/files:ro".to_string(),
    ];
    let args = apple_container_run_args(
        "moltis-sandbox-test",
        "ubuntu:25.10",
        None,
        &volumes,
        &ResourceLimits::default(),
    )
    .unwrap();

    assert_eq!(
        args.windows(2)
            .filter(|window| window[0] == "--volume")
            .map(|window| window[1].as_str())
            .collect::<Vec<_>>(),
        volumes.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn test_apple_container_run_args_apply_resource_limits() {
    let args = apple_container_run_args(
        "moltis-sandbox-test",
        "ubuntu:25.10",
        None,
        &[],
        &ResourceLimits {
            memory_limit: Some("1G".into()),
            cpu_quota: Some(2.0),
            pids_max: Some(512),
        },
    )
    .unwrap();

    assert_eq!(
        args.windows(2)
            .filter(|window| matches!(window[0].as_str(), "--memory" | "--cpus" | "--ulimit"))
            .map(|window| (window[0].as_str(), window[1].as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("--memory", "1G"),
            ("--cpus", "2"),
            ("--ulimit", "nproc=512")
        ]
    );
}

#[test]
fn test_apple_container_run_args_reject_fractional_cpu_quota() {
    let error = apple_container_run_args(
        "moltis-sandbox-test",
        "ubuntu:25.10",
        None,
        &[],
        &ResourceLimits {
            cpu_quota: Some(0.5),
            ..Default::default()
        },
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("cpu_quota to be a positive whole number")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_policy_fingerprint_includes_resource_limits() {
    let first = AppleContainerSandbox::new(SandboxConfig {
        resource_limits: ResourceLimits {
            pids_max: Some(256),
            ..Default::default()
        },
        ..Default::default()
    });
    let second = AppleContainerSandbox::new(SandboxConfig {
        resource_limits: ResourceLimits {
            pids_max: Some(512),
            ..Default::default()
        },
        ..Default::default()
    });

    assert_ne!(
        first.container_policy_fingerprint(),
        second.container_policy_fingerprint()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_managed_files_mount_coexists_with_home_persistence() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let sandbox = AppleContainerSandbox::new(SandboxConfig {
        host_data_dir: Some(host_data_dir.clone()),
        home_persistence: HomePersistence::Session,
        managed_files_mount: ManagedFilesMount::Rw,
        ..Default::default()
    });
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "apple-volumes".into(),
    };

    assert_eq!(sandbox.volumes(&id).unwrap(), vec![
        format!(
            "{}:/home/sandbox",
            host_data_dir
                .join("sandbox/home/session/apple-volumes")
                .display()
        ),
        format!(
            "{}:{SANDBOX_FILES_DIR}:rw",
            host_data_dir.join("files").display()
        ),
    ]);
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
fn test_apple_container_run_state_from_legacy_inspect() {
    assert_eq!(
        apple_container_run_state_from_inspect(
            r#"[{"id":"abc","status":"running","configuration":{}}]"#
        ),
        Some(ContainerRunState::Running)
    );
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc","status":"stopped"}]"#),
        Some(ContainerRunState::Stopped)
    );
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc","status":"exited"}]"#),
        Some(ContainerRunState::Exited)
    );
}

#[test]
fn test_apple_container_run_state_from_current_inspect() {
    assert_eq!(
        apple_container_run_state_from_inspect(
            r#"[
                {
                    "id": "abc",
                    "status": { "state": "running" },
                    "configuration": {}
                }
            ]"#
        ),
        Some(ContainerRunState::Running)
    );
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc","status":{"state":"stopped"}}]"#),
        Some(ContainerRunState::Stopped)
    );
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc","status":{"state":"exited"}}]"#),
        Some(ContainerRunState::Exited)
    );
}

#[test]
fn test_apple_container_run_state_rejects_missing_or_malformed_inspect() {
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc","status":{"state":"starting"}}]"#),
        Some(ContainerRunState::Unknown)
    );
    assert_eq!(apple_container_run_state_from_inspect("[]"), None);
    assert_eq!(apple_container_run_state_from_inspect(""), None);
    assert_eq!(apple_container_run_state_from_inspect("not json"), None);
    assert_eq!(
        apple_container_run_state_from_inspect(r#"[{"id":"abc"}]"#),
        None
    );
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
    assert_eq!(sandbox.runtime_info(&id).await.backend_name, "docker");
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
