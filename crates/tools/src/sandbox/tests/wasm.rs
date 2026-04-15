#![allow(clippy::unwrap_used, clippy::expect_used)]
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
