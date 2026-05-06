//! Sandbox FFI exports: status, image management, container lifecycle, and config.

use std::{collections::HashMap, ffi::c_char};

use moltis_tools::image_cache::ImageBuilder;

use crate::{
    helpers::{
        encode_error, encode_json, parse_ffi_request, record_call, record_error,
        sandbox_backend_name, sandbox_container_prefix, sandbox_effective_default_image,
        sandbox_shared_home_config_from_config, sandbox_status_from_config, trace_call,
        with_ffi_boundary,
    },
    state::BRIDGE,
    types::*,
};

/// Returns sandbox runtime status used by Settings > Sandboxes.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_status() -> *mut c_char {
    record_call("moltis_sandbox_status");
    trace_call("moltis_sandbox_status");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        encode_json(&sandbox_status_from_config(&config))
    })
}

/// Returns cached tool and sandbox images.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_list_images() -> *mut c_char {
    record_call("moltis_sandbox_list_images");
    trace_call("moltis_sandbox_list_images");

    with_ffi_boundary(|| {
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let (cached, sandbox) = BRIDGE.runtime.block_on(async {
            tokio::join!(
                builder.list_cached(),
                moltis_tools::sandbox::list_sandbox_images()
            )
        });

        let mut images = Vec::new();

        if let Ok(list) = cached {
            images.extend(list.into_iter().map(|img| SandboxImageEntry {
                tag: img.tag,
                size: img.size,
                created: img.created,
                kind: "tool".to_owned(),
            }));
        }

        if let Ok(list) = sandbox {
            images.extend(list.into_iter().map(|img| SandboxImageEntry {
                tag: img.tag,
                size: img.size,
                created: img.created,
                kind: "sandbox".to_owned(),
            }));
        }

        encode_json(&SandboxImagesResponse { images })
    })
}

/// Deletes one cached image by tag.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_delete_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_delete_image");
    trace_call("moltis_sandbox_delete_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxDeleteImageRequest>(
            "moltis_sandbox_delete_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let tag = request.tag.trim();
        if tag.is_empty() {
            record_error("moltis_sandbox_delete_image", "IMAGE_TAG_REQUIRED");
            return encode_error("IMAGE_TAG_REQUIRED", "tag is required");
        }

        let result = BRIDGE.runtime.block_on(async {
            if tag.contains("-sandbox:") {
                moltis_tools::sandbox::remove_sandbox_image(tag).await
            } else {
                let builder = moltis_tools::image_cache::DockerImageBuilder::new();
                let full_tag = if tag.starts_with("moltis-cache/") {
                    tag.to_owned()
                } else {
                    format!("moltis-cache/{tag}")
                };
                builder.remove_cached(&full_tag).await
            }
        });

        match result {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_sandbox_delete_image", IMAGE_CACHE_DELETE_FAILED);
                encode_error(IMAGE_CACHE_DELETE_FAILED, &error.to_string())
            },
        }
    })
}

/// Removes all cached tool and sandbox images.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_prune_images() -> *mut c_char {
    record_call("moltis_sandbox_prune_images");
    trace_call("moltis_sandbox_prune_images");

    with_ffi_boundary(|| {
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let (tool_result, sandbox_result) = BRIDGE.runtime.block_on(async {
            tokio::join!(
                builder.prune_all(),
                moltis_tools::sandbox::clean_sandbox_images()
            )
        });

        let mut count = 0usize;
        if let Ok(n) = tool_result {
            count += n;
        }
        if let Ok(n) = sandbox_result {
            count += n;
        }

        if let (Err(e1), Err(e2)) = (&tool_result, &sandbox_result) {
            let message = format!("tool images: {e1}; sandbox images: {e2}");
            record_error("moltis_sandbox_prune_images", IMAGE_CACHE_PRUNE_FAILED);
            return encode_error(IMAGE_CACHE_PRUNE_FAILED, &message);
        }

        encode_json(&SandboxPruneImagesResponse { pruned: count })
    })
}

/// Checks package presence in a base Docker image.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_check_packages(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_check_packages");
    trace_call("moltis_sandbox_check_packages");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxCheckPackagesRequest>(
            "moltis_sandbox_check_packages",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let base = request
            .base
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("ubuntu:25.10")
            .to_owned();
        let packages: Vec<String> = request
            .packages
            .into_iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();

        if packages.is_empty() {
            return encode_json(&SandboxCheckPackagesResponse {
                found: HashMap::new(),
            });
        }

        if !is_valid_image_ref(&base) {
            record_error("moltis_sandbox_check_packages", SANDBOX_BASE_IMAGE_INVALID);
            return encode_error(
                SANDBOX_BASE_IMAGE_INVALID,
                "base image contains invalid characters",
            );
        }

        if let Some(bad) = packages.iter().find(|p| !is_valid_package_name(p)) {
            record_error(
                "moltis_sandbox_check_packages",
                SANDBOX_PACKAGE_NAME_INVALID,
            );
            return encode_error(
                SANDBOX_PACKAGE_NAME_INVALID,
                &format!("invalid package name: {bad}"),
            );
        }

        let checks: Vec<String> = packages
            .iter()
            .map(|pkg| {
                format!(
                    r#"if dpkg -s '{pkg}' >/dev/null 2>&1 || command -v '{pkg}' >/dev/null 2>&1; then echo "FOUND:{pkg}"; fi"#
                )
            })
            .collect();
        let script = checks.join("\n");

        let cli = moltis_tools::sandbox::container_cli();
        let output = BRIDGE.runtime.block_on(async {
            tokio::process::Command::new(cli)
                .args(["run", "--rm", "--entrypoint", "sh", &base, "-c", &script])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
        });

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut found = HashMap::new();
                for pkg in packages {
                    let present = stdout
                        .lines()
                        .any(|line| line.trim() == format!("FOUND:{pkg}"));
                    found.insert(pkg, present);
                }
                encode_json(&SandboxCheckPackagesResponse { found })
            },
            Err(error) => {
                record_error(
                    "moltis_sandbox_check_packages",
                    SANDBOX_CHECK_PACKAGES_FAILED,
                );
                encode_error(SANDBOX_CHECK_PACKAGES_FAILED, &error.to_string())
            },
        }
    })
}

/// Builds a sandbox image from base image + apt package list.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_build_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_build_image");
    trace_call("moltis_sandbox_build_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxBuildImageRequest>(
            "moltis_sandbox_build_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_NAME_REQUIRED);
            return encode_error(SANDBOX_IMAGE_NAME_REQUIRED, "name is required");
        }

        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_NAME_INVALID);
            return encode_error(
                SANDBOX_IMAGE_NAME_INVALID,
                "name must be alphanumeric, dash, or underscore",
            );
        }

        let base = request
            .base
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("ubuntu:25.10")
            .to_owned();
        let packages: Vec<String> = request
            .packages
            .into_iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();

        if !is_valid_image_ref(&base) {
            record_error("moltis_sandbox_build_image", SANDBOX_BASE_IMAGE_INVALID);
            return encode_error(
                SANDBOX_BASE_IMAGE_INVALID,
                "base image contains invalid characters",
            );
        }

        if packages.is_empty() {
            record_error(
                "moltis_sandbox_build_image",
                SANDBOX_IMAGE_PACKAGES_REQUIRED,
            );
            return encode_error(SANDBOX_IMAGE_PACKAGES_REQUIRED, "packages list is empty");
        }

        if let Some(bad) = packages.iter().find(|p| !is_valid_package_name(p)) {
            record_error("moltis_sandbox_build_image", SANDBOX_PACKAGE_NAME_INVALID);
            return encode_error(
                SANDBOX_PACKAGE_NAME_INVALID,
                &format!("invalid package name: {bad}"),
            );
        }

        let pkg_list = packages.join(" ");
        let dockerfile_contents = format!(
            "FROM {base}\n\
RUN apt-get update && apt-get install -y {pkg_list}\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
WORKDIR /home/sandbox\n"
        );

        let tmp_dir = std::env::temp_dir().join(format!("moltis-build-{}", uuid::Uuid::new_v4()));
        if let Err(error) = std::fs::create_dir_all(&tmp_dir) {
            record_error("moltis_sandbox_build_image", SANDBOX_TMP_DIR_CREATE_FAILED);
            return encode_error(SANDBOX_TMP_DIR_CREATE_FAILED, &error.to_string());
        }

        let dockerfile_path = tmp_dir.join("Dockerfile");
        if let Err(error) = std::fs::write(&dockerfile_path, &dockerfile_contents) {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            record_error(
                "moltis_sandbox_build_image",
                SANDBOX_DOCKERFILE_WRITE_FAILED,
            );
            return encode_error(SANDBOX_DOCKERFILE_WRITE_FAILED, &error.to_string());
        }

        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let result =
            BRIDGE
                .runtime
                .block_on(builder.ensure_image(name, &dockerfile_path, &tmp_dir));
        let _ = std::fs::remove_dir_all(&tmp_dir);

        match result {
            Ok(tag) => encode_json(&SandboxBuildImageResponse { tag }),
            Err(error) => {
                record_error("moltis_sandbox_build_image", SANDBOX_IMAGE_BUILD_FAILED);
                encode_error(SANDBOX_IMAGE_BUILD_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns the effective default sandbox image.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_get_default_image() -> *mut c_char {
    record_call("moltis_sandbox_get_default_image");
    trace_call("moltis_sandbox_get_default_image");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let image = sandbox_effective_default_image(&config);
        encode_json(&SandboxDefaultImageResponse { image })
    })
}

/// Sets a runtime default sandbox image override.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_set_default_image(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_set_default_image");
    trace_call("moltis_sandbox_set_default_image");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxSetDefaultImageRequest>(
            "moltis_sandbox_set_default_image",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let config = moltis_config::discover_and_load();
        if sandbox_backend_name(&config) == "none" {
            record_error(
                "moltis_sandbox_set_default_image",
                SANDBOX_BACKEND_UNAVAILABLE,
            );
            return encode_error(SANDBOX_BACKEND_UNAVAILABLE, "no sandbox backend available");
        }

        let value = request
            .image
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);

        *BRIDGE
            .sandbox_default_image_override
            .write()
            .unwrap_or_else(|e| e.into_inner()) = value;

        let image = sandbox_effective_default_image(&config);
        encode_json(&SandboxDefaultImageResponse { image })
    })
}

/// Returns shared `/home/sandbox` persistence config.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_get_shared_home() -> *mut c_char {
    record_call("moltis_sandbox_get_shared_home");
    trace_call("moltis_sandbox_get_shared_home");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let response = sandbox_shared_home_config_from_config(&config);
        encode_json(&response)
    })
}

/// Updates shared `/home/sandbox` persistence config.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_set_shared_home(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_set_shared_home");
    trace_call("moltis_sandbox_set_shared_home");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxSharedHomeUpdateRequest>(
            "moltis_sandbox_set_shared_home",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let path = request
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        let update_result = moltis_config::update_config(|cfg| {
            cfg.tools.exec.sandbox.shared_home_dir = path.clone();
            if request.enabled {
                cfg.tools.exec.sandbox.home_persistence =
                    moltis_config::schema::HomePersistenceConfig::Shared;
            } else if matches!(
                cfg.tools.exec.sandbox.home_persistence,
                moltis_config::schema::HomePersistenceConfig::Shared
            ) {
                cfg.tools.exec.sandbox.home_persistence =
                    moltis_config::schema::HomePersistenceConfig::Off;
            }
        });

        match update_result {
            Ok(saved_path) => {
                let config = moltis_config::discover_and_load();
                let response = SandboxSharedHomeSaveResponse {
                    ok: true,
                    restart_required: true,
                    config_path: saved_path.display().to_string(),
                    config: sandbox_shared_home_config_from_config(&config),
                };
                encode_json(&response)
            },
            Err(error) => {
                record_error(
                    "moltis_sandbox_set_shared_home",
                    SANDBOX_SHARED_HOME_SAVE_FAILED,
                );
                encode_error(SANDBOX_SHARED_HOME_SAVE_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns running containers for the configured sandbox prefix.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_list_containers() -> *mut c_char {
    record_call("moltis_sandbox_list_containers");
    trace_call("moltis_sandbox_list_containers");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::list_running_containers(&prefix))
        {
            Ok(containers) => encode_json(&SandboxContainersResponse { containers }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_list_containers",
                    SANDBOX_CONTAINERS_LIST_FAILED,
                );
                encode_error(SANDBOX_CONTAINERS_LIST_FAILED, &error.to_string())
            },
        }
    })
}

/// Stops one sandbox container.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_stop_container(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_stop_container");
    trace_call("moltis_sandbox_stop_container");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxContainerNameRequest>(
            "moltis_sandbox_stop_container",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error(
                "moltis_sandbox_stop_container",
                "SANDBOX_CONTAINER_NAME_REQUIRED",
            );
            return encode_error("SANDBOX_CONTAINER_NAME_REQUIRED", "name is required");
        }

        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        if !name.starts_with(&prefix) {
            record_error(
                "moltis_sandbox_stop_container",
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
            );
            return encode_error(
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
                "container name does not match expected prefix",
            );
        }

        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::stop_container(name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_stop_container",
                    SANDBOX_CONTAINER_STOP_FAILED,
                );
                encode_error(SANDBOX_CONTAINER_STOP_FAILED, &error.to_string())
            },
        }
    })
}

/// Removes one sandbox container.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_remove_container(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_sandbox_remove_container");
    trace_call("moltis_sandbox_remove_container");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<SandboxContainerNameRequest>(
            "moltis_sandbox_remove_container",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error(
                "moltis_sandbox_remove_container",
                "SANDBOX_CONTAINER_NAME_REQUIRED",
            );
            return encode_error("SANDBOX_CONTAINER_NAME_REQUIRED", "name is required");
        }

        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        if !name.starts_with(&prefix) {
            record_error(
                "moltis_sandbox_remove_container",
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
            );
            return encode_error(
                SANDBOX_CONTAINER_PREFIX_MISMATCH,
                "container name does not match expected prefix",
            );
        }

        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::remove_container(name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_remove_container",
                    SANDBOX_CONTAINER_REMOVE_FAILED,
                );
                encode_error(SANDBOX_CONTAINER_REMOVE_FAILED, &error.to_string())
            },
        }
    })
}

/// Stops and removes all sandbox containers.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_clean_containers() -> *mut c_char {
    record_call("moltis_sandbox_clean_containers");
    trace_call("moltis_sandbox_clean_containers");

    with_ffi_boundary(|| {
        let config = moltis_config::discover_and_load();
        let prefix = sandbox_container_prefix(&config);
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::clean_all_containers(&prefix))
        {
            Ok(removed) => encode_json(&SandboxCleanContainersResponse { ok: true, removed }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_clean_containers",
                    SANDBOX_CONTAINERS_CLEAN_FAILED,
                );
                encode_error(SANDBOX_CONTAINERS_CLEAN_FAILED, &error.to_string())
            },
        }
    })
}

/// Returns container runtime disk usage.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_disk_usage() -> *mut c_char {
    record_call("moltis_sandbox_disk_usage");
    trace_call("moltis_sandbox_disk_usage");

    with_ffi_boundary(|| {
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::container_disk_usage())
        {
            Ok(usage) => encode_json(&SandboxDiskUsageResponse { usage }),
            Err(error) => {
                record_error("moltis_sandbox_disk_usage", SANDBOX_DISK_USAGE_FAILED);
                encode_error(SANDBOX_DISK_USAGE_FAILED, &error.to_string())
            },
        }
    })
}

/// Restarts the container daemon.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_sandbox_restart_daemon() -> *mut c_char {
    record_call("moltis_sandbox_restart_daemon");
    trace_call("moltis_sandbox_restart_daemon");

    with_ffi_boundary(|| {
        match BRIDGE
            .runtime
            .block_on(moltis_tools::sandbox::restart_container_daemon())
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error(
                    "moltis_sandbox_restart_daemon",
                    SANDBOX_DAEMON_RESTART_FAILED,
                );
                encode_error(SANDBOX_DAEMON_RESTART_FAILED, &error.to_string())
            },
        }
    })
}
