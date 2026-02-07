//! Plugin installation and removal.
//!
//! Plugins install to `~/.moltis/installed-plugins` and are tracked in
//! `~/.moltis/plugins-manifest.json`, separate from skills.

use std::path::{Component, Path, PathBuf};

use moltis_skills::{
    manifest::ManifestStore,
    types::{RepoEntry, SkillMetadata, SkillState},
};

use crate::formats::{PluginFormat, detect_format, scan_with_adapter};

/// Get the default plugin installation directory.
pub fn default_plugins_dir() -> anyhow::Result<PathBuf> {
    Ok(moltis_config::data_dir().join("installed-plugins"))
}

/// Default plugins manifest path: `~/.moltis/plugins-manifest.json`.
pub fn default_manifest_path() -> anyhow::Result<PathBuf> {
    Ok(moltis_config::data_dir().join("plugins-manifest.json"))
}

/// Install a plugin repo from GitHub into the target directory.
///
/// Clones the repo, detects the plugin format, scans for skills using the
/// format adapter, and records the result in the plugins manifest.
/// Returns an error if the repo is in native `SKILL.md` format (use skills
/// install for those).
pub async fn install_plugin(
    source: &str,
    install_dir: &Path,
) -> anyhow::Result<Vec<SkillMetadata>> {
    let (owner, repo) = parse_source(source)?;
    let dir_name = format!("{owner}-{repo}");
    let target = install_dir.join(&dir_name);

    if target.exists() {
        let manifest_path = default_manifest_path()?;
        let store = ManifestStore::new(manifest_path);
        let manifest = store.load()?;
        if manifest.find_repo(source).is_none() {
            tokio::fs::remove_dir_all(&target).await?;
        } else {
            anyhow::bail!(
                "plugin directory already exists: {}. Remove it first with `plugins remove`.",
                target.display()
            );
        }
    }

    tokio::fs::create_dir_all(install_dir).await?;

    let commit_sha = install_via_http(&owner, &repo, &target).await?;

    // Detect format — must be a non-Skill format for the plugins crate.
    let format = detect_format(&target);
    if format == PluginFormat::Skill {
        let _ = tokio::fs::remove_dir_all(&target).await;
        anyhow::bail!(
            "repository '{}' uses native SKILL.md format — install it via the skills page instead",
            source
        );
    }

    let skills_result = scan_with_adapter(&target, format);
    let (skills_meta, skill_states) = match skills_result {
        Some(result) => {
            let entries = result?;
            let meta: Vec<SkillMetadata> = entries.iter().map(|e| e.metadata.clone()).collect();
            let states: Vec<SkillState> = entries
                .iter()
                .map(|e| {
                    let relative = target
                        .strip_prefix(install_dir)
                        .unwrap_or(&target)
                        .to_string_lossy()
                        .to_string();
                    SkillState {
                        name: e.metadata.name.clone(),
                        relative_path: relative,
                        trusted: false,
                        enabled: false,
                    }
                })
                .collect();
            (meta, states)
        },
        None => {
            let _ = tokio::fs::remove_dir_all(&target).await;
            anyhow::bail!("no adapter available for format '{format}' in repo '{source}'");
        },
    };

    if skills_meta.is_empty() {
        let _ = tokio::fs::remove_dir_all(&target).await;
        anyhow::bail!(
            "plugin repository contains no skills (checked {})",
            target.display()
        );
    }

    // Write to plugins manifest.
    let manifest_path = default_manifest_path()?;
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
        format: PluginFormat::default(),
        skills: skill_states,
    });
    store.save(&manifest)?;

    tracing::info!(count = skills_meta.len(), %source, "installed plugin repo");
    Ok(skills_meta)
}

/// Remove a plugin repo: delete directory and manifest entry.
pub async fn remove_plugin(source: &str, install_dir: &Path) -> anyhow::Result<()> {
    let manifest_path = default_manifest_path()?;
    let store = ManifestStore::new(manifest_path);
    let mut manifest = store.load()?;

    let repo = manifest
        .find_repo(source)
        .ok_or_else(|| anyhow::anyhow!("plugin repo '{}' not found in manifest", source))?;
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
        .header("User-Agent", "moltis-plugins")
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

    tracing::info!(%owner, %repo, "installed plugin repo via HTTP tarball");
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
        .header("User-Agent", "moltis-plugins")
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

/// Parse `owner/repo` from a source string.
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
            "invalid plugin source '{}': expected 'owner/repo' or GitHub URL",
            source
        );
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_valid() {
        let (owner, repo) = parse_source("anthropics/claude-plugins-official").unwrap();
        assert_eq!(owner, "anthropics");
        assert_eq!(repo, "claude-plugins-official");
    }

    #[test]
    fn test_parse_source_github_url() {
        let (o, r) = parse_source("https://github.com/anthropics/claude-plugins-official").unwrap();
        assert_eq!(o, "anthropics");
        assert_eq!(r, "claude-plugins-official");
    }

    #[test]
    fn test_parse_source_invalid() {
        assert!(parse_source("noslash").is_err());
        assert!(parse_source("too/many/parts").is_err());
    }

    #[test]
    fn test_sanitize_archive_path_rejects_parent_dir() {
        let path = Path::new("repo-root/../../etc/passwd");
        assert!(sanitize_archive_path(path).is_err());
    }

    #[test]
    fn test_sanitize_archive_path_accepts_normal_path() {
        let path = Path::new("repo-root/plugins/demo.md");
        let sanitized = sanitize_archive_path(path).unwrap().unwrap();
        assert_eq!(sanitized, PathBuf::from("plugins/demo.md"));
    }

    #[test]
    fn test_default_plugins_dir() {
        let dir = default_plugins_dir().unwrap();
        assert!(dir.to_string_lossy().contains("installed-plugins"));
    }

    #[test]
    fn test_default_manifest_path() {
        let path = default_manifest_path().unwrap();
        assert!(path.to_string_lossy().contains("plugins-manifest.json"));
    }
}
