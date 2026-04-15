use std::{collections::HashSet, path::PathBuf, sync::Arc};

use tracing::info;

use moltis_sessions::store::SessionStore;

use super::seed_content::{
    DCG_GUARD_HANDLER_SH, DCG_GUARD_HOOK_MD, EXAMPLE_HOOK_MD, EXAMPLE_SKILL_MD, TMUX_SKILL_MD,
};

// ── Hook seeding helpers ─────────────────────────────────────────────────────

/// Seed a skeleton example hook into `~/.moltis/hooks/example/` on first run.
pub(crate) fn seed_example_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/example");
    let hook_md = hook_dir.join("HOOK.md");
    if hook_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        tracing::debug!("could not create example hook dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&hook_md, EXAMPLE_HOOK_MD) {
        tracing::debug!("could not write example HOOK.md: {e}");
    }
}

/// Marker string that must be present in an up-to-date seeded `handler.sh`.
pub(crate) const DCG_GUARD_HANDLER_FINGERPRINT: &str = "export PATH=";

/// Marker string that must be present in an up-to-date seeded `HOOK.md`.
pub(crate) const DCG_GUARD_HOOK_MD_FINGERPRINT: &str = "uv tool install destructive-command-guard";

/// Seed the `dcg-guard` hook into `~/.moltis/hooks/dcg-guard/` on first run,
/// and refresh on-disk files that predate the PATH-fix in #626.
pub(crate) async fn seed_dcg_guard_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/dcg-guard");
    let hook_md = hook_dir.join("HOOK.md");
    let handler = hook_dir.join("handler.sh");

    let dir_ok = match std::fs::create_dir_all(&hook_dir) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!("could not create dcg-guard hook dir: {e}");
            false
        },
    };

    if dir_ok {
        let hook_md_needs_write = match std::fs::read_to_string(&hook_md) {
            Ok(existing) => !existing.contains(DCG_GUARD_HOOK_MD_FINGERPRINT),
            Err(_) => true,
        };
        if hook_md_needs_write && let Err(e) = std::fs::write(&hook_md, DCG_GUARD_HOOK_MD) {
            tracing::debug!("could not write dcg-guard HOOK.md: {e}");
        }

        let handler_needs_write = match std::fs::read_to_string(&handler) {
            Ok(existing) => !existing.contains(DCG_GUARD_HANDLER_FINGERPRINT),
            Err(_) => true,
        };
        if handler_needs_write {
            if let Err(e) = std::fs::write(&handler, DCG_GUARD_HANDLER_SH) {
                tracing::debug!("could not write dcg-guard handler.sh: {e}");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&handler, std::fs::Permissions::from_mode(0o755));
            }
            if !hook_md_needs_write {
                tracing::info!(
                    "dcg-guard: refreshed stale handler.sh to apply PATH augmentation fix"
                );
            }
        }
    }

    log_dcg_guard_status().await;
}

/// PATH augmentation prepended by the dcg-guard handler script.
pub(crate) const DCG_GUARD_EXTRA_PATH_DIRS: &[&str] =
    &[".local/bin", "/usr/local/bin", "/opt/homebrew/bin"];

/// Fallback `$HOME` used by `resolve_dcg_binary`.
pub(crate) const DCG_GUARD_HOME_FALLBACK: &str = "/root";

/// Resolve `dcg` using the same augmented `PATH` as the handler script.
fn resolve_dcg_binary() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    let home_path = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DCG_GUARD_HOME_FALLBACK));
    for rel in DCG_GUARD_EXTRA_PATH_DIRS {
        if rel.starts_with('/') {
            dirs.push(PathBuf::from(rel));
        } else {
            dirs.push(home_path.join(rel));
        }
    }

    let existing = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    for entry in existing.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(entry));
    }

    for dir in dirs {
        let candidate = dir.join("dcg");
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&candidate)
                    && meta.permissions().mode() & 0o111 != 0
                {
                    return Some(candidate);
                }
                continue;
            }
            #[cfg(not(unix))]
            {
                return Some(candidate);
            }
        }
    }

    None
}

/// Emit a single startup log line describing whether the dcg-guard is active.
async fn log_dcg_guard_status() {
    let Some(path) = resolve_dcg_binary() else {
        tracing::warn!(
            "dcg-guard: 'dcg' not found on PATH; destructive command guard is INACTIVE. \
             Install dcg from https://github.com/Dicklesworthstone/destructive_command_guard"
        );
        return;
    };

    let version = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown version".to_string());

    tracing::info!(
        dcg_path = %path.display(),
        "dcg-guard: dcg {version} detected, guard active"
    );
}

/// Seed built-in personal skills into `~/.moltis/skills/`.
pub(crate) fn seed_example_skill() {
    seed_skill_if_missing("template-skill", EXAMPLE_SKILL_MD);
    seed_skill_if_missing("tmux", TMUX_SKILL_MD);
}

fn seed_skill_if_missing(name: &str, content: &str) {
    let skill_dir = moltis_config::data_dir().join(format!("skills/{name}"));
    let skill_md = skill_dir.join("SKILL.md");
    if skill_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        tracing::debug!("could not create {name} skill dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&skill_md, content) {
        tracing::debug!("could not write {name} SKILL.md: {e}");
    }
}

// ── Hook discovery ───────────────────────────────────────────────────────────

/// Metadata for built-in hooks (compiled Rust, always active).
fn builtin_hook_metadata() -> Vec<(
    &'static str,
    &'static str,
    Vec<moltis_common::hooks::HookEvent>,
    &'static str,
)> {
    use moltis_common::hooks::HookEvent;
    vec![
        (
            "command-logger",
            "Logs all slash-command invocations to a JSONL audit file at ~/.moltis/logs/commands.log.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/command_logger.rs",
        ),
        (
            "session-memory",
            "Saves the conversation history to a markdown file in the memory directory when a session is reset or a new session is created, making it searchable for future sessions.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/session_memory.rs",
        ),
    ]
}

/// Discover hooks from the filesystem, check eligibility, and build a
/// [`HookRegistry`] plus a `Vec<DiscoveredHookInfo>` for the web UI.
pub(crate) async fn discover_and_build_hooks(
    disabled: &HashSet<String>,
    session_store: Option<&Arc<SessionStore>>,
) -> (
    Option<Arc<moltis_common::hooks::HookRegistry>>,
    Vec<crate::state::DiscoveredHookInfo>,
) {
    use moltis_plugins::{
        bundled::{command_logger::CommandLoggerHook, session_memory::SessionMemoryHook},
        hook_discovery::{FsHookDiscoverer, HookDiscoverer, HookSource},
        hook_eligibility::check_hook_eligibility,
        shell_hook::ShellHookHandler,
    };

    let discoverer = FsHookDiscoverer::new(FsHookDiscoverer::default_paths());
    let discovered = discoverer.discover().await.unwrap_or_default();
    let session_export_mode = moltis_config::discover_and_load().memory.session_export;

    let mut registry = moltis_common::hooks::HookRegistry::new();
    let mut info_list = Vec::with_capacity(discovered.len());

    for (parsed, source) in &discovered {
        let meta = &parsed.metadata;
        let elig = check_hook_eligibility(meta);
        let is_disabled = disabled.contains(&meta.name);
        let is_enabled = elig.eligible && !is_disabled;

        if !elig.eligible {
            info!(
                hook = %meta.name,
                source = ?source,
                missing_os = elig.missing_os,
                missing_bins = ?elig.missing_bins,
                missing_env = ?elig.missing_env,
                "hook ineligible, skipping"
            );
        }

        let raw_content =
            std::fs::read_to_string(parsed.source_path.join("HOOK.md")).unwrap_or_default();

        let source_str = match source {
            HookSource::Project => "project",
            HookSource::User => "user",
            HookSource::Bundled => "bundled",
        };

        info_list.push(crate::state::DiscoveredHookInfo {
            name: meta.name.clone(),
            description: meta.description.clone(),
            emoji: meta.emoji.clone(),
            events: meta.events.iter().map(|e| e.to_string()).collect(),
            command: meta.command.clone(),
            timeout: meta.timeout,
            priority: meta.priority,
            source: source_str.to_string(),
            source_path: parsed.source_path.display().to_string(),
            eligible: elig.eligible,
            missing_os: elig.missing_os,
            missing_bins: elig.missing_bins.clone(),
            missing_env: elig.missing_env.clone(),
            enabled: is_enabled,
            body: raw_content,
            body_html: crate::services::markdown_to_html(&parsed.body),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });

        if is_enabled && let Some(ref command) = meta.command {
            let handler = ShellHookHandler::new(
                meta.name.clone(),
                command.clone(),
                meta.events.clone(),
                std::time::Duration::from_secs(meta.timeout),
                meta.env.clone(),
                Some(parsed.source_path.clone()),
            );
            registry.register(Arc::new(handler));
        }
    }

    // ── Built-in hooks (compiled Rust, always active) ──────────────────
    {
        let data = moltis_config::data_dir();

        let log_path =
            CommandLoggerHook::default_path().unwrap_or_else(|| data.join("logs/commands.log"));
        let logger = CommandLoggerHook::new(log_path);
        registry.register(Arc::new(logger));

        if let Some(store) = session_store
            && !matches!(session_export_mode, moltis_config::SessionExportMode::Off)
        {
            let memory_hook = SessionMemoryHook::new(data.clone(), Arc::clone(store));
            registry.register(Arc::new(memory_hook));
        }
    }

    for (name, description, events, source_file) in builtin_hook_metadata() {
        let enabled = if name == "session-memory" {
            !matches!(session_export_mode, moltis_config::SessionExportMode::Off)
        } else {
            true
        };
        info_list.push(crate::state::DiscoveredHookInfo {
            name: name.to_string(),
            description: description.to_string(),
            emoji: Some("\u{2699}\u{fe0f}".to_string()),
            events: events.iter().map(|e| e.to_string()).collect(),
            command: None,
            timeout: 0,
            priority: 0,
            source: "builtin".to_string(),
            source_path: source_file.to_string(),
            eligible: true,
            missing_os: false,
            missing_bins: vec![],
            missing_env: vec![],
            enabled,
            body: String::new(),
            body_html: format!(
                "<p><em>Built-in hook implemented in Rust.</em></p><p>{}</p>",
                description
            ),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });
    }

    if !info_list.is_empty() {
        info!(
            "{} hook(s) discovered ({} shell, {} built-in), {} registered",
            info_list.len(),
            discovered.len(),
            info_list.len() - discovered.len(),
            registry.handler_names().len()
        );
    }

    (Some(Arc::new(registry)), info_list)
}
