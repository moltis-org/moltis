#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::{test_helpers::MockSandbox, *},
    crate::{
        exec::ExecResult,
        sandbox::{SandboxConfig, types::SandboxScope},
    },
};

#[test]
fn transfer_timeout_scales_with_payload_size() {
    let small = transfer_opts(1024).timeout;
    assert_eq!(
        small, DEFAULT_SANDBOX_TIMEOUT,
        "small writes keep the default"
    );

    let large = transfer_opts(16 * 1024 * 1024).timeout;
    assert!(
        large > DEFAULT_SANDBOX_TIMEOUT,
        "a 16 MB transfer must not inherit the 30s default"
    );
    assert!(large <= Duration::from_secs(600), "timeout stays bounded");
}

#[test]
fn transfer_timeout_is_capped_for_absurd_sizes() {
    assert_eq!(
        transfer_opts(usize::MAX).timeout,
        Duration::from_secs(600),
        "an overflowing size must clamp, not wrap"
    );
}

fn test_id() -> SandboxId {
    SandboxId {
        scope: SandboxScope::Session,
        key: "test".to_string(),
    }
}

struct BlockingFileSystemSandbox {
    exec_entered: tokio::sync::Notify,
    release_exec: tokio::sync::Semaphore,
    cleanup_calls: std::sync::atomic::AtomicUsize,
}

impl Default for BlockingFileSystemSandbox {
    fn default() -> Self {
        Self {
            exec_entered: tokio::sync::Notify::new(),
            release_exec: tokio::sync::Semaphore::new(0),
            cleanup_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Sandbox for BlockingFileSystemSandbox {
    fn backend_name(&self) -> &'static str {
        "blocking-files"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        self.exec_entered.notify_one();
        let _permit = self.release_exec.acquire().await;
        Ok(ExecResult {
            stdout: BASE64.encode(b"contents"),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        self.cleanup_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn read_file_decodes_base64() {
    let encoded = BASE64.encode(b"hello sandbox");
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: encoded,
        stderr: String::new(),
        exit_code: 0,
    }]);
    let fs = CommandSandboxFileSystem::new(mock.clone(), test_id());

    let result = fs.read_file("/data/x.txt", 1024).await.unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"hello sandbox"),
        other => panic!("expected Ok, got {other:?}"),
    }
    assert!(mock.last_command().unwrap().contains("/data/x.txt"));
}

#[tokio::test]
async fn router_cleanup_waits_for_command_file_system_lifetime() {
    let backend = Arc::new(BlockingFileSystemSandbox::default());
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::clone(&backend) as Arc<dyn Sandbox>,
    ));
    let file_system = sandbox_file_system_for_session(&router, "session:active-files")
        .await
        .unwrap();
    let operation_file_system = Arc::clone(&file_system);
    let operation =
        tokio::spawn(async move { operation_file_system.read_file("/tmp/file", 1024).await });
    backend.exec_entered.notified().await;

    let cleanup_router = Arc::clone(&router);
    let mut cleanup =
        tokio::spawn(async move { cleanup_router.cleanup_session("session:active-files").await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut cleanup)
            .await
            .is_err(),
        "cleanup must wait for an active filesystem operation"
    );

    backend.release_exec.add_permits(1);
    assert!(matches!(
        operation.await.unwrap().unwrap(),
        SandboxReadResult::Ok(bytes) if bytes == b"contents"
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut cleanup)
            .await
            .is_err(),
        "cleanup must wait until the filesystem service is dropped"
    );

    drop(file_system);
    cleanup.await.unwrap().unwrap();
    assert_eq!(
        backend
            .cleanup_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn read_file_maps_too_large() {
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: "12345\n".to_string(),
        exit_code: EXIT_TOO_LARGE,
    }]);
    let fs = CommandSandboxFileSystem::new(mock, test_id());

    let result = fs.read_file("/big", 100).await.unwrap();
    assert!(matches!(result, SandboxReadResult::TooLarge(12345)));
}

#[tokio::test]
async fn write_file_encodes_content() {
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let fs = CommandSandboxFileSystem::new(mock.clone(), test_id());

    let result = fs.write_file("/data/out.txt", b"abc").await.unwrap();
    assert!(result.is_none());
    let cmd = mock.last_command().unwrap();
    assert!(cmd.contains("/data/out.txt"));
    assert!(cmd.contains(&BASE64.encode(b"abc")));
    assert!(cmd.contains("sync \"$tmp\""));
}

#[tokio::test]
async fn list_files_reads_find_output() {
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: "/data/a.rs\n/data/b.rs\n".to_string(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let fs = CommandSandboxFileSystem::new(mock, test_id());

    let files = fs.list_files("/data").await.unwrap();
    assert_eq!(files.files, vec!["/data/a.rs", "/data/b.rs"]);
    assert!(!files.truncated);
}

#[test]
fn parse_listed_files_marks_outputs_over_cap_as_truncated() {
    let result = parse_listed_files("/data/a.rs\n/data/b.rs\n/data/c.rs\n", 2);
    assert_eq!(result.files, vec!["/data/a.rs", "/data/b.rs"]);
    assert!(result.truncated);
    assert_eq!(result.limit, Some(2));
}

#[tokio::test]
async fn grep_content_applies_paging() {
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: "/data/lib.rs:3:fn alpha()\n/data/lib.rs:9:fn beta()\n".to_string(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let fs = CommandSandboxFileSystem::new(mock, test_id());

    let value = fs
        .grep(SandboxGrepOptions {
            pattern: "fn".to_string(),
            path: "/data".to_string(),
            mode: SandboxGrepMode::Content,
            case_insensitive: false,
            include_globs: Vec::new(),
            offset: 1,
            head_limit: Some(1),
            match_cap: None,
        })
        .await
        .unwrap();

    assert_eq!(value["mode"], "content");
    assert_eq!(value["truncated"], false);
    let matches = value["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["line"], 9);
}

#[test]
fn build_single_file_tar_round_trips() {
    let tar_bytes = build_single_file_tar("/tmp/example.txt", b"hello tar").unwrap();
    let result = extract_single_file_from_tar(&tar_bytes, "/tmp/example.txt", 1024).unwrap();
    match result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"hello tar"),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn extract_single_file_from_tar_rejects_large_entry() {
    let tar_bytes = build_single_file_tar("/tmp/example.txt", b"hello tar").unwrap();
    let result = extract_single_file_from_tar(&tar_bytes, "/tmp/example.txt", 4).unwrap();
    assert!(matches!(result, SandboxReadResult::TooLarge(9)));
}

#[tokio::test]
async fn oci_read_reports_copy_stderr_when_tar_stream_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let cli_path = dir.path().join("fake-oci");
    std::fs::write(
        &cli_path,
        "#!/bin/sh\n\
         if [ \"$1\" = \"exec\" ]; then printf 'file\\t5\\n'; exit 0; fi\n\
         if [ \"$1\" = \"cp\" ]; then echo 'Error: no such file or directory' >&2; exit 1; fi\n\
         exit 2\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&cli_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&cli_path, permissions).unwrap();
    }

    let result = oci_container_read_file(
        cli_path.to_str().unwrap(),
        "fake-container",
        "/tmp/example.txt",
        1024,
    )
    .await
    .unwrap();

    assert!(matches!(result, SandboxReadResult::NotFound));
}

#[test]
fn oci_copy_failure_detail_is_not_empty_when_stderr_is_empty() {
    assert_eq!(
        container_copy_failure_detail("", Some(1)),
        "copy command exited with code 1 and no stderr"
    );
    assert_eq!(
        container_copy_failure_detail("explicit failure", Some(1)),
        "explicit failure"
    );
}
