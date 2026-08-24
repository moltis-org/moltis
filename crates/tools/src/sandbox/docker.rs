//! Docker and Podman sandbox backends.

use {
    async_trait::async_trait,
    sha2::{Digest, Sha256},
    std::{
        collections::{HashMap, HashSet},
        path::{Path, PathBuf},
        sync::{Arc, OnceLock},
        time::Duration,
    },
    tokio::sync::{Mutex, Semaphore},
    tracing::{debug, info, warn},
};

const MANAGED_FILES_POLICY_LABEL: &str = "org.moltis.managed-files-mount";

use {
    super::{
        containers::{
            rebuildable_sandbox_image_tag, sandbox_image_dockerfile, sandbox_image_exists,
            sandbox_image_tag,
        },
        host::provision_packages,
        paths::{
            ManagedFilesPath, ensure_managed_files_host_dir,
            ensure_sandbox_home_persistence_host_dir, host_visible_data_dir,
            host_visible_managed_files_dir, resolve_home_persistence_guest_path_on_host,
            resolve_managed_files_guest_path_on_host, resolve_workspace_guest_path_on_host,
        },
        types::{
            BuildImageResult, DEFAULT_SANDBOX_IMAGE, ManagedFilesMount, NetworkPolicy,
            SANDBOX_FILES_DIR, SANDBOX_HOME_DIR, Sandbox, SandboxConfig, SandboxId, WorkspaceMount,
            canonical_sandbox_packages, tail_lines, truncate_output_for_display,
        },
    },
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
        sandbox::file_system::{
            SandboxListFilesResult, SandboxReadResult, native_host_list_files,
            native_host_list_files_strict, native_host_read_file, native_host_write_file,
            oci_container_list_files, oci_container_read_file, oci_container_write_file,
            permission_denied_payload, remap_host_list_result_to_guest,
        },
    },
};

const SANDBOX_HOST_PODMAN_SOCKET: &str = "/tmp/moltis-host-podman.sock";
const PODMAN_MODE_LABEL: &str = "org.moltis.podman-mode";
const PODMAN_SOCKET_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Distinguishes Docker from Podman for behaviour that differs between the two
/// OCI runtimes (hardening flags, host-gateway resolution, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    Docker,
    Podman,
}

/// Docker/Podman-based sandbox implementation.
///
/// The `cli` field selects the container CLI binary (`"docker"` or `"podman"`).
/// Podman's CLI is a drop-in replacement for Docker, so both backends share
/// this single implementation.  `kind` carries the typed backend identity for
/// behaviour branching without string comparisons.
pub struct DockerSandbox {
    pub config: SandboxConfig,
    pub(crate) kind: BackendKind,
    cli: &'static str,
    backend_label: &'static str,
    /// Container names that have already been provisioned in this process.
    /// Prevents repeated `apt-get install` runs on the same container.
    pub(crate) provisioned: Mutex<HashSet<String>>,
    /// Per-container startup gates. Parallel exec calls for the same session
    /// must not race through inspect-then-run with the same OCI container name.
    startup_gates: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl DockerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            kind: BackendKind::Docker,
            cli: "docker",
            backend_label: "docker",
            provisioned: Mutex::new(HashSet::new()),
            startup_gates: Mutex::new(HashMap::new()),
        }
    }

    pub fn podman(config: SandboxConfig) -> Self {
        Self {
            config,
            kind: BackendKind::Podman,
            cli: "podman",
            backend_label: "podman",
            provisioned: Mutex::new(HashSet::new()),
            startup_gates: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_cli(config: SandboxConfig, cli: &'static str) -> Self {
        Self {
            config,
            kind: BackendKind::Docker,
            cli,
            backend_label: "docker",
            provisioned: Mutex::new(HashSet::new()),
            startup_gates: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn podman_with_cli(config: SandboxConfig, cli: &'static str) -> Self {
        Self {
            config,
            kind: BackendKind::Podman,
            cli,
            backend_label: "podman",
            provisioned: Mutex::new(HashSet::new()),
            startup_gates: Mutex::new(HashMap::new()),
        }
    }

    fn image(&self) -> &str {
        self.config
            .image
            .as_deref()
            .unwrap_or(DEFAULT_SANDBOX_IMAGE)
    }

    fn container_prefix(&self) -> &str {
        self.config
            .container_prefix
            .as_deref()
            .unwrap_or("moltis-sandbox")
    }

    pub(crate) fn container_name(&self, id: &SandboxId) -> String {
        format!("{}-{}", self.container_prefix(), id.key)
    }

    pub(crate) fn image_repo(&self) -> &str {
        self.container_prefix()
    }

    #[cfg(test)]
    pub(crate) async fn startup_gate_for(&self, name: &str) -> Arc<Semaphore> {
        self.startup_gate_for_inner(name).await
    }

    async fn startup_gate_for_inner(&self, name: &str) -> Arc<Semaphore> {
        let mut gates = self.startup_gates.lock().await;
        Arc::clone(
            gates
                .entry(name.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    async fn remove_startup_gate_if_unshared(&self, name: &str, gate: &Arc<Semaphore>) {
        let mut gates = self.startup_gates.lock().await;
        let Some(stored) = gates.get(name) else {
            return;
        };
        if Arc::ptr_eq(stored, gate) && Arc::strong_count(gate) == 2 {
            gates.remove(name);
        }
    }

    async fn is_container_running(&self, name: &str) -> bool {
        let check = tokio::process::Command::new(self.cli)
            .args(["inspect", "--format", "{{.State.Running}}", name])
            .output()
            .await;

        let Ok(output) = check else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout).trim() == "true"
    }

    async fn container_podman_mode(&self, name: &str) -> Option<String> {
        let format = format!(r#"{{{{ index .Config.Labels "{PODMAN_MODE_LABEL}" }}}}"#);
        let output = tokio::process::Command::new(self.cli)
            .args(["inspect", "--format", &format, name])
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let mode = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!mode.is_empty() && mode != "<no value>").then_some(mode)
    }

    pub(crate) fn podman_mode_label_value(&self, socket_path: Option<&Path>) -> Result<String> {
        if self.config.allow_nested_podman {
            return Ok("nested".to_string());
        }
        let Some(path) = socket_path else {
            return Ok("hardened".to_string());
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;

            let metadata = std::fs::metadata(path).map_err(|error| {
                Error::message(format!(
                    "failed to identify host Podman socket '{}': {error}",
                    path.display()
                ))
            })?;
            Ok(format!(
                "host:{}:{}:{}:{}:{}",
                path.display(),
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec()
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::message(
                "allow_host_podman is supported only on Unix hosts",
            ))
        }
    }

    pub(crate) fn podman_mode_label_args(mode: &str) -> Vec<String> {
        vec!["--label".to_string(), format!("{PODMAN_MODE_LABEL}={mode}")]
    }

    pub(crate) fn podman_mode_matches(actual: Option<&str>, expected: &str) -> bool {
        actual == Some(expected)
    }

    async fn remove_container_checked(&self, name: &str) -> Result<()> {
        let output = tokio::process::Command::new(self.cli)
            .args(["rm", "-f", name])
            .output()
            .await?;
        if output.status.success() {
            return Ok(());
        }
        Err(Error::message(format!(
            "{} failed to remove stale container '{}': {}",
            self.cli,
            name,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    async fn managed_files_policy_matches(&self, name: &str) -> bool {
        let format = format!("{{{{ index .Config.Labels \"{MANAGED_FILES_POLICY_LABEL}\" }}}}");
        let Ok(output) = tokio::process::Command::new(self.cli)
            .args(["inspect", "--format", &format, name])
            .output()
            .await
        else {
            return false;
        };
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim()
                == self.managed_files_policy_fingerprint()
    }

    async fn container_configuration_matches(&self, name: &str, podman_mode: &str) -> bool {
        let managed_files_matches = self.managed_files_policy_matches(name);
        if self.kind != BackendKind::Podman {
            return managed_files_matches.await;
        }

        let (managed_files_matches, actual_podman_mode) =
            tokio::join!(managed_files_matches, self.container_podman_mode(name));
        managed_files_matches
            && Self::podman_mode_matches(actual_podman_mode.as_deref(), podman_mode)
    }

    pub(crate) fn managed_files_policy_fingerprint(&self) -> String {
        let source = host_visible_managed_files_dir(&self.config, Some(self.cli));
        let input = format!(
            "{}\0{}\0{}",
            self.config.managed_files_mount,
            self.config.workspace_mount,
            source.display()
        );
        format!("{:x}", Sha256::digest(input.as_bytes()))
    }

    fn mounted_host_path(&self, id: &SandboxId, guest_path: &str) -> Option<PathBuf> {
        let guest_path = Path::new(guest_path);
        resolve_workspace_guest_path_on_host(&self.config, Some(self.cli), guest_path).or_else(
            || {
                resolve_home_persistence_guest_path_on_host(
                    &self.config,
                    Some(self.cli),
                    id,
                    guest_path,
                )
            },
        )
    }

    fn managed_files_path(&self, guest_path: &str) -> ManagedFilesPath {
        resolve_managed_files_guest_path_on_host(
            &self.config,
            Some(self.cli),
            Path::new(guest_path),
        )
    }

    pub(crate) fn resource_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let limits = &self.config.resource_limits;
        if let Some(ref mem) = limits.memory_limit {
            args.extend(["--memory".to_string(), mem.clone()]);
        }
        if let Some(cpu) = limits.cpu_quota {
            args.extend(["--cpus".to_string(), cpu.to_string()]);
        }
        if let Some(pids) = limits.pids_max {
            args.extend(["--pids-limit".to_string(), pids.to_string()]);
        }
        if let Some(ref gpus) = self.config.gpus {
            args.extend(["--gpus".to_string(), gpus.clone()]);
        }
        args
    }

    pub(crate) fn network_run_args(&self) -> Vec<String> {
        match self.config.network {
            NetworkPolicy::Blocked => vec!["--network=none".to_string()],
            NetworkPolicy::Trusted => {
                // Ensure the container can reach the host proxy on all
                // platforms (Linux needs --add-host; macOS Docker Desktop
                // resolves host.docker.internal automatically).
                let gateway = self.resolve_host_gateway();
                vec![format!("--add-host=host.docker.internal:{gateway}")]
            },
            NetworkPolicy::Bypass => Vec::new(),
        }
    }

    /// Resolve the IP that containers use to reach the host.
    ///
    /// Docker (and Podman >= 5.0) support the special `host-gateway` token in
    /// `--add-host`.  Older Podman versions reject it with:
    ///
    ///   Error: invalid IP address in add-host: "host-gateway"
    ///
    /// For those we resolve the address ourselves: rootless Podman (< 5.0) uses
    /// slirp4netns by default, which maps the host to `10.0.2.2`.  Rootful
    /// Podman uses a bridge whose gateway we can query via
    /// `podman network inspect`.
    pub(crate) fn resolve_host_gateway(&self) -> String {
        if self.kind != BackendKind::Podman {
            return "host-gateway".to_string();
        }

        if podman_supports_host_gateway() {
            return "host-gateway".to_string();
        }

        // Podman < 5.0 — resolve the address manually.
        podman_resolve_host_ip().unwrap_or_else(|| {
            debug!(
                "could not resolve host gateway IP for podman; \
                 falling back to host-gateway (may fail)"
            );
            "host-gateway".to_string()
        })
    }

    pub(crate) fn proxy_exec_env_args(&self) -> Vec<String> {
        if self.config.network != NetworkPolicy::Trusted {
            return Vec::new();
        }
        let proxy_url = format!(
            "http://host.docker.internal:{}",
            moltis_network_filter::DEFAULT_PROXY_PORT
        );
        let mut args = Vec::new();
        for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
            args.extend(["-e".to_string(), format!("{key}={proxy_url}")]);
        }
        for key in ["NO_PROXY", "no_proxy"] {
            args.extend(["-e".to_string(), format!("{key}=localhost,127.0.0.1,::1")]);
        }
        args
    }

    /// Security hardening flags for `docker run`.
    ///
    /// `is_prebuilt` controls whether `--read-only` is applied: prebuilt images
    /// already have packages baked in so the root FS can be read-only, while
    /// non-prebuilt images need a writable root for `apt-get` provisioning.
    pub(crate) fn hardening_args(
        is_prebuilt: bool,
        kind: BackendKind,
        allow_nested_podman: bool,
    ) -> Vec<String> {
        let mut args = Vec::new();
        if kind == BackendKind::Podman && allow_nested_podman {
            args.push("--privileged".to_string());
        } else {
            args.extend([
                // --- Capability / privilege ---
                "--cap-drop".to_string(),
                "ALL".to_string(),
                "--security-opt".to_string(),
                "no-new-privileges".to_string(),
            ]);
        }

        args.extend([
            // --- Writable tmpfs mounts ---
            "--tmpfs".to_string(),
            "/tmp:rw,nosuid,size=256m".to_string(),
            "--tmpfs".to_string(),
            "/run:rw,nosuid,size=64m".to_string(),
            // --- Host metadata isolation ---
            // Give the container its own hostname so /proc/sys/kernel/hostname
            // and the `hostname` command do not reveal the host identity.
            "--hostname".to_string(),
            "sandbox".to_string(),
        ]);
        // Mask /sys subtrees that expose host hardware identifiers
        // (serial numbers, BIOS/UEFI data, disk models, LUKS UUIDs).
        // Empty read-only tmpfs overlays hide the underlying sysfs entries.
        //
        // Skipped for Podman: its OCI runtime performs "tmpcopyup" on sysfs
        // tmpfs mounts, copying directory contents into the tmpfs first.
        // With --cap-drop ALL some sysfs files are permission-denied even for
        // root, causing the mount (and container startup) to fail.  Podman
        // already masks /sys/firmware via its built-in OCI MaskedPaths.
        if kind != BackendKind::Podman {
            args.push("--init".to_string());
            for path in sysfs_paths_to_mask() {
                args.extend(["--tmpfs".to_string(), format!("{path}:ro,nosuid")]);
            }
        }
        if is_prebuilt && !(kind == BackendKind::Podman && allow_nested_podman) {
            args.push("--read-only".to_string());
        }
        args
    }

    pub(crate) fn podman_socket_run_args_for_path(
        socket_path: Option<&Path>,
    ) -> Result<Vec<String>> {
        let Some(socket_path) = socket_path else {
            return Err(Error::message(
                "allow_host_podman is enabled, but no host Podman socket was found; start the Podman API service (for example, `systemctl --user enable --now podman.socket`) and set CONTAINER_HOST if it uses a nonstandard path",
            ));
        };

        let host = format!("unix://{SANDBOX_HOST_PODMAN_SOCKET}");
        Ok(vec![
            "-v".to_string(),
            format!("{}:{SANDBOX_HOST_PODMAN_SOCKET}:rw", socket_path.display()),
            "--security-opt".to_string(),
            "label=disable".to_string(),
            "-e".to_string(),
            format!("CONTAINER_HOST={host}"),
            "-e".to_string(),
            format!("DOCKER_HOST={host}"),
        ])
    }

    pub(crate) async fn is_usable_podman_socket(path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt as _;

            path.is_absolute()
                && std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
                && matches!(
                    tokio::time::timeout(
                        PODMAN_SOCKET_PROBE_TIMEOUT,
                        tokio::net::UnixStream::connect(path)
                    )
                    .await,
                    Ok(Ok(_))
                )
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            false
        }
    }

    async fn host_podman_socket_path() -> Option<PathBuf> {
        for var in ["CONTAINER_HOST", "PODMAN_HOST"] {
            if let Ok(host) = std::env::var(var)
                && let Some(path) = host.strip_prefix("unix://")
            {
                let path = PathBuf::from(path);
                if Self::is_usable_podman_socket(&path).await {
                    return Some(path);
                }
            }
        }

        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let path = PathBuf::from(runtime_dir).join("podman/podman.sock");
            if Self::is_usable_podman_socket(&path).await {
                return Some(path);
            }
        }

        let path = PathBuf::from("/run/podman/podman.sock");
        Self::is_usable_podman_socket(&path).await.then_some(path)
    }

    pub(crate) async fn podman_socket_path(&self) -> Result<Option<PathBuf>> {
        if self.kind != BackendKind::Podman || !self.config.allow_host_podman {
            return Ok(None);
        }

        if cfg!(not(target_os = "linux")) {
            return Err(Error::message(
                "allow_host_podman is supported only on Linux hosts",
            ));
        }

        let socket_path = Self::host_podman_socket_path().await;
        if socket_path.is_none() {
            Self::podman_socket_run_args_for_path(None)?;
        }
        Ok(socket_path)
    }

    /// Mount the host `moltis-ctl` binary into the sandbox at `/usr/local/bin/moltis-ctl`.
    ///
    /// Locates the binary next to the current executable (same directory as `moltis`),
    /// and if found, bind-mounts it read-only. This allows skills to call `moltis-ctl`
    /// inside sandboxes to communicate with the gateway.
    fn moltis_ctl_mount_args() -> Vec<String> {
        let Ok(current_exe) = std::env::current_exe() else {
            return Vec::new();
        };
        let Some(exe_dir) = current_exe.parent() else {
            return Vec::new();
        };
        let ctl_binary = exe_dir.join("moltis-ctl");
        if !ctl_binary.is_file() {
            tracing::debug!(
                path = %ctl_binary.display(),
                "moltis-ctl binary not found next to server, skipping sandbox mount"
            );
            return Vec::new();
        }
        vec![
            "-v".to_string(),
            format!("{}:/usr/local/bin/moltis-ctl:ro", ctl_binary.display()),
        ]
    }

    pub(crate) fn workspace_args(&self) -> Vec<String> {
        let guest_workspace_dir = moltis_config::data_dir();
        let host_workspace_dir = host_visible_data_dir(&self.config, Some(self.cli));
        let guest_workspace_dir_str = guest_workspace_dir.display().to_string();
        let host_workspace_dir_str = host_workspace_dir.display().to_string();
        match self.config.workspace_mount {
            WorkspaceMount::Ro => vec![
                "-v".to_string(),
                format!("{host_workspace_dir_str}:{guest_workspace_dir_str}:ro"),
            ],
            WorkspaceMount::Rw => vec![
                "-v".to_string(),
                format!("{host_workspace_dir_str}:{guest_workspace_dir_str}:rw"),
            ],
            WorkspaceMount::None => Vec::new(),
        }
    }

    pub(crate) fn home_persistence_args(&self, id: &SandboxId) -> Result<Vec<String>> {
        let Some(host_dir) =
            ensure_sandbox_home_persistence_host_dir(&self.config, Some(self.cli), id)?
        else {
            return Ok(Vec::new());
        };
        let volume = format!("{}:{SANDBOX_HOME_DIR}:rw", host_dir.display());
        Ok(vec!["-v".to_string(), volume])
    }

    pub(crate) fn managed_files_args(&self) -> Result<Vec<String>> {
        let legacy_guest_dir = moltis_config::managed_files_dir();
        if self.config.managed_files_mount == ManagedFilesMount::None {
            let _host_dir = ensure_managed_files_host_dir(&self.config, Some(self.cli))?;
            let mut args = vec![
                "--tmpfs".to_string(),
                format!("{SANDBOX_FILES_DIR}:ro,nosuid,nodev,noexec,size=64k"),
            ];
            if self.config.workspace_mount != WorkspaceMount::None {
                args.extend([
                    "--tmpfs".to_string(),
                    format!(
                        "{}:ro,nosuid,nodev,noexec,size=64k",
                        legacy_guest_dir.display()
                    ),
                ]);
            }
            return Ok(args);
        }

        let host_dir = ensure_managed_files_host_dir(&self.config, Some(self.cli))?;
        let mode = self.config.managed_files_mount.to_string();
        let mut args = vec![
            "-v".to_string(),
            format!("{}:{SANDBOX_FILES_DIR}:{mode}", host_dir.display()),
        ];
        if self.config.workspace_mount != WorkspaceMount::None {
            args.extend([
                "-v".to_string(),
                format!(
                    "{}:{}:{mode}",
                    host_dir.display(),
                    legacy_guest_dir.display()
                ),
            ]);
        }
        Ok(args)
    }

    async fn resolve_local_image(&self, requested_image: &str) -> Result<String> {
        if sandbox_image_exists(self.cli, requested_image).await {
            debug!(image = requested_image, "sandbox image found locally");
            return Ok(requested_image.to_string());
        }

        let base_image = self.image().to_string();
        let packages = self.config.packages.clone();
        let Some(rebuild_tag) = rebuildable_sandbox_image_tag(
            requested_image,
            self.image_repo(),
            &base_image,
            &packages,
        ) else {
            return Ok(requested_image.to_string());
        };

        if requested_image == rebuild_tag {
            info!(
                image = requested_image,
                "sandbox image missing locally, rebuilding on demand"
            );
        } else {
            warn!(
                requested = requested_image,
                rebuilt = %rebuild_tag,
                "requested sandbox image missing locally, using deterministic tag from current config"
            );
        }

        let Some(result) = self.build_image(&base_image, &packages).await? else {
            return Ok(requested_image.to_string());
        };
        Ok(result.tag)
    }

    /// Export an image from BuildKit's cache into the Podman store.
    ///
    /// When Podman delegates `podman build` to a BuildKit daemon the image may
    /// land only in BuildKit's internal cache.  This method re-runs the build
    /// with `--output type=docker,dest=<file>` (a BuildKit cache-hit, so
    /// essentially free) and pipes the tarball into `podman load`.
    async fn export_buildkit_image_to_store(
        &self,
        tag: &str,
        dockerfile_path: &Path,
        context_dir: &Path,
    ) -> Result<()> {
        let tar_path = std::env::temp_dir().join(format!(
            "moltis-sandbox-export-{}.tar",
            uuid::Uuid::new_v4()
        ));

        // Re-build with docker-archive output.  The `-t` flag embeds the
        // correct tag in the archive so `podman load` names it correctly.
        // BuildKit's layer cache makes this a near-instant cache hit for the
        // same Dockerfile.
        let export_output = tokio::process::Command::new(self.cli)
            .args([
                "build",
                "--output",
                &format!("type=docker,dest={}", tar_path.display()),
                "-t",
                tag,
                "-f",
            ])
            .arg(dockerfile_path)
            .arg(context_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?;

        if !export_output.status.success() {
            let _ = std::fs::remove_file(&tar_path);
            let stderr = String::from_utf8_lossy(&export_output.stderr);
            return Err(Error::message(format!(
                "podman build --output failed for {tag}: {}",
                stderr.trim()
            )));
        }

        // Load the tarball into the Podman store.
        let load_output = tokio::process::Command::new(self.cli)
            .args(["load", "-i"])
            .arg(&tar_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?;

        let _ = std::fs::remove_file(&tar_path);

        if !load_output.status.success() {
            let stderr = String::from_utf8_lossy(&load_output.stderr);
            return Err(Error::message(format!(
                "podman load failed for {tag}: {}",
                stderr.trim()
            )));
        }

        // Final verification.
        if !sandbox_image_exists(self.cli, tag).await {
            return Err(Error::message(format!(
                "image {tag} still missing from podman store after BuildKit export"
            )));
        }

        info!(tag, "successfully exported BuildKit image to podman store");
        Ok(())
    }

    async fn ensure_ready_locked(
        &self,
        id: &SandboxId,
        image_override: Option<&str>,
    ) -> Result<()> {
        let name = self.container_name(id);
        let podman_socket_path = self.podman_socket_path().await?;
        let podman_mode = self.podman_mode_label_value(podman_socket_path.as_deref())?;

        if self.is_container_running(&name).await {
            if self
                .container_configuration_matches(&name, &podman_mode)
                .await
            {
                debug!(container = %name, "sandbox container already running");
                return Ok(());
            }

            info!(container = %name, "recreating sandbox after container configuration changed");
            self.remove_container_checked(&name).await?;
            self.provisioned.lock().await.remove(&name);
        }

        // Resolve image first so we know whether it's prebuilt (affects hardening).
        let requested_image = image_override.unwrap_or_else(|| self.image());
        let image = self.resolve_local_image(requested_image).await?;
        let is_prebuilt = image.starts_with(&format!("{}:", self.image_repo()));
        debug!(container = %name, %image, is_prebuilt, "resolved sandbox image");

        // Start a new container.
        info!(container = %name, %image, "starting new sandbox container");
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            name.clone(),
            "--label".to_string(),
            format!(
                "{MANAGED_FILES_POLICY_LABEL}={}",
                self.managed_files_policy_fingerprint()
            ),
        ];

        args.extend(self.network_run_args());

        if let Some(ref tz) = self.config.timezone {
            args.extend(["-e".to_string(), format!("TZ={tz}")]);
        }

        args.extend(self.resource_args());
        args.extend(Self::hardening_args(
            is_prebuilt,
            self.kind,
            self.config.allow_nested_podman,
        ));
        args.extend(self.workspace_args());
        args.extend(self.home_persistence_args(id)?);
        args.extend(self.managed_files_args()?);
        if self.kind == BackendKind::Podman {
            args.extend(Self::podman_mode_label_args(&podman_mode));
        }
        if let Some(socket_path) = podman_socket_path.as_deref() {
            args.extend(Self::podman_socket_run_args_for_path(Some(socket_path))?);
        }
        args.extend(Self::moltis_ctl_mount_args());

        args.push(image);
        args.extend(["sleep".to_string(), "infinity".to_string()]);

        let output = tokio::process::Command::new(self.cli)
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_container_name_conflict(&stderr) {
                if self.is_container_running(&name).await
                    && self
                        .container_configuration_matches(&name, &podman_mode)
                        .await
                {
                    debug!(
                        container = %name,
                        "{} run reported a name conflict, existing container is running",
                        self.cli
                    );
                    return Ok(());
                }

                warn!(
                    container = %name,
                    "{} run reported a name conflict for a stale or incompatible container, recreating",
                    self.cli
                );
                self.provisioned.lock().await.remove(&name);
                self.remove_container_checked(&name).await?;

                let retry_output = tokio::process::Command::new(self.cli)
                    .args(&args)
                    .output()
                    .await?;
                if !retry_output.status.success() {
                    let retry_stderr = String::from_utf8_lossy(&retry_output.stderr);
                    return Err(Error::message(format!(
                        "{} run failed after removing stale container '{}': {}",
                        self.cli,
                        name,
                        format_container_run_stderr(self.kind, &retry_stderr)
                    )));
                }
            } else {
                return Err(Error::message(format!(
                    "{} run failed: {}",
                    self.cli,
                    format_container_run_stderr(self.kind, &stderr)
                )));
            }
        }

        // Skip provisioning if the image is a pre-built instance sandbox image
        // (packages are already baked in — including /home/sandbox from the Dockerfile).
        if !is_prebuilt {
            let needs_provisioning = {
                let mut provisioned = self.provisioned.lock().await;
                if provisioned.contains(&name) {
                    false
                } else {
                    provisioned.insert(name.clone());
                    true
                }
            };
            if needs_provisioning {
                if let Err(e) = provision_packages(self.cli, &name, &self.config.packages).await {
                    self.provisioned.lock().await.remove(&name);
                    return Err(e);
                }
            } else {
                debug!(
                    container = %name,
                    "skipping provisioning, already completed for container"
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn backend_name(&self) -> &'static str {
        self.backend_label
    }

    async fn runtime_name(&self, id: &SandboxId) -> Option<String> {
        Some(self.container_name(id))
    }

    fn provides_fs_isolation(&self) -> bool {
        true
    }

    fn exposes_managed_files(&self) -> bool {
        self.config.managed_files_mount != ManagedFilesMount::None
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        let name = self.container_name(id);
        let gate = self.startup_gate_for_inner(&name).await;
        let _permit = gate
            .acquire()
            .await
            .map_err(|_| Error::message("sandbox startup gate closed"))?;
        let result = self.ensure_ready_locked(id, image_override).await;
        if result.is_err() {
            self.remove_startup_gate_if_unshared(&name, &gate).await;
        }
        result
    }

    async fn build_image(
        &self,
        base: &str,
        packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        if packages.is_empty() {
            return Ok(None);
        }

        let tag = sandbox_image_tag(self.image_repo(), base, packages);

        // Check if image already exists.
        if sandbox_image_exists(self.cli, &tag).await {
            debug!(
                tag,
                "pre-built sandbox image already exists, skipping build"
            );
            return Ok(Some(BuildImageResult { tag, built: false }));
        }

        // Generate Dockerfile in a temp dir.
        let tmp_dir =
            std::env::temp_dir().join(format!("moltis-sandbox-build-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir)?;

        let pkg_list = canonical_sandbox_packages(packages).join(" ");
        let dockerfile = sandbox_image_dockerfile(base, packages);
        let dockerfile_path = tmp_dir.join("Dockerfile");
        std::fs::write(&dockerfile_path, &dockerfile)?;

        info!(tag, packages = %pkg_list, "building pre-built sandbox image");

        let output = tokio::process::Command::new(self.cli)
            .args(["build", "-t", &tag, "-f"])
            .arg(&dockerfile_path)
            .arg(&tmp_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        let output = output?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                tag,
                stdout = %tail_lines(&stdout, 20),
                stderr = %tail_lines(&stderr, 20),
                "{} build failed",
                self.cli,
            );
            let status = output.status.code().map_or_else(
                || output.status.to_string(),
                |code| format!("exit code {code}"),
            );
            return Err(Error::message(format!(
                "{} build failed for {tag}: {}",
                self.cli, status
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!(tag, output = %tail_lines(&stdout, 20), "docker build output");

        // Podman with BuildKit: the build may succeed (exit 0) but leave the
        // image in BuildKit's internal cache instead of the Podman store.
        // Verify the image is actually present and recover if not.
        if self.kind == BackendKind::Podman && !sandbox_image_exists(self.cli, &tag).await {
            warn!(
                tag,
                "podman build succeeded but image missing from store \
                 (likely BuildKit delegation), exporting via tarball"
            );
            let export_result = self
                .export_buildkit_image_to_store(&tag, &dockerfile_path, &tmp_dir)
                .await;
            // Clean up temp dir regardless of export result.
            let _ = std::fs::remove_dir_all(&tmp_dir);
            export_result?;
        } else {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }

        info!(tag, "pre-built sandbox image ready");
        Ok(Some(BuildImageResult { tag, built: true }))
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let name = self.container_name(id);

        let mut args = vec!["exec".to_string()];

        if let Some(ref dir) = opts.working_dir {
            args.extend(["-w".to_string(), dir.display().to_string()]);
        }

        // Inject proxy env vars so traffic routes through the trusted-network
        // proxy running on the host.
        args.extend(self.proxy_exec_env_args());

        for (k, v) in &opts.env {
            args.extend(["-e".to_string(), format!("{}={}", k, v)]);
        }

        args.push(name);
        args.extend(["sh".to_string(), "-c".to_string(), command.to_string()]);

        let child = tokio::process::Command::new(self.cli)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                truncate_output_for_display(&mut stdout, opts.max_output_bytes);
                truncate_output_for_display(&mut stderr, opts.max_output_bytes);

                Ok(ExecResult {
                    stdout,
                    stderr,
                    exit_code: output.status.code().unwrap_or(-1),
                })
            },
            Ok(Err(e)) => {
                return Err(Error::message(format!("{} exec failed: {e}", self.cli)));
            },
            Err(_) => {
                return Err(Error::message(format!(
                    "{} exec timed out after {}s",
                    self.cli,
                    opts.timeout.as_secs()
                )));
            },
        }
    }

    async fn read_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        match self.managed_files_path(file_path) {
            ManagedFilesPath::Unavailable => return Ok(SandboxReadResult::PermissionDenied),
            ManagedFilesPath::ReadOnly(_) | ManagedFilesPath::ReadWrite(_) => {
                let container_name = self.container_name(id);
                return oci_container_read_file(self.cli, &container_name, file_path, max_bytes)
                    .await;
            },
            ManagedFilesPath::Unmanaged => {},
        }

        let Some(host_path) = self.mounted_host_path(id, file_path) else {
            let container_name = self.container_name(id);
            return oci_container_read_file(self.cli, &container_name, file_path, max_bytes).await;
        };

        let host_result = match host_path.to_str() {
            Some(host_path) => native_host_read_file(host_path, max_bytes).await,
            None => Err(Error::message("mounted host path contains invalid UTF-8")),
        };
        match host_result {
            Ok(result @ (SandboxReadResult::Ok(_) | SandboxReadResult::TooLarge(_))) => {
                return Ok(result);
            },
            Ok(result) => {
                debug!(
                    guest_path = file_path,
                    host_path = %host_path.display(),
                    ?result,
                    "mounted host read failed; falling back to container read"
                );
            },
            Err(error) => {
                debug!(
                    guest_path = file_path,
                    host_path = %host_path.display(),
                    %error,
                    "mounted host read failed; falling back to container read"
                );
            },
        }

        let container_name = self.container_name(id);
        oci_container_read_file(self.cli, &container_name, file_path, max_bytes).await
    }

    async fn write_file(
        &self,
        id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        match self.managed_files_path(file_path) {
            ManagedFilesPath::Unavailable => {
                return Ok(Some(permission_denied_payload(
                    file_path,
                    "managed Files are disabled in this sandbox",
                )));
            },
            ManagedFilesPath::ReadOnly(_) => {
                return Ok(Some(permission_denied_payload(
                    file_path,
                    "managed Files are mounted read-only in this sandbox",
                )));
            },
            ManagedFilesPath::ReadWrite(_) => {
                let container_name = self.container_name(id);
                return oci_container_write_file(self.cli, &container_name, file_path, content)
                    .await;
            },
            ManagedFilesPath::Unmanaged => {},
        }

        if let Some(host_path) = self.mounted_host_path(id, file_path) {
            let host_result = match host_path.to_str() {
                Some(host_path) => native_host_write_file(host_path, content).await,
                None => Err(Error::message("mounted host path contains invalid UTF-8")),
            };
            match host_result {
                Ok(payload) => return Ok(payload),
                Err(error) => {
                    debug!(
                        guest_path = file_path,
                        host_path = %host_path.display(),
                        %error,
                        "mounted host write failed; falling back to container write"
                    );
                },
            }
        }

        let container_name = self.container_name(id);
        oci_container_write_file(self.cli, &container_name, file_path, content).await
    }

    async fn list_files(&self, id: &SandboxId, root: &str) -> Result<SandboxListFilesResult> {
        match self.managed_files_path(root) {
            ManagedFilesPath::Unavailable => {
                return Err(Error::message("managed Files are disabled in this sandbox"));
            },
            ManagedFilesPath::ReadOnly(_) | ManagedFilesPath::ReadWrite(_) => {
                let container_name = self.container_name(id);
                return oci_container_list_files(self.cli, &container_name, root).await;
            },
            ManagedFilesPath::Unmanaged => {},
        }

        if let Some(host_path) = self.mounted_host_path(id, root) {
            let host_result = match host_path.to_str() {
                Some(host_path) => native_host_list_files_strict(host_path).await,
                None => Err(Error::message("mounted host path contains invalid UTF-8")),
            };
            match host_result {
                Ok(host_files) => {
                    return remap_host_list_result_to_guest(root, &host_path, host_files);
                },
                Err(error) => {
                    debug!(
                        guest_path = root,
                        host_path = %host_path.display(),
                        %error,
                        "mounted host list failed; falling back to container list"
                    );
                },
            }
        }

        let container_name = self.container_name(id);
        oci_container_list_files(self.cli, &container_name, root).await
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let name = self.container_name(id);
        self.provisioned.lock().await.remove(&name);
        self.startup_gates.lock().await.remove(&name);
        let _ = tokio::process::Command::new(self.cli)
            .args(["rm", "-f", &name])
            .output()
            .await;
        Ok(())
    }
}

pub(crate) fn is_container_name_conflict(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("already in use")
        && (lower.contains("container name")
            || lower.contains("the name \"")
            || lower.contains("the name '"))
}

pub(crate) fn is_podman_rootless_reexec_restricted(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("cannot clone") && lower.contains("cannot re-exec process")
}

pub(crate) fn format_container_run_stderr(kind: BackendKind, stderr: &str) -> String {
    let trimmed = stderr.trim();
    if kind != BackendKind::Podman || !is_podman_rootless_reexec_restricted(trimmed) {
        return trimmed.to_string();
    }

    format!(
        "{trimmed}\n\nRootless Podman cannot start from a process restricted by systemd NoNewPrivileges=true or equivalent namespace/privilege hardening. If Moltis uses the bundled system service, install deploy/moltis-podman.conf as a systemd drop-in; custom units must also provide writable Podman home and runtime directories. Running podman inside an already sandboxed Moltis container requires an explicit escape hatch: allow_host_podman=true for host socket passthrough or allow_nested_podman=true for a privileged nested-Podman sandbox."
    )
}

/// No-op sandbox that passes through to direct execution.
pub struct NoSandbox;

#[async_trait]
impl Sandbox for NoSandbox {
    fn backend_name(&self) -> &'static str {
        "none"
    }

    fn is_real(&self) -> bool {
        false
    }

    fn provides_fs_isolation(&self) -> bool {
        false
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        crate::exec::exec_command(command, opts).await
    }

    async fn read_file(
        &self,
        _id: &SandboxId,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<SandboxReadResult> {
        native_host_read_file(file_path, max_bytes).await
    }

    async fn write_file(
        &self,
        _id: &SandboxId,
        file_path: &str,
        content: &[u8],
    ) -> Result<Option<serde_json::Value>> {
        native_host_write_file(file_path, content).await
    }

    async fn list_files(&self, _id: &SandboxId, root: &str) -> Result<SandboxListFilesResult> {
        native_host_list_files(root).await
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

/// Sysfs paths to mask with empty read-only tmpfs overlays.
///
/// On Linux, Docker shares the host kernel's sysfs.  Paths that don't exist
/// on the host (ARM devices without DMI, WSL2, etc.) would cause Docker to
/// fail with "mkdirat: read-only file system" when it tries to create
/// mountpoints on the read-only sysfs.  We probe each path and only mount
/// the ones that actually exist.
///
/// On non-Linux hosts (macOS), Docker Desktop runs in a Linux VM with full
/// sysfs, so all paths are included unconditionally — the host `/sys` layout
/// is irrelevant.
pub(crate) const SYSFS_MASK_PATHS: &[&str] = &[
    "/sys/firmware",
    "/sys/class/dmi",
    "/sys/devices/virtual/dmi",
    "/sys/class/block",
];

pub(crate) fn sysfs_paths_to_mask() -> Vec<&'static str> {
    static PATHS: OnceLock<Vec<&'static str>> = OnceLock::new();
    PATHS
        .get_or_init(|| {
            let paths = sysfs_paths_to_mask_from("/sys");
            let skipped = SYSFS_MASK_PATHS.len() - paths.len();
            if skipped > 0 {
                warn!(
                    skipped,
                    "some sysfs mask paths do not exist on this host and will be skipped"
                );
            }
            paths
        })
        .clone()
}

/// Testable inner helper: probes each `SYSFS_MASK_PATHS` entry and returns
/// only those that exist under `sysfs_root`.  If `sysfs_root` itself doesn't
/// exist (macOS), all paths are returned — Docker Desktop's VM will have them.
pub(crate) fn sysfs_paths_to_mask_from(sysfs_root: &str) -> Vec<&'static str> {
    let root = Path::new(sysfs_root);
    if !root.exists() {
        // Non-Linux host (macOS): Docker runs in a VM with full sysfs.
        return SYSFS_MASK_PATHS.to_vec();
    }
    SYSFS_MASK_PATHS
        .iter()
        .copied()
        .filter(|p| {
            // Strip the canonical "/sys/" prefix so the path is relative,
            // then probe under the supplied root (real or test tempdir).
            let rel = p.strip_prefix("/sys/").unwrap_or(p);
            root.join(rel).exists()
        })
        .collect()
}

/// Return `true` when the installed Podman version supports `host-gateway`
/// in `--add-host` (added in Podman 5.0).
pub(crate) fn podman_supports_host_gateway() -> bool {
    let Ok(output) = std::process::Command::new("podman")
        .args(["version", "--format", "{{.Client.Version}}"])
        .output()
    else {
        return false;
    };
    let version_str = String::from_utf8_lossy(&output.stdout);
    let major: u32 = version_str
        .trim()
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    major >= 5
}

/// Resolve the host IP that a Podman container (< 5.0) can use to reach the
/// host.  Rootless Podman defaults to slirp4netns where the host is always
/// `10.0.2.2`.  Rootful Podman uses a bridge network whose gateway we query
/// with `podman network inspect`.
pub(crate) fn podman_resolve_host_ip() -> Option<String> {
    let rootless = std::process::Command::new("podman")
        .args(["info", "--format", "{{.Host.Security.Rootless}}"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    if rootless.as_deref() == Some("true") {
        // slirp4netns (default rootless network before Podman 5.0) maps the
        // host to 10.0.2.2.
        return Some("10.0.2.2".to_string());
    }

    // Rootful — ask for the gateway of the default "podman" network.
    let output = std::process::Command::new("podman")
        .args([
            "network",
            "inspect",
            "podman",
            "--format",
            "{{(index .Subnets 0).Gateway}}",
        ])
        .output()
        .ok()?;
    let gateway = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if gateway.is_empty() {
        None
    } else {
        Some(gateway)
    }
}
