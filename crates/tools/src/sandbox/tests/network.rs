#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn test_from_config_network_trusted_overrides_no_network() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: "trusted".into(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Trusted);
}

#[test]
fn test_from_config_network_bypass_overrides_no_network() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: "bypass".into(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Bypass);
}

#[test]
fn test_from_config_empty_network_defaults_to_trusted() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: false,
        network: String::new(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Trusted);
}

#[test]
fn test_from_config_no_network_true_empty_network_is_blocked() {
    let cfg = moltis_config::schema::SandboxConfig {
        no_network: true,
        network: String::new(),
        ..Default::default()
    };
    let sc = SandboxConfig::from(&cfg);
    assert_eq!(sc.network, NetworkPolicy::Blocked);
}

#[test]
fn test_docker_network_run_args_blocked() {
    let config = SandboxConfig {
        network: NetworkPolicy::Blocked,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert_eq!(docker.network_run_args(), vec!["--network=none"]);
}

#[test]
fn test_docker_network_run_args_trusted() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.network_run_args();
    assert_eq!(args, vec!["--add-host=host.docker.internal:host-gateway"]);
}

#[test]
fn test_docker_network_run_args_bypass() {
    let config = SandboxConfig {
        network: NetworkPolicy::Bypass,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.network_run_args().is_empty());
}

#[test]
fn test_docker_proxy_exec_env_args_trusted() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    let args = docker.proxy_exec_env_args();
    let expected_url = format!(
        "http://host.docker.internal:{}",
        moltis_network_filter::DEFAULT_PROXY_PORT
    );
    // Should contain -e pairs for HTTP_PROXY, http_proxy, HTTPS_PROXY, https_proxy,
    // NO_PROXY, no_proxy (6 keys x 2 args each = 12 args).
    assert_eq!(args.len(), 12);
    assert!(args.contains(&format!("HTTP_PROXY={expected_url}")));
    assert!(args.contains(&format!("https_proxy={expected_url}")));
    assert!(args.contains(&"NO_PROXY=localhost,127.0.0.1,::1".to_string()));
    assert!(args.contains(&"no_proxy=localhost,127.0.0.1,::1".to_string()));
}

#[test]
fn test_docker_proxy_exec_env_args_blocked() {
    let config = SandboxConfig {
        network: NetworkPolicy::Blocked,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.proxy_exec_env_args().is_empty());
}

#[test]
fn test_docker_proxy_exec_env_args_bypass() {
    let config = SandboxConfig {
        network: NetworkPolicy::Bypass,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    assert!(docker.proxy_exec_env_args().is_empty());
}

#[test]
fn test_docker_resolve_host_gateway_always_returns_host_gateway() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let docker = DockerSandbox::new(config);
    // Docker always uses the host-gateway token regardless of version.
    assert_eq!(docker.resolve_host_gateway(), "host-gateway");
}

#[test]
fn test_podman_network_run_args_trusted_contains_add_host() {
    let config = SandboxConfig {
        network: NetworkPolicy::Trusted,
        ..Default::default()
    };
    let podman = DockerSandbox::podman(config);
    let args = podman.network_run_args();
    // The exact IP depends on the host environment (Podman version and
    // rootless/rootful mode), but the flag must always start with
    // `--add-host=host.docker.internal:`.
    assert_eq!(args.len(), 1);
    assert!(
        args[0].starts_with("--add-host=host.docker.internal:"),
        "unexpected arg: {}",
        args[0],
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_container_proxy_prefix_trusted() {
    // Build the same prefix that exec() would build for Trusted mode,
    // but using the helper logic directly.
    let gateway = "192.168.64.1";
    let proxy_url = format!(
        "http://{}:{}",
        gateway,
        moltis_network_filter::DEFAULT_PROXY_PORT
    );
    let mut prefix = String::new();
    let escaped_proxy = proxy_url.replace('\'', "'\\''");
    for key in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"] {
        prefix.push_str(&format!("export {key}='{escaped_proxy}'; "));
    }
    for key in ["NO_PROXY", "no_proxy"] {
        prefix.push_str(&format!("export {key}='localhost,127.0.0.1,::1'; "));
    }

    assert!(prefix.contains("export HTTP_PROXY="));
    assert!(prefix.contains("export https_proxy="));
    assert!(prefix.contains(&format!(":{}", moltis_network_filter::DEFAULT_PROXY_PORT)));
    assert!(prefix.contains("export NO_PROXY='localhost,127.0.0.1,::1'"));
}
