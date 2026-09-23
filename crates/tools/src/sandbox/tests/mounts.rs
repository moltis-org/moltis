//! Per-agent extra bind mounts: argument assembly, the rules that gate them,
//! and the router/failover delegation that must not drop them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

pub(super) fn mount_config(
    source: &str,
    target: &str,
    access: SandboxMountAccess,
) -> SandboxMountConfig {
    SandboxMountConfig {
        source: source.to_string(),
        target: target.to_string(),
        access,
    }
}

#[test]
fn test_docker_extra_mount_args_emits_rw_and_ro_triples() {
    let mounts = SandboxMount::try_from_configs(&[
        mount_config("/srv/vault", "/home/sandbox/vault", SandboxMountAccess::Rw),
        mount_config("/srv/docs", "/home/sandbox/docs", SandboxMountAccess::Ro),
    ])
    .unwrap();

    let args = DockerSandbox::extra_mount_args(&mounts).unwrap();

    assert_eq!(args, vec![
        "-v",
        "/srv/vault:/home/sandbox/vault:rw",
        "-v",
        "/srv/docs:/home/sandbox/docs:ro",
    ]);
}

#[test]
fn test_docker_extra_mount_args_is_empty_without_mounts() {
    assert!(DockerSandbox::extra_mount_args(&[]).unwrap().is_empty());
}

#[test]
fn test_docker_extra_mount_args_rejects_relative_source() {
    // A relative source is the named-volume trap: Docker silently creates a
    // named volume instead of binding the host path, so the agent gets an
    // empty directory and no error. It has to fail before the container runs.
    // Built through the test-only escape hatch on purpose: production code
    // cannot spell this value at all, and the point of the check in
    // `extra_mount_args` is that it holds even for a value that skipped the
    // rules.
    let mounts = vec![SandboxMount::from_parts_unchecked(
        "vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Ro,
    )];

    let error = DockerSandbox::extra_mount_args(&mounts)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("absolute"),
        "a relative source must be rejected, got: {error}"
    );
    assert!(
        SandboxMount::try_from_configs(&[mount_config(
            "vault",
            "/home/sandbox/vault",
            SandboxMountAccess::Ro,
        )])
        .is_err(),
        "try_from_configs must reject a relative source too"
    );
}

#[test]
fn test_sandbox_mount_rejects_moltis_owned_targets() {
    // `data_dir()` below is the process-global the sibling tests move around
    // with `DataDirGuard`, so read it under the same lock they take - without
    // it this builds a target from one tempdir while another test points the
    // override somewhere else.
    let _data_dir = DATA_DIR_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Shadowing one of these does not look like a bad config from inside the
    // container, it looks like a broken sandbox.
    for target in [
        SANDBOX_HOME_DIR.to_string(),
        SANDBOX_FILES_DIR.to_string(),
        MOLTIS_CTL_GUEST_PATH.to_string(),
        moltis_config::data_dir().display().to_string(),
    ] {
        let result = SandboxMount::try_from_configs(&[mount_config(
            "/srv/vault",
            &target,
            SandboxMountAccess::Rw,
        )]);
        let error = result
            .expect_err(&format!("target {target} must be rejected as reserved"))
            .to_string();
        assert!(
            error.contains("reserved"),
            "unexpected error for {target}: {error}"
        );
    }
}

#[test]
fn test_sandbox_mount_rejects_duplicate_targets() {
    // The set-level rule a per-mount TryFrom structurally cannot see: each
    // mount on its own is fine, the pair is not.
    let configs = [
        mount_config("/srv/one", "/home/sandbox/vault", SandboxMountAccess::Ro),
        mount_config("/srv/two", "/home/sandbox/vault", SandboxMountAccess::Rw),
    ];
    assert!(
        SandboxMount::try_from(&configs[0]).is_ok(),
        "each mount is valid on its own"
    );
    assert!(SandboxMount::try_from(&configs[1]).is_ok());

    let error = SandboxMount::try_from_configs(&configs)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("share the target"),
        "duplicate targets must be rejected by the set-level check, got: {error}"
    );
}

#[test]
fn test_container_policy_fingerprint_tracks_extra_mounts() {
    let docker = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Ro,
        ..Default::default()
    });
    let vault_ro = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Ro,
    )])
    .unwrap();
    let vault_rw = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();
    let docs_ro = SandboxMount::try_from_configs(&[mount_config(
        "/srv/docs",
        "/home/sandbox/docs",
        SandboxMountAccess::Ro,
    )])
    .unwrap();

    // An unchanged mount set keeps the container: same hash.
    assert_eq!(
        docker.container_policy_fingerprint(&vault_ro, None),
        docker.container_policy_fingerprint(
            &SandboxMount::try_from_configs(&[mount_config(
                "/srv/vault",
                "/home/sandbox/vault",
                SandboxMountAccess::Ro,
            )])
            .unwrap(),
            None
        )
    );

    // Any change to the set means the running container is the wrong one.
    assert_ne!(
        docker.container_policy_fingerprint(&vault_ro, None),
        docker.container_policy_fingerprint(&docs_ro, None),
        "a different mount must change the fingerprint"
    );
    assert_ne!(
        docker.container_policy_fingerprint(&vault_ro, None),
        docker.container_policy_fingerprint(&vault_rw, None),
        "a changed access mode must change the fingerprint"
    );
    assert_ne!(
        docker.container_policy_fingerprint(&[], None),
        docker.container_policy_fingerprint(&vault_ro, None),
        "adding a mount must change the fingerprint"
    );
}

/// A backend that records what `ensure_ready_with` actually handed it.
///
/// Asserting on the recorded mounts is the only way to see a delegation that
/// drops the options: a dropped option set still returns `Ok`.
pub(super) struct MountRecordingSandbox {
    name: &'static str,
    ensure_ready_error: Option<String>,
    exec_error: Option<String>,
    seen: std::sync::Mutex<Vec<Vec<String>>>,
    seen_run_as: std::sync::Mutex<Vec<Option<String>>>,
}

impl MountRecordingSandbox {
    pub(super) fn new(name: &'static str, ensure_ready_error: Option<&str>) -> Self {
        Self {
            name,
            ensure_ready_error: ensure_ready_error.map(ToOwned::to_owned),
            exec_error: None,
            seen: std::sync::Mutex::new(Vec::new()),
            seen_run_as: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Make `exec` fail with `message`, so a `FailoverSandbox` in front of this
    /// backend takes its exec-time recovery path.
    pub(super) fn failing_exec(name: &'static str, message: &str) -> Self {
        Self {
            exec_error: Some(message.to_string()),
            ..Self::new(name, None)
        }
    }

    pub(super) fn seen(&self) -> Vec<Vec<String>> {
        self.seen.lock().unwrap().clone()
    }

    pub(super) fn seen_run_as(&self) -> Vec<Option<String>> {
        self.seen_run_as.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Sandbox for MountRecordingSandbox {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    fn supports_extra_mounts(&self) -> bool {
        true
    }

    fn supports_run_as(&self) -> bool {
        true
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        self.ensure_ready_with(id, EnsureReadyOpts {
            image_override,
            ..Default::default()
        })
        .await
    }

    async fn ensure_ready_with(&self, _id: &SandboxId, opts: EnsureReadyOpts<'_>) -> Result<()> {
        self.seen
            .lock()
            .unwrap()
            .push(opts.extra_mounts.iter().map(SandboxMount::to_arg).collect());
        self.seen_run_as
            .lock()
            .unwrap()
            .push(opts.run_as.map(|user| user.to_arg()));
        match self.ensure_ready_error {
            Some(ref message) => Err(Error::message(message)),
            None => Ok(()),
        }
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        if let Some(ref message) = self.exec_error {
            return Err(Error::message(message));
        }
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

#[tokio::test]
async fn test_router_agent_sandbox_policy_set_resolve_remove() {
    let router = SandboxRouter::new(SandboxConfig::default());

    assert!(router.resolve_agent_sandbox("session:a").await.is_none());
    assert!(
        router
            .resolve_agent_mounts("session:a")
            .await
            .unwrap()
            .is_empty(),
        "a session with no policy asks for no mounts"
    );

    let policy = AgentSandboxPolicy {
        mounts: vec![mount_config(
            "/srv/vault",
            "/home/sandbox/vault",
            SandboxMountAccess::Rw,
        )],
        ..Default::default()
    };
    router.set_agent_sandbox("session:a", policy.clone()).await;

    assert_eq!(
        router.resolve_agent_sandbox("session:a").await,
        Some(policy)
    );
    let mounts = router.resolve_agent_mounts("session:a").await.unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].to_arg(), "/srv/vault:/home/sandbox/vault:rw");
    assert!(
        router.resolve_agent_sandbox("session:b").await.is_none(),
        "the policy is per session, not global"
    );

    router.remove_agent_sandbox("session:a").await;
    assert!(router.resolve_agent_sandbox("session:a").await.is_none());
    assert!(
        router
            .resolve_agent_mounts("session:a")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_force_flag_beats_a_session_override() {
    // The other half of the rule: an agent that says it may never run outside
    // a sandbox keeps its container even when the session was switched to
    // direct, whatever the global mode says.
    let router = docker_router::router_with_real_backend(SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    });
    router.set_override("session:walter", false).await;

    router
        .set_agent_sandbox("session:walter", AgentSandboxPolicy {
            force: true,
            ..Default::default()
        })
        .await;

    assert!(
        router.is_sandboxed("session:walter").await,
        "force must pin the sandbox on despite sandboxEnabled=false"
    );

    router.remove_agent_sandbox("session:walter").await;
    assert!(
        !router.is_sandboxed("session:walter").await,
        "dropping the policy hands the decision back to the session override"
    );
}

#[tokio::test]
async fn test_declared_mounts_alone_do_not_force_the_sandbox() {
    // Mounts live in the `[sandbox]` block: they configure the sandbox for the
    // turns that run in one, they are not a request to have one. Only an
    // explicit `force` says "this agent may never run outside a sandbox", so a
    // session that was switched to direct stays direct.
    let router = docker_router::router_with_real_backend(SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    });
    router.set_override("session:walter", false).await;

    router
        .set_agent_sandbox("session:walter", AgentSandboxPolicy {
            mounts: vec![mount_config(
                "/srv/vault",
                "/home/sandbox/vault",
                SandboxMountAccess::Ro,
            )],
            ..Default::default()
        })
        .await;

    assert!(
        !router.is_sandboxed("session:walter").await,
        "mounts configure the sandbox, they must not force it on"
    );
}

#[tokio::test]
async fn test_router_resolve_agent_mounts_rejects_an_invalid_set() {
    // The stored shape is config, not runtime mounts, so this is the one seat
    // that converts. A bad set must fail the call, never resolve to nothing.
    let router = SandboxRouter::new(SandboxConfig::default());
    router
        .set_agent_sandbox("session:bad", AgentSandboxPolicy {
            mounts: vec![
                mount_config("/srv/one", "/home/sandbox/vault", SandboxMountAccess::Ro),
                mount_config("/srv/two", "/home/sandbox/vault", SandboxMountAccess::Rw),
            ],
            ..Default::default()
        })
        .await;

    let error = router
        .resolve_agent_mounts("session:bad")
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("share the target"),
        "a duplicate target must fail the resolve, got: {error}"
    );
}

#[tokio::test]
async fn test_ensure_ready_with_errors_when_backend_cannot_mount() {
    // A container quietly missing the paths its agent was told it has is
    // worse than a container that never started.
    let sandbox = TestSandbox::new("test-backend", None, None);
    assert!(!sandbox.supports_extra_mounts());
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "no-mount-support".into(),
    };
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();

    let error = sandbox
        .ensure_ready_with(&id, EnsureReadyOpts {
            image_override: None,
            extra_mounts: &mounts,
            run_as: None,
        })
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("cannot bind extra mounts"),
        "unexpected error: {error}"
    );
    assert_eq!(
        sandbox.ensure_ready_calls.load(Ordering::SeqCst),
        0,
        "the refusal must come before the container is started"
    );

    // Without mounts the same backend is untouched by any of this.
    sandbox
        .ensure_ready_with(&id, EnsureReadyOpts::default())
        .await
        .unwrap();
    assert_eq!(sandbox.ensure_ready_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_failover_sandbox_forwards_extra_mounts_to_the_active_backend() {
    let primary = Arc::new(MountRecordingSandbox::new(
        "docker",
        Some("cannot connect to the docker daemon"),
    ));
    let fallback = Arc::new(MountRecordingSandbox::new("restricted-host", None));
    let primary_backend: Arc<dyn Sandbox> = Arc::clone(&primary) as Arc<dyn Sandbox>;
    let fallback_backend: Arc<dyn Sandbox> = Arc::clone(&fallback) as Arc<dyn Sandbox>;
    let failover = FailoverSandbox::new(primary_backend, fallback_backend);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "failover-mounts".into(),
    };
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();

    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            image_override: None,
            extra_mounts: &mounts,
            run_as: None,
        })
        .await
        .unwrap();

    let expected = vec![vec!["/srv/vault:/home/sandbox/vault:rw".to_string()]];
    assert_eq!(primary.seen(), expected, "the primary must see the mounts");
    assert_eq!(
        fallback.seen(),
        expected,
        "the fallback must see the same mounts; dropping them on delegation \
         starts a container without the agent's paths"
    );
}

#[tokio::test]
async fn test_failover_sandbox_reports_extra_mount_support_of_active_backend() {
    let primary = Arc::new(MountRecordingSandbox::new(
        "docker",
        Some("cannot connect to the docker daemon"),
    ));
    let fallback: Arc<dyn Sandbox> = Arc::new(TestSandbox::new("restricted-host", None, None));
    let failover = FailoverSandbox::new(primary, fallback);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "failover-mount-support".into(),
    };

    assert!(
        failover.supports_extra_mounts(),
        "before failover the primary is active and it does support mounts"
    );

    failover.ensure_ready(&id, None).await.unwrap();

    assert_eq!(failover.backend_name(), "restricted-host");
    assert!(
        !failover.supports_extra_mounts(),
        "after failover the answer must come from the backend that would run \
         the container, not from the primary"
    );
}

#[test]
fn test_sandbox_mount_rejects_ancestors_of_reserved_targets() {
    // Exact-match was the whole rule, so `/home` shadowed `/home/sandbox` just
    // as thoroughly and passed. From inside the container the two are
    // indistinguishable: a broken sandbox, not a bad config.
    for target in ["/home", "/usr/local/bin", "/usr", "/usr/local"] {
        let error = SandboxMount::try_from_configs(&[mount_config(
            "/srv/vault",
            target,
            SandboxMountAccess::Rw,
        )])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("reserved by moltis"),
            "target {target:?} shadows a moltis-owned path and must be refused, got: {error}"
        );
    }
}

#[test]
fn test_sandbox_mount_reserved_targets_are_matched_after_normalization() {
    // `/home/sandbox/`, `//home/sandbox` and `/home/./sandbox` are one
    // directory to the container runtime, and only the bare spelling used to be
    // refused.
    for target in [
        "/home/sandbox/",
        "//home/sandbox",
        "/home/./sandbox",
        "/home/sandbox//",
    ] {
        let error = SandboxMount::try_from_configs(&[mount_config(
            "/srv/vault",
            target,
            SandboxMountAccess::Rw,
        )])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("reserved by moltis"),
            "target {target:?} is /home/sandbox and must be refused, got: {error}"
        );
    }
}

#[test]
fn test_sandbox_mount_rejects_sources_that_resolve_to_root() {
    // `/.` and `//` are `/` to Docker, so binding either hands the whole host
    // filesystem to the container while passing a rule that only refused `/`.
    for source in ["/", "//", "/.", "/./", "///."] {
        let error = SandboxMount::try_from_configs(&[mount_config(
            source,
            "/home/sandbox/vault",
            SandboxMountAccess::Rw,
        )])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("must not resolve to"),
            "source {source:?} resolves to / and must be refused, got: {error}"
        );
    }
}

#[test]
fn test_sandbox_mount_duplicate_targets_are_matched_after_normalization() {
    let error = SandboxMount::try_from_configs(&[
        mount_config("/srv/a", "/srv/shared", SandboxMountAccess::Rw),
        mount_config("/srv/b", "/srv/shared/", SandboxMountAccess::Ro),
    ])
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("share the target"),
        "a trailing slash must not buy a second mount at the same target: {error}"
    );
}

#[test]
fn test_sandbox_mount_normalizes_the_argument_it_emits() {
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "//srv//vault/",
        "/home/sandbox/./vault/",
        SandboxMountAccess::Ro,
    )])
    .unwrap();
    assert_eq!(mounts[0].to_arg(), "/srv/vault:/home/sandbox/vault:ro");
    assert_eq!(mounts[0].source(), "/srv/vault");
    assert_eq!(mounts[0].target(), "/home/sandbox/vault");
    assert_eq!(mounts[0].access(), SandboxMountAccess::Ro);
}

#[tokio::test]
async fn test_failover_sandbox_exec_recovery_replays_the_extra_mounts() {
    // The recovery path inside `exec`, which no other test reaches: the
    // failover tests above all go through `ensure_ready_with`, which has the
    // options in hand. `exec` does not, so it called `ensure_ready(id, None)`
    // and the fallback came up without the agent's paths - silently, because
    // `None` also skips `check_ensure_ready_opts`.
    let primary = Arc::new(MountRecordingSandbox::failing_exec(
        "docker",
        "Cannot connect to the Docker daemon",
    ));
    let fallback = Arc::new(MountRecordingSandbox::new("podman", None));
    let failover = FailoverSandbox::new(
        Arc::clone(&primary) as Arc<dyn Sandbox>,
        Arc::clone(&fallback) as Arc<dyn Sandbox>,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();

    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            extra_mounts: &mounts,
            ..Default::default()
        })
        .await
        .unwrap();
    failover
        .exec(&id, "true", &ExecOpts::default())
        .await
        .unwrap();

    assert_eq!(
        fallback.seen(),
        vec![vec!["/srv/vault:/home/sandbox/vault:rw".to_string()]],
        "the fallback must be made ready with the agent's mounts, not without them"
    );
}

#[tokio::test]
async fn test_failover_sandbox_exec_recovery_refuses_an_incapable_fallback() {
    // The other half of replaying the options: they carry
    // `check_ensure_ready_opts` with them. A fallback that cannot bind the
    // mounts has to fail the turn, because the alternative is the silently
    // pathless container the refusal exists to prevent - and with `None` it
    // could not even be asked.
    let primary = Arc::new(MountRecordingSandbox::failing_exec(
        "docker",
        "Cannot connect to the Docker daemon",
    ));
    // A `TestSandbox` on purpose: it does not override `ensure_ready_with`, so
    // the refusal comes from the trait's own `check_ensure_ready_opts` rather
    // than from anything this test arranged.
    let fallback = Arc::new(TestSandbox::new("restricted-host", None, None));
    let failover = FailoverSandbox::new(
        Arc::clone(&primary) as Arc<dyn Sandbox>,
        Arc::clone(&fallback) as Arc<dyn Sandbox>,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();

    // Made ready on the primary while it still works, so the options are known.
    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            extra_mounts: &mounts,
            ..Default::default()
        })
        .await
        .unwrap();

    let error = failover
        .exec(&id, "true", &ExecOpts::default())
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("cannot bind extra mounts"),
        "an incapable fallback must refuse, not run without the paths: {error}"
    );
    assert_eq!(
        fallback.ensure_ready_calls.load(Ordering::SeqCst),
        0,
        "the incapable fallback must never have been started"
    );
}

#[tokio::test]
async fn test_failover_sandbox_exec_recovery_refuses_an_unrecorded_id() {
    // The blocker that was supposed to be closed, returning through the
    // default. An id `exec` reaches with nothing recorded in this process -
    // a gateway restart with the container still up is the ordinary way - got
    // `OwnedEnsureReadyOpts::default()`: no mounts, no `run_as`, and a
    // `check_ensure_ready_opts` with nothing left to check. Nothing recorded
    // is not a statement that nothing was asked for.
    let primary = Arc::new(MountRecordingSandbox::failing_exec(
        "docker",
        "Cannot connect to the Docker daemon",
    ));
    let fallback = Arc::new(MountRecordingSandbox::new("podman", None));
    let failover = FailoverSandbox::new(
        Arc::clone(&primary) as Arc<dyn Sandbox>,
        Arc::clone(&fallback) as Arc<dyn Sandbox>,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-unrecorded".into(),
    };

    let error = failover
        .exec(&id, "true", &ExecOpts::default())
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("no option set recorded"),
        "an unrecorded id must refuse, not recover with the permissive default: {error}"
    );
    assert!(
        fallback.seen().is_empty(),
        "the fallback must never have been started for an unrecorded id"
    );
}

#[tokio::test]
async fn test_failover_sandbox_cleanup_forgets_the_recorded_opts() {
    // `cleanup` removed nothing from `last_opts`, so the map kept one entry
    // per sandbox id for the life of the process while `DockerSandbox::cleanup`
    // cleared all three of its siblings. Observed through the recovery path,
    // which is the only reader: once the sandbox is gone there is nothing
    // recorded for it, so a later `exec` failover refuses instead of replaying
    // a dead container's options.
    let primary = Arc::new(MountRecordingSandbox::failing_exec(
        "docker",
        "Cannot connect to the Docker daemon",
    ));
    let fallback = Arc::new(MountRecordingSandbox::new("podman", None));
    let failover = FailoverSandbox::new(
        Arc::clone(&primary) as Arc<dyn Sandbox>,
        Arc::clone(&fallback) as Arc<dyn Sandbox>,
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let mounts = SandboxMount::try_from_configs(&[mount_config(
        "/srv/vault",
        "/home/sandbox/vault",
        SandboxMountAccess::Rw,
    )])
    .unwrap();

    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            extra_mounts: &mounts,
            ..Default::default()
        })
        .await
        .unwrap();
    failover.cleanup(&id).await.unwrap();

    let error = failover
        .exec(&id, "true", &ExecOpts::default())
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("no option set recorded"),
        "cleanup must drop the recorded option set, not keep it forever: {error}"
    );
}
