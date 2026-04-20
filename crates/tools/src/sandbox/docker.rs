//! Docker and Podman sandbox backends.

use {
    async_trait::async_trait,
    std::collections::HashSet,
    tokio::sync::Mutex,
    tracing::{debug, info, warn},
};

use {
    super::{
        containers::{
            rebuildable_sandbox_image_tag, sandbox_image_dockerfile, sandbox_image_exists,
            sandbox_image_tag,
        },
        host::provision_packages,
        paths::{
            ensure_sandbox_home_persistence_host_dir, host_visible_data_dir,
            resolve_home_persistence_guest_path_on_host, resolve_workspace_guest_path_on_host,
        },
        types::{
            BuildImageResult, DEFAULT_SANDBOX_IMAGE, NetworkPolicy, SANDBOX_HOME_DIR, Sandbox,
            SandboxConfig, SandboxId, WorkspaceMount, canonical_sandbox_packages, tail_lines,
            truncate_output_for_display,
        },
    },
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
        sandbox::file_system::{
            SandboxListFilesResult, SandboxReadResult, native_host_list_files,
            native_host_read_file, native_host_write_file, oci_container_list_files,
            oci_container_read_file, oci_container_write_file, remap_host_list_result_to_guest,
        },
    },
};

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
}

impl DockerSandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            kind: BackendKind::Docker,
            cli: "docker",
            backend_label: "docker",
            provisioned: Mutex::new(HashSet::new()),
        }
    }

    pub fn podman(config: SandboxConfig) -> Self {
        Self {
            config,
            kind: BackendKind::Podman,
            cli: "podman",
            backend_label: "podman",
            provisioned: Mutex::new(HashSet::new()),
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

    fn mounted_host_path(&self, id: &SandboxId, guest_path: &str) -> Option<std::path::PathBuf> {
        let guest_path = std::path::Path::new(guest_path);
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
    pub(crate) fn hardening_args(is_prebuilt: bool, kind: BackendKind) -> Vec<String> {
        let mut args = vec![
            // --- Capability / privilege ---
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges".to_string(),
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
        ];
        // Mask /sys subtrees that expose host hardware identifiers
        // (serial numbers, BIOS/UEFI data, disk models, LUKS UUIDs).
        // Empty read-only tmpfs overlays hide the underlying sysfs entries.
        //
        // Podman is excluded: its OCI runtime performs "tmpcopyup" on sysfs
        // tmpfs mounts, copying directory contents into the tmpfs first.
        // With --cap-drop ALL some sysfs files are permission-denied even for
        // root, causing the mount (and container startup) to fail.  Podman
        // already masks /sys/firmware via its built-in OCI MaskedPaths.
        if kind != BackendKind::Podman {
            args.extend([
                "--tmpfs".to_string(),
                "/sys/firmware:ro,nosuid".to_string(),
                "--tmpfs".to_string(),
                "/sys/class/dmi:ro,nosuid".to_string(),
                "--tmpfs".to_string(),
                "/sys/devices/virtual/dmi:ro,nosuid".to_string(),
                "--tmpfs".to_string(),
                "/sys/class/block:ro,nosuid".to_string(),
            ]);
        }
        if is_prebuilt {
            args.push("--read-only".to_string());
        }
        args
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
        dockerfile_path: &std::path::Path,
        context_dir: &std::path::Path,
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
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn backend_name(&self) -> &'static str {
        self.backend_label
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        let name = self.container_name(id);

        // Check if container already running.
        let check = tokio::process::Command::new(self.cli)
            .args(["inspect", "--format", "{{.State.Running}}", &name])
            .output()
            .await;

        if let Ok(output) = check {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim() == "true" {
                debug!(container = %name, "sandbox container already running");
                return Ok(());
            }
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
        ];

        args.extend(self.network_run_args());

        if let Some(ref tz) = self.config.timezone {
            args.extend(["-e".to_string(), format!("TZ={tz}")]);
        }

        args.extend(self.resource_args());
        args.extend(Self::hardening_args(is_prebuilt, self.kind));
        args.extend(self.workspace_args());
        args.extend(self.home_persistence_args(id)?);

        args.push(image.clone());
        args.extend(["sleep".to_string(), "infinity".to_string()]);

        let output = tokio::process::Command::new(self.cli)
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::message(format!(
                "{} run failed: {}",
                self.cli,
                stderr.trim()
            )));
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
            info!(
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
            warn!(
                tag,
                stdout = %tail_lines(&stdout, 20),
                stderr = %stderr.trim(),
                "{} build failed",
                self.cli,
            );
            return Err(Error::message(format!(
                "{} build failed for {tag}: {}",
                self.cli,
                stderr.trim()
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
        if let Some(host_path) = self.mounted_host_path(id, file_path) {
            return native_host_read_file(
                host_path
                    .to_str()
                    .ok_or_else(|| Error::message("mounted host path contains invalid UTF-8"))?,
                max_bytes,
            )
            .await;
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
        if let Some(host_path) = self.mounted_host_path(id, file_path) {
            return native_host_write_file(
                host_path
                    .to_str()
                    .ok_or_else(|| Error::message("mounted host path contains invalid UTF-8"))?,
                content,
            )
            .await;
        }

        let container_name = self.container_name(id);
        oci_container_write_file(self.cli, &container_name, file_path, content).await
    }

    async fn list_files(&self, id: &SandboxId, root: &str) -> Result<SandboxListFilesResult> {
        if let Some(host_path) = self.mounted_host_path(id, root) {
            let host_files = native_host_list_files(
                host_path
                    .to_str()
                    .ok_or_else(|| Error::message("mounted host path contains invalid UTF-8"))?,
            )
            .await?;
            return remap_host_list_result_to_guest(root, &host_path, host_files);
        }

        let container_name = self.container_name(id);
        oci_container_list_files(self.cli, &container_name, root).await
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        let name = self.container_name(id);
        self.provisioned.lock().await.remove(&name);
        let _ = tokio::process::Command::new(self.cli)
            .args(["rm", "-f", &name])
            .output()
            .await;
        Ok(())
    }
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
