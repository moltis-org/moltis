//! Trait interfaces for domain services the gateway delegates to.
//! Each trait has a `Noop` implementation that returns empty/default responses,
//! allowing the gateway to run standalone before domain crates are wired in.

use {
    async_trait::async_trait, moltis_channels::ChannelOutbound, serde_json::Value, std::sync::Arc,
};

/// Error type returned by service methods.
pub type ServiceError = String;
pub type ServiceResult<T = Value> = Result<T, ServiceError>;

/// Convert markdown to sanitized HTML using pulldown-cmark.
fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

// ── Agent ───────────────────────────────────────────────────────────────────

#[async_trait]
pub trait AgentService: Send + Sync {
    async fn run(&self, params: Value) -> ServiceResult;
    async fn run_wait(&self, params: Value) -> ServiceResult;
    async fn identity_get(&self) -> ServiceResult;
    async fn list(&self) -> ServiceResult;
}

pub struct NoopAgentService;

#[async_trait]
impl AgentService for NoopAgentService {
    async fn run(&self, _params: Value) -> ServiceResult {
        Err("agent service not configured".into())
    }

    async fn run_wait(&self, _params: Value) -> ServiceResult {
        Err("agent service not configured".into())
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "name": "moltis", "avatar": null }))
    }

    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

// ── Sessions ────────────────────────────────────────────────────────────────

#[async_trait]
pub trait SessionService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn preview(&self, params: Value) -> ServiceResult;
    async fn resolve(&self, params: Value) -> ServiceResult;
    async fn patch(&self, params: Value) -> ServiceResult;
    async fn reset(&self, params: Value) -> ServiceResult;
    async fn delete(&self, params: Value) -> ServiceResult;
    async fn compact(&self, params: Value) -> ServiceResult;
    async fn search(&self, params: Value) -> ServiceResult;
}

pub struct NoopSessionService;

#[async_trait]
impl SessionService for NoopSessionService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn preview(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn resolve(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn patch(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn reset(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn delete(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn compact(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn search(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

// ── Channels ────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ChannelService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn logout(&self, params: Value) -> ServiceResult;
    async fn send(&self, params: Value) -> ServiceResult;
    async fn add(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn senders_list(&self, params: Value) -> ServiceResult;
    async fn sender_approve(&self, params: Value) -> ServiceResult;
    async fn sender_deny(&self, params: Value) -> ServiceResult;
}

pub struct NoopChannelService;

#[async_trait]
impl ChannelService for NoopChannelService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "channels": [] }))
    }

    async fn logout(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn send(&self, _p: Value) -> ServiceResult {
        Err("no channels configured".into())
    }

    async fn add(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn remove(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn update(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn senders_list(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn sender_approve(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }

    async fn sender_deny(&self, _p: Value) -> ServiceResult {
        Err("no channel service configured".into())
    }
}

// ── Config ──────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ConfigService: Send + Sync {
    async fn get(&self, params: Value) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn apply(&self, params: Value) -> ServiceResult;
    async fn patch(&self, params: Value) -> ServiceResult;
    async fn schema(&self) -> ServiceResult;
}

pub struct NoopConfigService;

#[async_trait]
impl ConfigService for NoopConfigService {
    async fn get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn apply(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn patch(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn schema(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

// ── Cron ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait CronService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn add(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn run(&self, params: Value) -> ServiceResult;
    async fn runs(&self, params: Value) -> ServiceResult;
}

pub struct NoopCronService;

#[async_trait]
impl CronService for NoopCronService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "running": false }))
    }

    async fn add(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn update(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn remove(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn run(&self, _p: Value) -> ServiceResult {
        Err("cron not configured".into())
    }

    async fn runs(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ChatService: Send + Sync {
    async fn send(&self, params: Value) -> ServiceResult;
    /// Run a chat send synchronously (inline, no spawn) and return token usage.
    /// Returns `{ "text": "...", "inputTokens": N, "outputTokens": N }`.
    async fn send_sync(&self, params: Value) -> ServiceResult {
        self.send(params).await
    }
    async fn abort(&self, params: Value) -> ServiceResult;
    async fn history(&self, params: Value) -> ServiceResult;
    async fn inject(&self, params: Value) -> ServiceResult;
    async fn clear(&self, params: Value) -> ServiceResult;
    async fn compact(&self, params: Value) -> ServiceResult;
    async fn context(&self, params: Value) -> ServiceResult;
}

pub struct NoopChatService;

#[async_trait]
impl ChatService for NoopChatService {
    async fn send(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn abort(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn history(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn inject(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn clear(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn compact(&self, _p: Value) -> ServiceResult {
        Err("chat not configured".into())
    }

    async fn context(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "session": {}, "project": null, "tools": [], "providers": [] }))
    }
}

// ── TTS ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait TtsService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn providers(&self) -> ServiceResult;
    async fn enable(&self, params: Value) -> ServiceResult;
    async fn disable(&self) -> ServiceResult;
    async fn convert(&self, params: Value) -> ServiceResult;
    async fn set_provider(&self, params: Value) -> ServiceResult;
}

pub struct NoopTtsService;

#[async_trait]
impl TtsService for NoopTtsService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "enabled": false }))
    }

    async fn providers(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn enable(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }

    async fn disable(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn convert(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }

    async fn set_provider(&self, _p: Value) -> ServiceResult {
        Err("tts not available".into())
    }
}

// ── MCP (Model Context Protocol) ────────────────────────────────────────────

#[async_trait]
pub trait McpService: Send + Sync {
    /// List all configured MCP servers with status.
    async fn list(&self) -> ServiceResult;
    /// Add a new MCP server.
    async fn add(&self, params: Value) -> ServiceResult;
    /// Remove an MCP server.
    async fn remove(&self, params: Value) -> ServiceResult;
    /// Enable an MCP server.
    async fn enable(&self, params: Value) -> ServiceResult;
    /// Disable an MCP server.
    async fn disable(&self, params: Value) -> ServiceResult;
    /// Get status of a specific server.
    async fn status(&self, params: Value) -> ServiceResult;
    /// List tools from a specific server.
    async fn tools(&self, params: Value) -> ServiceResult;
    /// Restart an MCP server.
    async fn restart(&self, params: Value) -> ServiceResult;
    /// Update an MCP server's configuration.
    async fn update(&self, params: Value) -> ServiceResult;
}

pub struct NoopMcpService;

#[async_trait]
impl McpService for NoopMcpService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!({ "servers": [] }))
    }

    async fn add(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn remove(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn enable(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn disable(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn status(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn tools(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn restart(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }

    async fn update(&self, _params: Value) -> ServiceResult {
        Err("MCP not configured".into())
    }
}

// ── Skills ──────────────────────────────────────────────────────────────────

#[async_trait]
pub trait SkillsService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn bins(&self) -> ServiceResult;
    async fn install(&self, params: Value) -> ServiceResult;
    async fn update(&self, params: Value) -> ServiceResult;
    async fn list(&self) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn repos_list(&self) -> ServiceResult;
    /// Full repos list with per-skill details (for search). Heavyweight.
    async fn repos_list_full(&self) -> ServiceResult;
    async fn repos_remove(&self, params: Value) -> ServiceResult;
    async fn skill_enable(&self, params: Value) -> ServiceResult;
    async fn skill_disable(&self, params: Value) -> ServiceResult;
    async fn skill_detail(&self, params: Value) -> ServiceResult;
    async fn install_dep(&self, params: Value) -> ServiceResult;
}

// ── Plugins ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait PluginsService: Send + Sync {
    async fn install(&self, params: Value) -> ServiceResult;
    async fn remove(&self, params: Value) -> ServiceResult;
    async fn repos_list(&self) -> ServiceResult;
    /// Full repos list with per-skill details (for search). Heavyweight.
    async fn repos_list_full(&self) -> ServiceResult;
    async fn repos_remove(&self, params: Value) -> ServiceResult;
    async fn skill_enable(&self, params: Value) -> ServiceResult;
    async fn skill_disable(&self, params: Value) -> ServiceResult;
    async fn skill_detail(&self, params: Value) -> ServiceResult;
}

pub struct NoopSkillsService;

#[async_trait]
impl SkillsService for NoopSkillsService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "installed": [] }))
    }

    async fn bins(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn install(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter (owner/repo format)".to_string())?;
        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;
        let skills = moltis_skills::install::install_skill(source, &install_dir)
            .await
            .map_err(|e| e.to_string())?;
        let installed: Vec<_> = skills
            .iter()
            .map(|m| {
                serde_json::json!({
                    "name": m.name,
                    "description": m.description,
                    "path": m.path.to_string_lossy(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "installed": installed }))
    }

    async fn update(&self, _p: Value) -> ServiceResult {
        Err("skills not available".into())
    }

    async fn list(&self) -> ServiceResult {
        use moltis_skills::{
            discover::{FsSkillDiscoverer, SkillDiscoverer},
            requirements::check_requirements,
        };
        let cwd = std::env::current_dir().unwrap_or_default();
        let search_paths = FsSkillDiscoverer::default_paths(&cwd);
        let discoverer = FsSkillDiscoverer::new(search_paths);
        let skills = discoverer.discover().await.map_err(|e| e.to_string())?;
        let items: Vec<_> = skills
            .iter()
            .map(|s| {
                let elig = check_requirements(s);
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "license": s.license,
                    "allowed_tools": s.allowed_tools,
                    "path": s.path.to_string_lossy(),
                    "source": s.source,
                    "eligible": elig.eligible,
                    "missing_bins": elig.missing_bins,
                    "install_options": elig.install_options,
                })
            })
            .collect();
        Ok(serde_json::json!(items))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;
        moltis_skills::install::remove_repo(source, &install_dir)
            .await
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({ "removed": source }))
    }

    async fn repos_list(&self) -> ServiceResult {
        let manifest_path =
            moltis_skills::manifest::ManifestStore::default_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let enabled = repo.skills.iter().filter(|s| s.enabled).count();
                // Re-detect format for repos that predate the formats module
                let format = if repo.format == moltis_plugins::formats::PluginFormat::Skill {
                    let repo_dir = install_dir.join(&repo.repo_name);
                    moltis_plugins::formats::detect_format(&repo_dir)
                } else {
                    repo.format
                };
                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "format": format,
                    "skill_count": repo.skills.len(),
                    "enabled_count": enabled,
                })
            })
            .collect();

        Ok(serde_json::json!(repos))
    }

    async fn repos_list_full(&self) -> ServiceResult {
        use moltis_skills::requirements::check_requirements;

        let manifest_path =
            moltis_skills::manifest::ManifestStore::default_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let skills: Vec<_> = repo
                    .skills
                    .iter()
                    .map(|s| {
                        let skill_dir = install_dir.join(&s.relative_path);
                        let skill_md = skill_dir.join("SKILL.md");
                        let meta_json = moltis_skills::parse::read_meta_json(&skill_dir);
                        let (description, display_name, elig) =
                            if let Ok(content) = std::fs::read_to_string(&skill_md) {
                                if let Ok(meta) =
                                    moltis_skills::parse::parse_metadata(&content, &skill_dir)
                                {
                                    let e = check_requirements(&meta);
                                    let desc = if meta.description.is_empty() {
                                        meta_json
                                            .as_ref()
                                            .and_then(|m| m.display_name.clone())
                                            .unwrap_or_default()
                                    } else {
                                        meta.description
                                    };
                                    let dn =
                                        meta_json.as_ref().and_then(|m| m.display_name.clone());
                                    (desc, dn, Some(e))
                                } else {
                                    let dn =
                                        meta_json.as_ref().and_then(|m| m.display_name.clone());
                                    (dn.clone().unwrap_or_default(), dn, None)
                                }
                            } else {
                                let dn = meta_json.as_ref().and_then(|m| m.display_name.clone());
                                (dn.clone().unwrap_or_default(), dn, None)
                            };
                        serde_json::json!({
                            "name": s.name,
                            "description": description,
                            "display_name": display_name,
                            "relative_path": s.relative_path,
                            "enabled": s.enabled,
                            "eligible": elig.as_ref().map(|e| e.eligible).unwrap_or(true),
                            "missing_bins": elig.as_ref().map(|e| e.missing_bins.clone()).unwrap_or_default(),
                        })
                    })
                    .collect();
                // Re-detect format for repos that predate the formats module
                let format = if repo.format == moltis_plugins::formats::PluginFormat::Skill {
                    let repo_dir = install_dir.join(&repo.repo_name);
                    moltis_plugins::formats::detect_format(&repo_dir)
                } else {
                    repo.format
                };
                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "format": format,
                    "skills": skills,
                })
            })
            .collect();

        Ok(serde_json::json!(repos))
    }

    async fn repos_remove(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;
        moltis_skills::install::remove_repo(source, &install_dir)
            .await
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({ "removed": source }))
    }

    async fn skill_enable(&self, params: Value) -> ServiceResult {
        toggle_skill(&params, true)
    }

    async fn skill_disable(&self, params: Value) -> ServiceResult {
        toggle_skill(&params, false)
    }

    async fn skill_detail(&self, params: Value) -> ServiceResult {
        use moltis_skills::requirements::check_requirements;

        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;
        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'skill' parameter".to_string())?;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(|e| e.to_string())?;
        let manifest_path =
            moltis_skills::manifest::ManifestStore::default_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let repo = manifest
            .repos
            .iter()
            .find(|r| r.source == source)
            .ok_or_else(|| format!("repo '{source}' not found"))?;
        let skill_state = repo
            .skills
            .iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| format!("skill '{skill_name}' not found in repo '{source}'"))?;

        let skill_dir = install_dir.join(&skill_state.relative_path);
        let skill_md = skill_dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&skill_md)
            .map_err(|e| format!("failed to read SKILL.md: {e}"))?;

        let content = moltis_skills::parse::parse_skill(&raw, &skill_dir)
            .map_err(|e| format!("failed to parse SKILL.md: {e}"))?;

        let elig = check_requirements(&content.metadata);
        let meta_json = moltis_skills::parse::read_meta_json(&skill_dir);
        let display_name = meta_json.as_ref().and_then(|m| m.display_name.clone());
        let author = meta_json.as_ref().and_then(|m| m.owner.clone());
        let version = meta_json
            .as_ref()
            .and_then(|m| m.latest.as_ref())
            .and_then(|l| l.version.clone());

        // Build a direct link to the skill source on GitHub
        let source_url: Option<String> = {
            let rel = &skill_state.relative_path;
            // relative_path starts with repo_name/, strip it to get path within repo
            rel.strip_prefix(&repo.repo_name)
                .and_then(|p| p.strip_prefix('/'))
                .map(|path_in_repo| {
                    if source.starts_with("https://") || source.starts_with("http://") {
                        format!(
                            "{}/tree/main/{}",
                            source.trim_end_matches('/'),
                            path_in_repo
                        )
                    } else {
                        format!("https://github.com/{}/tree/main/{}", source, path_in_repo)
                    }
                })
        };

        Ok(serde_json::json!({
            "name": content.metadata.name,
            "display_name": display_name,
            "description": content.metadata.description,
            "author": author,
            "homepage": content.metadata.homepage,
            "version": version,
            "license": content.metadata.license,
            "compatibility": content.metadata.compatibility,
            "allowed_tools": content.metadata.allowed_tools,
            "requires": content.metadata.requires,
            "eligible": elig.eligible,
            "missing_bins": elig.missing_bins,
            "install_options": elig.install_options,
            "enabled": skill_state.enabled,
            "source_url": source_url,
            "body": content.body,
            "body_html": markdown_to_html(&content.body),
            "source": source,
        }))
    }

    async fn install_dep(&self, params: Value) -> ServiceResult {
        use moltis_skills::{
            discover::{FsSkillDiscoverer, SkillDiscoverer},
            requirements::{check_requirements, run_install},
        };

        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'skill' parameter".to_string())?;
        let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        // Discover the skill to get its requirements
        let cwd = std::env::current_dir().unwrap_or_default();
        let search_paths = FsSkillDiscoverer::default_paths(&cwd);
        let discoverer = FsSkillDiscoverer::new(search_paths);
        let skills = discoverer.discover().await.map_err(|e| e.to_string())?;

        let meta = skills
            .iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| format!("skill '{skill_name}' not found"))?;

        let elig = check_requirements(meta);
        let spec = elig
            .install_options
            .get(index)
            .ok_or_else(|| format!("install option index {index} out of range"))?;

        let result = run_install(spec).await.map_err(|e| e.to_string())?;

        if result.success {
            Ok(serde_json::json!({
                "success": true,
                "stdout": result.stdout,
                "stderr": result.stderr,
            }))
        } else {
            Err(format!(
                "install failed: {}",
                if result.stderr.is_empty() {
                    result.stdout
                } else {
                    result.stderr
                }
            ))
        }
    }
}

pub struct NoopPluginsService;

#[async_trait]
impl PluginsService for NoopPluginsService {
    async fn install(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter (owner/repo format)".to_string())?;
        let install_dir =
            moltis_plugins::install::default_plugins_dir().map_err(|e| e.to_string())?;
        let skills = moltis_plugins::install::install_plugin(source, &install_dir)
            .await
            .map_err(|e| e.to_string())?;
        let installed: Vec<_> = skills
            .iter()
            .map(|m| {
                serde_json::json!({
                    "name": m.name,
                    "description": m.description,
                    "path": m.path.to_string_lossy(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "installed": installed }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;
        let install_dir =
            moltis_plugins::install::default_plugins_dir().map_err(|e| e.to_string())?;
        moltis_plugins::install::remove_plugin(source, &install_dir)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "removed": source }))
    }

    async fn repos_list(&self) -> ServiceResult {
        let manifest_path =
            moltis_plugins::install::default_manifest_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let install_dir =
            moltis_plugins::install::default_plugins_dir().map_err(|e| e.to_string())?;

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let enabled = repo.skills.iter().filter(|s| s.enabled).count();
                let format = {
                    let repo_dir = install_dir.join(&repo.repo_name);
                    moltis_plugins::formats::detect_format(&repo_dir)
                };
                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "format": format,
                    "skill_count": repo.skills.len(),
                    "enabled_count": enabled,
                })
            })
            .collect();

        Ok(serde_json::json!(repos))
    }

    async fn repos_list_full(&self) -> ServiceResult {
        let manifest_path =
            moltis_plugins::install::default_manifest_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let install_dir =
            moltis_plugins::install::default_plugins_dir().map_err(|e| e.to_string())?;

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let repo_dir = install_dir.join(&repo.repo_name);
                let format = moltis_plugins::formats::detect_format(&repo_dir);

                // Scan adapter to get enriched metadata (display_name, description).
                let entries = moltis_plugins::formats::scan_with_adapter(&repo_dir, format)
                    .and_then(|r| r.ok())
                    .unwrap_or_default();

                let skills: Vec<_> = repo
                    .skills
                    .iter()
                    .map(|s| {
                        // Find the matching adapter entry for extra info.
                        let entry = entries.iter().find(|e| e.metadata.name == s.name);
                        serde_json::json!({
                            "name": s.name,
                            "description": entry.map(|e| e.metadata.description.as_str()).unwrap_or(""),
                            "display_name": entry.and_then(|e| e.display_name.as_deref()),
                            "relative_path": s.relative_path,
                            "enabled": s.enabled,
                            "eligible": true,
                            "missing_bins": [],
                        })
                    })
                    .collect();

                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "format": format,
                    "skills": skills,
                })
            })
            .collect();

        Ok(serde_json::json!(repos))
    }

    async fn repos_remove(&self, params: Value) -> ServiceResult {
        self.remove(params).await
    }

    async fn skill_enable(&self, params: Value) -> ServiceResult {
        toggle_plugin_skill(&params, true)
    }

    async fn skill_disable(&self, params: Value) -> ServiceResult {
        toggle_plugin_skill(&params, false)
    }

    async fn skill_detail(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;
        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'skill' parameter".to_string())?;

        let install_dir =
            moltis_plugins::install::default_plugins_dir().map_err(|e| e.to_string())?;
        let manifest_path =
            moltis_plugins::install::default_manifest_path().map_err(|e| e.to_string())?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let manifest = store.load().map_err(|e| e.to_string())?;

        let repo = manifest
            .repos
            .iter()
            .find(|r| r.source == source)
            .ok_or_else(|| format!("plugin repo '{source}' not found"))?;
        let skill_state = repo
            .skills
            .iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| format!("skill '{skill_name}' not found in plugin repo '{source}'"))?;

        // For plugin skills, re-scan the adapter to get the body + extra info.
        let repo_dir = install_dir.join(&repo.repo_name);
        let format = moltis_plugins::formats::detect_format(&repo_dir);
        let entries = moltis_plugins::formats::scan_with_adapter(&repo_dir, format)
            .ok_or_else(|| format!("no adapter for format '{format}'"))?
            .map_err(|e| format!("scan error: {e}"))?;

        let entry = entries
            .into_iter()
            .find(|e| e.metadata.name == skill_name)
            .ok_or_else(|| format!("skill '{skill_name}' not found on disk"))?;

        // Build a direct link to the source file on GitHub.
        // source_file is already relative to the repo root.
        let source_url: Option<String> = entry.source_file.as_ref().map(|file| {
            if source.starts_with("https://") || source.starts_with("http://") {
                format!("{}/blob/main/{}", source.trim_end_matches('/'), file)
            } else {
                format!("https://github.com/{}/blob/main/{}", source, file)
            }
        });

        let empty: Vec<String> = Vec::new();
        Ok(serde_json::json!({
            "name": entry.metadata.name,
            "display_name": entry.display_name,
            "description": entry.metadata.description,
            "author": entry.author,
            "homepage": entry.metadata.homepage,
            "version": null,
            "license": entry.metadata.license,
            "compatibility": entry.metadata.compatibility,
            "allowed_tools": entry.metadata.allowed_tools,
            "requires": entry.metadata.requires,
            "eligible": true,
            "missing_bins": empty,
            "install_options": empty,
            "enabled": skill_state.enabled,
            "source_url": source_url,
            "body": entry.body,
            "body_html": markdown_to_html(&entry.body),
            "source": source,
        }))
    }
}

fn toggle_plugin_skill(params: &Value, enabled: bool) -> ServiceResult {
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'source' parameter".to_string())?;
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    let manifest_path =
        moltis_plugins::install::default_manifest_path().map_err(|e| e.to_string())?;
    let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
    let mut manifest = store.load().map_err(|e| e.to_string())?;

    if !manifest.set_skill_enabled(source, skill_name, enabled) {
        return Err(format!(
            "skill '{skill_name}' not found in plugin repo '{source}'"
        ));
    }
    store.save(&manifest).map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "source": source, "skill": skill_name, "enabled": enabled }))
}

fn toggle_skill(params: &Value, enabled: bool) -> ServiceResult {
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'source' parameter".to_string())?;
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    let manifest_path =
        moltis_skills::manifest::ManifestStore::default_path().map_err(|e| e.to_string())?;
    let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
    let mut manifest = store.load().map_err(|e| e.to_string())?;

    if !manifest.set_skill_enabled(source, skill_name, enabled) {
        return Err(format!("skill '{skill_name}' not found in repo '{source}'"));
    }
    store.save(&manifest).map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "source": source, "skill": skill_name, "enabled": enabled }))
}

// ── Browser ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait BrowserService: Send + Sync {
    async fn request(&self, params: Value) -> ServiceResult;
}

pub struct NoopBrowserService;

#[async_trait]
impl BrowserService for NoopBrowserService {
    async fn request(&self, _p: Value) -> ServiceResult {
        Err("browser not available".into())
    }
}

// ── Usage ───────────────────────────────────────────────────────────────────

#[async_trait]
pub trait UsageService: Send + Sync {
    async fn status(&self) -> ServiceResult;
    async fn cost(&self, params: Value) -> ServiceResult;
}

pub struct NoopUsageService;

#[async_trait]
impl UsageService for NoopUsageService {
    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "totalCost": 0, "requests": 0 }))
    }

    async fn cost(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "cost": 0 }))
    }
}

// ── Exec Approvals ──────────────────────────────────────────────────────────

#[async_trait]
pub trait ExecApprovalService: Send + Sync {
    async fn get(&self) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn node_get(&self, params: Value) -> ServiceResult;
    async fn node_set(&self, params: Value) -> ServiceResult;
    async fn request(&self, params: Value) -> ServiceResult;
    async fn resolve(&self, params: Value) -> ServiceResult;
}

pub struct NoopExecApprovalService;

#[async_trait]
impl ExecApprovalService for NoopExecApprovalService {
    async fn get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "mode": "always" }))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn node_get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "mode": "always" }))
    }

    async fn node_set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn request(&self, _p: Value) -> ServiceResult {
        Err("approvals not configured".into())
    }

    async fn resolve(&self, _p: Value) -> ServiceResult {
        Err("approvals not configured".into())
    }
}

// ── Onboarding ──────────────────────────────────────────────────────────────

#[async_trait]
pub trait OnboardingService: Send + Sync {
    async fn wizard_start(&self, params: Value) -> ServiceResult;
    async fn wizard_next(&self, params: Value) -> ServiceResult;
    async fn wizard_cancel(&self) -> ServiceResult;
    async fn wizard_status(&self) -> ServiceResult;
    async fn identity_get(&self) -> ServiceResult;
    async fn identity_update(&self, params: Value) -> ServiceResult;
    async fn identity_update_soul(&self, soul: Option<String>) -> ServiceResult;
}

pub struct NoopOnboardingService;

#[async_trait]
impl OnboardingService for NoopOnboardingService {
    async fn wizard_start(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0 }))
    }

    async fn wizard_next(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0, "done": true }))
    }

    async fn wizard_cancel(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn wizard_status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "active": false }))
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "name": "moltis", "avatar": null }))
    }

    async fn identity_update(&self, _params: Value) -> ServiceResult {
        Err("onboarding service not configured".into())
    }

    async fn identity_update_soul(&self, _soul: Option<String>) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

// ── Update ──────────────────────────────────────────────────────────────────

#[async_trait]
pub trait UpdateService: Send + Sync {
    async fn run(&self, params: Value) -> ServiceResult;
}

pub struct NoopUpdateService;

#[async_trait]
impl UpdateService for NoopUpdateService {
    async fn run(&self, _p: Value) -> ServiceResult {
        Err("update not available".into())
    }
}

// ── Model ───────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ModelService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    /// Disable a model (hide it from the list).
    async fn disable(&self, params: Value) -> ServiceResult;
    /// Enable a model (un-hide it).
    async fn enable(&self, params: Value) -> ServiceResult;
}

pub struct NoopModelService;

#[async_trait]
impl ModelService for NoopModelService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn disable(&self, _params: Value) -> ServiceResult {
        Err("model service not configured".into())
    }

    async fn enable(&self, _params: Value) -> ServiceResult {
        Err("model service not configured".into())
    }
}

// ── Web Login ───────────────────────────────────────────────────────────────

#[async_trait]
pub trait WebLoginService: Send + Sync {
    async fn start(&self, params: Value) -> ServiceResult;
    async fn wait(&self, params: Value) -> ServiceResult;
}

pub struct NoopWebLoginService;

#[async_trait]
impl WebLoginService for NoopWebLoginService {
    async fn start(&self, _p: Value) -> ServiceResult {
        Err("web login not available".into())
    }

    async fn wait(&self, _p: Value) -> ServiceResult {
        Err("web login not available".into())
    }
}

// ── Voicewake ───────────────────────────────────────────────────────────────

#[async_trait]
pub trait VoicewakeService: Send + Sync {
    async fn get(&self) -> ServiceResult;
    async fn set(&self, params: Value) -> ServiceResult;
    async fn wake(&self, params: Value) -> ServiceResult;
    async fn talk_mode(&self, params: Value) -> ServiceResult;
}

pub struct NoopVoicewakeService;

#[async_trait]
impl VoicewakeService for NoopVoicewakeService {
    async fn get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "enabled": false }))
    }

    async fn set(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn wake(&self, _p: Value) -> ServiceResult {
        Err("voicewake not available".into())
    }

    async fn talk_mode(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait LogsService: Send + Sync {
    async fn tail(&self, params: Value) -> ServiceResult;
    async fn list(&self, params: Value) -> ServiceResult;
    async fn status(&self) -> ServiceResult;
    async fn ack(&self) -> ServiceResult;
}

pub struct NoopLogsService;

#[async_trait]
impl LogsService for NoopLogsService {
    async fn tail(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "subscribed": true }))
    }

    async fn list(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!({ "entries": [] }))
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "unseen_warns": 0, "unseen_errors": 0 }))
    }

    async fn ack(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }
}

// ── Provider Setup ──────────────────────────────────────────────────────────

#[async_trait]
pub trait ProviderSetupService: Send + Sync {
    async fn available(&self) -> ServiceResult;
    async fn save_key(&self, params: Value) -> ServiceResult;
    async fn oauth_start(&self, params: Value) -> ServiceResult;
    async fn oauth_status(&self, params: Value) -> ServiceResult;
    async fn remove_key(&self, params: Value) -> ServiceResult;
}

// ── Local LLM ───────────────────────────────────────────────────────────────

/// Service for managing local LLM provider (GGUF/MLX).
#[async_trait]
pub trait LocalLlmService: Send + Sync {
    /// Get system info (RAM, GPU, memory tier).
    async fn system_info(&self) -> ServiceResult;
    /// Get available models with recommendations based on memory tier.
    async fn models(&self) -> ServiceResult;
    /// Configure and load a model by ID (from registry).
    async fn configure(&self, params: Value) -> ServiceResult;
    /// Get current provider status (loading/loaded/error).
    async fn status(&self) -> ServiceResult;
    /// Search HuggingFace for models by query and backend.
    async fn search_hf(&self, params: Value) -> ServiceResult;
    /// Configure a custom model from HuggingFace repo URL.
    async fn configure_custom(&self, params: Value) -> ServiceResult;
    /// Remove a configured model by ID.
    async fn remove_model(&self, params: Value) -> ServiceResult;
}

pub struct NoopLocalLlmService;

#[async_trait]
impl LocalLlmService for NoopLocalLlmService {
    async fn system_info(&self) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn models(&self) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn configure(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn status(&self) -> ServiceResult {
        Ok(serde_json::json!({ "status": "unavailable" }))
    }

    async fn search_hf(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn configure_custom(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }

    async fn remove_model(&self, _params: Value) -> ServiceResult {
        Err("local-llm feature not enabled".into())
    }
}

pub struct NoopProviderSetupService;

#[async_trait]
impl ProviderSetupService for NoopProviderSetupService {
    async fn available(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn save_key(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn oauth_start(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn oauth_status(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }

    async fn remove_key(&self, _p: Value) -> ServiceResult {
        Err("provider setup not configured".into())
    }
}

// ── Project ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ProjectService: Send + Sync {
    async fn list(&self) -> ServiceResult;
    async fn get(&self, params: Value) -> ServiceResult;
    async fn upsert(&self, params: Value) -> ServiceResult;
    async fn delete(&self, params: Value) -> ServiceResult;
    async fn detect(&self, params: Value) -> ServiceResult;
    async fn complete_path(&self, params: Value) -> ServiceResult;
    async fn context(&self, params: Value) -> ServiceResult;
}

pub struct NoopProjectService;

#[async_trait]
impl ProjectService for NoopProjectService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn get(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!(null))
    }

    async fn upsert(&self, _p: Value) -> ServiceResult {
        Err("project service not configured".into())
    }

    async fn delete(&self, _p: Value) -> ServiceResult {
        Err("project service not configured".into())
    }

    async fn detect(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn complete_path(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!([]))
    }

    async fn context(&self, _p: Value) -> ServiceResult {
        Ok(serde_json::json!(null))
    }
}

// ── Bundled services ────────────────────────────────────────────────────────

/// All domain services the gateway delegates to.
pub struct GatewayServices {
    pub agent: Arc<dyn AgentService>,
    pub session: Arc<dyn SessionService>,
    pub channel: Arc<dyn ChannelService>,
    pub config: Arc<dyn ConfigService>,
    pub cron: Arc<dyn CronService>,
    pub chat: Arc<dyn ChatService>,
    pub tts: Arc<dyn TtsService>,
    pub stt: Arc<dyn crate::voice::SttService>,
    pub skills: Arc<dyn SkillsService>,
    pub mcp: Arc<dyn McpService>,
    pub plugins: Arc<dyn PluginsService>,
    pub browser: Arc<dyn BrowserService>,
    pub usage: Arc<dyn UsageService>,
    pub exec_approval: Arc<dyn ExecApprovalService>,
    pub onboarding: Arc<dyn OnboardingService>,
    pub update: Arc<dyn UpdateService>,
    pub model: Arc<dyn ModelService>,
    pub web_login: Arc<dyn WebLoginService>,
    pub voicewake: Arc<dyn VoicewakeService>,
    pub logs: Arc<dyn LogsService>,
    pub provider_setup: Arc<dyn ProviderSetupService>,
    pub project: Arc<dyn ProjectService>,
    pub local_llm: Arc<dyn LocalLlmService>,
    /// Optional channel outbound for sending replies back to channels.
    channel_outbound: Option<Arc<dyn ChannelOutbound>>,
    /// Optional session metadata for cross-service access (e.g. channel binding).
    pub session_metadata: Option<Arc<moltis_sessions::metadata::SqliteSessionMetadata>>,
    /// Optional session store for message-index lookups (e.g. deduplication).
    pub session_store: Option<Arc<moltis_sessions::store::SessionStore>>,
}

impl GatewayServices {
    pub fn with_chat(mut self, chat: Arc<dyn ChatService>) -> Self {
        self.chat = chat;
        self
    }

    pub fn with_model(mut self, model: Arc<dyn ModelService>) -> Self {
        self.model = model;
        self
    }

    pub fn with_cron(mut self, cron: Arc<dyn CronService>) -> Self {
        self.cron = cron;
        self
    }

    pub fn with_provider_setup(mut self, ps: Arc<dyn ProviderSetupService>) -> Self {
        self.provider_setup = ps;
        self
    }

    pub fn with_channel_outbound(mut self, outbound: Arc<dyn ChannelOutbound>) -> Self {
        self.channel_outbound = Some(outbound);
        self
    }

    pub fn channel_outbound_arc(&self) -> Option<Arc<dyn ChannelOutbound>> {
        self.channel_outbound.clone()
    }

    /// Create a service bundle with all noop implementations.
    pub fn noop() -> Self {
        Self {
            agent: Arc::new(NoopAgentService),
            session: Arc::new(NoopSessionService),
            channel: Arc::new(NoopChannelService),
            config: Arc::new(NoopConfigService),
            cron: Arc::new(NoopCronService),
            chat: Arc::new(NoopChatService),
            tts: Arc::new(NoopTtsService),
            stt: Arc::new(crate::voice::NoopSttService),
            skills: Arc::new(NoopSkillsService),
            mcp: Arc::new(NoopMcpService),
            plugins: Arc::new(NoopPluginsService),
            browser: Arc::new(NoopBrowserService),
            usage: Arc::new(NoopUsageService),
            exec_approval: Arc::new(NoopExecApprovalService),
            onboarding: Arc::new(NoopOnboardingService),
            update: Arc::new(NoopUpdateService),
            model: Arc::new(NoopModelService),
            web_login: Arc::new(NoopWebLoginService),
            voicewake: Arc::new(NoopVoicewakeService),
            logs: Arc::new(NoopLogsService),
            provider_setup: Arc::new(NoopProviderSetupService),
            project: Arc::new(NoopProjectService),
            local_llm: Arc::new(NoopLocalLlmService),
            channel_outbound: None,
            session_metadata: None,
            session_store: None,
        }
    }

    pub fn with_local_llm(mut self, local_llm: Arc<dyn LocalLlmService>) -> Self {
        self.local_llm = local_llm;
        self
    }

    pub fn with_onboarding(mut self, onboarding: Arc<dyn OnboardingService>) -> Self {
        self.onboarding = onboarding;
        self
    }

    pub fn with_project(mut self, project: Arc<dyn ProjectService>) -> Self {
        self.project = project;
        self
    }

    pub fn with_session_metadata(
        mut self,
        meta: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
    ) -> Self {
        self.session_metadata = Some(meta);
        self
    }

    pub fn with_session_store(mut self, store: Arc<moltis_sessions::store::SessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    pub fn with_tts(mut self, tts: Arc<dyn TtsService>) -> Self {
        self.tts = tts;
        self
    }

    pub fn with_stt(mut self, stt: Arc<dyn crate::voice::SttService>) -> Self {
        self.stt = stt;
        self
    }
}
