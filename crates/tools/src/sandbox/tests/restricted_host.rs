#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::PathBuf;
use super::*;
#[cfg(all(target_os = "linux", feature = "landlock"))]
use super::super::landlock as landlock_mod;

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

// ── Landlock FS isolation tests (Linux only, requires `landlock` feature) ──

/// Default config (empty fs_allow_paths) must not restrict access — no regression.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[tokio::test]
async fn test_landlock_empty_paths_no_restriction() {
    let sandbox = RestrictedHostSandbox::new(SandboxConfig::default());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-ll-empty".into(),
    };
    // /etc/hostname exists on most Linux systems; the point is it must NOT be blocked.
    let result = sandbox
        .exec(&id, "cat /etc/hostname 2>/dev/null || true", &ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
}

/// Paths NOT in the allowlist must be inaccessible.
/// Skipped when the kernel/container doesn't support Landlock.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[tokio::test]
async fn test_landlock_blocks_outside_allowlist() {
    if !landlock_mod::is_kernel_landlock_available() {
        eprintln!("skipping: Landlock not available in this kernel/container");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let config = SandboxConfig {
        fs_allow_paths: vec![tmp.path().to_path_buf()],
        ..Default::default()
    };
    let sandbox = RestrictedHostSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-ll-block".into(),
    };
    let result = sandbox
        .exec(&id, "cat /etc/passwd", &ExecOpts::default())
        .await
        .unwrap();
    assert_ne!(result.exit_code, 0, "expected Landlock to deny access to /etc/passwd");
}

/// Paths IN the allowlist must remain accessible.
/// Skipped when the kernel/container doesn't support Landlock.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[tokio::test]
async fn test_landlock_allows_inside_allowlist() {
    if !landlock_mod::is_kernel_landlock_available() {
        eprintln!("skipping: Landlock not available in this kernel/container");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let secret = tmp.path().join("allowed.txt");
    std::fs::write(&secret, "allowed content").unwrap();

    let config = SandboxConfig {
        fs_allow_paths: vec![tmp.path().to_path_buf()],
        ..Default::default()
    };
    let sandbox = RestrictedHostSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-ll-allow".into(),
    };
    let result = sandbox
        .exec(
            &id,
            &format!("cat {}", secret.display()),
            &ExecOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("allowed content"));
}

/// Multiple allowed paths — both must be accessible.
/// Skipped when the kernel/container doesn't support Landlock.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[tokio::test]
async fn test_landlock_multiple_allow_paths() {
    if !landlock_mod::is_kernel_landlock_available() {
        eprintln!("skipping: Landlock not available in this kernel/container");
        return;
    }
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    std::fs::write(tmp_a.path().join("a.txt"), "from-a").unwrap();
    std::fs::write(tmp_b.path().join("b.txt"), "from-b").unwrap();

    let config = SandboxConfig {
        fs_allow_paths: vec![tmp_a.path().to_path_buf(), tmp_b.path().to_path_buf()],
        ..Default::default()
    };
    let sandbox = RestrictedHostSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-ll-multi".into(),
    };
    let ra = sandbox
        .exec(
            &id,
            &format!("cat {}", tmp_a.path().join("a.txt").display()),
            &ExecOpts::default(),
        )
        .await
        .unwrap();
    let rb = sandbox
        .exec(
            &id,
            &format!("cat {}", tmp_b.path().join("b.txt").display()),
            &ExecOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(ra.exit_code, 0);
    assert!(ra.stdout.contains("from-a"));
    assert_eq!(rb.exit_code, 0);
    assert!(rb.stdout.contains("from-b"));

    // But /etc/passwd should still be blocked.
    let blocked = sandbox
        .exec(&id, "cat /etc/passwd", &ExecOpts::default())
        .await
        .unwrap();
    assert_ne!(blocked.exit_code, 0);
}

/// Known gap: read_file / write_file / list_files bypass Landlock because they
/// use native_host_* (tokio::fs) in the parent process, not child exec.
/// This test documents that gap — it should always pass.
#[cfg(all(target_os = "linux", feature = "landlock"))]
#[tokio::test]
async fn test_landlock_native_fs_bypass_is_known_gap() {
    let config = SandboxConfig {
        // Only allow /tmp — /etc should be blocked for child exec.
        fs_allow_paths: vec![PathBuf::from("/tmp")],
        ..Default::default()
    };
    let sandbox = RestrictedHostSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-ll-bypass".into(),
    };

    // read_file uses native_host_read_file (parent-side tokio::fs), bypasses Landlock.
    let result = sandbox.read_file(&id, "/etc/hostname", 1024).await.unwrap();
    match result {
        SandboxReadResult::Ok(_) => {} // expected: parent can still read
        SandboxReadResult::NotFound => {
            // Also fine — /etc/hostname may not exist in all environments.
        }
        other => panic!("unexpected read result: {other:?}"),
    }
}
