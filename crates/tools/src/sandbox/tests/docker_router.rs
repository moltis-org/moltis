#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;

use super::*;

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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_apple_container_home_read_uses_mounted_host_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let sandbox = AppleContainerSandbox::new(config.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "apple-home-read".into(),
    };
    let guest_file = guest_visible_sandbox_home_persistence_host_dir(&config, &id)
        .unwrap()
        .join("history.txt");
    let host_file = sandbox_home_persistence_host_dir(&config, Some("container"), &id)
        .unwrap()
        .join("history.txt");
    std::fs::create_dir_all(host_file.parent().unwrap()).unwrap();
    std::fs::write(&host_file, "apple mounted read").unwrap();

    let result = sandbox
        .read_file(&id, &guest_file.display().to_string(), 1024)
        .await
        .unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"apple mounted read"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_apple_container_home_write_uses_mounted_host_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let sandbox = AppleContainerSandbox::new(config.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "apple-home-write".into(),
    };
    let guest_file = guest_visible_sandbox_home_persistence_host_dir(&config, &id)
        .unwrap()
        .join("history.txt");
    let host_file = sandbox_home_persistence_host_dir(&config, Some("container"), &id)
        .unwrap()
        .join("history.txt");
    std::fs::create_dir_all(host_file.parent().unwrap()).unwrap();

    let result = sandbox
        .write_file(
            &id,
            &guest_file.display().to_string(),
            b"apple mounted write",
        )
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(
        std::fs::read_to_string(host_file).unwrap(),
        "apple mounted write"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_apple_container_home_list_remaps_mounted_host_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let host_data_dir = temp_dir.path().join("moltis-data");
    let config = SandboxConfig {
        home_persistence: HomePersistence::Session,
        host_data_dir: Some(host_data_dir.clone()),
        ..Default::default()
    };
    let sandbox = AppleContainerSandbox::new(config.clone());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "apple-home-list".into(),
    };
    let guest_root = guest_visible_sandbox_home_persistence_host_dir(&config, &id)
        .unwrap()
        .join("notes");
    let host_root = sandbox_home_persistence_host_dir(&config, Some("container"), &id)
        .unwrap()
        .join("notes");
    std::fs::create_dir_all(host_root.join("nested")).unwrap();
    std::fs::write(host_root.join("todo.txt"), "a").unwrap();
    std::fs::write(host_root.join("nested/done.txt"), "b").unwrap();

    let files = sandbox
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
async fn test_no_sandbox_read_file_native() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "native read").unwrap();

    let sandbox = NoSandbox;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-read".into(),
    };

    let result = sandbox
        .read_file(&id, &file.display().to_string(), 1024)
        .await
        .unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"native read"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test]
async fn test_no_sandbox_write_file_native() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");

    let sandbox = NoSandbox;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-write".into(),
    };

    let result = sandbox
        .write_file(&id, &file.display().to_string(), b"native write")
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "native write");
}

#[tokio::test]
async fn test_no_sandbox_list_files_native() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let first = dir.path().join("a.txt");
    let second = nested.join("b.txt");
    std::fs::write(&first, "a").unwrap();
    std::fs::write(&second, "b").unwrap();

    let sandbox = NoSandbox;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-list".into(),
    };

    let files = sandbox
        .list_files(&id, &dir.path().display().to_string())
        .await
        .unwrap();
    assert_eq!(files.files, vec![
        first.display().to_string(),
        second.display().to_string(),
    ]);
    assert!(!files.truncated);
}

#[cfg(unix)]
#[tokio::test]
async fn test_no_sandbox_write_file_rejects_symlink_native() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&real, "original").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let sandbox = NoSandbox;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-symlink".into(),
    };

    let result = sandbox
        .write_file(&id, &link.display().to_string(), b"nope")
        .await
        .unwrap();
    let payload = result.expect("expected typed payload");
    assert_eq!(payload["kind"], "path_denied");
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "original");
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
fn test_sandbox_image_dockerfile_adds_nodesource_for_nodejs() {
    let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into(), "nodejs".into()]);
    assert!(dockerfile.contains("nodesource.gpg"));
    assert!(dockerfile.contains("node_22.x"));
    // Bootstraps curl+gnupg before using them
    assert!(dockerfile.contains("apt-get install -y -qq curl gnupg"));
    // nodejs should remain in the main apt-get install line
    assert!(dockerfile.contains("nodejs"));
    // npm is superseded by NodeSource nodejs and should be filtered out
    let dockerfile_with_npm = sandbox_image_dockerfile("ubuntu:25.10", &[
        "curl".into(),
        "nodejs".into(),
        "npm".into(),
    ]);
    assert!(!dockerfile_with_npm.contains(" npm "));
}

#[test]
fn test_sandbox_image_dockerfile_no_nodesource_without_nodejs() {
    let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["curl".into(), "git".into()]);
    assert!(!dockerfile.contains("nodesource"));
}

#[test]
fn test_sandbox_image_dockerfile_npm_without_nodejs_kept() {
    // npm without nodejs is a valid config — should not be filtered
    let dockerfile = sandbox_image_dockerfile("ubuntu:25.10", &["npm".into(), "curl".into()]);
    assert!(dockerfile.contains("npm"));
    assert!(!dockerfile.contains("nodesource"));
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

/// E2E regression test for #796: Podman+BuildKit may leave images in
/// BuildKit's cache instead of the Podman store.  Gated behind
/// `MOLTIS_SANDBOX_RUNTIME_E2E=1` and requires Podman to be installed.
#[tokio::test]
async fn test_podman_build_image_exists_in_store() {
    let enabled = env::var("MOLTIS_SANDBOX_RUNTIME_E2E")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if !enabled || !is_cli_available("podman") {
        eprintln!(
            "skipping test_podman_build_image_exists_in_store (set MOLTIS_SANDBOX_RUNTIME_E2E=1 and install podman)"
        );
        return;
    }

    let sandbox = DockerSandbox::podman(SandboxConfig::default());
    let packages = vec!["curl".into()];
    let tag = sandbox_image_tag(sandbox.image_repo(), "ubuntu:25.10", &packages);

    // Remove any pre-existing image so we exercise the full build path.
    let _ = tokio::process::Command::new("podman")
        .args(["rmi", "-f", &tag])
        .output()
        .await;

    let result = sandbox
        .build_image("ubuntu:25.10", &packages)
        .await
        .expect("build_image should succeed");
    let result = result.expect("build_image should return Some for non-empty packages");
    assert_eq!(result.tag, tag);

    // The critical assertion: the image must be in the Podman store.
    assert!(
        sandbox_image_exists("podman", &tag).await,
        "image {tag} must exist in podman store after build_image"
    );

    // Cleanup.
    let _ = tokio::process::Command::new("podman")
        .args(["rmi", "-f", &tag])
        .output()
        .await;
}
