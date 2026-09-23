#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[cfg(target_os = "macos")]
use super::apple::*;
#[cfg(target_os = "macos")]
use crate::sandbox::file_system::{
    oci_container_list_files, oci_container_read_file, oci_container_write_file,
};
#[cfg(target_os = "macos")]
use std::env;
use {
    super::{containers::*, docker::*, host::*, paths::*, platform::*, router::*, types::*, *},
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
        sandbox::file_system::SandboxReadResult,
    },
};

#[cfg(target_os = "macos")]
const OCI_RUNTIME_E2E_ENV: &str = "MOLTIS_SANDBOX_RUNTIME_E2E";
#[cfg(target_os = "macos")]
const OCI_RUNTIME_E2E_IMAGE: &str = "alpine:3.21";

#[cfg(target_os = "macos")]
fn runtime_container_e2e_enabled(cli: &str) -> bool {
    let requested = env::var(OCI_RUNTIME_E2E_ENV)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes")
        })
        .unwrap_or(false);
    if !requested || !is_cli_available(cli) {
        return false;
    }
    std::process::Command::new(cli)
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
struct RuntimeContainerGuard {
    cli: String,
    name: String,
}

#[cfg(target_os = "macos")]
impl RuntimeContainerGuard {
    async fn start(cli: &str) -> Result<Self> {
        let name = format!("moltis-runtime-e2e-{}", uuid::Uuid::new_v4().simple());
        let output = tokio::process::Command::new(cli)
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &name,
                OCI_RUNTIME_E2E_IMAGE,
                "sleep",
                "600",
            ])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::message(format!(
                "{cli} run failed for runtime e2e container '{name}': {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(Self {
            cli: cli.to_string(),
            name,
        })
    }

    async fn exec(&self, command: &str) -> Result<String> {
        let output = tokio::process::Command::new(&self.cli)
            .args(["exec", &self.name, "sh", "-c", command])
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::message(format!(
                "{} exec failed in runtime e2e container '{}': {}",
                self.cli,
                self.name,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(target_os = "macos")]
impl Drop for RuntimeContainerGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.cli)
            .args(["rm", "-f", &self.name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(target_os = "macos")]
async fn assert_runtime_oci_file_transfers(cli: &str) -> Result<()> {
    let container = RuntimeContainerGuard::start(cli).await?;
    container
        .exec(
            "mkdir -p /tmp/moltis-e2e/list && \
             printf 'hello runtime\\n' > /tmp/moltis-e2e/read.txt && \
             printf 'alpha\\n' > /tmp/moltis-e2e/list/a.txt && \
             printf 'beta\\n' > /tmp/moltis-e2e/list/b.txt",
        )
        .await?;

    let read_result =
        oci_container_read_file(cli, &container.name, "/tmp/moltis-e2e/read.txt", 1024).await?;
    match read_result {
        SandboxReadResult::Ok(bytes) => assert_eq!(bytes, b"hello runtime\n"),
        other => panic!("expected Ok from runtime OCI read, got {other:?}"),
    }

    assert!(
        oci_container_write_file(
            cli,
            &container.name,
            "/tmp/moltis-e2e/write.txt",
            b"written from host"
        )
        .await?
        .is_none()
    );
    let written = container.exec("cat /tmp/moltis-e2e/write.txt").await?;
    assert_eq!(written, "written from host");

    let files = oci_container_list_files(cli, &container.name, "/tmp/moltis-e2e/list").await?;
    assert_eq!(files.files, vec![
        "/tmp/moltis-e2e/list/a.txt".to_string(),
        "/tmp/moltis-e2e/list/b.txt".to_string(),
    ]);
    assert!(!files.truncated);

    Ok(())
}

/// Serialises the tests that move the process-global data dir.
///
/// `moltis_config::set_data_dir` is a process-wide override, so a test that
/// points it at its own tempdir has to be the only one doing so. Poisoning is
/// swallowed the way the config crate handles its own statics, so one panicking
/// test does not take the rest down with it.
static DATA_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point `data_dir()` at a temporary directory for the life of the guard.
///
/// Restores by value rather than through `clear_data_dir`, which is not in
/// `moltis_config`'s re-export list and does not need to be widened for this.
struct DataDirGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: PathBuf,
}

impl DataDirGuard {
    fn new(path: PathBuf) -> Self {
        let lock = DATA_DIR_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = moltis_config::data_dir();
        moltis_config::set_data_dir(path);
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        moltis_config::set_data_dir(self.previous.clone());
    }
}

/// The uid and gid this test process runs as, read back from a directory it
/// just created rather than from `libc`.
#[cfg(unix)]
pub(crate) fn process_uid_gid() -> (u32, u32) {
    use std::os::unix::fs::MetadataExt;

    let probe = tempfile::tempdir().unwrap();
    let metadata = std::fs::metadata(probe.path()).unwrap();
    (metadata.uid(), metadata.gid())
}

/// A `run_as` user whose per-uid HOME this process can actually create.
///
/// Derived from the running process, never hard-coded.
/// `ensure_sandbox_home_persistence_host_dir` creates the per-uid home owned by
/// *this* process at 0755, and `check_writable_by` resolves the POSIX bits the
/// way POSIX does - owner bits or group bits or other bits, never two of them -
/// so a hard-coded `1000` would pass only on a runner that happens to be uid
/// 1000. The `rust-test` job is uid 0 inside a `container:` and uid 501 on
/// `macos-latest`.
///
/// Root is the one uid this cannot simply mirror, because uid 0 is refused by
/// design. A root process gets a stand-in uid instead, and [`open_run_as_home`]
/// opens that uid's home up - which is exactly the work an operator has to do
/// to run a container as a uid the gateway is not.
#[cfg(unix)]
pub(crate) fn test_run_as_user() -> SandboxUser {
    let (uid, gid) = process_uid_gid();
    if uid == 0 {
        return SandboxUser::try_from("1000:1000").unwrap();
    }
    // A non-root process in the root group still cannot spell a `SandboxUser`
    // with gid 0, so fall back to the uid's own value as the gid: the owner
    // bits decide writability here either way.
    let gid = if gid == 0 {
        uid
    } else {
        gid
    };
    SandboxUser::try_from(format!("{uid}:{gid}").as_str()).unwrap()
}

/// Make `home` writable by the uid [`test_run_as_user`] returns.
///
/// A no-op unless this process is root: every other uid owns what it creates.
#[cfg(unix)]
pub(crate) fn open_run_as_home(home: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    if process_uid_gid().0 != 0 {
        return;
    }
    std::fs::create_dir_all(home).unwrap();
    std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o777)).unwrap();
}

fn session_id(key: &str) -> SandboxId {
    SandboxId {
        scope: SandboxScope::Session,
        key: key.into(),
    }
}

#[test]
fn test_ensure_sandbox_home_persistence_host_dir_propagates_guest_visible_create_error() {
    let temp_dir = tempfile::tempdir().unwrap();
    let blocking_file = temp_dir.path().join("blocking-file");
    std::fs::write(&blocking_file, "x").unwrap();
    let config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        shared_home_dir: Some(blocking_file.join("nested")),
        ..Default::default()
    };
    let id = session_id("sess-1");

    let result = ensure_sandbox_home_persistence_host_dir(&config, None, &id, None);
    assert!(result.is_err());
}

#[test]
fn test_ensure_sandbox_home_persistence_host_dir_allows_translated_create_error() {
    // The guest-visible path of this config resolves through `data_dir()`, and
    // the function now creates it, so the data dir has to be the test's own or
    // this would write into the real ~/.moltis.
    let temp_dir = tempfile::tempdir().unwrap();
    let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
    let blocking_file = temp_dir.path().join("blocking-file");
    std::fs::write(&blocking_file, "x").unwrap();
    let config = SandboxConfig {
        host_data_dir: Some(blocking_file.join("host")),
        ..Default::default()
    };
    let id = session_id("sess-1");

    let result = ensure_sandbox_home_persistence_host_dir(&config, Some("docker"), &id, None)
        .unwrap()
        .unwrap();
    assert_eq!(result, blocking_file.join("host/sandbox/home/shared"));
    // The translated path still cannot be created and that is still tolerated,
    // but the guest-visible one - the bind source - now exists, created by us
    // rather than by the container runtime as root.
    let guest_visible =
        guest_visible_sandbox_home_persistence_host_dir(&config, &id, None).unwrap();
    assert!(
        guest_visible.is_dir(),
        "guest-visible bind source must exist: {}",
        guest_visible.display()
    );
}

#[test]
fn guest_visible_dir_is_created_when_host_data_dir_is_set() {
    // The root cause. With `host_data_dir` set the translated path does not
    // exist inside the gateway, so only the guest-visible one can be created
    // here - and if it is not, the docker daemon creates the bind source at
    // first run and it lands root:root inside a data dir the gateway owns.
    let temp_dir = tempfile::tempdir().unwrap();
    let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
    let config = SandboxConfig {
        host_data_dir: Some(temp_dir.path().join("host")),
        ..Default::default()
    };
    let id = session_id("sess-1");

    ensure_sandbox_home_persistence_host_dir(&config, Some("docker"), &id, None).unwrap();

    let guest_visible =
        guest_visible_sandbox_home_persistence_host_dir(&config, &id, None).unwrap();
    assert!(
        guest_visible.is_dir(),
        "the guest-visible bind source must be created, not left to the runtime: {}",
        guest_visible.display()
    );
}

#[cfg(unix)]
#[test]
fn run_as_home_is_per_uid() {
    let temp_dir = tempfile::tempdir().unwrap();
    let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
    let id = session_id("sess-1");
    let user = test_run_as_user();
    let uid = user.uid().to_string();

    // No operator setting: a sibling of `shared`, never a child of it, so
    // nothing a root session already wrote there is inherited.
    let default_config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        ..Default::default()
    };
    open_run_as_home(
        &sandbox_home_persistence_host_dir(&default_config, None, &id, Some(&user)).unwrap(),
    );
    let path =
        ensure_sandbox_home_persistence_host_dir(&default_config, None, &id, Some(&user)).unwrap();
    let path = path.unwrap();
    assert!(
        path.ends_with(format!("user/{uid}")),
        "a run_as home must be per uid, got {}",
        path.display()
    );
    assert_ne!(
        path,
        ensure_sandbox_home_persistence_host_dir(&default_config, None, &id, None)
            .unwrap()
            .unwrap(),
        "a run_as session must never share a HOME with a root session"
    );

    // With an operator setting the per-uid dir hangs under it: honouring that
    // setting is the point, silently overriding it is not.
    let operator_dir = temp_dir.path().join("op");
    let operator_config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        shared_home_dir: Some(operator_dir.clone()),
        ..Default::default()
    };
    open_run_as_home(&operator_dir.join("user").join(&uid));
    let path =
        ensure_sandbox_home_persistence_host_dir(&operator_config, None, &id, Some(&user)).unwrap();
    assert_eq!(path, Some(operator_dir.join("user").join(&uid)));
}

#[test]
fn run_as_home_errors_when_not_writable() {
    // 0o500 denies the write to the owner itself, so this holds whatever uid
    // the test process is - which is the only reason it can assert anything on
    // a root runner, where every other mode bit is advisory.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
        let operator_dir = temp_dir.path().join("op");
        let config = SandboxConfig {
            home_persistence: HomePersistence::Shared,
            shared_home_dir: Some(operator_dir.clone()),
            ..Default::default()
        };
        let id = session_id("sess-1");
        let user = SandboxUser::try_from("1000:1000").unwrap();
        let home = operator_dir.join("user").join("1000");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o500)).unwrap();

        let error = ensure_sandbox_home_persistence_host_dir(&config, None, &id, Some(&user))
            .expect_err("an unwritable home must fail the container start");

        // Restore before the tempdir is dropped, or the cleanup fails.
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            error.to_string().contains("not writable"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn missing_dir_is_created_never_unavailable() {
    // A missing directory is created, never reported as "unavailable" and
    // never silently passed on to the runtime to create as root.
    let temp_dir = tempfile::tempdir().unwrap();
    let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
    let config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        shared_home_dir: Some(temp_dir.path().join("never-created")),
        ..Default::default()
    };
    let id = session_id("sess-1");

    let path = ensure_sandbox_home_persistence_host_dir(&config, None, &id, None)
        .unwrap()
        .expect("a computable home is never None");
    assert!(path.is_dir(), "{} must exist", path.display());
}

struct TestSandbox {
    name: &'static str,
    ensure_ready_error: Option<String>,
    exec_error: Option<String>,
    ensure_ready_calls: AtomicUsize,
    exec_calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
}

impl TestSandbox {
    fn new(name: &'static str, ensure_ready_error: Option<&str>, exec_error: Option<&str>) -> Self {
        Self {
            name,
            ensure_ready_error: ensure_ready_error.map(ToOwned::to_owned),
            exec_error: exec_error.map(ToOwned::to_owned),
            ensure_ready_calls: AtomicUsize::new(0),
            exec_calls: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
        }
    }

    #[cfg(target_os = "macos")]
    fn ensure_ready_calls(&self) -> usize {
        self.ensure_ready_calls.load(Ordering::SeqCst)
    }

    #[cfg(target_os = "macos")]
    fn exec_calls(&self) -> usize {
        self.exec_calls.load(Ordering::SeqCst)
    }
}

#[test]
fn truncate_output_for_display_handles_multibyte_boundary() {
    let mut output = format!("{}л{}", "a".repeat(1999), "z".repeat(10));
    truncate_output_for_display(&mut output, 2000);
    assert!(output.contains("[output truncated]"));
    assert!(!output.contains('л'));
}

#[async_trait::async_trait]
impl Sandbox for TestSandbox {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    fn provides_fs_isolation(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        self.ensure_ready_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref msg) = self.ensure_ready_error {
            return Err(Error::message(msg));
        }
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref msg) = self.exec_error {
            return Err(Error::message(msg));
        }
        Ok(ExecResult {
            stdout: "ok".into(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod apple;
mod core;
mod docker_router;
#[cfg(target_os = "linux")]
mod linux;
mod mounts;
mod network;
mod platform;
mod restricted_host;
mod run_as;
#[cfg(feature = "wasm")]
mod wasm;
