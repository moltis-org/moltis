use std::path::{Component, Path, PathBuf};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, skills as skills_metrics};

use crate::{
    formats::{PluginFormat, detect_format, scan_with_adapter},
    manifest::ManifestStore,
    parse,
    types::{RepoEntry, SkillMetadata, SkillState},
};

/// Install a skill repo from GitHub into the target directory.
///
/// Downloads the repo to `install_dir/<owner>-<repo>/`, auto-detects its format
/// (SKILL.md, Claude Code `.claude-plugin/`, etc.), scans for skills using the
/// appropriate adapter, and records the repo + skills in the manifest.
pub async fn install_skill(source: &str, install_dir: &Path) -> anyhow::Result<Vec<SkillMetadata>> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    counter!(skills_metrics::INSTALLATION_ATTEMPTS_TOTAL).increment(1);

    let (owner, repo) = parse_source(source)?;
    let dir_name = format!("{owner}-{repo}");
    let target = install_dir.join(&dir_name);

    if target.exists() {
        let manifest_path = ManifestStore::default_path()?;
        let store = ManifestStore::new(manifest_path);
        let manifest = store.load()?;
        if manifest.find_repo(source).is_none() {
            tokio::fs::remove_dir_all(&target).await?;
        } else {
            anyhow::bail!(
                "repo directory already exists: {}. Remove it first with `skills remove`.",
                target.display()
            );
        }
    }

    tokio::fs::create_dir_all(install_dir).await?;

    #[cfg(feature = "metrics")]
    counter!("moltis_skills_git_clone_fallback_total").increment(1);
    let commit_sha = install_via_http(&owner, &repo, &target).await?;

    // Auto-detect repo format and scan accordingly.
    let format = detect_format(&target);
    let (skills_meta, skill_states) = match format {
        PluginFormat::Skill => scan_repo_skills(&target, install_dir).await?,
        _ => match scan_with_adapter(&target, format) {
            Some(result) => {
                let entries = result?;
                let relative = target
                    .strip_prefix(install_dir)
                    .unwrap_or(&target)
                    .to_string_lossy()
                    .to_string();
                let meta: Vec<SkillMetadata> = entries.iter().map(|e| e.metadata.clone()).collect();
                let states: Vec<SkillState> = entries
                    .iter()
                    .map(|e| SkillState {
                        name: e.metadata.name.clone(),
                        relative_path: relative.clone(),
                        trusted: false,
                        enabled: false,
                    })
                    .collect();
                (meta, states)
            },
            None => {
                let _ = tokio::fs::remove_dir_all(&target).await;
                anyhow::bail!("no adapter available for format '{format}' in repo '{source}'");
            },
        },
    };

    if skills_meta.is_empty() {
        let _ = tokio::fs::remove_dir_all(&target).await;
        anyhow::bail!(
            "repository contains no skills (checked {})",
            target.display()
        );
    }

    // Write manifest.
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    let mut manifest = store.load()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    manifest.add_repo(RepoEntry {
        source: format!("{owner}/{repo}"),
        repo_name: dir_name,
        installed_at_ms: now,
        commit_sha,
        format,
        skills: skill_states,
    });
    store.save(&manifest)?;

    #[cfg(feature = "metrics")]
    histogram!(skills_metrics::INSTALLATION_DURATION_SECONDS).record(start.elapsed().as_secs_f64());

    tracing::info!(count = skills_meta.len(), %source, "installed repo skills");
    Ok(skills_meta)
}

/// Remove a repo: delete directory and manifest entry.
pub async fn remove_repo(source: &str, install_dir: &Path) -> anyhow::Result<()> {
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    let mut manifest = store.load()?;

    let repo = manifest
        .find_repo(source)
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found in manifest", source))?;
    let dir = install_dir.join(&repo.repo_name);

    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
    }

    manifest.remove_repo(source);
    store.save(&manifest)?;
    Ok(())
}

/// Install by fetching a tarball from GitHub's API.
async fn install_via_http(
    owner: &str,
    repo: &str,
    target: &Path,
) -> anyhow::Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/tarball");
    let client = reqwest::Client::new();
    let commit_sha = fetch_latest_commit_sha(&client, owner, repo).await;
    let resp = client
        .get(&url)
        .header("User-Agent", "moltis-skills")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("failed to fetch {}/{}: HTTP {}", owner, repo, resp.status());
    }

    let bytes = resp.bytes().await?;

    tokio::fs::create_dir_all(target).await?;
    let target_owned = target.to_path_buf();
    let owner_owned = owner.to_string();
    let repo_owned = repo.to_string();
    tokio::task::spawn_blocking(move || {
        let canonical_target = std::fs::canonicalize(&target_owned)?;
        let decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.header().entry_type().is_symlink()
                || entry.header().entry_type().is_hard_link()
            {
                tracing::warn!(owner = %owner_owned, repo = %repo_owned, "skipping symlink/hardlink archive entry");
                continue;
            }

            let path = entry.path()?.into_owned();
            let Some(stripped) = sanitize_archive_path(&path)? else {
                continue;
            };

            let dest = target_owned.join(&stripped);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
                let canonical_parent = std::fs::canonicalize(parent)?;
                if !canonical_parent.starts_with(&canonical_target) {
                    anyhow::bail!("archive entry escaped install directory");
                }
            }

            if dest.exists() {
                let meta = std::fs::symlink_metadata(&dest)?;
                if meta.file_type().is_symlink() {
                    anyhow::bail!("archive entry resolves to symlink destination");
                }
            }

            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&dest)?;
                continue;
            }

            entry.unpack(&dest)?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await??;

    tracing::info!(%owner, %repo, "installed skill repo via HTTP tarball");
    Ok(commit_sha)
}

async fn fetch_latest_commit_sha(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Option<String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/commits?per_page=1");
    let response = client
        .get(url)
        .header("User-Agent", "moltis-skills")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: serde_json::Value = response.json().await.ok()?;
    value
        .as_array()?
        .first()?
        .get("sha")?
        .as_str()
        .filter(|sha| sha.len() == 40)
        .map(ToOwned::to_owned)
}

fn sanitize_archive_path(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    let stripped: PathBuf = path.components().skip(1).collect();
    if stripped.as_os_str().is_empty() {
        return Ok(None);
    }

    for component in stripped.components() {
        match component {
            Component::Normal(_) => {},
            Component::CurDir => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("archive contains unsafe path component: {}", path.display());
            },
        }
    }

    Ok(Some(stripped))
}

/// Recursively scan a cloned repo for SKILL.md files.
/// Returns (Vec<SkillMetadata>, Vec<SkillState>) — metadata for callers and
/// state entries for the manifest.
async fn scan_repo_skills(
    repo_dir: &Path,
    install_dir: &Path,
) -> anyhow::Result<(Vec<SkillMetadata>, Vec<SkillState>)> {
    // Check root SKILL.md (single-skill repo).
    let root_skill_md = repo_dir.join("SKILL.md");
    if root_skill_md.is_file() {
        let content = tokio::fs::read_to_string(&root_skill_md).await?;
        let mut meta = parse::parse_metadata(&content, repo_dir)?;
        meta.source = Some(crate::types::SkillSource::Registry);

        let relative = repo_dir
            .strip_prefix(install_dir)
            .unwrap_or(repo_dir)
            .to_string_lossy()
            .to_string();

        let state = SkillState {
            name: meta.name.clone(),
            relative_path: relative,
            trusted: false,
            enabled: false,
        };
        return Ok((vec![meta], vec![state]));
    }

    // Multi-skill: recursively scan for SKILL.md.
    let mut skills_meta = Vec::new();
    let mut skill_states = Vec::new();
    let mut dirs_to_scan = vec![repo_dir.to_path_buf()];

    while let Some(dir) = dirs_to_scan.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Some(entry) = entries.next_entry().await? {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            let skill_md = subdir.join("SKILL.md");
            if skill_md.is_file() {
                let content = match tokio::fs::read_to_string(&skill_md).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!(?skill_md, %e, "skipping unreadable SKILL.md");
                        continue;
                    },
                };
                match parse::parse_metadata(&content, &subdir) {
                    Ok(mut meta) => {
                        meta.source = Some(crate::types::SkillSource::Registry);
                        let relative = subdir
                            .strip_prefix(install_dir)
                            .unwrap_or(&subdir)
                            .to_string_lossy()
                            .to_string();
                        skill_states.push(SkillState {
                            name: meta.name.clone(),
                            relative_path: relative,
                            trusted: false,
                            enabled: false,
                        });
                        skills_meta.push(meta);
                    },
                    Err(e) => {
                        tracing::debug!(?skill_md, %e, "skipping non-conforming SKILL.md");
                    },
                }
            } else {
                dirs_to_scan.push(subdir);
            }
        }
    }

    Ok((skills_meta, skill_states))
}

/// Parse `owner/repo` from a source string.
/// Accepts `owner/repo`, `https://github.com/owner/repo`, or with trailing slash/`.git`.
fn parse_source(source: &str) -> anyhow::Result<(String, String)> {
    let s = source.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s
        .strip_prefix("https://github.com/")
        .or_else(|| s.strip_prefix("http://github.com/"))
        .or_else(|| s.strip_prefix("github.com/"))
        .unwrap_or(s);
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        anyhow::bail!(
            "invalid skill source '{}': expected 'owner/repo' or GitHub URL",
            source
        );
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Get the default installation directory.
pub fn default_install_dir() -> anyhow::Result<PathBuf> {
    Ok(moltis_config::data_dir().join("installed-skills"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_valid() {
        let (owner, repo) = parse_source("vercel-labs/agent-skills").unwrap();
        assert_eq!(owner, "vercel-labs");
        assert_eq!(repo, "agent-skills");
    }

    #[test]
    fn test_parse_source_github_url() {
        let (o, r) = parse_source("https://github.com/remotion-dev/skills").unwrap();
        assert_eq!(o, "remotion-dev");
        assert_eq!(r, "skills");

        let (o, r) = parse_source("https://github.com/owner/repo/").unwrap();
        assert_eq!(o, "owner");
        assert_eq!(r, "repo");

        let (o, r) = parse_source("https://github.com/owner/repo.git").unwrap();
        assert_eq!(o, "owner");
        assert_eq!(r, "repo");

        let (o, r) = parse_source("github.com/owner/repo").unwrap();
        assert_eq!(o, "owner");
        assert_eq!(r, "repo");
    }

    #[test]
    fn test_parse_source_invalid() {
        assert!(parse_source("noslash").is_err());
        assert!(parse_source("too/many/parts").is_err());
        assert!(parse_source("/empty-owner").is_err());
        assert!(parse_source("empty-repo/").is_err());
    }

    #[test]
    fn test_sanitize_archive_path_rejects_parent_dir() {
        let path = Path::new("repo-root/../../etc/passwd");
        assert!(sanitize_archive_path(path).is_err());
    }

    #[test]
    fn test_sanitize_archive_path_accepts_normal_path() {
        let path = Path::new("repo-root/skills/demo/SKILL.md");
        let sanitized = sanitize_archive_path(path).unwrap().unwrap();
        assert_eq!(sanitized, PathBuf::from("skills/demo/SKILL.md"));
    }

    #[tokio::test]
    async fn test_scan_single_skill_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path();
        let repo_dir = install_dir.join("my-repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::write(
            repo_dir.join("SKILL.md"),
            "---\nname: single\ndescription: test\n---\nbody\n",
        )
        .unwrap();

        let (meta, states) = scan_repo_skills(&repo_dir, install_dir).await.unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].name, "single");
        assert_eq!(states.len(), 1);
        assert!(!states[0].enabled);
        assert_eq!(states[0].relative_path, "my-repo");
    }

    #[test]
    fn test_detect_format_routes_claude_code() {
        use crate::formats::{PluginFormat, detect_format, scan_with_adapter};

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create Claude Code plugin structure
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"test-plugin","description":"A test plugin","author":"test-author"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("agents/helper.md"),
            "Use this agent to help with tasks.\n\nDetailed instructions.",
        )
        .unwrap();

        let format = detect_format(root);
        assert_eq!(format, PluginFormat::ClaudeCode);

        // scan_with_adapter should return Some for ClaudeCode
        let result = scan_with_adapter(root, format);
        assert!(result.is_some());
        let entries = result.unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].metadata.name, "test-plugin:helper");

        // Convert to skill states (same logic as install_skill)
        let states: Vec<SkillState> = entries
            .iter()
            .map(|e| SkillState {
                name: e.metadata.name.clone(),
                relative_path: "test-owner-test-repo".into(),
                trusted: false,
                enabled: false,
            })
            .collect();
        assert_eq!(states.len(), 1);
        assert!(!states[0].enabled);
        assert!(!states[0].trusted);
    }

    #[tokio::test]
    async fn test_scan_multi_skill_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path();
        let repo_dir = install_dir.join("multi");
        std::fs::create_dir_all(repo_dir.join("skills/a")).unwrap();
        std::fs::create_dir_all(repo_dir.join("skills/b")).unwrap();
        std::fs::write(
            repo_dir.join("skills/a/SKILL.md"),
            "---\nname: skill-a\ndescription: A\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            repo_dir.join("skills/b/SKILL.md"),
            "---\nname: skill-b\ndescription: B\n---\nbody\n",
        )
        .unwrap();

        let (meta, states) = scan_repo_skills(&repo_dir, install_dir).await.unwrap();
        assert_eq!(meta.len(), 2);
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|s| !s.enabled));
    }
}
