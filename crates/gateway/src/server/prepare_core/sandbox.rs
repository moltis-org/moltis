//! Sandbox initialization helpers: router construction, background image build,
//! host provisioning, and startup container garbage collection.

use std::sync::{Arc, atomic::Ordering};

use {
    moltis_tools::sandbox::SandboxConfig,
    tracing::{debug, info, warn},
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    server::helpers::should_prebuild_sandbox_image,
    state::GatewayState,
};

/// Type alias for the deferred state used in prepare_core.
type DeferredState = tokio::sync::OnceCell<Arc<GatewayState>>;

/// Build the sandbox router with all configured backends registered.
pub(super) fn build_sandbox_router(
    sandbox_config: &SandboxConfig,
    container_prefix: &str,
    timezone: Option<&str>,
) -> moltis_tools::sandbox::SandboxRouter {
    let mut config = sandbox_config.clone();
    config.container_prefix = Some(container_prefix.to_string());
    config.timezone = timezone.map(ToOwned::to_owned);

    let mut router = moltis_tools::sandbox::SandboxRouter::new(config.clone());

    // Register additional remote backends that have credentials configured.
    // Env vars (VERCEL_TOKEN, DAYTONA_API_KEY) are resolved by the config crate
    // into the config fields, so checking config.*.is_some() is sufficient.
    for (name, has_creds) in [
        ("vercel", config.vercel_token.is_some()),
        ("daytona", config.daytona_api_key.is_some()),
        (
            "firecracker",
            config.firecracker_bin.is_some()
                || std::path::Path::new("/usr/local/bin/firecracker").exists(),
        ),
    ] {
        if has_creds && router.backend_name() != name {
            let backend = moltis_tools::sandbox::router::select_backend_by_name(name, &config);
            if backend.backend_name() == name {
                router.register_backend(backend);
            }
        }
    }

    router
}

/// Spawn background sandbox tasks: image pre-build, host provisioning, and
/// startup container GC.
pub(super) fn spawn_sandbox_background_tasks(
    sandbox_router: &Arc<moltis_tools::sandbox::SandboxRouter>,
    deferred_state: &Arc<DeferredState>,
) {
    // Background image pre-build.
    {
        let router = Arc::clone(sandbox_router);
        let backend = Arc::clone(router.backend());
        let packages = router.config().packages.clone();
        let base_image = router
            .config()
            .image
            .clone()
            .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string());

        if should_prebuild_sandbox_image(router.mode(), &packages) {
            let deferred_for_build = Arc::clone(deferred_state);
            sandbox_router.building_flag.store(true, Ordering::Relaxed);
            let build_router = Arc::clone(sandbox_router);
            tokio::spawn(async move {
                if let Some(state) = deferred_for_build.get() {
                    broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({
                            "phase": "start",
                            "package_count": packages.len(),
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match backend.build_image(&base_image, &packages).await {
                    Ok(Some(result)) => {
                        info!(
                            tag = %result.tag,
                            built = result.built,
                            "sandbox image pre-build complete"
                        );
                        router.set_global_image(Some(result.tag.clone())).await;
                        build_router.building_flag.store(false, Ordering::Relaxed);
                        build_router.build_complete.notify_waiters();

                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                        build_router.building_flag.store(false, Ordering::Relaxed);
                        build_router.build_complete.notify_waiters();
                    },
                    Err(e) => {
                        warn!("sandbox image pre-build failed: {e}");
                        build_router.building_flag.store(false, Ordering::Relaxed);
                        build_router.build_complete.notify_waiters();
                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // Host package provisioning when no container runtime is available.
    {
        let packages = sandbox_router.config().packages.clone();
        if sandbox_router.backend_name() == "none"
            && !packages.is_empty()
            && moltis_tools::sandbox::is_debian_host()
        {
            let deferred_for_host = Arc::clone(deferred_state);
            let pkg_count = packages.len();
            tokio::spawn(async move {
                if let Some(state) = deferred_for_host.get() {
                    broadcast(
                        state,
                        "sandbox.host.provision",
                        serde_json::json!({
                            "phase": "start",
                            "count": pkg_count,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match moltis_tools::sandbox::provision_host_packages(&packages).await {
                    Ok(Some(result)) => {
                        info!(
                            installed = result.installed.len(),
                            skipped = result.skipped.len(),
                            sudo = result.used_sudo,
                            "host package provisioning complete"
                        );
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "installed": result.installed.len(),
                                    "skipped": result.skipped.len(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!("host package provisioning: no-op (not debian or empty packages)");
                    },
                    Err(e) => {
                        warn!("host package provisioning failed: {e}");
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // Startup GC: remove orphaned session containers.
    if sandbox_router.backend_name() != "none" {
        let prefix = sandbox_router.config().container_prefix.clone();
        tokio::spawn(async move {
            if let Some(prefix) = prefix {
                match moltis_tools::sandbox::clean_all_containers(&prefix).await {
                    Ok(0) => {},
                    Ok(n) => info!(
                        removed = n,
                        "startup GC: cleaned orphaned session containers"
                    ),
                    Err(e) => debug!("startup GC: container cleanup skipped: {e}"),
                }
            }
        });
    }
}
