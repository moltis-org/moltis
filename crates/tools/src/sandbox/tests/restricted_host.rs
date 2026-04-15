#![allow(clippy::unwrap_used, clippy::expect_used)]
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
async fn test_restricted_host_sandbox_read_file_native() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "restricted read").unwrap();

    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-read".into(),
    };

    let result = sandbox
        .read_file(&id, &file.display().to_string(), 1024)
        .await
        .unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"restricted read"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[tokio::test]
async fn test_restricted_host_sandbox_write_file_native() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");

    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-write".into(),
    };

    let result = sandbox
        .write_file(&id, &file.display().to_string(), b"restricted write")
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "restricted write");
}

#[tokio::test]
async fn test_restricted_host_sandbox_list_files_native() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let first = dir.path().join("a.txt");
    let second = nested.join("b.txt");
    std::fs::write(&first, "a").unwrap();
    std::fs::write(&second, "b").unwrap();

    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-list".into(),
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
async fn test_restricted_host_sandbox_write_rejects_symlink_native() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&real, "original").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-symlink".into(),
    };

    let result = sandbox
        .write_file(&id, &link.display().to_string(), b"nope")
        .await
        .unwrap();
    let payload = result.expect("expected typed payload");
    assert_eq!(payload["kind"], "path_denied");
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "original");
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
