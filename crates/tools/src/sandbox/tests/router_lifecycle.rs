#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::*;

struct BlockingPrepareSandbox {
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
    release_first: tokio::sync::Semaphore,
    images: Mutex<Vec<String>>,
}

impl Default for BlockingPrepareSandbox {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            release_first: tokio::sync::Semaphore::new(0),
            images: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Sandbox for BlockingPrepareSandbox {
    fn backend_name(&self) -> &'static str {
        "blocking"
    }

    async fn ensure_ready(&self, _id: &SandboxId, image: Option<&str>) -> Result<()> {
        self.images
            .lock()
            .unwrap()
            .push(image.unwrap_or_default().to_string());
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.notify_one();
            let _permit = self.release_first.acquire().await;
        }
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        Ok(ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

#[test]
fn sandbox_ids_unconditionally_hash_original_keys() {
    let router = SandboxRouter::with_backend(SandboxConfig::default(), Arc::new(NoSandbox));
    let slash = router.sandbox_id_for("session:a/b");
    let colon = router.sandbox_id_for("session:a:b");
    let safe = router.sandbox_id_for("main");
    let literal_safe_collision = router.sandbox_id_for(&slash.key);

    assert_ne!(slash.key, colon.key);
    assert_ne!(slash.key, literal_safe_collision.key);
    assert_eq!(slash.key, router.sandbox_id_for("session:a/b").key);
    for id in [&slash, &colon, &safe, &literal_safe_collision] {
        assert!(
            id.key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
        );
    }
    assert!(slash.key.starts_with("session-a-b-"));
    assert!(colon.key.starts_with("session-a-b-"));
    assert!(safe.key.starts_with("main-"));
    assert_eq!(slash.key.len(), "session-a-b-".len() + 12);
    assert_eq!(colon.key.len(), "session-a-b-".len() + 12);
    assert_eq!(safe.key.len(), "main-".len() + 12);
    assert!(
        literal_safe_collision
            .key
            .starts_with(&format!("{}-", slash.key))
    );
}

#[tokio::test]
async fn cancelled_prepare_owner_releases_future_callers() {
    let backend = Arc::new(BlockingPrepareSandbox::default());
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::clone(&backend) as Arc<dyn Sandbox>,
    ));
    let owner_router = Arc::clone(&router);
    let owner = tokio::spawn(async move {
        owner_router
            .prepare_session("session:cancelled", None)
            .await
    });
    backend.entered.notified().await;
    owner.abort();
    match owner.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("aborted preparation unexpectedly completed"),
    }

    tokio::time::timeout(
        std::time::Duration::from_millis(250),
        router.prepare_session("session:cancelled", None),
    )
    .await
    .expect("a cancelled owner must release the session permit")
    .unwrap();
    assert!(router.is_prepared("session:cancelled").await);
}

#[tokio::test]
async fn image_override_waits_for_prepare_and_invalidates_old_readiness() {
    let backend = Arc::new(BlockingPrepareSandbox::default());
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::clone(&backend) as Arc<dyn Sandbox>,
    ));
    let prepare_router = Arc::clone(&router);
    let prepare = tokio::spawn(async move {
        prepare_router
            .prepare_session("session:override-race", None)
            .await
    });
    backend.entered.notified().await;

    let setter_router = Arc::clone(&router);
    let mut setter = tokio::spawn(async move {
        setter_router
            .set_image_override("session:override-race", "replacement:image".into())
            .await;
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut setter)
            .await
            .is_err(),
        "the override must serialize behind active preparation"
    );

    backend.release_first.add_permits(1);
    prepare.await.unwrap().unwrap();
    setter.await.unwrap();
    assert!(!router.is_prepared("session:override-race").await);

    router
        .prepare_session("session:override-race", None)
        .await
        .unwrap();
    assert_eq!(backend.images.lock().unwrap().as_slice(), [
        DEFAULT_SANDBOX_IMAGE,
        "replacement:image"
    ]);
}

#[derive(Default)]
struct SyncOutRetrySandbox {
    sync_out_checks: AtomicUsize,
    cleanup_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Sandbox for SyncOutRetrySandbox {
    fn backend_name(&self) -> &'static str {
        "sync-out-retry"
    }

    fn is_isolated(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        let (stdout, stderr, exit_code) = if command.starts_with("if [ -d") {
            self.sync_out_checks.fetch_add(1, Ordering::SeqCst);
            ("non-empty".into(), String::new(), 0)
        } else if command.starts_with("tar -czf") {
            (String::new(), "remote transfer failed".into(), 1)
        } else {
            (String::new(), String::new(), 0)
        };
        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn normal_cleanup_preserves_state_on_sync_failure_and_force_discards() {
    let workspace = tempfile::tempdir().unwrap();
    let backend = Arc::new(SyncOutRetrySandbox::default());
    let router = SandboxRouter::with_backend(
        SandboxConfig {
            mode: SandboxMode::Off,
            shared_home_dir: Some(workspace.path().to_path_buf()),
            ..Default::default()
        },
        Arc::clone(&backend) as Arc<dyn Sandbox>,
    );
    let session = "session:durable";
    router.set_override(session, true).await;
    router
        .set_image_override(session, "custom:image".into())
        .await;

    let error = router.cleanup_session(session).await.unwrap_err();
    assert!(error.to_string().contains("remote transfer failed"));
    assert_eq!(backend.cleanup_calls.load(Ordering::SeqCst), 0);
    assert!(router.is_sandboxed(session).await);
    assert_eq!(
        router.resolve_image_nowait(session, None).await,
        "custom:image"
    );

    router.force_cleanup_session(session).await.unwrap();
    assert_eq!(backend.sync_out_checks.load(Ordering::SeqCst), 1);
    assert_eq!(backend.cleanup_calls.load(Ordering::SeqCst), 1);
    assert!(!router.is_sandboxed(session).await);
    assert_ne!(
        router.resolve_image_nowait(session, None).await,
        "custom:image"
    );
}

#[tokio::test]
async fn successful_cleanup_retires_preparation_permits() {
    let router = SandboxRouter::with_backend(SandboxConfig::default(), Arc::new(NoSandbox));
    let session = "session:retire-permit";

    drop(router.prepare_session(session, None).await.unwrap());
    assert_eq!(router.preparation_permit_count().await, 1);
    router.cleanup_session(session).await.unwrap();
    assert_eq!(router.preparation_permit_count().await, 0);

    drop(router.prepare_session(session, None).await.unwrap());
    assert_eq!(router.preparation_permit_count().await, 1);
    router.force_cleanup_session(session).await.unwrap();
    assert_eq!(router.preparation_permit_count().await, 0);
}

#[test]
fn explicit_coder_without_template_fails_closed() {
    let router = SandboxRouter::new(SandboxConfig {
        backend: "coder".into(),
        coder_url: Some("https://coder.example.com".into()),
        coder_token: Some(secrecy::Secret::new("token".into())),
        ..Default::default()
    });

    assert_eq!(router.backend_name(), "coder");
    assert!(!router.backend().is_real());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let error = router.prepare_session("main", None).await.err().unwrap();
        assert!(error.to_string().contains("CODER_TEMPLATE_ID"));
    });
}
