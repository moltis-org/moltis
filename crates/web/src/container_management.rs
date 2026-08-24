use moltis_tools::sandbox::SandboxRouter;

fn configured_sandbox_prefix(config: &moltis_config::MoltisConfig) -> String {
    config
        .tools
        .exec
        .sandbox
        .container_prefix
        .clone()
        .unwrap_or_else(|| "moltis-sandbox".to_string())
}

fn browser_prefix(_config: &moltis_config::MoltisConfig) -> String {
    "moltis-browser".to_string()
}

pub(crate) fn managed_container_prefixes(
    config: &moltis_config::MoltisConfig,
    router: Option<&SandboxRouter>,
) -> Vec<String> {
    let sandbox_prefix = router
        .and_then(|router| router.config().container_prefix.clone())
        .unwrap_or_else(|| configured_sandbox_prefix(config));
    let browser_prefix = browser_prefix(config);
    if sandbox_prefix == browser_prefix {
        vec![sandbox_prefix]
    } else {
        vec![sandbox_prefix, browser_prefix]
    }
}

pub(crate) fn managed_container_name(
    config: &moltis_config::MoltisConfig,
    router: Option<&SandboxRouter>,
    name: &str,
) -> bool {
    managed_container_prefixes(config, router)
        .iter()
        .any(|prefix| {
            name.starts_with(prefix)
                || moltis_tools::sandbox::has_apple_container_prefix(name, prefix)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_runtime_sandbox_prefix_for_management() {
        let config = moltis_config::MoltisConfig::default();
        let router = SandboxRouter::new(moltis_tools::sandbox::SandboxConfig {
            container_prefix: Some("moltis-identity-scoped-sandbox".to_string()),
            ..Default::default()
        });
        let apple_name = moltis_tools::sandbox::apple_container_name(
            "moltis-identity-scoped-sandbox",
            "session-4fc5159e-0cb6-49f2-8eba-553ba3ec897b",
            0,
        );

        assert_eq!(
            managed_container_prefixes(&config, Some(&router))[0],
            "moltis-identity-scoped-sandbox"
        );
        assert!(managed_container_name(&config, Some(&router), &apple_name));
        assert!(!managed_container_name(
            &config,
            Some(&router),
            "moltis-sandbox-unrelated"
        ));
    }

    #[test]
    fn falls_back_to_configured_prefix_and_deduplicates() {
        let mut config = moltis_config::MoltisConfig::default();
        config.tools.exec.sandbox.container_prefix = Some("moltis-browser".to_string());

        assert_eq!(managed_container_prefixes(&config, None), vec![
            "moltis-browser"
        ]);
    }
}
