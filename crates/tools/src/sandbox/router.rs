//! Sandbox orchestration: backend selection, failover, routing.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use {
    async_trait::async_trait,
    tokio::sync::RwLock,
    tracing::{debug, warn},
};

#[cfg(target_os = "macos")]
use super::apple::{AppleContainerSandbox, ensure_apple_container_service};
#[cfg(feature = "wasm")]
use super::wasm::WasmSandbox;
use {
    super::{
        containers::{
            is_apple_container_corruption_error, is_cli_available, is_docker_daemon_available,
            should_use_docker_backend,
        },
        docker::{DockerSandbox, NoSandbox},
        platform::RestrictedHostSandbox,
        types::{
            BuildImageResult, DEFAULT_SANDBOX_IMAGE, Sandbox, SandboxConfig, SandboxId, SandboxMode,
        },
    },
    crate::{
        error::{Error, Result},
        exec::{ExecOpts, ExecResult},
    },
};

/// Wrapper sandbox that can fail over from a primary backend to a fallback backend.
///
/// This is used on macOS to fail over from Apple Container to Docker when the
/// Apple runtime enters a corrupted state (stale metadata, missing config.json,
/// service errors, etc.).
pub struct FailoverSandbox {
    primary: Arc<dyn Sandbox>,
    fallback: Arc<dyn Sandbox>,
    primary_name: &'static str,
    fallback_name: &'static str,
    use_fallback: RwLock<bool>,
}

impl FailoverSandbox {
    pub fn new(primary: Arc<dyn Sandbox>, fallback: Arc<dyn Sandbox>) -> Self {
        let primary_name = primary.backend_name();
        let fallback_name = fallback.backend_name();
        Self {
            primary,
            fallback,
            primary_name,
            fallback_name,
            use_fallback: RwLock::new(false),
        }
    }

    async fn fallback_enabled(&self) -> bool {
        *self.use_fallback.read().await
    }

    async fn switch_to_fallback(&self, error: &Error) {
        let mut use_fallback = self.use_fallback.write().await;
        if !*use_fallback {
            warn!(
                primary = self.primary_name,
                fallback = self.fallback_name,
                %error,
                "sandbox primary backend failed, switching to fallback backend"
            );
            *use_fallback = true;
        }
    }

    fn should_failover(&self, error: &Error) -> bool {
        let message = format!("{error:#}");
        match self.primary_name {
            "apple-container" => is_apple_container_corruption_error(&message),
            "docker" => is_docker_failover_error(&message),
            "podman" => is_podman_failover_error(&message),
            _ => false,
        }
    }
}

#[async_trait]
impl Sandbox for FailoverSandbox {
    fn backend_name(&self) -> &'static str {
        self.primary_name
    }

    async fn ensure_ready(&self, id: &SandboxId, image_override: Option<&str>) -> Result<()> {
        if self.fallback_enabled().await {
            return self.fallback.ensure_ready(id, image_override).await;
        }

        match self.primary.ensure_ready(id, image_override).await {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                let primary_message = format!("{primary_error:#}");
                self.fallback
                    .ensure_ready(id, image_override)
                    .await
                    .map_err(|fallback_error| {
                        Error::message(format!(
                            "primary sandbox backend ({}) failed: {}; fallback backend ({}) also failed: {}",
                            self.primary_name,
                            primary_message,
                            self.fallback_name,
                            fallback_error
                        ))
                    })
            },
        }
    }

    async fn exec(&self, id: &SandboxId, command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        if self.fallback_enabled().await {
            return self.fallback.exec(id, command, opts).await;
        }

        match self.primary.exec(id, command, opts).await {
            Ok(result) => Ok(result),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                let primary_message = format!("{primary_error:#}");
                self.fallback
                    .ensure_ready(id, None)
                    .await
                    .map_err(|fallback_error| {
                        Error::message(format!(
                            "primary sandbox backend ({}) failed during exec: {}; fallback backend ({}) failed to initialize: {}",
                            self.primary_name,
                            primary_message,
                            self.fallback_name,
                            fallback_error
                        ))
                    })?;
                self.fallback.exec(id, command, opts).await
            },
        }
    }

    async fn cleanup(&self, id: &SandboxId) -> Result<()> {
        if self.fallback_enabled().await {
            let result = self.fallback.cleanup(id).await;
            if let Err(error) = self.primary.cleanup(id).await {
                debug!(
                    backend = self.primary_name,
                    %error,
                    "primary sandbox cleanup failed after failover"
                );
            }
            return result;
        }

        self.primary.cleanup(id).await
    }

    async fn build_image(
        &self,
        base: &str,
        packages: &[String],
    ) -> Result<Option<BuildImageResult>> {
        if self.fallback_enabled().await {
            return self.fallback.build_image(base, packages).await;
        }

        match self.primary.build_image(base, packages).await {
            Ok(result) => Ok(result),
            Err(primary_error) => {
                if !self.should_failover(&primary_error) {
                    return Err(primary_error);
                }

                self.switch_to_fallback(&primary_error).await;
                self.fallback.build_image(base, packages).await
            },
        }
    }
}

/// Create the appropriate sandbox backend based on config and platform.
pub fn create_sandbox(config: SandboxConfig) -> Arc<dyn Sandbox> {
    if config.mode == SandboxMode::Off {
        return Arc::new(NoSandbox);
    }

    select_backend(config)
}

/// Create a real sandbox backend regardless of mode (for use by SandboxRouter,
/// which may need a real backend even when global mode is Off because per-session
/// overrides can enable sandboxing dynamically).
pub(crate) fn create_sandbox_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    select_backend(config)
}

/// Select the sandbox backend based on config and platform availability.
///
/// When `backend` is `"auto"` (the default):
/// - On macOS, prefer Apple Container if the `container` CLI is installed
///   (each sandbox runs in a lightweight VM — stronger isolation than Docker).
/// - Prefer Podman (daemonless, rootless) over Docker when available.
/// - Fall back to Docker, then restricted-host otherwise.
pub(crate) fn select_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    match config.backend.as_str() {
        "docker" => Arc::new(DockerSandbox::new(config)),
        "podman" => Arc::new(DockerSandbox::podman(config)),
        #[cfg(target_os = "macos")]
        "apple-container" => {
            if !ensure_apple_container_service() {
                tracing::warn!(
                    "apple container service could not be started; \
                     run `container system start` manually, then restart moltis"
                );
            }
            let apple_backend: Arc<dyn Sandbox> =
                Arc::new(AppleContainerSandbox::new(config.clone()));
            maybe_wrap_with_failover(apple_backend, &config)
        },
        "restricted-host" => {
            tracing::info!("sandbox backend: restricted-host (env clearing, rlimits)");
            Arc::new(RestrictedHostSandbox::new(config))
        },
        "wasm" | "wasmtime" => create_wasm_backend(config),
        _ => auto_detect_backend(config),
    }
}

/// Create a WASM sandbox backend, falling back to `RestrictedHostSandbox` if
/// the feature is disabled or initialisation fails.
fn create_wasm_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    #[cfg(feature = "wasm")]
    {
        match WasmSandbox::new(config.clone()) {
            Ok(sandbox) => {
                tracing::info!("sandbox backend: wasm (WASI-isolated execution)");
                Arc::new(sandbox)
            },
            Err(e) => {
                tracing::warn!(%e, "failed to initialize wasmtime engine, falling back to restricted-host");
                Arc::new(RestrictedHostSandbox::new(config))
            },
        }
    }
    #[cfg(not(feature = "wasm"))]
    {
        tracing::warn!("wasm sandbox requested but feature not compiled in; using restricted-host");
        Arc::new(RestrictedHostSandbox::new(config))
    }
}

/// Wrap a primary sandbox backend with a failover chain.
///
/// Tries Podman, then Docker as fallback, then restricted-host, returning the
/// primary unwrapped if no fallback runtime is available.
#[cfg(target_os = "macos")]
fn maybe_wrap_with_failover(primary: Arc<dyn Sandbox>, config: &SandboxConfig) -> Arc<dyn Sandbox> {
    let primary_name = primary.backend_name();

    // Try Podman as fallback (skip if primary is already Podman).
    if primary_name != "podman" && is_cli_available("podman") {
        tracing::info!(
            primary = primary_name,
            fallback = "podman",
            "sandbox backend failover enabled"
        );
        return Arc::new(FailoverSandbox::new(
            primary,
            Arc::new(DockerSandbox::podman(config.clone())),
        ));
    }

    // Try Docker as fallback (skip if primary is already Docker).
    if primary_name != "docker"
        && should_use_docker_backend(is_cli_available("docker"), is_docker_daemon_available())
    {
        tracing::info!(
            primary = primary_name,
            fallback = "docker",
            "sandbox backend failover enabled"
        );
        return Arc::new(FailoverSandbox::new(
            primary,
            Arc::new(DockerSandbox::new(config.clone())),
        ));
    }

    // Use restricted-host as fallback if no OCI runtime is available.
    tracing::info!(
        primary = primary_name,
        fallback = "restricted-host",
        "sandbox backend failover enabled (restricted-host)"
    );
    Arc::new(FailoverSandbox::new(
        primary,
        Arc::new(RestrictedHostSandbox::new(config.clone())),
    ))
}

/// Check whether an error message indicates a Docker daemon connectivity issue.
pub(crate) fn is_docker_failover_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("cannot connect to the docker daemon")
        || lower.contains("is the docker daemon running")
        || lower.contains("error during connect")
        || lower.contains("connection refused")
}

/// Check whether an error message indicates a Podman runtime issue that warrants
/// failover. Podman is daemonless so most Docker-daemon errors don't apply, but
/// socket/service errors or missing runtimes do.
pub(crate) fn is_podman_failover_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("cannot connect to podman")
        || lower.contains("no such file or directory") && lower.contains("podman")
        || lower.contains("connection refused")
        || lower.contains("runtime") && lower.contains("not found")
}

pub fn auto_detect_backend(config: SandboxConfig) -> Arc<dyn Sandbox> {
    #[cfg(target_os = "macos")]
    {
        if is_cli_available("container") {
            if ensure_apple_container_service() {
                tracing::info!("sandbox backend: apple-container (VM-isolated, preferred)");
                let apple_backend: Arc<dyn Sandbox> =
                    Arc::new(AppleContainerSandbox::new(config.clone()));
                return maybe_wrap_with_failover(apple_backend, &config);
            }
            tracing::warn!(
                "apple container CLI found but service could not be started; \
                 falling back to podman/docker"
            );
        }
    }

    // Prefer Podman (daemonless, rootless by default) over Docker.
    if is_cli_available("podman") {
        tracing::info!("sandbox backend: podman (daemonless, preferred over docker)");
        return Arc::new(DockerSandbox::podman(config));
    }

    if should_use_docker_backend(is_cli_available("docker"), is_docker_daemon_available()) {
        tracing::info!("sandbox backend: docker");
        return Arc::new(DockerSandbox::new(config));
    }

    if is_cli_available("docker") {
        tracing::warn!(
            "docker CLI detected but daemon is not accessible; \
             falling back to restricted-host sandbox"
        );
    }

    // Use restricted-host sandbox before falling back to NoSandbox.
    tracing::info!(
        "sandbox backend: restricted-host (env clearing, rlimits; no container runtime available)"
    );
    Arc::new(RestrictedHostSandbox::new(config))
}

/// Events emitted by the sandbox subsystem for UI feedback.
#[derive(Debug, Clone)]
pub enum SandboxEvent {
    /// First-run container/image setup is about to begin for a session.
    Preparing {
        session_key: String,
        backend: String,
        image: String,
    },
    /// First-run container/image setup completed for a session.
    Prepared {
        session_key: String,
        backend: String,
        image: String,
    },
    /// First-run container/image setup failed for a session.
    PrepareFailed {
        session_key: String,
        backend: String,
        image: String,
        error: String,
    },
    /// Package provisioning started (Apple Container per-container install).
    Provisioning {
        container: String,
        packages: Vec<String>,
    },
    /// Package provisioning finished.
    Provisioned { container: String },
    /// Package provisioning failed (non-fatal).
    ProvisionFailed { container: String, error: String },
}

/// Routes sandbox decisions per-session, with per-session overrides on top of global config.
pub struct SandboxRouter {
    config: SandboxConfig,
    backend: Arc<dyn Sandbox>,
    /// Per-session overrides: true = sandboxed, false = direct execution.
    overrides: RwLock<HashMap<String, bool>>,
    /// Per-session image overrides.
    image_overrides: RwLock<HashMap<String, String>>,
    /// Runtime override for the global default image (set via API, persisted externally).
    global_image_override: RwLock<Option<String>>,
    /// Event channel for sandbox lifecycle events (prepare/provision/build feedback).
    event_tx: tokio::sync::broadcast::Sender<SandboxEvent>,
    /// Session keys that have already completed sandbox initialization.
    /// Used to avoid repeating first-run preparation banners on every command.
    prepared_sessions: RwLock<HashSet<String>>,
    /// Whether a sandbox image pre-build is currently in progress.
    /// Used by the gateway to show a banner in the UI.
    pub building_flag: std::sync::atomic::AtomicBool,
}

impl SandboxRouter {
    pub fn new(config: SandboxConfig) -> Self {
        // Always create a real sandbox backend, even when global mode is Off,
        // because per-session overrides can enable sandboxing dynamically.
        let backend = create_sandbox_backend(config.clone());
        let (event_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            config,
            backend,
            overrides: RwLock::new(HashMap::new()),
            image_overrides: RwLock::new(HashMap::new()),
            global_image_override: RwLock::new(None),
            event_tx,
            prepared_sessions: RwLock::new(HashSet::new()),
            building_flag: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a router with a custom sandbox backend (useful for testing).
    pub fn with_backend(config: SandboxConfig, backend: Arc<dyn Sandbox>) -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            config,
            backend,
            overrides: RwLock::new(HashMap::new()),
            image_overrides: RwLock::new(HashMap::new()),
            global_image_override: RwLock::new(None),
            event_tx,
            prepared_sessions: RwLock::new(HashSet::new()),
            building_flag: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Subscribe to sandbox lifecycle events.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<SandboxEvent> {
        self.event_tx.subscribe()
    }

    /// Emit a sandbox event. Silently drops if no subscribers.
    pub fn emit_event(&self, event: SandboxEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Mark a session as preparing for sandbox first-run work.
    /// Returns `true` only the first time for a session key.
    pub async fn mark_preparing_once(&self, session_key: &str) -> bool {
        self.prepared_sessions
            .write()
            .await
            .insert(session_key.to_string())
    }

    /// Clear preparation marker for a session (used on cleanup or prepare failure).
    pub async fn clear_prepared_session(&self, session_key: &str) {
        self.prepared_sessions.write().await.remove(session_key);
    }

    /// Check whether a session should run sandboxed.
    /// Returns `false` when no real container runtime is available, regardless of
    /// config mode or per-session overrides. Otherwise, per-session override takes
    /// priority, then falls back to global mode.
    pub async fn is_sandboxed(&self, session_key: &str) -> bool {
        if !self.backend.is_real() {
            return false;
        }
        if let Some(&override_val) = self.overrides.read().await.get(session_key) {
            return override_val;
        }
        match self.config.mode {
            SandboxMode::Off => false,
            SandboxMode::All => true,
            SandboxMode::NonMain => session_key != "main",
        }
    }

    /// Set a per-session sandbox override.
    pub async fn set_override(&self, session_key: &str, enabled: bool) {
        self.overrides
            .write()
            .await
            .insert(session_key.to_string(), enabled);
    }

    /// Remove a per-session override (revert to global mode).
    pub async fn remove_override(&self, session_key: &str) {
        self.overrides.write().await.remove(session_key);
    }

    /// Derive a SandboxId for a given session key.
    /// The key is sanitized for use as a container name (only alphanumeric, dash, underscore, dot).
    pub fn sandbox_id_for(&self, session_key: &str) -> SandboxId {
        let sanitized: String = session_key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        SandboxId {
            scope: self.config.scope.clone(),
            key: sanitized,
        }
    }

    /// Clean up sandbox resources for a session.
    pub async fn cleanup_session(&self, session_key: &str) -> Result<()> {
        let id = self.sandbox_id_for(session_key);
        self.backend.cleanup(&id).await?;
        self.remove_override(session_key).await;
        self.clear_prepared_session(session_key).await;
        Ok(())
    }

    /// Access the sandbox backend.
    pub fn backend(&self) -> &Arc<dyn Sandbox> {
        &self.backend
    }

    /// Access the global sandbox mode.
    pub fn mode(&self) -> &SandboxMode {
        &self.config.mode
    }

    /// Access the global sandbox config.
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Human-readable name of the sandbox backend (e.g. "docker", "apple-container").
    pub fn backend_name(&self) -> &'static str {
        self.backend.backend_name()
    }

    /// Set a per-session image override.
    pub async fn set_image_override(&self, session_key: &str, image: String) {
        self.image_overrides
            .write()
            .await
            .insert(session_key.to_string(), image);
    }

    /// Remove a per-session image override.
    pub async fn remove_image_override(&self, session_key: &str) {
        self.image_overrides.write().await.remove(session_key);
    }

    /// Set a runtime override for the global default image.
    /// Pass `None` to revert to the config/hardcoded default.
    pub async fn set_global_image(&self, image: Option<String>) {
        *self.global_image_override.write().await = image;
    }

    /// Get the current effective default image (runtime override > config > hardcoded).
    pub async fn default_image(&self) -> String {
        if let Some(ref img) = *self.global_image_override.read().await {
            return img.clone();
        }
        self.config
            .image
            .clone()
            .unwrap_or_else(|| DEFAULT_SANDBOX_IMAGE.to_string())
    }

    /// Resolve the container image for a session.
    ///
    /// Priority (highest to lowest):
    /// 1. `skill_image` — from a skill's Dockerfile cache
    /// 2. Per-session override (`session.sandbox_image`)
    /// 3. Runtime global override (`set_global_image`)
    /// 4. Global config (`config.tools.exec.sandbox.image`)
    /// 5. Default constant (`DEFAULT_SANDBOX_IMAGE`)
    pub async fn resolve_image(&self, session_key: &str, skill_image: Option<&str>) -> String {
        if let Some(img) = skill_image {
            return img.to_string();
        }
        if let Some(img) = self.image_overrides.read().await.get(session_key) {
            return img.clone();
        }
        self.default_image().await
    }
}
