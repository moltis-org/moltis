use std::{collections::HashMap, path::Path as FsPath};

use moltis_tools::approval::{ApprovalMode, SecurityLevel};

#[cfg(feature = "qmd")]
#[test]
fn sanitize_qmd_index_name_normalizes_non_alphanumeric_segments() {
    let path = FsPath::new("/Users/Penso/.moltis/data///");
    assert_eq!(
        crate::server::helpers::sanitize_qmd_index_name(path),
        "moltis-users_penso_moltis_data"
    );
}

#[cfg(feature = "qmd")]
#[test]
fn sanitize_qmd_index_name_falls_back_for_empty_root() {
    assert_eq!(
        crate::server::helpers::sanitize_qmd_index_name(FsPath::new("///")),
        "moltis"
    );
}

#[test]
fn summarize_model_ids_for_logs_returns_all_when_within_limit() {
    let model_ids = vec!["a", "b", "c"]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let summary = crate::server::helpers::summarize_model_ids_for_logs(&model_ids, 8);
    assert_eq!(summary, model_ids);
}

#[test]
fn summarize_model_ids_for_logs_truncates_to_head_and_tail() {
    let model_ids = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let summary = crate::server::helpers::summarize_model_ids_for_logs(&model_ids, 7);
    let expected = vec!["a", "b", "c", "...", "h", "i", "j"]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    assert_eq!(summary, expected);
}

#[test]
fn approval_manager_uses_config_values() {
    let mut cfg = moltis_config::MoltisConfig::default();
    cfg.tools.exec.approval_mode = "always".into();
    cfg.tools.exec.security_level = "strict".into();
    cfg.tools.exec.allowlist = vec!["git*".into()];

    let manager = crate::server::helpers::approval_manager_from_config(&cfg);
    assert_eq!(manager.mode, ApprovalMode::Always);
    assert_eq!(manager.security_level, SecurityLevel::Deny);
    assert_eq!(manager.allowlist, vec!["git*".to_string()]);
}

#[test]
fn approval_manager_falls_back_for_invalid_values() {
    let mut cfg = moltis_config::MoltisConfig::default();
    cfg.tools.exec.approval_mode = "bogus".into();
    cfg.tools.exec.security_level = "bogus".into();

    let manager = crate::server::helpers::approval_manager_from_config(&cfg);
    assert_eq!(manager.mode, ApprovalMode::OnMiss);
    assert_eq!(manager.security_level, SecurityLevel::Allowlist);
}

#[cfg(feature = "fs-tools")]
#[test]
fn fs_tools_host_warning_message_only_triggers_without_real_backend() {
    use {
        moltis_tools::{
            exec::{ExecOpts, ExecResult},
            sandbox::{Sandbox, SandboxId},
        },
        std::sync::Arc,
    };

    struct TestRealSandbox;

    #[async_trait::async_trait]
    impl Sandbox for TestRealSandbox {
        fn backend_name(&self) -> &'static str {
            "test-real"
        }

        async fn ensure_ready(
            &self,
            _id: &SandboxId,
            _image_override: Option<&str>,
        ) -> moltis_tools::Result<()> {
            Ok(())
        }

        async fn exec(
            &self,
            _id: &SandboxId,
            _command: &str,
            _opts: &ExecOpts,
        ) -> moltis_tools::Result<ExecResult> {
            Ok(ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        }

        async fn cleanup(&self, _id: &SandboxId) -> moltis_tools::Result<()> {
            Ok(())
        }
    }

    let real_backend: Arc<dyn Sandbox> = Arc::new(TestRealSandbox);
    let real_router = moltis_tools::sandbox::SandboxRouter::with_backend(
        moltis_tools::sandbox::SandboxConfig::default(),
        real_backend,
    );
    assert!(crate::server::helpers::fs_tools_host_warning_message(&real_router).is_none());

    let no_backend: Arc<dyn Sandbox> = Arc::new(moltis_tools::sandbox::NoSandbox);
    let no_router = moltis_tools::sandbox::SandboxRouter::with_backend(
        moltis_tools::sandbox::SandboxConfig::default(),
        no_backend,
    );
    let warning =
        crate::server::helpers::fs_tools_host_warning_message(&no_router).expect("warning");
    assert!(warning.contains("fs tools are registered"));
    assert!(warning.contains("[tools.fs].allow_paths"));
}

#[test]
fn prebuild_runs_only_when_mode_enabled_and_packages_present() {
    let packages = vec!["curl".to_string()];
    assert!(crate::server::helpers::should_prebuild_sandbox_image(
        &moltis_tools::sandbox::SandboxMode::All,
        &packages
    ));
    assert!(crate::server::helpers::should_prebuild_sandbox_image(
        &moltis_tools::sandbox::SandboxMode::NonMain,
        &packages
    ));
    assert!(!crate::server::helpers::should_prebuild_sandbox_image(
        &moltis_tools::sandbox::SandboxMode::Off,
        &packages
    ));
    assert!(!crate::server::helpers::should_prebuild_sandbox_image(
        &moltis_tools::sandbox::SandboxMode::All,
        &[]
    ));
}

#[test]
fn proxy_tls_validation_rejects_common_misconfiguration() {
    let err = crate::server::helpers::validate_proxy_tls_configuration(true, true, false)
        .expect_err("behind proxy with TLS should fail without explicit override");
    let message = err.to_string();
    assert!(message.contains("MOLTIS_BEHIND_PROXY=true"));
    assert!(message.contains("--no-tls"));
}

#[test]
fn proxy_tls_validation_allows_proxy_mode_when_tls_is_disabled() {
    assert!(crate::server::helpers::validate_proxy_tls_configuration(true, false, false).is_ok());
}

#[test]
fn proxy_tls_validation_allows_explicit_tls_override() {
    assert!(crate::server::helpers::validate_proxy_tls_configuration(true, true, true).is_ok());
}

#[test]
fn env_value_with_overrides_uses_override_when_process_env_missing() {
    let unique_key = format!("MOLTIS_TEST_LOOKUP_{}", std::process::id());
    let overrides = HashMap::from([(unique_key.clone(), "override-value".to_string())]);
    assert_eq!(
        crate::server::helpers::env_value_with_overrides(&overrides, &unique_key).as_deref(),
        Some("override-value")
    );
}
