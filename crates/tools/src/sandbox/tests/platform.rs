#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

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
