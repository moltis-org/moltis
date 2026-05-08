//! Sandbox initialization helpers: router construction, background image build,
//! host provisioning, and startup container garbage collection.

#[cfg(target_os = "linux")]
use std::path::Path as FsPath;
use std::sync::{Arc, atomic::Ordering};

use {
    moltis_tools::sandbox::SandboxConfig,
    secrecy::{ExposeSecret, Secret},
    tracing::{debug, info, warn},
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    server::helpers::should_prebuild_sandbox_image,
    state::GatewayState,
};

/// Type alias for the deferred state used in prepare_core.
type DeferredState = tokio::sync::OnceCell<Arc<GatewayState>>;

fn has_secret(secret: &Option<Secret<String>>) -> bool {
    secret
        .as_ref()
        .is_some_and(|secret| !secret.expose_secret().is_empty())
}

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
    // into the config fields.
    for (name, has_creds) in [
        ("vercel", has_secret(&config.vercel_token)),
        ("daytona", has_secret(&config.daytona_api_key)),
    ] {
        if has_creds && router.backend_name() != name {
            let backend = moltis_tools::sandbox::router::select_backend_by_name(name, &config);
            if backend.backend_name() == name {
                router.register_backend(backend);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let name = "firecracker";
        let has_creds = moltis_tools::sandbox::firecracker_bin_available(
            config.firecracker_bin.as_deref().map(FsPath::new),
        );
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
        let backends = router.available_backend_instances();
        let default_backend_name = router.backend_name().to_string();
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

                let mut built_any = false;
                let mut images = Vec::new();
                let mut errors = Vec::new();
                let mut default_result = None;

                for backend in backends {
                    let backend_name = backend.backend_name();
                    match backend.build_image(&base_image, &packages).await {
                        Ok(Some(result)) => {
                            info!(
                                backend = backend_name,
                                tag = %result.tag,
                                built = result.built,
                                "sandbox image pre-build complete"
                            );
                            built_any |= result.built;
                            if let Err(error) = router
                                .set_backend_image(backend_name, result.tag.clone())
                                .await
                            {
                                warn!(
                                    backend = backend_name,
                                    %error,
                                    "sandbox image pre-build result could not be registered"
                                );
                                errors.push(serde_json::json!({
                                    "backend": backend_name,
                                    "error": error.to_string(),
                                }));
                                continue;
                            }
                            if backend_name == default_backend_name {
                                router.set_global_image(Some(result.tag.clone())).await;
                                default_result = Some(result.clone());
                            }
                            images.push(serde_json::json!({
                                "backend": backend_name,
                                "tag": result.tag,
                                "built": result.built,
                            }));
                        },
                        Ok(None) => {
                            debug!(
                                backend = backend_name,
                                "sandbox image pre-build: no-op (no packages or unsupported backend)"
                            );
                        },
                        Err(error) => {
                            warn!(
                                backend = backend_name,
                                "sandbox image pre-build failed: {error}"
                            );
                            errors.push(serde_json::json!({
                                "backend": backend_name,
                                "error": error.to_string(),
                            }));
                        },
                    }
                }

                build_router.building_flag.store(false, Ordering::Relaxed);
                build_router.build_complete.notify_waiters();

                if images.is_empty() && errors.is_empty() {
                    debug!("sandbox image pre-build: no-op (no packages or unsupported backends)");
                }

                if let Some(state) = deferred_for_build.get() {
                    if !images.is_empty() {
                        let mut payload = serde_json::json!({
                            "phase": "done",
                            "built": built_any,
                            "images": images,
                            "errors": errors,
                        });
                        if let Some(result) = default_result
                            && let Some(payload) = payload.as_object_mut()
                        {
                            payload.insert("tag".to_string(), serde_json::json!(result.tag));
                        }

                        broadcast(state, "sandbox.image.build", payload, BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        })
                        .await;
                    } else if errors.is_empty() {
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                    } else {
                        broadcast(
                            state,
                            "sandbox.image.build",
                            serde_json::json!({
                                "phase": "error",
                                "error": "sandbox image pre-build failed",
                                "errors": errors,
                            }),
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
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
