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

// ── Sandbox escape regression tests (issue #923) ───────────────────────────

#[test]
fn test_restricted_host_sandbox_does_not_provide_fs_isolation() {
    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    assert!(
        !sandbox.provides_fs_isolation(),
        "restricted-host must NOT claim filesystem isolation"
    );
}

#[test]
fn test_check_restricted_host_path_allows_data_dir() {
    let data = moltis_config::data_dir().join("notes/todo.txt");
    check_restricted_host_path(&data.display().to_string()).unwrap();
}

#[test]
fn test_check_restricted_host_path_allows_tmp() {
    check_restricted_host_path("/tmp/sandbox-work/out.txt").unwrap();
}

#[test]
fn test_check_restricted_host_path_blocks_etc_passwd() {
    let result = check_restricted_host_path("/etc/passwd");
    assert!(result.is_err(), "must block /etc/passwd");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("outside the allowed directories"));
}

#[test]
fn test_check_restricted_host_path_blocks_dot_dot_traversal() {
    // /tmp/../etc/passwd normalizes to /etc/passwd — must be blocked.
    let result = check_restricted_host_path("/tmp/../etc/passwd");
    assert!(result.is_err(), "must block /tmp/../etc/passwd traversal");
}

#[test]
fn test_check_restricted_host_path_blocks_nested_traversal() {
    let result = check_restricted_host_path("/tmp/a/b/../../../etc/shadow");
    assert!(result.is_err(), "must block nested .. traversal");
}

#[test]
fn test_check_restricted_host_path_allows_dot_dot_within_tmp() {
    // /tmp/a/../b stays within /tmp — should be allowed.
    check_restricted_host_path("/tmp/a/../b/file.txt").unwrap();
}

#[test]
fn test_check_restricted_host_path_blocks_home_ssh() {
    let result = check_restricted_host_path("/home/user/.ssh/id_rsa");
    assert!(result.is_err(), "must block ~/.ssh");
}

#[test]
fn test_check_restricted_host_path_blocks_root_dir() {
    let result = check_restricted_host_path("/root/.bashrc");
    assert!(result.is_err(), "must block /root");
}

#[tokio::test]
async fn test_restricted_host_read_blocks_etc_passwd() {
    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-block-read".into(),
    };
    let result = sandbox.read_file(&id, "/etc/passwd", 4096).await;
    assert!(result.is_err(), "read_file must block /etc/passwd");
}

#[tokio::test]
async fn test_restricted_host_write_blocks_outside_allowlist() {
    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-block-write".into(),
    };
    let result = sandbox.write_file(&id, "/var/log/evil.txt", b"nope").await;
    assert!(result.is_err(), "write_file must block /var/log");
}

#[tokio::test]
async fn test_restricted_host_list_blocks_outside_allowlist() {
    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-block-list".into(),
    };
    let result = sandbox.list_files(&id, "/etc").await;
    assert!(result.is_err(), "list_files must block /etc");
}

#[tokio::test]
async fn test_restricted_host_grep_blocks_outside_allowlist() {
    use crate::sandbox::file_system::{SandboxGrepMode, SandboxGrepOptions};

    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-rh-block-grep".into(),
    };
    let result = sandbox
        .grep(&id, SandboxGrepOptions {
            pattern: "root".to_string(),
            path: "/etc".to_string(),
            mode: SandboxGrepMode::Content,
            case_insensitive: false,
            include_globs: Vec::new(),
            offset: 0,
            head_limit: None,
            match_cap: None,
        })
        .await;
    assert!(result.is_err(), "grep must block /etc");
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
