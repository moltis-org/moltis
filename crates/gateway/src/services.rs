//! Trait interfaces for domain services the gateway delegates to.
//! Each trait has a `Noop` implementation that returns empty/default responses,
//! allowing the gateway to run standalone before domain crates are wired in.
//!
//! Pure trait definitions and simple noop implementations live in `moltis-service-traits`.
//! This module re-exports everything from that crate and adds gateway-specific implementations.

// Re-export all trait definitions and simple noops from service-traits.
pub use moltis_service_traits::*;

use {
    async_trait::async_trait,
    serde_json::Value,
    std::{collections::HashSet, path::Path, sync::Arc},
};

fn security_audit(event: &str, details: Value) {
    let dir = moltis_config::data_dir().join("logs");
    let path = dir.join("security-audit.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let line = serde_json::json!({
        "ts": now_ms,
        "event": event,
        "details": details,
    })
    .to_string();

    let _ = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        use std::io::Write as _;
        writeln!(file, "{line}")?;
        Ok(())
    })();
}

async fn command_available(command: &str) -> bool {
    tokio::process::Command::new(command)
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn run_mcp_scan(installed_dir: &Path) -> anyhow::Result<Value> {
    let mut cmd = if command_available("uvx").await {
        let mut c = tokio::process::Command::new("uvx");
        c.arg("mcp-scan@latest");
        c
    } else {
        tokio::process::Command::new("mcp-scan")
    };

    cmd.arg("--skills")
        .arg(installed_dir)
        .arg("--json")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(std::time::Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("mcp-scan timed out after 5 minutes"))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(if stderr.is_empty() {
            "mcp-scan failed".to_string()
        } else {
            format!("mcp-scan failed: {stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let parsed: Value = serde_json::from_str(&stdout)
        .map_err(|e| anyhow::anyhow!("invalid mcp-scan JSON output: {e}"))?;
    Ok(parsed)
}

fn is_protected_discovered_skill(name: &str) -> bool {
    matches!(name, "template-skill" | "template" | "tmux")
}

fn commit_url_for_source(source: &str, sha: &str) -> Option<String> {
    if sha.trim().is_empty() {
        return None;
    }
    if source.starts_with("https://") || source.starts_with("http://") {
        return Some(format!("{}/commit/{}", source.trim_end_matches('/'), sha));
    }
    if source.contains('/') {
        return Some(format!("https://github.com/{}/commit/{}", source, sha));
    }
    None
}

fn license_url_for_source(source: &str, license: Option<&str>) -> Option<String> {
    let text = license?.to_ascii_lowercase();
    let file = if text.contains("license.txt") {
        "LICENSE.txt"
    } else if text.contains("license.md") {
        "LICENSE.md"
    } else if text.contains("license") {
        "LICENSE"
    } else {
        return None;
    };

    if source.starts_with("https://") || source.starts_with("http://") {
        Some(format!(
            "{}/blob/main/{}",
            source.trim_end_matches('/'),
            file
        ))
    } else if source.contains('/') {
        Some(format!("https://github.com/{}/blob/main/{}", source, file))
    } else {
        None
    }
}

fn local_repo_head_timestamp_ms(repo_dir: &Path) -> Option<u64> {
    let repo = gix::open(repo_dir).ok()?;
    let obj = repo.rev_parse_single("HEAD").ok()?;
    let commit = repo.find_commit(obj.detach()).ok()?;
    let secs = commit.time().ok()?.seconds;
    Some((secs as i128).max(0) as u64 * 1000)
}

fn commit_age_days(commit_ts_ms: Option<u64>) -> Option<u64> {
    let ts = commit_ts_ms?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(now_ms.saturating_sub(ts) / 86_400_000)
}

fn risky_install_pattern(command: &str) -> Option<&'static str> {
    let c = command.to_ascii_lowercase();
    if (c.contains("curl") || c.contains("wget")) && (c.contains("| sh") || c.contains("|bash")) {
        return Some("piped shell execution");
    }

    let patterns = [
        ("base64", "obfuscated payload decoding"),
        ("xattr -d com.apple.quarantine", "quarantine bypass"),
        ("bash -c", "inline shell execution"),
        ("sh -c", "inline shell execution"),
        ("python -c", "inline code execution"),
        ("node -e", "inline code execution"),
    ];
    patterns
        .into_iter()
        .find_map(|(needle, reason)| c.contains(needle).then_some(reason))
}

/// Convert markdown to sanitized HTML using pulldown-cmark.
pub(crate) fn markdown_to_html(md: &str) -> String {
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

// ── Skills (Noop — complex impl that depends on gateway-specific crates) ────

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
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        let skills = moltis_skills::install::install_skill(source, &install_dir)
            .await
            .map_err(ServiceError::message)?;
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
        security_audit(
            "skills.install",
            serde_json::json!({
                "source": source,
                "installed_count": installed.len(),
            }),
        );
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
        let search_paths = FsSkillDiscoverer::default_paths();
        let discoverer = FsSkillDiscoverer::new(search_paths);
        let skills = discoverer.discover().await.map_err(ServiceError::message)?;
        let items: Vec<_> = skills
            .iter()
            .map(|s| {
                let elig = check_requirements(s);
                let protected = matches!(
                    s.source,
                    Some(moltis_skills::types::SkillSource::Personal)
                        | Some(moltis_skills::types::SkillSource::Project)
                ) && is_protected_discovered_skill(&s.name);
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "license": s.license,
                    "allowed_tools": s.allowed_tools,
                    "path": s.path.to_string_lossy(),
                    "source": s.source,
                    "protected": protected,
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
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        moltis_skills::install::remove_repo(source, &install_dir)
            .await
            .map_err(ServiceError::message)?;

        security_audit("skills.remove", serde_json::json!({ "source": source }));

        Ok(serde_json::json!({ "removed": source }))
    }

    async fn repos_list(&self) -> ServiceResult {
        let install_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        let manifest_path = moltis_skills::manifest::ManifestStore::default_path()
            .map_err(ServiceError::message)?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let mut manifest = store.load().map_err(ServiceError::message)?;
        let (drift_changed, drifted_sources) =
            detect_and_mark_repo_drift(&mut manifest, &install_dir);
        if drift_changed {
            store.save(&manifest).map_err(ServiceError::message)?;
        }

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let enabled = repo.skills.iter().filter(|s| s.enabled).count();
                // Re-detect format for repos that predate the formats module
                let format = if repo.format == moltis_skills::formats::PluginFormat::Skill {
                    let repo_dir = install_dir.join(&repo.repo_name);
                    moltis_skills::formats::detect_format(&repo_dir)
                } else {
                    repo.format
                };
                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "commit_sha": repo.commit_sha,
                    "drifted": drifted_sources.contains(&repo.source),
                    "format": format,
                    "skill_count": repo.skills.len(),
                    "enabled_count": enabled,
                })
            })
            .collect();

        let mut repos = repos;
        if let Ok(entries) = std::fs::read_dir(&install_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let repo_name = entry.file_name().to_string_lossy().to_string();
                if manifest.repos.iter().any(|r| r.repo_name == repo_name) {
                    continue;
                }
                let format = moltis_skills::formats::detect_format(&path);
                repos.push(serde_json::json!({
                    "source": format!("orphan:{repo_name}"),
                    "repo_name": repo_name,
                    "installed_at_ms": 0,
                    "commit_sha": null,
                    "drifted": false,
                    "orphaned": true,
                    "format": format,
                    "skill_count": 0,
                    "enabled_count": 0,
                }));
            }
        }

        Ok(serde_json::json!(repos))
    }

    async fn repos_list_full(&self) -> ServiceResult {
        use moltis_skills::requirements::check_requirements;

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        let manifest_path = moltis_skills::manifest::ManifestStore::default_path()
            .map_err(ServiceError::message)?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let mut manifest = store.load().map_err(ServiceError::message)?;
        let (drift_changed, drifted_sources) =
            detect_and_mark_repo_drift(&mut manifest, &install_dir);
        if drift_changed {
            store.save(&manifest).map_err(ServiceError::message)?;
        }

        let repos: Vec<_> = manifest
            .repos
            .iter()
            .map(|repo| {
                let repo_dir = install_dir.join(&repo.repo_name);
                // Re-detect format for repos that predate the formats module
                let format = if repo.format == moltis_skills::formats::PluginFormat::Skill {
                    moltis_skills::formats::detect_format(&repo_dir)
                } else {
                    repo.format
                };

                // For non-SKILL.md formats, scan with adapter to get enriched metadata.
                let adapter_entries = match format {
                    moltis_skills::formats::PluginFormat::Skill => None,
                    _ => moltis_skills::formats::scan_with_adapter(&repo_dir, format)
                        .and_then(|r| r.ok()),
                };

                let skills: Vec<_> = repo
                    .skills
                    .iter()
                    .map(|s| {
                        // If we have adapter entries, match by name for enriched data.
                        if let Some(ref entries) = adapter_entries {
                            let entry = entries.iter().find(|e| e.metadata.name == s.name);
                            serde_json::json!({
                                "name": s.name,
                                "description": entry.map(|e| e.metadata.description.as_str()).unwrap_or(""),
                                "display_name": entry.and_then(|e| e.display_name.as_deref()),
                                "relative_path": s.relative_path,
                                "trusted": s.trusted,
                                "enabled": s.enabled,
                                "drifted": drifted_sources.contains(&repo.source),
                                "eligible": true,
                                "missing_bins": [],
                            })
                        } else {
                            // SKILL.md format: parse from disk.
                            let skill_dir = install_dir.join(&s.relative_path);
                            let skill_md = skill_dir.join("SKILL.md");
                            let meta_json = moltis_skills::parse::read_meta_json(&skill_dir);
                            let (description, display_name, elig) =
                                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                                    if let Ok(meta) = moltis_skills::parse::parse_metadata(
                                        &content, &skill_dir,
                                    ) {
                                        let e = check_requirements(&meta);
                                        let desc = if meta.description.is_empty() {
                                            meta_json
                                                .as_ref()
                                                .and_then(|m| m.display_name.clone())
                                                .unwrap_or_default()
                                        } else {
                                            meta.description
                                        };
                                        let dn = meta_json
                                            .as_ref()
                                            .and_then(|m| m.display_name.clone());
                                        (desc, dn, Some(e))
                                    } else {
                                        let dn = meta_json
                                            .as_ref()
                                            .and_then(|m| m.display_name.clone());
                                        (dn.clone().unwrap_or_default(), dn, None)
                                    }
                                } else {
                                    let dn =
                                        meta_json.as_ref().and_then(|m| m.display_name.clone());
                                    (dn.clone().unwrap_or_default(), dn, None)
                                };
                            serde_json::json!({
                                "name": s.name,
                                "description": description,
                                "display_name": display_name,
                                "relative_path": s.relative_path,
                                "trusted": s.trusted,
                                "enabled": s.enabled,
                                "drifted": drifted_sources.contains(&repo.source),
                                "eligible": elig.as_ref().map(|e| e.eligible).unwrap_or(true),
                                "missing_bins": elig.as_ref().map(|e| e.missing_bins.clone()).unwrap_or_default(),
                            })
                        }
                    })
                    .collect();

                serde_json::json!({
                    "source": repo.source,
                    "repo_name": repo.repo_name,
                    "installed_at_ms": repo.installed_at_ms,
                    "commit_sha": repo.commit_sha,
                    "drifted": drifted_sources.contains(&repo.source),
                    "format": format,
                    "skills": skills,
                })
            })
            .collect();

        let mut repos = repos;
        if let Ok(entries) = std::fs::read_dir(&install_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let repo_name = entry.file_name().to_string_lossy().to_string();
                if manifest.repos.iter().any(|r| r.repo_name == repo_name) {
                    continue;
                }
                let format = moltis_skills::formats::detect_format(&path);
                repos.push(serde_json::json!({
                    "source": format!("orphan:{repo_name}"),
                    "repo_name": repo_name,
                    "installed_at_ms": 0,
                    "commit_sha": null,
                    "drifted": false,
                    "orphaned": true,
                    "format": format,
                    "skills": [],
                }));
            }
        }

        Ok(serde_json::json!(repos))
    }

    async fn repos_remove(&self, params: Value) -> ServiceResult {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'source' parameter".to_string())?;

        if let Some(repo_name) = source.strip_prefix("orphan:") {
            let install_dir =
                moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
            let dir = install_dir.join(repo_name);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(ServiceError::message)?;
            }
            security_audit(
                "skills.orphan.remove",
                serde_json::json!({ "source": source, "repo_name": repo_name }),
            );
            return Ok(serde_json::json!({ "removed": source }));
        }

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        moltis_skills::install::remove_repo(source, &install_dir)
            .await
            .map_err(ServiceError::message)?;

        security_audit(
            "skills.repos.remove",
            serde_json::json!({ "source": source }),
        );

        Ok(serde_json::json!({ "removed": source }))
    }

    async fn emergency_disable(&self) -> ServiceResult {
        let manifest_path = moltis_skills::manifest::ManifestStore::default_path()
            .map_err(ServiceError::message)?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let mut manifest = store.load().map_err(ServiceError::message)?;

        let mut disabled = 0_u64;
        for repo in &mut manifest.repos {
            for skill in &mut repo.skills {
                if skill.enabled {
                    disabled += 1;
                }
                skill.enabled = false;
            }
        }
        store.save(&manifest).map_err(ServiceError::message)?;

        security_audit(
            "skills.emergency_disable",
            serde_json::json!({ "disabled": disabled }),
        );

        Ok(serde_json::json!({ "disabled": disabled }))
    }

    async fn skill_enable(&self, params: Value) -> ServiceResult {
        toggle_skill(&params, true)
    }

    async fn skill_disable(&self, params: Value) -> ServiceResult {
        let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("");

        // Personal/project skills live as files — delete the directory to disable.
        if source == "personal" || source == "project" {
            return delete_discovered_skill(source, &params);
        }

        toggle_skill(&params, false)
    }

    async fn skill_trust(&self, params: Value) -> ServiceResult {
        set_skill_trusted(&params, true)
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

        // Personal/project skills: look up directly by name in discovered paths.
        if source == "personal" || source == "project" {
            return skill_detail_discovered(source, skill_name);
        }

        let install_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        let manifest_path = moltis_skills::manifest::ManifestStore::default_path()
            .map_err(ServiceError::message)?;
        let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
        let mut manifest = store.load().map_err(ServiceError::message)?;
        let (drift_changed, drifted_sources) =
            detect_and_mark_repo_drift(&mut manifest, &install_dir);
        if drift_changed {
            store.save(&manifest).map_err(ServiceError::message)?;
        }

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

        let repo_dir = install_dir.join(&repo.repo_name);
        let commit_sha = repo.commit_sha.clone();
        let commit_url = commit_sha
            .as_ref()
            .and_then(|sha| commit_url_for_source(source, sha));
        let commit_age_days = commit_age_days(local_repo_head_timestamp_ms(&repo_dir));

        // Route by format: SKILL.md repos parse the file; others use format adapters.
        match repo.format {
            moltis_skills::formats::PluginFormat::Skill => {
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
                let license_url =
                    license_url_for_source(source, content.metadata.license.as_deref());
                let source_url: Option<String> = {
                    let rel = &skill_state.relative_path;
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
                    "license_url": license_url,
                    "compatibility": content.metadata.compatibility,
                    "allowed_tools": content.metadata.allowed_tools,
                    "requires": content.metadata.requires,
                    "eligible": elig.eligible,
                    "missing_bins": elig.missing_bins,
                    "install_options": elig.install_options,
                    "trusted": skill_state.trusted,
                    "enabled": skill_state.enabled,
                    "drifted": drifted_sources.contains(source),
                    "commit_sha": commit_sha,
                    "commit_url": commit_url,
                    "commit_age_days": commit_age_days,
                    "source_url": source_url,
                    "body": content.body,
                    "body_html": markdown_to_html(&content.body),
                    "source": source,
                }))
            },
            format => {
                // Non-SKILL.md format: use adapter to scan for skill body + metadata.
                let entries = moltis_skills::formats::scan_with_adapter(&repo_dir, format)
                    .ok_or_else(|| format!("no adapter for format '{format}'"))?
                    .map_err(|e| format!("scan error: {e}"))?;
                let entry = entries
                    .into_iter()
                    .find(|e| e.metadata.name == skill_name)
                    .ok_or_else(|| format!("skill '{skill_name}' not found on disk"))?;
                let source_url: Option<String> = entry.source_file.as_ref().map(|file| {
                    if source.starts_with("https://") || source.starts_with("http://") {
                        format!("{}/blob/main/{}", source.trim_end_matches('/'), file)
                    } else {
                        format!("https://github.com/{}/blob/main/{}", source, file)
                    }
                });
                let license_url = license_url_for_source(source, entry.metadata.license.as_deref());
                let empty: Vec<String> = Vec::new();
                Ok(serde_json::json!({
                    "name": entry.metadata.name,
                    "display_name": entry.display_name,
                    "description": entry.metadata.description,
                    "author": entry.author,
                    "homepage": entry.metadata.homepage,
                    "version": null,
                    "license": entry.metadata.license,
                    "license_url": license_url,
                    "compatibility": entry.metadata.compatibility,
                    "allowed_tools": entry.metadata.allowed_tools,
                    "requires": entry.metadata.requires,
                    "eligible": true,
                    "missing_bins": empty,
                    "install_options": empty,
                    "trusted": skill_state.trusted,
                    "enabled": skill_state.enabled,
                    "drifted": drifted_sources.contains(source),
                    "commit_sha": commit_sha,
                    "commit_url": commit_url,
                    "commit_age_days": commit_age_days,
                    "source_url": source_url,
                    "body": entry.body,
                    "body_html": markdown_to_html(&entry.body),
                    "source": source,
                }))
            },
        }
    }

    async fn install_dep(&self, params: Value) -> ServiceResult {
        use {
            moltis_skills::{
                discover::{FsSkillDiscoverer, SkillDiscoverer},
                requirements::{check_requirements, install_command_preview, run_install},
            },
            moltis_tools::approval::{
                ApprovalAction, ApprovalManager, ApprovalMode, SecurityLevel,
            },
        };

        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'skill' parameter".to_string())?;
        let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let confirm = params
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let allow_host_install = params
            .get("allow_host_install")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let allow_risky_install = params
            .get("allow_risky_install")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Discover the skill to get its requirements
        let search_paths = FsSkillDiscoverer::default_paths();
        let discoverer = FsSkillDiscoverer::new(search_paths);
        let skills = discoverer.discover().await.map_err(ServiceError::message)?;

        let meta = skills
            .iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| format!("skill '{skill_name}' not found"))?;

        let elig = check_requirements(meta);
        let spec = elig
            .install_options
            .get(index)
            .ok_or_else(|| format!("install option index {index} out of range"))?;

        let command_preview = install_command_preview(spec).map_err(ServiceError::message)?;
        if !confirm {
            return Err(format!(
                "dependency install requires explicit confirmation. Re-run with confirm=true after reviewing command: {command_preview}"
            )
            .into());
        }

        if let Some(reason) = risky_install_pattern(&command_preview)
            && !allow_risky_install
        {
            security_audit(
                "skills.install_dep_blocked",
                serde_json::json!({
                    "skill": skill_name,
                    "command": command_preview,
                    "reason": reason,
                }),
            );
            return Err(format!(
                "dependency install blocked as risky ({reason}). Re-run with allow_risky_install=true only after manual review"
            )
            .into());
        }

        let config = moltis_config::discover_and_load();
        if config.tools.exec.sandbox.mode == "off" && !allow_host_install {
            return Err("dependency install blocked because sandbox mode is off. Enable sandbox or re-run with allow_host_install=true and confirm=true".into());
        }

        let mut approval = ApprovalManager::default();
        approval.mode =
            ApprovalMode::parse(&config.tools.exec.approval_mode).unwrap_or(ApprovalMode::OnMiss);
        approval.security_level = SecurityLevel::parse(&config.tools.exec.security_level)
            .unwrap_or(SecurityLevel::Allowlist);
        approval.allowlist = config.tools.exec.allowlist;

        match approval
            .check_command(&command_preview)
            .await
            .map_err(ServiceError::message)?
        {
            ApprovalAction::Proceed => {},
            // skills.install_dep is an interactive RPC invoked by the user in the UI;
            // `confirm=true` is treated as the explicit approval for this action.
            ApprovalAction::NeedsApproval => {},
        }

        let result = run_install(spec).await.map_err(ServiceError::message)?;

        security_audit(
            "skills.install_dep",
            serde_json::json!({
                "skill": skill_name,
                "command": command_preview,
                "success": result.success,
            }),
        );

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
            )
            .into())
        }
    }

    async fn security_status(&self) -> ServiceResult {
        let installed_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        let mcp_scan_available = command_available("mcp-scan").await;
        let uvx_available = command_available("uvx").await;
        Ok(serde_json::json!({
            "mcp_scan_available": mcp_scan_available,
            "uvx_available": uvx_available,
            "supported": mcp_scan_available || uvx_available,
            "installed_skills_dir": installed_dir,
            "install_hint": "Install uv (https://docs.astral.sh/uv/) or mcp-scan to run skill security scans",
        }))
    }

    async fn security_scan(&self) -> ServiceResult {
        let installed_dir =
            moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
        if !installed_dir.exists() {
            return Ok(serde_json::json!({
                "ok": true,
                "message": "No installed skills directory found",
                "results": null,
            }));
        }

        let status = self.security_status().await?;
        let supported = status
            .get("supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !supported {
            return Err("mcp-scan is not available. Install uvx or mcp-scan binary first".into());
        }

        let results = run_mcp_scan(&installed_dir)
            .await
            .map_err(ServiceError::message)?;
        security_audit(
            "skills.security.scan",
            serde_json::json!({ "installed_dir": installed_dir, "status": "ok" }),
        );
        Ok(serde_json::json!({
            "ok": true,
            "installed_skills_dir": installed_dir,
            "results": results,
        }))
    }
}

fn local_repo_head_sha(repo_dir: &Path) -> Option<String> {
    let repo = gix::open(repo_dir).ok()?;
    let obj = repo.rev_parse_single("HEAD").ok()?;
    Some(obj.detach().to_hex().to_string())
}

fn detect_and_mark_repo_drift(
    manifest: &mut moltis_skills::types::SkillsManifest,
    install_dir: &Path,
) -> (bool, HashSet<String>) {
    let mut changed = false;
    let mut drifted = HashSet::new();

    for repo in &mut manifest.repos {
        let Some(expected_sha) = repo.commit_sha.clone() else {
            continue;
        };

        let repo_dir = install_dir.join(&repo.repo_name);
        let Some(current_sha) = local_repo_head_sha(&repo_dir) else {
            continue;
        };

        if current_sha != expected_sha {
            drifted.insert(repo.source.clone());
            repo.commit_sha = Some(current_sha);
            for skill in &mut repo.skills {
                skill.trusted = false;
                skill.enabled = false;
            }
            security_audit(
                "skills.source_drift_detected",
                serde_json::json!({
                    "source": repo.source,
                    "new_commit_sha": repo.commit_sha,
                }),
            );
            changed = true;
        }
    }

    (changed, drifted)
}

/// Delete a personal or project skill directory to disable it.
fn delete_discovered_skill(source_type: &str, params: &Value) -> ServiceResult {
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    if is_protected_discovered_skill(skill_name) {
        return Err(
            format!("skill '{skill_name}' is protected and cannot be deleted from the UI").into(),
        );
    }

    if !moltis_skills::parse::validate_name(skill_name) {
        return Err(format!("invalid skill name '{skill_name}'").into());
    }

    let search_dir = if source_type == "personal" {
        moltis_config::data_dir().join("skills")
    } else {
        moltis_config::data_dir().join(".moltis/skills")
    };

    let skill_dir = search_dir.join(skill_name);
    if !skill_dir.exists() {
        return Err(format!("skill '{skill_name}' not found").into());
    }

    std::fs::remove_dir_all(&skill_dir)
        .map_err(|e| format!("failed to delete skill '{skill_name}': {e}"))?;

    security_audit(
        "skills.discovered.delete",
        serde_json::json!({
            "source": source_type,
            "skill": skill_name,
        }),
    );

    Ok(serde_json::json!({ "source": source_type, "skill": skill_name, "deleted": true }))
}

/// Load skill detail for a personal or project skill by name.
fn skill_detail_discovered(source_type: &str, skill_name: &str) -> ServiceResult {
    use moltis_skills::requirements::check_requirements;

    // Build search paths for the requested source type.
    let search_dir = if source_type == "personal" {
        moltis_config::data_dir().join("skills")
    } else {
        moltis_config::data_dir().join(".moltis/skills")
    };

    let skill_dir = search_dir.join(skill_name);
    let skill_md = skill_dir.join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_md)
        .map_err(|e| format!("failed to read SKILL.md for '{skill_name}': {e}"))?;

    let content = moltis_skills::parse::parse_skill(&raw, &skill_dir)
        .map_err(|e| format!("failed to parse SKILL.md: {e}"))?;

    let elig = check_requirements(&content.metadata);

    Ok(serde_json::json!({
        "name": content.metadata.name,
        "description": content.metadata.description,
        "license": content.metadata.license,
        "license_url": license_url_for_source(source_type, content.metadata.license.as_deref()),
        "compatibility": content.metadata.compatibility,
        "allowed_tools": content.metadata.allowed_tools,
        "requires": content.metadata.requires,
        "eligible": elig.eligible,
        "missing_bins": elig.missing_bins,
        "install_options": elig.install_options,
        "trusted": true,
        "enabled": true,
        "protected": is_protected_discovered_skill(skill_name),
        "body": content.body,
        "body_html": markdown_to_html(&content.body),
        "source": source_type,
        "path": skill_dir.to_string_lossy(),
    }))
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
        moltis_skills::manifest::ManifestStore::default_path().map_err(ServiceError::message)?;
    let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
    let mut manifest = store.load().map_err(ServiceError::message)?;

    let install_dir =
        moltis_skills::install::default_install_dir().map_err(ServiceError::message)?;
    let (drift_changed, drifted_sources) = detect_and_mark_repo_drift(&mut manifest, &install_dir);
    if drift_changed {
        store.save(&manifest).map_err(ServiceError::message)?;
    }

    if enabled {
        if drifted_sources.contains(source) {
            return Err(format!(
                "skill '{skill_name}' source changed since it was last trusted. Review and run skills.skill.trust before enabling"
            )
            .into());
        }

        let trusted = manifest
            .find_repo(source)
            .and_then(|r| r.skills.iter().find(|s| s.name == skill_name))
            .map(|s| s.trusted)
            .ok_or_else(|| format!("skill '{skill_name}' not found in repo '{source}'"))?;
        if !trusted {
            return Err(format!(
                "skill '{skill_name}' is not trusted. Review it and run skills.skill.trust before enabling"
            )
            .into());
        }
    }

    if !manifest.set_skill_enabled(source, skill_name, enabled) {
        return Err(format!("skill '{skill_name}' not found in repo '{source}'").into());
    }
    store.save(&manifest).map_err(ServiceError::message)?;

    security_audit(
        "skills.skill.toggle",
        serde_json::json!({
            "source": source,
            "skill": skill_name,
            "enabled": enabled,
        }),
    );

    Ok(serde_json::json!({ "source": source, "skill": skill_name, "enabled": enabled }))
}

fn set_skill_trusted(params: &Value, trusted: bool) -> ServiceResult {
    let source = params
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'source' parameter".to_string())?;
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    let manifest_path =
        moltis_skills::manifest::ManifestStore::default_path().map_err(ServiceError::message)?;
    let store = moltis_skills::manifest::ManifestStore::new(manifest_path);
    let mut manifest = store.load().map_err(ServiceError::message)?;

    if !manifest.set_skill_trusted(source, skill_name, trusted) {
        return Err(format!("skill '{skill_name}' not found in repo '{source}'").into());
    }

    if !trusted {
        let _ = manifest.set_skill_enabled(source, skill_name, false);
    }

    store.save(&manifest).map_err(ServiceError::message)?;
    security_audit(
        "skills.skill.trust",
        serde_json::json!({
            "source": source,
            "skill": skill_name,
            "trusted": trusted,
        }),
    );
    Ok(serde_json::json!({ "source": source, "skill": skill_name, "trusted": trusted }))
}

// ── Browser (Real implementation — depends on moltis-browser) ───────────────

/// Real browser service using BrowserManager.
pub struct RealBrowserService {
    manager: moltis_browser::BrowserManager,
}

impl RealBrowserService {
    pub fn new(config: &moltis_config::schema::BrowserConfig, container_prefix: String) -> Self {
        let mut browser_config = moltis_browser::BrowserConfig::from(config);
        browser_config.container_prefix = container_prefix;
        Self {
            manager: moltis_browser::BrowserManager::new(browser_config),
        }
    }

    pub fn from_config(
        config: &moltis_config::schema::MoltisConfig,
        container_prefix: String,
    ) -> Option<Self> {
        if !config.tools.browser.enabled {
            return None;
        }
        // Check if Chrome/Chromium is available and warn if not
        moltis_browser::detect::check_and_warn(config.tools.browser.chrome_path.as_deref());
        Some(Self::new(&config.tools.browser, container_prefix))
    }
}

#[async_trait]
impl BrowserService for RealBrowserService {
    async fn request(&self, params: Value) -> ServiceResult {
        let request: moltis_browser::BrowserRequest =
            serde_json::from_value(params).map_err(|e| format!("invalid request: {e}"))?;

        let response = self.manager.handle_request(request).await;

        Ok(serde_json::to_value(&response).map_err(|e| format!("serialization error: {e}"))?)
    }

    async fn cleanup_idle(&self) {
        self.manager.cleanup_idle().await;
    }

    async fn shutdown(&self) {
        self.manager.shutdown().await;
    }

    async fn close_all(&self) {
        self.manager.shutdown().await;
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
    pub stt: Arc<dyn SttService>,
    pub skills: Arc<dyn SkillsService>,
    pub mcp: Arc<dyn McpService>,
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
    pub network_audit: Arc<dyn crate::network_audit::NetworkAuditService>,
    /// Optional channel outbound for sending replies back to channels.
    channel_outbound: Option<Arc<dyn moltis_channels::ChannelOutbound>>,
    /// Optional channel stream outbound for edit-in-place channel streaming.
    channel_stream_outbound: Option<Arc<dyn moltis_channels::ChannelStreamOutbound>>,
    /// Optional session metadata for cross-service access (e.g. channel binding).
    pub session_metadata: Option<Arc<moltis_sessions::metadata::SqliteSessionMetadata>>,
    /// Optional session store for message-index lookups (e.g. deduplication).
    pub session_store: Option<Arc<moltis_sessions::store::SessionStore>>,
    /// Optional session share store for immutable snapshot links.
    pub session_share_store: Option<Arc<crate::share_store::ShareStore>>,
    /// Optional agent persona store for multi-agent support.
    pub agent_persona_store: Option<Arc<crate::agent_persona::AgentPersonaStore>>,
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

    pub fn with_channel_outbound(
        mut self,
        outbound: Arc<dyn moltis_channels::ChannelOutbound>,
    ) -> Self {
        self.channel_outbound = Some(outbound);
        self
    }

    pub fn with_channel_stream_outbound(
        mut self,
        outbound: Arc<dyn moltis_channels::ChannelStreamOutbound>,
    ) -> Self {
        self.channel_stream_outbound = Some(outbound);
        self
    }

    pub fn channel_outbound_arc(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
        self.channel_outbound.clone()
    }

    pub fn channel_stream_outbound_arc(
        &self,
    ) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
        self.channel_stream_outbound.clone()
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
            stt: Arc::new(NoopSttService),
            skills: Arc::new(NoopSkillsService),
            mcp: Arc::new(NoopMcpService),
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
            network_audit: Arc::new(crate::network_audit::NoopNetworkAuditService),
            channel_outbound: None,
            channel_stream_outbound: None,
            session_metadata: None,
            session_store: None,
            session_share_store: None,
            agent_persona_store: None,
        }
    }

    pub fn with_local_llm(mut self, local_llm: Arc<dyn LocalLlmService>) -> Self {
        self.local_llm = local_llm;
        self
    }

    pub fn with_network_audit(
        mut self,
        svc: Arc<dyn crate::network_audit::NetworkAuditService>,
    ) -> Self {
        self.network_audit = svc;
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

    pub fn with_session_share_store(mut self, store: Arc<crate::share_store::ShareStore>) -> Self {
        self.session_share_store = Some(store);
        self
    }

    pub fn with_agent_persona_store(
        mut self,
        store: Arc<crate::agent_persona::AgentPersonaStore>,
    ) -> Self {
        self.agent_persona_store = Some(store);
        self
    }

    pub fn with_tts(mut self, tts: Arc<dyn TtsService>) -> Self {
        self.tts = tts;
        self
    }

    pub fn with_stt(mut self, stt: Arc<dyn SttService>) -> Self {
        self.stt = stt;
        self
    }

    /// Create a [`Services`] bundle for sharing with the GraphQL schema.
    ///
    /// Clones all service `Arc`s (cheap pointer bumps) into the shared bundle.
    /// The `system_info` service is provided separately because it needs the
    /// fully-constructed `GatewayState` which isn't available during
    /// `GatewayServices` construction.
    pub fn to_services(&self, system_info: Arc<dyn SystemInfoService>) -> Arc<Services> {
        Arc::new(Services {
            agent: self.agent.clone(),
            session: self.session.clone(),
            channel: self.channel.clone(),
            config: self.config.clone(),
            cron: self.cron.clone(),
            chat: self.chat.clone(),
            tts: self.tts.clone(),
            stt: self.stt.clone(),
            skills: self.skills.clone(),
            mcp: self.mcp.clone(),
            browser: self.browser.clone(),
            usage: self.usage.clone(),
            exec_approval: self.exec_approval.clone(),
            onboarding: self.onboarding.clone(),
            update: self.update.clone(),
            model: self.model.clone(),
            web_login: self.web_login.clone(),
            voicewake: self.voicewake.clone(),
            logs: self.logs.clone(),
            provider_setup: self.provider_setup.clone(),
            project: self.project.clone(),
            local_llm: self.local_llm.clone(),
            system_info,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risky_install_pattern_detects_piped_shell() {
        assert_eq!(
            risky_install_pattern("curl https://example.com/install.sh | sh"),
            Some("piped shell execution")
        );
    }

    #[test]
    fn risky_install_pattern_allows_plain_package_install() {
        assert_eq!(risky_install_pattern("cargo install ripgrep"), None);
    }
}
