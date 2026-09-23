//! Bind-mount and `run_as` assembly for the Docker and Podman backends.
//!
//! Its own module rather than more methods on `docker.rs`: everything here
//! answers one question - which host paths the container gets and which uid
//! writes to them - and that is the part of the run assembly a reviewer has to
//! read closely. Keeping it separate also holds `docker.rs` under the repo's
//! file-size gate.

use {
    super::{
        docker::DockerSandbox,
        paths::{
            ManagedFilesPath, ensure_managed_files_host_dir,
            ensure_sandbox_home_persistence_host_dir, host_visible_data_dir,
            resolve_home_persistence_guest_path_on_host, resolve_managed_files_guest_path_on_host,
            resolve_workspace_guest_path_on_host,
        },
        types::{
            MOLTIS_CTL_GUEST_PATH, ManagedFilesMount, SANDBOX_FILES_DIR, SANDBOX_HOME_DIR,
            SandboxId, SandboxMount, SandboxUser, WorkspaceMount,
        },
    },
    crate::error::Result,
    std::path::{Path, PathBuf},
};

impl DockerSandbox {
    /// The `run_as` the container for this id was started with, if any.
    ///
    /// Reads what `ensure_ready_with` recorded rather than re-deriving it from
    /// a policy that may have changed since, because the host paths this
    /// answers for are the ones the running container actually has mounted.
    pub(crate) fn recorded_run_as(&self, id: &SandboxId) -> Option<SandboxUser> {
        let name = self.container_name(id);
        self.run_as_by_container
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&name)
            .copied()
    }

    pub(crate) fn mounted_host_path(&self, id: &SandboxId, guest_path: &str) -> Option<PathBuf> {
        let guest_path = Path::new(guest_path);
        let run_as = self.recorded_run_as(id);
        resolve_workspace_guest_path_on_host(&self.config, Some(self.cli), guest_path).or_else(
            || {
                resolve_home_persistence_guest_path_on_host(
                    &self.config,
                    Some(self.cli),
                    id,
                    run_as.as_ref(),
                    guest_path,
                )
            },
        )
    }

    pub(crate) fn managed_files_path(&self, guest_path: &str) -> ManagedFilesPath {
        resolve_managed_files_guest_path_on_host(
            &self.config,
            Some(self.cli),
            Path::new(guest_path),
        )
    }

    /// Mount the host `moltis-ctl` binary into the sandbox at `/usr/local/bin/moltis-ctl`.
    ///
    /// Locates the binary next to the current executable (same directory as `moltis`),
    /// and if found, bind-mounts it read-only. This allows skills to call `moltis-ctl`
    /// inside sandboxes to communicate with the gateway.
    pub(crate) fn moltis_ctl_mount_args() -> Vec<String> {
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
            format!("{}:{MOLTIS_CTL_GUEST_PATH}:ro", ctl_binary.display()),
        ]
    }

    /// `-v source:target:access` for each extra mount this agent asked for.
    ///
    /// The rules run again here, right before `docker run`. This is the last
    /// place a bad mount can be caught, and it must fail the container start
    /// rather than quietly drop the argument: a container missing a mount the
    /// agent believes it has is far worse than a container that did not start.
    pub(crate) fn extra_mount_args(mounts: &[SandboxMount]) -> Result<Vec<String>> {
        if mounts.is_empty() {
            return Ok(Vec::new());
        }
        let configs = mounts
            .iter()
            .map(SandboxMount::to_config)
            .collect::<Vec<_>>();
        let checked = SandboxMount::try_from_configs(&configs)?;
        let mut args = Vec::with_capacity(checked.len() * 2);
        for mount in &checked {
            args.push("-v".to_string());
            args.push(mount.to_arg());
        }
        Ok(args)
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

    pub(crate) fn home_persistence_args(
        &self,
        id: &SandboxId,
        run_as: Option<&SandboxUser>,
    ) -> Result<Vec<String>> {
        let Some(host_dir) =
            ensure_sandbox_home_persistence_host_dir(&self.config, Some(self.cli), id, run_as)?
        else {
            // Persistence is off, so there is no host dir to own and no
            // precondition to check - but the image's /home/sandbox is
            // root:root 0755, and a prebuilt image additionally gets
            // --read-only. Without a writable HOME a run_as container cannot
            // write its own dotfiles. The nosuid and size= match the siblings
            // in `hardening_args`. SANDBOX_FILES_DIR is nested under this
            // tmpfs, and docker mounts parents before children, so the
            // managed-files bind still lands on top.
            return Ok(match run_as {
                Some(user) => vec![
                    "--tmpfs".to_string(),
                    format!(
                        "{SANDBOX_HOME_DIR}:rw,nosuid,size=256m,uid={},gid={}",
                        user.uid(),
                        user.gid()
                    ),
                ],
                None => Vec::new(),
            });
        };
        let volume = format!("{}:{SANDBOX_HOME_DIR}:rw", host_dir.display());
        Ok(vec!["-v".to_string(), volume])
    }

    /// `--user uid:gid` for the run assembly, empty when no `run_as` is set.
    ///
    /// Its own helper next to `hardening_args` because that is what it is: the
    /// difference between a container writing root-owned files into a host
    /// bind mount and one writing files the operator can read, move and delete.
    ///
    /// The rules run again here, right before `docker run`, exactly as
    /// `extra_mount_args` re-runs them. `SandboxUser` has private fields and a
    /// checked constructor, but that is a claim about construction, and this is
    /// the last place a `--user 0:0` can be stopped from reaching the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error when the user is not one this backend may run as.
    pub(crate) fn run_as_args(run_as: Option<&SandboxUser>) -> Result<Vec<String>> {
        let Some(user) = run_as else {
            return Ok(Vec::new());
        };
        let checked = SandboxUser::try_from(user.to_arg().as_str())?;
        Ok(vec!["--user".to_string(), checked.to_arg()])
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
}
