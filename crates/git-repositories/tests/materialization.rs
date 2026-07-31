#![allow(clippy::unwrap_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

use {
    moltis_git_repositories::{
        Access, Error, HttpsCredentials, HttpsSource, MaterializationLimits, Materializer,
        RepositoryBackend, RepositorySource, RequestedRevision, SshCredentials, SshSource,
    },
    secrecy::Secret,
};

fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Moltis Test")
        .env("GIT_AUTHOR_EMAIL", "test@moltis.invalid")
        .env("GIT_COMMITTER_NAME", "Moltis Test")
        .env("GIT_COMMITTER_EMAIL", "test@moltis.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn repository() -> (tempfile::TempDir, String, String) {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q", "--initial-branch=main"]);
    fs::write(directory.path().join("tracked.txt"), "one\n").unwrap();
    fs::write(
        directory.path().join(".mcp.json"),
        r#"{"mcpServers":{"tracked":{"command":"./tracked.txt"}}}"#,
    )
    .unwrap();
    git(directory.path(), &["add", "tracked.txt", ".mcp.json"]);
    git(directory.path(), &["commit", "-q", "-m", "one"]);
    let first = git(directory.path(), &["rev-parse", "HEAD"]);
    git(directory.path(), &["tag", "v1"]);
    fs::write(directory.path().join("tracked.txt"), "two\n").unwrap();
    fs::write(directory.path().join("second.txt"), "committed\n").unwrap();
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "-q", "-m", "two"]);
    let second = git(directory.path(), &["rev-parse", "HEAD"]);
    (directory, first, second)
}

fn commit_all(directory: &Path) -> String {
    git(directory, &["add", "."]);
    git(directory, &["commit", "-q", "-m", "fixture"]);
    git(directory, &["rev-parse", "HEAD"])
}

fn init_repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q", "--initial-branch=main"]);
    directory
}

fn materialize_head(source: &Path) -> (tempfile::TempDir, PathBuf) {
    let revisions = tempfile::tempdir().unwrap();
    let materialized = Materializer::default()
        .materialize(
            &RepositorySource::local(source).unwrap(),
            &RequestedRevision::head(),
            revisions.path(),
            None,
            None,
        )
        .unwrap();
    (revisions, materialized.path)
}

#[test]
fn local_exact_ref_ignores_dirty_files_and_does_not_mutate_source() {
    let (source, first, head) = repository();
    fs::write(source.path().join("tracked.txt"), "dirty\n").unwrap();
    fs::write(source.path().join("untracked.txt"), "untracked\n").unwrap();
    let status_before = git(source.path(), &[
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
    ]);
    let root = tempfile::tempdir().unwrap();

    let materialized = Materializer::default()
        .materialize(
            &RepositorySource::local(source.path()).unwrap(),
            &RequestedRevision::parse("v1").unwrap(),
            root.path(),
            None,
            None,
        )
        .unwrap();

    assert_eq!(materialized.commit, first);
    assert_eq!(
        fs::read_to_string(materialized.path.join("tracked.txt")).unwrap(),
        "one\n"
    );
    assert!(materialized.path.join(".mcp.json").is_file());
    assert!(!materialized.path.join("second.txt").exists());
    assert!(!materialized.path.join("untracked.txt").exists());
    assert_eq!(git(source.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(source.path(), &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all"
        ]),
        status_before
    );
}

#[test]
fn local_exact_commit_is_materialized() {
    let (source, first, _) = repository();
    let root = tempfile::tempdir().unwrap();

    let materialized = Materializer::default()
        .materialize(
            &RepositorySource::local(source.path()).unwrap(),
            &RequestedRevision::parse(&first).unwrap(),
            root.path(),
            None,
            None,
        )
        .unwrap();

    assert_eq!(materialized.commit, first);
    assert_eq!(
        fs::read_to_string(materialized.path.join("tracked.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn publication_is_canonical_and_idempotent() {
    let (source, _, head) = repository();
    let root = tempfile::tempdir().unwrap();
    let materializer = Materializer::default();
    let repository_source = RepositorySource::local(source.path()).unwrap();
    let revision = RequestedRevision::head();

    let first = materializer
        .materialize(&repository_source, &revision, root.path(), None, None)
        .unwrap();
    let second = materializer
        .materialize(&repository_source, &revision, root.path(), None, None)
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.commit, head);
    assert_eq!(
        first.path,
        fs::canonicalize(root.path().join(head)).unwrap()
    );
}

#[test]
fn local_materialization_ignores_network_fetch_deadline() {
    let (source, _, head) = repository();
    let root = tempfile::tempdir().unwrap();
    let limits = MaterializationLimits {
        max_fetch_duration: Duration::ZERO,
        ..MaterializationLimits::default()
    };

    let materialized = Materializer::default()
        .with_limits(limits)
        .materialize(
            &RepositorySource::local(source.path()).unwrap(),
            &RequestedRevision::head(),
            root.path(),
            None,
            None,
        )
        .unwrap();

    assert_eq!(materialized.commit, head);
}

#[test]
fn failed_materialization_rolls_back_staging_and_publication() {
    let (source, _, head) = repository();
    let root = tempfile::tempdir().unwrap();
    let limits = MaterializationLimits {
        max_files: 0,
        ..MaterializationLimits::default()
    };
    let result = Materializer::default().with_limits(limits).materialize(
        &RepositorySource::local(source.path()).unwrap(),
        &RequestedRevision::head(),
        root.path(),
        None,
        None,
    );

    assert!(matches!(result, Err(Error::LimitExceeded(_))));
    assert!(!root.path().join(head).exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn marketplace_materializes_only_eight_mcp_plugin_roots() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".claude-plugin")).unwrap();
    let plugins = (1..=130)
        .map(|index| format!(r#"{{"name":"plugin-{index}","source":"./plugins/plugin-{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        source.path().join(".claude-plugin/marketplace.json"),
        format!(r#"{{"name":"large","plugins":[{plugins}]}}"#),
    )
    .unwrap();
    for index in 1..=130 {
        let plugin = source.path().join(format!("plugins/plugin-{index}"));
        fs::create_dir_all(plugin.join("scripts")).unwrap();
        fs::write(plugin.join("unrelated-large.bin"), vec![b'x'; 64 * 1024]).unwrap();
        if index <= 8 {
            fs::write(
                plugin.join(".mcp.json"),
                format!(
                    r#"{{"mcpServers":{{"plugin-{index}":{{"command":"python","args":["scripts/server.py"]}}}}}}"#
                ),
            )
            .unwrap();
            fs::write(plugin.join("scripts/server.py"), "print('selected')\n").unwrap();
            fs::write(
                plugin.join("pyproject.toml"),
                "[project]\nname='selected'\n",
            )
            .unwrap();
        } else {
            fs::write(plugin.join("scripts/skill.py"), "print('unrelated')\n").unwrap();
        }
    }
    fs::write(source.path().join("huge-unrelated.bin"), vec![
        b'y';
        128 * 1024
    ])
    .unwrap();
    commit_all(source.path());

    let (_revisions, materialized) = materialize_head(source.path());

    assert!(
        materialized
            .join(".claude-plugin/marketplace.json")
            .is_file()
    );
    let extracted_plugins = fs::read_dir(materialized.join("plugins")).unwrap().count();
    assert_eq!(extracted_plugins, 8);
    for index in 1..=8 {
        let root = materialized.join(format!("plugins/plugin-{index}"));
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join("scripts/server.py").is_file());
        assert!(root.join("pyproject.toml").is_file());
        assert!(root.join("unrelated-large.bin").is_file());
    }
    assert!(!materialized.join("plugins/plugin-9").exists());
    assert!(!materialized.join("huge-unrelated.bin").exists());
}

#[test]
fn inline_marketplace_server_needs_only_marketplace_manifest() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".claude-plugin")).unwrap();
    fs::write(
        source.path().join(".claude-plugin/marketplace.json"),
        r#"{
            "name":"inline",
            "plugins":[{
                "name":"inline",
                "source":".",
                "mcpServers":{"remote":{"type":"http","url":"https://example.test/mcp"}}
            }]
        }"#,
    )
    .unwrap();
    fs::write(source.path().join("unrelated.bin"), "not selected").unwrap();
    commit_all(source.path());

    let (_revisions, materialized) = materialize_head(source.path());

    assert!(
        materialized
            .join(".claude-plugin/marketplace.json")
            .is_file()
    );
    assert!(!materialized.join("unrelated.bin").exists());
}

#[test]
fn moltis_manifest_materializes_declared_roots_only() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".moltis")).unwrap();
    fs::create_dir_all(source.path().join("servers/one/scripts")).unwrap();
    fs::create_dir_all(source.path().join("servers/two")).unwrap();
    fs::write(
        source.path().join(".moltis/mcp-repository.json"),
        r#"{
            "version":1,
            "plugins":[{"root":"servers/one"},{"root":"servers/two"}],
            "mcpServers":{}
        }"#,
    )
    .unwrap();
    fs::write(source.path().join("servers/one/scripts/server.py"), "one").unwrap();
    fs::write(source.path().join("servers/two/server.js"), "two").unwrap();
    fs::write(source.path().join("unrelated.bin"), "no").unwrap();
    commit_all(source.path());

    let (_revisions, materialized) = materialize_head(source.path());

    assert!(materialized.join(".moltis/mcp-repository.json").is_file());
    assert!(materialized.join("servers/one/scripts/server.py").is_file());
    assert!(materialized.join("servers/two/server.js").is_file());
    assert!(!materialized.join("unrelated.bin").exists());
}

#[test]
fn declared_plugin_root_traversal_and_absolute_paths_are_rejected() {
    for root in ["../outside", "/absolute", r"C:\absolute", r"..\outside"] {
        let source = init_repository();
        fs::create_dir_all(source.path().join(".moltis")).unwrap();
        fs::write(
            source.path().join(".moltis/mcp-repository.json"),
            format!(
                r#"{{"version":1,"plugins":[{{"root":{}}}]}}"#,
                serde_json::to_string(root).unwrap()
            ),
        )
        .unwrap();
        commit_all(source.path());
        let revisions = tempfile::tempdir().unwrap();

        let result = Materializer::default().materialize(
            &RepositorySource::local(source.path()).unwrap(),
            &RequestedRevision::head(),
            revisions.path(),
            None,
            None,
        );

        assert!(
            matches!(result, Err(Error::UnsafeDeclaredPath(_))),
            "{root}"
        );
    }
}

#[cfg(unix)]
#[test]
fn executable_modes_are_preserved_and_symlinks_are_inert() {
    use std::os::unix::fs::PermissionsExt;

    let source = init_repository();
    fs::create_dir_all(source.path().join("plugin")).unwrap();
    fs::create_dir_all(source.path().join(".moltis")).unwrap();
    fs::write(
        source.path().join(".moltis/mcp-repository.json"),
        r#"{"version":1,"plugins":[{"root":"plugin"}]}"#,
    )
    .unwrap();
    fs::write(source.path().join("plugin/server.sh"), "#!/bin/sh\n").unwrap();
    let mut permissions = fs::metadata(source.path().join("plugin/server.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(source.path().join("plugin/server.sh"), permissions).unwrap();
    std::os::unix::fs::symlink("../../outside", source.path().join("plugin/link")).unwrap();
    commit_all(source.path());

    let (_revisions, materialized) = materialize_head(source.path());

    assert_ne!(
        fs::metadata(materialized.join("plugin/server.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        fs::read_to_string(materialized.join("plugin/link")).unwrap(),
        "../../outside"
    );
    assert!(
        !fs::symlink_metadata(materialized.join("plugin/link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn root_mcp_materializes_inferred_paths_and_bounded_runtime_files() {
    let source = init_repository();
    fs::create_dir_all(source.path().join("scripts")).unwrap();
    fs::write(
        source.path().join(".mcp.json"),
        r#"{
            "mcpServers":{
                "one":{"command":"python","args":["scripts/server.py","--config=config.json"]},
                "two":{"command":"scripts","args":["scripts/server.py"]},
                "remote":{"type":"http","url":"https://example.test/mcp"}
            }
        }"#,
    )
    .unwrap();
    fs::write(source.path().join("scripts/server.py"), "print('server')\n").unwrap();
    fs::write(source.path().join("config.json"), "{}\n").unwrap();
    fs::write(
        source.path().join("pyproject.toml"),
        "[project]\nname='server'\n",
    )
    .unwrap();
    fs::write(source.path().join("uv.lock"), "version = 1\n").unwrap();
    fs::write(source.path().join("scripts/unreferenced.py"), "no\n").unwrap();
    fs::write(source.path().join("large-unrelated.bin"), vec![
        b'z';
        64 * 1024
    ])
    .unwrap();
    commit_all(source.path());

    let (_revisions, materialized) = materialize_head(source.path());

    assert!(materialized.join(".mcp.json").is_file());
    assert!(materialized.join("scripts/server.py").is_file());
    assert!(materialized.join("config.json").is_file());
    assert!(materialized.join("pyproject.toml").is_file());
    assert!(materialized.join("uv.lock").is_file());
    assert!(materialized.join("scripts/unreferenced.py").is_file());
    assert!(!materialized.join("large-unrelated.bin").exists());
}

#[test]
fn unsupported_repository_fails_instead_of_extracting_all_files() {
    let source = init_repository();
    fs::write(source.path().join("unrelated.bin"), "large repository").unwrap();
    commit_all(source.path());
    let revisions = tempfile::tempdir().unwrap();

    let result = Materializer::default().materialize(
        &RepositorySource::local(source.path()).unwrap(),
        &RequestedRevision::head(),
        revisions.path(),
        None,
        None,
    );

    assert!(matches!(result, Err(Error::NoSupportedManifest)));
}

#[test]
fn marketplace_plugin_root_bound_is_enforced() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".claude-plugin")).unwrap();
    fs::write(
        source.path().join(".claude-plugin/marketplace.json"),
        r#"{
            "plugins":[
                {"name":"one","source":"plugins/one"},
                {"name":"two","source":"plugins/two"}
            ]
        }"#,
    )
    .unwrap();
    commit_all(source.path());
    let revisions = tempfile::tempdir().unwrap();
    let limits = MaterializationLimits {
        max_plugin_roots: 1,
        ..MaterializationLimits::default()
    };

    let result = Materializer::default().with_limits(limits).materialize(
        &RepositorySource::local(source.path()).unwrap(),
        &RequestedRevision::head(),
        revisions.path(),
        None,
        None,
    );

    assert!(matches!(result, Err(Error::LimitExceeded(_))));
}

#[test]
fn manifest_depth_file_blob_and_total_bounds_are_enforced() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".claude-plugin")).unwrap();
    fs::create_dir_all(source.path().join("plugin/scripts/nested")).unwrap();
    fs::create_dir_all(source.path().join("plugin/.claude-plugin")).unwrap();
    fs::write(
        source.path().join(".claude-plugin/marketplace.json"),
        r#"{"plugins":[{"name":"one","source":"plugin"}]}"#,
    )
    .unwrap();
    fs::write(
        source.path().join("plugin/.claude-plugin/plugin.json"),
        r#"{"mcpServers":"../server.json"}"#,
    )
    .unwrap();
    fs::write(
        source.path().join("plugin/scripts/nested/server.py"),
        vec![b'x'; 128],
    )
    .unwrap();
    commit_all(source.path());

    let cases = [
        MaterializationLimits {
            max_manifests: 1,
            ..MaterializationLimits::default()
        },
        MaterializationLimits {
            max_tree_depth: 2,
            ..MaterializationLimits::default()
        },
        MaterializationLimits {
            max_files: 1,
            ..MaterializationLimits::default()
        },
        MaterializationLimits {
            max_blob_bytes: 64,
            ..MaterializationLimits::default()
        },
        MaterializationLimits {
            max_total_bytes: 150,
            ..MaterializationLimits::default()
        },
    ];
    for limits in cases {
        let revisions = tempfile::tempdir().unwrap();
        let result = Materializer::default().with_limits(limits).materialize(
            &RepositorySource::local(source.path()).unwrap(),
            &RequestedRevision::head(),
            revisions.path(),
            None,
            None,
        );
        assert!(matches!(result, Err(Error::LimitExceeded(_))));
    }
}

#[test]
fn packed_oversized_blob_is_rejected_from_header_before_loading() {
    let source = init_repository();
    fs::create_dir_all(source.path().join(".moltis")).unwrap();
    fs::create_dir_all(source.path().join("plugin")).unwrap();
    fs::write(
        source.path().join(".moltis/mcp-repository.json"),
        r#"{"version":1,"plugins":[{"root":"plugin"}]}"#,
    )
    .unwrap();
    fs::write(source.path().join("plugin/large.txt"), vec![
        b'x';
        1024 * 1024
    ])
    .unwrap();
    commit_all(source.path());
    git(source.path(), &["gc", "--aggressive", "--prune=now"]);
    let revisions = tempfile::tempdir().unwrap();
    let limits = MaterializationLimits {
        max_blob_bytes: 128,
        ..MaterializationLimits::default()
    };

    let result = Materializer::default().with_limits(limits).materialize(
        &RepositorySource::local(source.path()).unwrap(),
        &RequestedRevision::head(),
        revisions.path(),
        None,
        None,
    );

    assert!(matches!(result, Err(Error::LimitExceeded(message)) if message.contains("large.txt")));
}

#[test]
fn manifest_specific_blob_bound_is_checked_before_parsing() {
    let source = init_repository();
    let manifest = format!(
        r#"{{"mcpServers":{{}},"padding":"{}"}}"#,
        "x".repeat(1024 * 1024)
    );
    fs::write(source.path().join(".mcp.json"), manifest).unwrap();
    commit_all(source.path());
    git(source.path(), &["gc", "--aggressive", "--prune=now"]);
    let revisions = tempfile::tempdir().unwrap();
    let limits = MaterializationLimits {
        max_blob_bytes: 2 * 1024 * 1024,
        max_manifest_bytes: 16,
        ..MaterializationLimits::default()
    };

    let result = Materializer::default().with_limits(limits).materialize(
        &RepositorySource::local(source.path()).unwrap(),
        &RequestedRevision::head(),
        revisions.path(),
        None,
        None,
    );

    assert!(matches!(result, Err(Error::LimitExceeded(message)) if message.contains("manifest")));
}

#[derive(Clone)]
struct FakeBackend {
    local: PathBuf,
    calls: Arc<Mutex<Vec<(&'static str, Duration)>>>,
}

impl FakeBackend {
    fn clone_local(&self, destination: &Path) -> Result<(), Error> {
        let output = Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg("--bare")
            .arg("--")
            .arg(&self.local)
            .arg(destination)
            .output()
            .map_err(|error| Error::GitSubprocess(error.to_string()))?;
        if !output.status.success() {
            Err(Error::GitSubprocess(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        } else {
            let head = Command::new("git")
                .arg("-C")
                .arg(destination)
                .args(["rev-parse", "HEAD"])
                .output()
                .map_err(|error| Error::GitSubprocess(error.to_string()))?;
            if !head.status.success() {
                return Err(Error::GitSubprocess(
                    String::from_utf8_lossy(&head.stderr).into_owned(),
                ));
            }
            fs::write(destination.join("FETCH_HEAD"), head.stdout)
                .map_err(|error| Error::GitSubprocess(error.to_string()))
        }
    }
}

impl RepositoryBackend for FakeBackend {
    fn fetch_https(
        &self,
        _source: &HttpsSource,
        _credentials: Option<&HttpsCredentials>,
        _revision: &RequestedRevision,
        destination: &Path,
        _max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error> {
        self.calls
            .lock()
            .unwrap()
            .push(("https", max_fetch_duration));
        self.clone_local(destination)
    }

    fn fetch_ssh(
        &self,
        _source: &SshSource,
        _credentials: &SshCredentials,
        _revision: &RequestedRevision,
        destination: &Path,
        _max_fetch_bytes: u64,
        max_fetch_duration: Duration,
    ) -> Result<(), Error> {
        self.calls.lock().unwrap().push(("ssh", max_fetch_duration));
        self.clone_local(destination)
    }
}

#[test]
fn injected_backend_materializes_https_without_network() {
    let (source, _, head) = repository();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = FakeBackend {
        local: source.path().to_path_buf(),
        calls: calls.clone(),
    };
    let root = tempfile::tempdir().unwrap();
    let remote = RepositorySource::Https(
        HttpsSource::new("https://git.example/owner/repo.git", Access::Private).unwrap(),
    );
    let credentials = HttpsCredentials::new(
        "git.example",
        "moltis",
        Secret::new("top-secret-token".to_string()),
    )
    .unwrap();

    let limits = MaterializationLimits {
        max_fetch_duration: Duration::from_secs(7),
        ..MaterializationLimits::default()
    };
    let result = Materializer::new(backend)
        .with_limits(limits)
        .materialize(
            &remote,
            &RequestedRevision::head(),
            root.path(),
            Some(&credentials),
            None,
        )
        .unwrap();

    assert_eq!(result.commit, head);
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "https");
    assert!(calls[0].1 <= Duration::from_secs(7));
    assert!(calls[0].1 > Duration::from_secs(6));
}

#[test]
fn injected_backend_materializes_ssh_without_network() {
    let (source, _, head) = repository();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = FakeBackend {
        local: source.path().to_path_buf(),
        calls: calls.clone(),
    };
    let root = tempfile::tempdir().unwrap();
    let remote = RepositorySource::Ssh(
        SshSource::new("git@git.example:owner/repo.git", Access::Private).unwrap(),
    );
    let credentials = SshCredentials::new(
        "git.example",
        Secret::new("PRIVATE KEY DATA".to_string()),
        Secret::new("git.example ssh-ed25519 AAAATEST".to_string()),
    )
    .unwrap();

    let limits = MaterializationLimits {
        max_fetch_duration: Duration::from_secs(9),
        ..MaterializationLimits::default()
    };
    let result = Materializer::new(backend)
        .with_limits(limits)
        .materialize(
            &remote,
            &RequestedRevision::head(),
            root.path(),
            None,
            Some(&credentials),
        )
        .unwrap();

    assert_eq!(result.commit, head);
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "ssh");
    assert!(calls[0].1 <= Duration::from_secs(9));
    assert!(calls[0].1 > Duration::from_secs(8));
}

#[test]
fn host_mismatch_is_rejected_before_transport() {
    let (source, ..) = repository();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let backend = FakeBackend {
        local: source.path().to_path_buf(),
        calls: calls.clone(),
    };
    let root = tempfile::tempdir().unwrap();
    let remote = RepositorySource::Https(
        HttpsSource::new("https://git.example/owner/repo.git", Access::Private).unwrap(),
    );
    let credentials =
        HttpsCredentials::new("evil.example", "moltis", Secret::new("secret".to_string())).unwrap();

    let result = Materializer::new(backend).materialize(
        &remote,
        &RequestedRevision::head(),
        root.path(),
        Some(&credentials),
        None,
    );

    assert!(matches!(result, Err(Error::CredentialHostMismatch { .. })));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn sources_and_revisions_reject_unsafe_input() {
    for source in [
        "http://git.example/owner/repo.git",
        "file:///tmp/repo",
        "https://user@git.example/owner/repo.git",
        "https://git.example/owner/repo.git?token=x",
        "https://git.example/owner/repo.git#main",
    ] {
        assert!(
            HttpsSource::new(source, Access::Public).is_err(),
            "{source}"
        );
    }
    for source in [
        "git.example",
        "-oProxyCommand=id:repo",
        "git@host:",
        "git@host:owner/repo;touch-pwned",
        "ssh://git:password@host/owner/repo.git",
        "ssh://git%40evil@host/owner/repo.git",
        "ssh://git@host/owner/repo%20unsafe",
    ] {
        assert!(SshSource::new(source, Access::Private).is_err(), "{source}");
    }
    for revision in ["", "main..other", "main~1", "refs/heads/a.lock", "@{1}"] {
        assert!(RequestedRevision::parse(revision).is_err(), "{revision}");
    }
}

#[test]
fn ssh_url_accepts_safe_username_and_binds_explicit_port() {
    let source =
        SshSource::new("ssh://git@example.com:2222/owner/repo.git", Access::Private).unwrap();
    assert_eq!(source.host(), "[example.com]:2222");

    let credentials = SshCredentials::new(
        "[example.com]:2222",
        Secret::new("PRIVATE KEY DATA".to_string()),
        Secret::new("[example.com]:2222 ssh-ed25519 AAAATEST".to_string()),
    )
    .unwrap();
    assert_eq!(credentials.host(), source.host());
}

#[test]
fn credential_debug_is_redacted() {
    let https = HttpsCredentials::new(
        "git.example",
        "moltis",
        Secret::new("top-secret-token".to_string()),
    )
    .unwrap();
    let ssh = SshCredentials::new(
        "git.example",
        Secret::new("PRIVATE KEY DATA".to_string()),
        Secret::new("git.example ssh-ed25519 AAAATEST".to_string()),
    )
    .unwrap();
    let https_debug = format!("{https:?}");
    let ssh_debug = format!("{ssh:?}");

    assert!(!https_debug.contains("top-secret-token"));
    assert!(!ssh_debug.contains("PRIVATE KEY DATA"));
    assert!(!ssh_debug.contains("AAAATEST"));
    assert!(https_debug.contains("[REDACTED]"));
    assert!(ssh_debug.contains("[REDACTED]"));
}
