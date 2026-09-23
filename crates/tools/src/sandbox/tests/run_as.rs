//! Per-agent `run_as`: parsing, the `--user` and HOME arguments it produces,
//! and the router/failover delegation that must not drop it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{mounts::MountRecordingSandbox, *};

#[test]
fn test_sandbox_user_parses_a_well_formed_pair() {
    let user = SandboxUser::try_from("1000:1001").unwrap();
    assert_eq!(user.uid(), 1000);
    assert_eq!(user.gid(), 1001);
    assert_eq!(user.to_arg(), "1000:1001");
}

#[test]
fn test_sandbox_user_rejects_malformed_values() {
    // Fail closed at the runtime layer too: there is no spelling of run_as
    // that parses into "just run as root".
    for value in [
        "1000",
        "1000:1000:1000",
        "1000:",
        ":1000",
        "",
        "walter:walter",
        "-1:1000",
        "1000:-1",
        "1000 : 1000",
        "+1000:1000",
    ] {
        assert!(
            SandboxUser::try_from(value).is_err(),
            "run_as {value:?} must not parse"
        );
    }
}

#[test]
fn test_sandbox_user_rejects_root_uid_and_root_gid() {
    let error = SandboxUser::try_from("0:0").unwrap_err().to_string();
    assert!(error.contains("uid 0"), "unexpected error: {error}");
    assert!(
        SandboxUser::try_from("0:1000").is_err(),
        "uid 0 is refused whatever the gid is"
    );
    // The group is the other half of the same hazard: `1000:0` puts the
    // container in the root group, which is group-write on every root-owned
    // path the agent's mounts expose.
    let error = SandboxUser::try_from("1000:0").unwrap_err().to_string();
    assert!(error.contains("gid 0"), "unexpected error: {error}");
}

#[test]
fn test_docker_run_as_args_present_when_set_and_absent_when_unset() {
    let user = SandboxUser::try_from("1000:1000").unwrap();
    assert_eq!(DockerSandbox::run_as_args(Some(&user)).unwrap(), vec![
        "--user".to_string(),
        "1000:1000".to_string(),
    ]);
    assert!(DockerSandbox::run_as_args(None).unwrap().is_empty());
}

#[test]
fn test_docker_home_persistence_args_off_tmpfs_for_run_as_only() {
    // With persistence off there is no host dir to own, and the image's
    // /home/sandbox is root:root 0755 - under --read-only a run_as container
    // could not write its own HOME at all without this.
    let off = SandboxConfig {
        home_persistence: HomePersistence::Off,
        ..Default::default()
    };
    let docker = DockerSandbox::new(off);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let user = SandboxUser::try_from("1000:1001").unwrap();

    assert_eq!(
        docker.home_persistence_args(&id, Some(&user)).unwrap(),
        vec![
            "--tmpfs".to_string(),
            "/home/sandbox:rw,nosuid,size=256m,uid=1000,gid=1001".to_string(),
        ]
    );
    assert!(
        docker.home_persistence_args(&id, None).unwrap().is_empty(),
        "no run_as, no tmpfs: the container keeps the image's own /home/sandbox"
    );

    // And it is the `off` case only: with persistence on, the home is a bind
    // mount and a tmpfs over it would hide the persisted files. This third part
    // really creates the per-uid home, so the uid comes from the running
    // process: a hard-coded 1000 would pass on a uid-1000 runner only.
    let temp_dir = tempfile::tempdir().unwrap();
    let _data_dir = DataDirGuard::new(temp_dir.path().join("data"));
    let shared_config = SandboxConfig::default();
    let persisted_user = test_run_as_user();
    open_run_as_home(
        &sandbox_home_persistence_host_dir(
            &shared_config,
            Some("docker"),
            &id,
            Some(&persisted_user),
        )
        .unwrap(),
    );
    let shared = DockerSandbox::new(shared_config);
    let args = shared
        .home_persistence_args(&id, Some(&persisted_user))
        .unwrap();
    assert_eq!(args[0], "-v", "shared persistence still binds: {args:?}");
    assert!(
        !args.iter().any(|arg| arg == "--tmpfs"),
        "no tmpfs when the home is persisted: {args:?}"
    );
}

#[test]
fn test_container_policy_fingerprint_tracks_run_as() {
    let docker = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Ro,
        ..Default::default()
    });
    let first = SandboxUser::try_from("1000:1000").unwrap();
    let second = SandboxUser::try_from("1001:1001").unwrap();

    assert_eq!(
        docker.container_policy_fingerprint(&[], Some(&first)),
        docker
            .container_policy_fingerprint(&[], Some(&SandboxUser::try_from("1000:1000").unwrap())),
        "an unchanged run_as keeps the container"
    );
    assert_ne!(
        docker.container_policy_fingerprint(&[], Some(&first)),
        docker.container_policy_fingerprint(&[], Some(&second)),
        "a changed uid must recreate the container"
    );
    assert_ne!(
        docker.container_policy_fingerprint(&[], None),
        docker.container_policy_fingerprint(&[], Some(&first)),
        "adding a run_as must recreate the container"
    );
}

#[test]
fn test_container_policy_fingerprint_of_an_empty_policy_is_unchanged_by_this_patch() {
    // The upgrade-recreates-nothing property. Asserted against a label measured
    // on the tree before mounts and run_as existed, not against a freshly
    // computed one, so it cannot pass by both sides drifting together. The
    // config pins `host_data_dir` so the hash does not depend on this machine's
    // data dir.
    let docker = DockerSandbox::new(SandboxConfig {
        host_data_dir: Some(PathBuf::from("/host/one")),
        managed_files_mount: ManagedFilesMount::Ro,
        ..Default::default()
    });

    assert_eq!(
        docker.container_policy_fingerprint(&[], None),
        // sha256 of "ro\0ro\0/host/one/files", the pre-patch input verbatim.
        "9c68c93f9b1a4cc6649611391df318b8eca8e92f206cb8337fcb5e0a6e42db4d",
        "an install with neither mounts nor run_as must keep the label it \
         already has, or every container in the fleet is recreated on upgrade"
    );
}

#[tokio::test]
async fn test_router_resolve_agent_run_as_converts_and_rejects() {
    // The single seat where the stored string becomes a SandboxUser. A bad
    // value fails the call; it never resolves to None, which would be root.
    let router = SandboxRouter::new(SandboxConfig::default());
    assert!(
        router
            .resolve_agent_run_as("session:a")
            .await
            .unwrap()
            .is_none()
    );

    router
        .set_agent_sandbox("session:a", AgentSandboxPolicy {
            run_as: Some("1000:1000".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(
        router
            .resolve_agent_run_as("session:a")
            .await
            .unwrap()
            .map(|user| user.to_arg()),
        Some("1000:1000".to_string())
    );

    for value in ["1000", "0:0", "walter:walter"] {
        router
            .set_agent_sandbox("session:bad", AgentSandboxPolicy {
                run_as: Some(value.into()),
                ..Default::default()
            })
            .await;
        assert!(
            router.resolve_agent_run_as("session:bad").await.is_err(),
            "run_as {value:?} must fail the resolve, never resolve to root"
        );
    }
}

#[tokio::test]
async fn test_declared_run_as_alone_does_not_force_the_sandbox() {
    // The mounts' sibling under the new rule: a uid:gid configures the
    // container this agent gets, it does not demand one. Only `force` does.
    let router = docker_router::router_with_real_backend(SandboxConfig {
        mode: SandboxMode::Off,
        ..Default::default()
    });
    router.set_override("session:walter", false).await;

    router
        .set_agent_sandbox("session:walter", AgentSandboxPolicy {
            run_as: Some("1000:1000".into()),
            ..Default::default()
        })
        .await;

    assert!(
        !router.is_sandboxed("session:walter").await,
        "a run_as configures the sandbox, it must not force it on"
    );
}

#[tokio::test]
async fn test_ensure_ready_with_errors_when_backend_cannot_run_as() {
    // Dropping run_as would hand a writable host mount to a root container,
    // which is the precise hazard the field exists to close.
    let sandbox = TestSandbox::new("test-backend", None, None);
    assert!(
        !sandbox.supports_run_as(),
        "the capability default is fail-safe false"
    );
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "no-run-as-support".into(),
    };
    let user = SandboxUser::try_from("1000:1000").unwrap();

    let error = sandbox
        .ensure_ready_with(&id, EnsureReadyOpts {
            run_as: Some(&user),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("cannot run a container as a configured user"),
        "unexpected error: {error}"
    );
    assert_eq!(
        sandbox.ensure_ready_calls.load(Ordering::SeqCst),
        0,
        "the refusal must come before the container is started"
    );
}

#[tokio::test]
async fn test_failover_sandbox_forwards_run_as_to_the_active_backend() {
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
        key: "failover-run-as".into(),
    };
    let user = SandboxUser::try_from("1000:1000").unwrap();

    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            run_as: Some(&user),
            ..Default::default()
        })
        .await
        .unwrap();

    let expected = vec![Some("1000:1000".to_string())];
    assert_eq!(primary.seen_run_as(), expected);
    assert_eq!(
        fallback.seen_run_as(),
        expected,
        "the fallback must see the same run_as; dropping it on delegation \
         starts the container as root"
    );
}

#[tokio::test]
async fn test_failover_sandbox_reports_run_as_support_of_active_backend() {
    let primary = Arc::new(MountRecordingSandbox::new(
        "docker",
        Some("cannot connect to the docker daemon"),
    ));
    let fallback: Arc<dyn Sandbox> = Arc::new(TestSandbox::new("restricted-host", None, None));
    let failover = FailoverSandbox::new(primary, fallback);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "failover-run-as-support".into(),
    };

    assert!(
        failover.supports_run_as(),
        "before failover the primary is active and it does support run_as"
    );

    failover.ensure_ready(&id, None).await.unwrap();

    assert_eq!(failover.backend_name(), "restricted-host");
    assert!(
        !failover.supports_run_as(),
        "after failover the answer must come from the backend that would run \
         the container, not from the primary"
    );
}

#[tokio::test]
async fn test_router_sync_workspace_for_resolves_under_the_run_as_uid() {
    // The two call sites hold a session key and no agent policy, so the router
    // owns this lookup. A wrong None here syncs the root session's home.
    let config = SandboxConfig {
        home_persistence: HomePersistence::Shared,
        ..Default::default()
    };
    let router = SandboxRouter::new(config);

    let base = router
        .sync_workspace_for("session:plain")
        .await
        .expect("a session with no policy still has a workspace");
    assert!(
        !base.ends_with("user/1000"),
        "no run_as, no per-uid path: {}",
        base.display()
    );

    router
        .set_agent_sandbox("session:walter", AgentSandboxPolicy {
            run_as: Some("1000:1000".into()),
            ..Default::default()
        })
        .await;
    let per_uid = router
        .sync_workspace_for("session:walter")
        .await
        .expect("a run_as session has a workspace too");
    assert!(
        per_uid.ends_with("user/1000"),
        "a run_as session must sync its own home: {}",
        per_uid.display()
    );
}

#[test]
fn test_docker_run_as_args_refuses_a_user_that_skipped_the_rules() {
    // The L3 re-assert, the sibling of the one `extra_mount_args` does. Private
    // fields make `SandboxUser::try_from` the only constructor production code
    // has, but a claim about construction is not evidence about the process
    // about to be spawned - and `--user 0:0` is the one argument this whole
    // field exists to never emit.
    for (uid, gid) in [(0, 0), (0, 1000), (1000, 0)] {
        let user = SandboxUser::from_parts_unchecked(uid, gid);
        let error = DockerSandbox::run_as_args(Some(&user))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("run_as"),
            "--user {uid}:{gid} must be refused before docker sees it, got: {error}"
        );
    }
}

#[tokio::test]
async fn test_docker_records_run_as_only_after_a_successful_start() {
    // Two bugs at the edges of the same map. The entry was written before the
    // startup gate and before the container existed, so a failed start left a
    // record of a container that never ran; and the `None` arm removed a live
    // entry, so a plain `ensure_ready(id, None)` - the option-less trait
    // method, not a statement that this agent has no run_as - pointed
    // read_file/write_file/list_files at `home/shared` while the container kept
    // writing to its per-uid home.
    let docker = DockerSandbox::with_cli(SandboxConfig::default(), "moltis-no-such-container-cli");
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let user = SandboxUser::try_from("1000:1000").unwrap();

    assert!(
        docker
            .ensure_ready_with(&id, EnsureReadyOpts {
                run_as: Some(&user),
                ..Default::default()
            })
            .await
            .is_err(),
        "a CLI that does not exist cannot start a container"
    );
    assert_eq!(
        docker.recorded_run_as(&id),
        None,
        "a failed start must leave no record of a container that never ran"
    );

    // Now a live container's record, and the option-less path over it.
    docker
        .run_as_by_container
        .lock()
        .unwrap()
        .insert(docker.container_name(&id), user);
    assert!(docker.ensure_ready(&id, None).await.is_err());
    assert_eq!(
        docker.recorded_run_as(&id),
        Some(user),
        "ensure_ready(id, None) must not erase the run_as of a running container"
    );
}

#[tokio::test]
async fn test_failover_sandbox_exec_recovery_replays_the_run_as() {
    // The `exec` recovery path, which the tests above do not reach: they all go
    // through `ensure_ready_with`, which has the options in hand. `exec` does
    // not, so it called `ensure_ready(id, None)` and the fallback came up as
    // root - silently, because `None` also skips `check_ensure_ready_opts`.
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
    let user = SandboxUser::try_from("1000:1000").unwrap();

    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            run_as: Some(&user),
            ..Default::default()
        })
        .await
        .unwrap();
    failover
        .exec(&id, "true", &ExecOpts::default())
        .await
        .unwrap();

    assert_eq!(
        fallback.seen_run_as(),
        vec![Some("1000:1000".to_string())],
        "the fallback must be made ready with the agent's run_as, not as root"
    );
}

#[tokio::test]
async fn test_failover_sandbox_exec_recovery_refuses_a_fallback_that_cannot_run_as() {
    // The other half of replaying the options: they carry
    // `check_ensure_ready_opts` with them. A fallback that cannot honour the
    // run_as has to fail the turn, because the alternative is the root
    // container the field exists to prevent - and with `None` it could not even
    // be asked.
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
    let user = SandboxUser::try_from("1000:1000").unwrap();

    // Made ready on the primary while it still works, so the options are known.
    failover
        .ensure_ready_with(&id, EnsureReadyOpts {
            run_as: Some(&user),
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
        error.contains("cannot run a container as a configured user"),
        "an incapable fallback must refuse, not run as root: {error}"
    );
    assert_eq!(
        fallback.ensure_ready_calls.load(Ordering::SeqCst),
        0,
        "the incapable fallback must never have been started"
    );
}

#[tokio::test]
async fn test_forget_container_records_drops_the_run_as_too() {
    // The name-conflict recreate branch dropped `provisioned` and left
    // `run_as_by_container` behind, so after a `run_as` -> no-`run_as`
    // transition `recorded_run_as` still pointed `read_file`/`write_file` at
    // `<home>/user/<uid>` while the replacement container wrote the shared
    // home. Both recreate branches and `cleanup` now go through this one
    // method, which is what stops them drifting apart again.
    let docker = DockerSandbox::with_cli(SandboxConfig::default(), "moltis-no-such-container-cli");
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess-1".into(),
    };
    let name = docker.container_name(&id);
    let user = SandboxUser::try_from("1000:1000").unwrap();

    docker
        .run_as_by_container
        .lock()
        .unwrap()
        .insert(name.clone(), user);
    docker.provisioned.lock().await.insert(name.clone());

    docker.forget_container_records(&name).await;

    assert_eq!(
        docker.recorded_run_as(&id),
        None,
        "removing the container must drop the run_as that described it"
    );
    assert!(
        !docker.provisioned.lock().await.contains(&name),
        "removing the container must drop its provisioned marker"
    );
}
