use {super::*, serial_test::serial};

fn clear_container_mount_test_state() {
    TEST_CONTAINER_MOUNT_OVERRIDES
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

fn set_test_container_mount_override(cli: &str, reference: &str, mounts: Vec<ContainerMount>) {
    TEST_CONTAINER_MOUNT_OVERRIDES
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(test_container_mount_override_key(cli, reference), mounts);
}

#[test]
fn test_find_available_port() {
    let port = find_available_port().unwrap();
    assert!(port > 0);
}

#[test]
fn test_new_browser_container_name_prefix() {
    let name = new_browser_container_name("moltis-test-browser");
    assert!(name.starts_with("moltis-test-browser-"));
}

#[test]
fn test_parse_docker_container_names_filters_prefix() {
    let input = b"moltis-test-browser-abc\nother-container\nmoltis-test-browser-def\n";
    let parsed = parse_docker_container_names(input, "moltis-test-browser");
    assert_eq!(parsed, vec![
        "moltis-test-browser-abc".to_string(),
        "moltis-test-browser-def".to_string()
    ]);
}

#[cfg(target_os = "macos")]
#[test]
fn test_parse_apple_container_names_filters_prefix() {
    let json = br#"[
          {"configuration":{"id":"moltis-test-browser-123"}},
          {"configuration":{"id":"not-browser"}},
          {"configuration":{"id":"moltis-test-browser-456"}}
        ]"#;
    let parsed = parse_apple_container_names_for_prefix(json, "moltis-test-browser").unwrap();
    assert_eq!(parsed, vec![
        "moltis-test-browser-123".to_string(),
        "moltis-test-browser-456".to_string()
    ]);
}

#[test]
fn test_is_docker_available() {
    let _ = is_docker_available();
}

#[test]
fn test_is_container_available() {
    let _ = is_container_available();
}

#[test]
fn test_docker_backend_cli() {
    assert_eq!(ContainerBackend::Docker.cli(), "docker");
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_backend_cli() {
    assert_eq!(ContainerBackend::AppleContainer.cli(), "container");
}

#[test]
fn test_detect_backend_returns_some() {
    let result = detect_backend();
    if is_container_available() {
        assert!(result.is_ok());
    } else {
        assert!(result.is_err());
    }
}

#[test]
fn test_build_container_launch_args_without_low_memory() {
    let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
    assert_eq!(args, r#"DEFAULT_LAUNCH_ARGS=["--window-size=1920,1080"]"#);
}

#[test]
fn test_build_container_launch_args_with_profile_dir() {
    let args = build_container_launch_args(
        1920,
        1080,
        0,
        Some("/data/browser-profile"),
        ContainerBackend::Docker,
    );
    assert!(args.contains("--user-data-dir=/data/browser-profile"));
    assert!(args.contains("--window-size=1920,1080"));
}

#[test]
fn test_build_container_launch_args_without_profile_dir() {
    let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
    assert!(!args.contains("--user-data-dir"));
}

#[test]
fn test_parse_container_mounts_from_inspect() {
    let mounts = parse_container_mounts_from_inspect(
        r#"[
                {
                    "Mounts": [
                        {
                            "Source": "/var/lib/docker/volumes/moltis-data/_data",
                            "Destination": "/home/moltis/.moltis"
                        }
                    ]
                }
            ]"#,
    );

    assert_eq!(mounts, vec![ContainerMount {
        source: PathBuf::from("/var/lib/docker/volumes/moltis-data/_data"),
        destination: PathBuf::from("/home/moltis/.moltis"),
    }]);
}

#[test]
fn browser_profile_mount_path_uses_configured_host_data_dir() {
    let guest_profile = moltis_config::data_dir()
        .join("browser")
        .join("profile")
        .join("sandbox")
        .join("browser-abc");

    let mount_dir = profile_mount_dir_for_backend(
        ContainerBackend::Docker,
        &guest_profile,
        Some(Path::new("/host/moltis-data")),
    );

    assert_eq!(
        mount_dir,
        PathBuf::from("/host/moltis-data/browser/profile/sandbox/browser-abc")
    );
}

#[test]
fn browser_profile_mount_path_ignores_relative_host_data_dir() {
    let guest_profile = moltis_config::data_dir()
        .join("browser")
        .join("profile")
        .join("sandbox")
        .join("browser-relative");

    let mount_dir = profile_mount_dir_for_backend(
        ContainerBackend::Docker,
        &guest_profile,
        Some(Path::new("relative-host-data")),
    );

    assert_eq!(mount_dir, guest_profile);
}

#[test]
#[serial(browser_container_mount_overrides)]
fn browser_profile_mount_path_auto_detects_host_data_dir() {
    clear_container_mount_test_state();
    let guest_data_dir = moltis_config::data_dir();
    set_test_container_mount_override("docker", "parent-container", vec![ContainerMount {
        source: PathBuf::from("/var/lib/docker/volumes/moltis-data/_data"),
        destination: guest_data_dir.clone(),
    }]);
    let guest_profile = guest_data_dir
        .join("browser")
        .join("profile")
        .join("sandbox")
        .join("browser-issue-977");

    let mount_dir =
        host_visible_path_with_references("docker", None, &guest_profile, &[String::from(
            "parent-container",
        )]);

    assert_eq!(
        mount_dir,
        PathBuf::from(
            "/var/lib/docker/volumes/moltis-data/_data/browser/profile/sandbox/browser-issue-977"
        )
    );
}

#[test]
fn browser_profile_mount_path_keeps_custom_paths_outside_data_dir() {
    let mount_dir = profile_mount_dir_for_backend(
        ContainerBackend::Docker,
        Path::new("/custom/browser/profile"),
        Some(Path::new("/host/moltis-data")),
    );

    assert_eq!(mount_dir, PathBuf::from("/custom/browser/profile"));
}

#[test]
fn browser_profile_precreate_uses_guest_path_when_mount_is_translated() {
    let guest_dir = Path::new("/home/moltis/.moltis/browser/profile/sandbox/browser-abc");
    let mount_dir = Path::new("/host/moltis-data/browser/profile/sandbox/browser-abc");

    assert_eq!(
        profile_precreate_dir(Some(guest_dir), Some(mount_dir)),
        Some(guest_dir)
    );
}

#[test]
fn browser_profile_precreate_skips_untranslated_mount() {
    let guest_dir = Path::new("/custom/browser/profile");

    assert_eq!(
        profile_precreate_dir(Some(guest_dir), Some(guest_dir)),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_build_container_launch_args_apple_container_has_disable_shm() {
    let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::AppleContainer);
    assert!(args.contains("--disable-dev-shm-usage"));
    assert!(args.contains("--window-size=1920,1080"));
}

#[test]
fn test_build_container_launch_args_docker_no_disable_shm() {
    let args = build_container_launch_args(1920, 1080, 0, None, ContainerBackend::Docker);
    assert!(!args.contains("--disable-dev-shm-usage"));
}

#[test]
fn test_browserless_session_timeout_uses_moltis_lifecycle_floor() {
    let timeout_ms = browserless_session_timeout_ms(300, 30_000, 1800);
    assert_eq!(timeout_ms, 1_800_000);
}

#[test]
fn test_browserless_session_timeout_caps_at_max_lifetime() {
    let timeout_ms = browserless_session_timeout_ms(3_600, 30_000, 1800);
    assert_eq!(timeout_ms, 1_800_000);
}

#[test]
fn test_browserless_session_timeout_caps_large_navigation_timeout() {
    let timeout_ms = browserless_session_timeout_ms(60, 3_900_000, 1800);
    assert_eq!(timeout_ms, 1_800_000);
}

#[test]
fn test_browserless_session_timeout_nav_within_ceiling() {
    let timeout_ms = browserless_session_timeout_ms(60, 600_000, 1800);
    assert_eq!(timeout_ms, 1_800_000);
}

#[test]
fn test_browserless_container_env_includes_timeout() {
    let env = browserless_container_env(1_800_000);
    assert_eq!(env[0], "TIMEOUT=1800000");
    assert!(env.contains(&"MAX_CONCURRENT_SESSIONS=1".to_string()));
    assert!(env.contains(&"PREBOOT_CHROME=true".to_string()));
}
