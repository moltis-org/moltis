use std::{collections::HashSet, sync::Arc};

use {
    super::common::LocalModelConfigTestGuard,
    crate::server::{
        hooks::{discover_and_build_hooks, seed_dcg_guard_hook},
        seed_content::{DCG_GUARD_HANDLER_SH, DCG_GUARD_HOOK_MD},
    },
};

#[tokio::test]
async fn discover_hooks_registers_builtin_handlers() {
    let _guard = LocalModelConfigTestGuard::new();
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let project_dir = tempfile::tempdir().unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(project_dir.path()).unwrap();
    std::fs::write(
        config_dir.path().join("moltis.toml"),
        "[memory]\nsession_export = \"on-new-or-reset\"\n",
    )
    .unwrap();
    moltis_config::set_config_dir(config_dir.path().to_path_buf());
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_store = Arc::new(moltis_sessions::store::SessionStore::new(sessions_dir));

    let (registry, info) = discover_and_build_hooks(&HashSet::new(), Some(&session_store)).await;
    let registry = registry.expect("expected hook registry to be created");
    let handler_names = registry.handler_names();

    assert!(handler_names.iter().any(|n| n == "command-logger"));
    assert!(handler_names.iter().any(|n| n == "session-memory"));
    assert!(
        info.iter()
            .any(|h| h.name == "command-logger" && h.source == "builtin")
    );
    assert!(
        info.iter()
            .any(|h| h.name == "session-memory" && h.source == "builtin")
    );

    std::env::set_current_dir(old_cwd).unwrap();
    moltis_config::clear_config_dir();
}

#[tokio::test]
async fn discover_hooks_respects_session_export_mode_off() {
    let _guard = LocalModelConfigTestGuard::new();
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let project_dir = tempfile::tempdir().unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(project_dir.path()).unwrap();
    std::fs::write(
        config_dir.path().join("moltis.toml"),
        "[memory]\nsession_export = \"off\"\n",
    )
    .unwrap();
    moltis_config::set_config_dir(config_dir.path().to_path_buf());

    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_store = Arc::new(moltis_sessions::store::SessionStore::new(sessions_dir));

    let (registry, info) = discover_and_build_hooks(&HashSet::new(), Some(&session_store)).await;
    let registry = registry.expect("expected hook registry to be created");
    let handler_names = registry.handler_names();

    assert!(handler_names.iter().any(|n| n == "command-logger"));
    assert!(!handler_names.iter().any(|n| n == "session-memory"));
    assert!(
        info.iter()
            .any(|h| h.name == "session-memory" && h.source == "builtin" && !h.enabled)
    );

    std::env::set_current_dir(old_cwd).unwrap();
    moltis_config::clear_config_dir();
}

#[tokio::test]
async fn command_hook_dispatch_saves_session_memory_file() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_store = Arc::new(moltis_sessions::store::SessionStore::new(sessions_dir));

    session_store
        .append(
            "smoke-session",
            &serde_json::json!({"role": "user", "content": "Hello from smoke test"}),
        )
        .await
        .unwrap();
    session_store
        .append(
            "smoke-session",
            &serde_json::json!({"role": "assistant", "content": "Hi there"}),
        )
        .await
        .unwrap();

    let mut registry = moltis_common::hooks::HookRegistry::new();
    registry.register(Arc::new(
        moltis_plugins::bundled::session_memory::SessionMemoryHook::new(
            tmp.path().to_path_buf(),
            Arc::clone(&session_store),
        ),
    ));

    let payload = moltis_common::hooks::HookPayload::Command {
        session_key: "smoke-session".into(),
        action: "new".into(),
        sender_id: None,
    };
    let result = registry.dispatch(&payload).await.unwrap();
    assert!(matches!(result, moltis_common::hooks::HookAction::Continue));

    let memory_dir = tmp.path().join("memory");
    assert!(memory_dir.is_dir());

    let files: Vec<_> = std::fs::read_dir(&memory_dir).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);

    let content = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(content.contains("smoke-session"));
    assert!(content.contains("Hello from smoke test"));
    assert!(content.contains("Hi there"));
}

#[test]
fn dcg_guard_handler_has_path_augmentation() {
    assert!(DCG_GUARD_HANDLER_SH.contains(".local/bin"));
    assert!(DCG_GUARD_HANDLER_SH.contains("/usr/local/bin"));
    assert!(DCG_GUARD_HANDLER_SH.contains("/opt/homebrew/bin"));
    assert!(DCG_GUARD_HANDLER_SH.contains("export PATH="));
}

#[test]
fn dcg_guard_handler_warns_when_dcg_missing() {
    assert!(DCG_GUARD_HANDLER_SH.contains("NOT scanned"));
    assert!(DCG_GUARD_HANDLER_SH.contains(">&2"));
    let warn_idx = DCG_GUARD_HANDLER_SH
        .find("NOT scanned")
        .expect("handler must contain the NOT scanned warning");
    let drain_idx = DCG_GUARD_HANDLER_SH
        .find("cat >/dev/null")
        .expect("handler still drains stdin in the missing-dcg branch");
    assert!(warn_idx < drain_idx);
}

#[test]
fn dcg_guard_hook_md_removes_cargo_install() {
    assert!(!DCG_GUARD_HOOK_MD.contains("cargo install dcg"));
    assert!(DCG_GUARD_HOOK_MD.contains("github.com/Dicklesworthstone/destructive_command_guard"));
    assert!(
        DCG_GUARD_HOOK_MD.contains("uv tool install destructive-command-guard")
            || DCG_GUARD_HOOK_MD.contains("pipx install destructive-command-guard")
    );
}

#[tokio::test]
async fn seed_dcg_guard_hook_writes_handler_with_path_fix() {
    let _guard = LocalModelConfigTestGuard::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    moltis_config::set_data_dir(tmp.path().to_path_buf());

    seed_dcg_guard_hook().await;

    let handler_path = tmp.path().join("hooks/dcg-guard/handler.sh");
    let written =
        std::fs::read_to_string(&handler_path).expect("handler.sh should have been written");
    assert!(written.contains("export PATH="));
    assert!(written.contains(".local/bin"));
    assert!(written.contains("NOT scanned"));

    let hook_md_path = tmp.path().join("hooks/dcg-guard/HOOK.md");
    let hook_md = std::fs::read_to_string(&hook_md_path).expect("HOOK.md written");
    assert!(!hook_md.contains("cargo install dcg"));
}

#[tokio::test]
async fn seed_dcg_guard_hook_refreshes_stale_handler() {
    let _guard = LocalModelConfigTestGuard::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    moltis_config::set_data_dir(tmp.path().to_path_buf());

    let hook_dir = tmp.path().join("hooks/dcg-guard");
    std::fs::create_dir_all(&hook_dir).expect("create hook dir");

    let stale_handler = "#!/usr/bin/env bash\n\
         set -euo pipefail\n\
         if ! command -v dcg >/dev/null 2>&1; then\n    \
             cat >/dev/null\n    \
             exit 0\n\
         fi\n";
    let stale_hook_md = "+++\nname = \"dcg-guard\"\n+++\n\n## Install dcg\n\
         \n```bash\ncargo install dcg\n```\n";

    let handler_path = hook_dir.join("handler.sh");
    let hook_md_path = hook_dir.join("HOOK.md");
    std::fs::write(&handler_path, stale_handler).expect("seed stale handler");
    std::fs::write(&hook_md_path, stale_hook_md).expect("seed stale HOOK.md");

    seed_dcg_guard_hook().await;

    let refreshed_handler =
        std::fs::read_to_string(&handler_path).expect("handler.sh must still exist");
    assert!(refreshed_handler.contains("export PATH="));
    assert!(refreshed_handler.contains("NOT scanned"));

    let refreshed_hook_md =
        std::fs::read_to_string(&hook_md_path).expect("HOOK.md must still exist");
    assert!(!refreshed_hook_md.contains("cargo install dcg"));
    assert!(refreshed_hook_md.contains("uv tool install destructive-command-guard"));
}

#[test]
fn dcg_guard_extra_path_dirs_match_handler_script() {
    for rel in crate::server::hooks::DCG_GUARD_EXTRA_PATH_DIRS {
        let needle = if rel.starts_with('/') {
            (*rel).to_string()
        } else {
            format!("/{rel}")
        };
        assert!(
            DCG_GUARD_HANDLER_SH.contains(&needle),
            "handler script missing PATH entry for {rel:?} (needle={needle:?})"
        );
    }
    assert!(DCG_GUARD_HANDLER_SH.contains(&format!(
        "${{HOME:-{}}}/.local/bin",
        crate::server::hooks::DCG_GUARD_HOME_FALLBACK
    )));
}

#[test]
fn dcg_guard_handler_home_fallback_matches_rust_probe() {
    assert!(DCG_GUARD_HANDLER_SH.contains("${HOME:-/root}/.local/bin"));
    assert_eq!(crate::server::hooks::DCG_GUARD_HOME_FALLBACK, "/root");
}

#[tokio::test]
async fn seed_dcg_guard_hook_logs_status_even_if_mkdir_fails() {
    let _guard = LocalModelConfigTestGuard::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").expect("write blocker file");
    moltis_config::set_data_dir(blocker.clone());

    assert!(std::fs::create_dir_all(blocker.join("hooks/dcg-guard")).is_err());

    seed_dcg_guard_hook().await;
}
