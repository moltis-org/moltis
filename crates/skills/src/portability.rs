use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

use {
    flate2::{Compression, read::GzDecoder, write::GzEncoder},
    tar::{Archive, Builder, Header},
    walkdir::WalkDir,
};

use crate::{
    error::{Error, Result},
    formats::{PluginFormat, detect_format, scan_with_adapter},
    install::scan_repo_skills,
    manifest::ManifestStore,
    types::{RepoEntry, RepoProvenance, SkillMetadata, SkillState},
};

const BUNDLE_MANIFEST_PATH: &str = "bundle.json";
const BUNDLE_REPO_PREFIX: &str = "repo";
const BUNDLE_VERSION: u32 = 1;
const IMPORT_QUARANTINE_REASON: &str =
    "Imported from a portable bundle, review and clear quarantine before enabling";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PortableRepoBundle {
    version: u32,
    exported_at_ms: u64,
    repo: RepoEntry,
}

#[derive(Debug, Clone)]
pub struct ExportedRepoBundle {
    pub bundle_path: PathBuf,
    pub repo: RepoEntry,
}

#[derive(Debug, Clone)]
pub struct ImportedRepoBundle {
    pub bundle_path: PathBuf,
    pub source: String,
    pub repo_name: String,
    pub format: PluginFormat,
    pub skills: Vec<SkillMetadata>,
}

pub fn default_export_dir() -> Result<PathBuf> {
    Ok(moltis_config::data_dir().join("skill-exports"))
}

pub async fn export_repo_bundle(
    source: &str,
    install_dir: &Path,
    output_path: Option<&Path>,
) -> Result<ExportedRepoBundle> {
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    export_repo_bundle_with_store(source, install_dir, output_path, &store).await
}

pub async fn import_repo_bundle(
    bundle_path: &Path,
    install_dir: &Path,
) -> Result<ImportedRepoBundle> {
    let manifest_path = ManifestStore::default_path()?;
    let store = ManifestStore::new(manifest_path);
    import_repo_bundle_with_store(bundle_path, install_dir, &store).await
}

pub async fn export_repo_bundle_with_store(
    source: &str,
    install_dir: &Path,
    output_path: Option<&Path>,
    store: &ManifestStore,
) -> Result<ExportedRepoBundle> {
    let manifest = store.load()?;
    let repo = manifest
        .find_repo(source)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("repo '{source}' not found")))?;
    let repo_dir = install_dir.join(&repo.repo_name);
    if !repo_dir.is_dir() {
        return Err(Error::NotFound(format!(
            "repo directory missing on disk: {}",
            repo_dir.display()
        )));
    }

    let bundle_path = resolve_bundle_output_path(output_path, &repo.repo_name)?;
    if let Some(parent) = bundle_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let bundle = PortableRepoBundle {
        version: BUNDLE_VERSION,
        exported_at_ms: current_time_ms(),
        repo: repo.clone(),
    };
    let bundle_path_clone = bundle_path.clone();
    let repo_dir_clone = repo_dir.clone();
    tokio::task::spawn_blocking(move || {
        write_bundle_archive(&bundle, &repo_dir_clone, &bundle_path_clone)
    })
    .await??;

    Ok(ExportedRepoBundle { bundle_path, repo })
}

pub async fn import_repo_bundle_with_store(
    bundle_path: &Path,
    install_dir: &Path,
    store: &ManifestStore,
) -> Result<ImportedRepoBundle> {
    tokio::fs::create_dir_all(install_dir).await?;

    let bundle_path = bundle_path.to_path_buf();
    let manifest_bundle = {
        let bundle_path = bundle_path.clone();
        tokio::task::spawn_blocking(move || read_bundle_manifest(&bundle_path)).await??
    };

    let mut manifest = store.load()?;
    let source = unique_source(&manifest, &manifest_bundle.repo.source);
    let repo_name = unique_repo_name(&manifest, install_dir, &manifest_bundle.repo.repo_name);
    let original_source = manifest_bundle.repo.source.clone();
    let original_commit_sha = manifest_bundle.repo.commit_sha.clone();
    let exported_at_ms = manifest_bundle.exported_at_ms;
    let repo_dir = install_dir.join(&repo_name);

    {
        let bundle_path = bundle_path.clone();
        let repo_dir = repo_dir.clone();
        tokio::task::spawn_blocking(move || extract_bundle_archive(&bundle_path, &repo_dir))
            .await??;
    }

    let (format, skills_meta, skill_states) = scan_imported_repo(&repo_dir, install_dir).await?;
    if skills_meta.is_empty() {
        let _ = tokio::fs::remove_dir_all(&repo_dir).await;
        return Err(Error::Bundle(format!(
            "imported bundle '{}' contains no usable skills",
            bundle_path.display()
        )));
    }

    let mut repo = manifest_bundle.repo;
    repo.source = source.clone();
    repo.repo_name = repo_name.clone();
    repo.installed_at_ms = current_time_ms();
    repo.format = format;
    repo.quarantined = true;
    repo.quarantine_reason = Some(IMPORT_QUARANTINE_REASON.into());
    repo.provenance = Some(RepoProvenance {
        original_source,
        original_commit_sha,
        imported_from: Some(bundle_path.display().to_string()),
        exported_at_ms: Some(exported_at_ms),
    });
    repo.skills = skill_states
        .into_iter()
        .map(|skill| SkillState {
            trusted: false,
            enabled: false,
            ..skill
        })
        .collect();

    manifest.add_repo(repo);
    store.save(&manifest)?;

    Ok(ImportedRepoBundle {
        bundle_path,
        source,
        repo_name,
        format,
        skills: skills_meta,
    })
}

async fn scan_imported_repo(
    repo_dir: &Path,
    install_dir: &Path,
) -> Result<(PluginFormat, Vec<SkillMetadata>, Vec<SkillState>)> {
    let format = detect_format(repo_dir);
    let (skills_meta, skill_states) = match format {
        PluginFormat::Skill => scan_repo_skills(repo_dir, install_dir).await?,
        _ => match scan_with_adapter(repo_dir, format) {
            Some(result) => {
                let entries = result?;
                let relative = repo_dir
                    .strip_prefix(install_dir)
                    .unwrap_or(repo_dir)
                    .to_string_lossy()
                    .to_string();
                let meta: Vec<SkillMetadata> =
                    entries.iter().map(|entry| entry.metadata.clone()).collect();
                let states: Vec<SkillState> = entries
                    .into_iter()
                    .map(|entry| SkillState {
                        name: entry.metadata.name,
                        relative_path: relative.clone(),
                        trusted: false,
                        enabled: false,
                    })
                    .collect();
                (meta, states)
            },
            None => {
                return Err(Error::Bundle(format!(
                    "no adapter available for imported repo format '{}'",
                    format
                )));
            },
        },
    };

    Ok((format, skills_meta, skill_states))
}

fn resolve_bundle_output_path(output_path: Option<&Path>, repo_name: &str) -> Result<PathBuf> {
    let default_name = format!("{repo_name}-{}.tar.gz", current_time_ms());
    let path = match output_path {
        Some(path) if path.is_dir() => path.join(default_name),
        Some(path) => path.to_path_buf(),
        None => default_export_dir()?.join(default_name),
    };
    Ok(path)
}

fn write_bundle_archive(
    bundle: &PortableRepoBundle,
    repo_dir: &Path,
    bundle_path: &Path,
) -> Result<()> {
    let file = File::create(bundle_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    let manifest_json = serde_json::to_vec_pretty(bundle)?;
    let mut header = Header::new_gnu();
    header.set_size(u64::try_from(manifest_json.len()).unwrap_or(u64::MAX));
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, BUNDLE_MANIFEST_PATH, manifest_json.as_slice())?;

    for entry in WalkDir::new(repo_dir).min_depth(1).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }

        let relative = path
            .strip_prefix(repo_dir)
            .map_err(|_| Error::Bundle(format!("failed to relativize {}", path.display())))?;
        let archive_path = Path::new(BUNDLE_REPO_PREFIX).join(relative);
        if metadata.is_dir() {
            builder.append_dir(&archive_path, path)?;
            continue;
        }
        if metadata.is_file() {
            builder.append_path_with_name(path, &archive_path)?;
        }
    }

    builder.finish()?;
    Ok(())
}

fn read_bundle_manifest(bundle_path: &Path) -> Result<PortableRepoBundle> {
    let file = File::open(bundle_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path == Path::new(BUNDLE_MANIFEST_PATH) {
            let mut data = String::new();
            entry.read_to_string(&mut data)?;
            let bundle: PortableRepoBundle = serde_json::from_str(&data)?;
            if bundle.version != BUNDLE_VERSION {
                return Err(Error::Bundle(format!(
                    "unsupported skill bundle version {} (expected {})",
                    bundle.version, BUNDLE_VERSION
                )));
            }
            return Ok(bundle);
        }
    }

    Err(Error::Bundle(format!(
        "bundle '{}' is missing {}",
        bundle_path.display(),
        BUNDLE_MANIFEST_PATH
    )))
}

fn extract_bundle_archive(bundle_path: &Path, target_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(target_dir)?;
    let canonical_target = std::fs::canonicalize(target_dir)?;

    let file = File::open(bundle_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(Error::Bundle(format!(
                "bundle '{}' contains unsupported link entries",
                bundle_path.display()
            )));
        }

        let path = entry.path()?.into_owned();
        if path == Path::new(BUNDLE_MANIFEST_PATH) {
            continue;
        }
        let Some(relative) = sanitize_bundle_repo_path(&path)? else {
            continue;
        };
        let dest = target_dir.join(relative);

        if entry_type.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
            let canonical_parent = std::fs::canonicalize(parent)?;
            if !canonical_parent.starts_with(&canonical_target) {
                return Err(Error::Bundle(
                    "bundle entry escaped import directory".into(),
                ));
            }
        }

        if dest.exists() {
            let metadata = std::fs::symlink_metadata(&dest)?;
            if metadata.file_type().is_symlink() {
                return Err(Error::Bundle(
                    "bundle entry resolves to a symlink destination".into(),
                ));
            }
        }

        entry.unpack(&dest)?;
    }

    Ok(())
}

fn sanitize_bundle_repo_path(path: &Path) -> Result<Option<PathBuf>> {
    let mut components = path.components();
    let Some(Component::Normal(prefix)) = components.next() else {
        return Err(Error::Bundle(format!(
            "bundle contains invalid path '{}'",
            path.display()
        )));
    };
    if prefix != BUNDLE_REPO_PREFIX {
        return Ok(None);
    }

    let stripped: PathBuf = components.collect();
    if stripped.as_os_str().is_empty() {
        return Ok(None);
    }

    for component in stripped.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {},
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::Bundle(format!(
                    "bundle contains unsafe path '{}'",
                    path.display()
                )));
            },
        }
    }

    Ok(Some(stripped))
}

fn unique_source(manifest: &crate::types::SkillsManifest, base: &str) -> String {
    if manifest.find_repo(base).is_none() {
        return base.to_string();
    }

    let mut index = 2_u32;
    loop {
        let candidate = format!("{base}#imported-{index}");
        if manifest.find_repo(&candidate).is_none() {
            return candidate;
        }
        index += 1;
    }
}

fn unique_repo_name(
    manifest: &crate::types::SkillsManifest,
    install_dir: &Path,
    base: &str,
) -> String {
    if manifest.repos.iter().all(|repo| repo.repo_name != base) && !install_dir.join(base).exists()
    {
        return base.to_string();
    }

    let mut index = 2_u32;
    loop {
        let candidate = format!("{base}-imported-{index}");
        if manifest
            .repos
            .iter()
            .all(|repo| repo.repo_name != candidate)
            && !install_dir.join(&candidate).exists()
        {
            return candidate;
        }
        index += 1;
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            manifest::ManifestStore,
            types::{RepoEntry, SkillSource, SkillsManifest},
        },
    };

    #[test]
    fn sanitize_bundle_repo_path_rejects_escape() {
        let path = Path::new("repo/../../etc/passwd");
        assert!(sanitize_bundle_repo_path(path).is_err());
    }

    #[test]
    fn sanitize_bundle_repo_path_accepts_repo_relative_path() {
        let path = Path::new("repo/skills/demo/SKILL.md");
        let relative = sanitize_bundle_repo_path(path).unwrap().unwrap();
        assert_eq!(relative, PathBuf::from("skills/demo/SKILL.md"));
    }

    #[tokio::test]
    async fn export_import_roundtrip_marks_repo_quarantined() {
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("installed-skills");
        let export_dir = tmp.path().join("exports");
        let manifest_path = tmp.path().join("skills-manifest.json");
        std::fs::create_dir_all(&install_dir).unwrap();

        let repo_dir = install_dir.join("demo-repo");
        std::fs::create_dir_all(repo_dir.join("skills/demo")).unwrap();
        std::fs::write(
            repo_dir.join("skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: test\n---\nbody\n",
        )
        .unwrap();

        let store = ManifestStore::new(manifest_path);
        let mut manifest = SkillsManifest::default();
        manifest.add_repo(RepoEntry {
            source: "owner/demo".into(),
            repo_name: "demo-repo".into(),
            installed_at_ms: 1,
            commit_sha: Some("abc123".into()),
            format: PluginFormat::Skill,
            quarantined: false,
            quarantine_reason: None,
            provenance: None,
            skills: vec![SkillState {
                name: "demo".into(),
                relative_path: "demo-repo/skills/demo".into(),
                trusted: true,
                enabled: true,
            }],
        });
        store.save(&manifest).unwrap();

        let exported =
            export_repo_bundle_with_store("owner/demo", &install_dir, Some(&export_dir), &store)
                .await
                .unwrap();
        assert!(exported.bundle_path.exists());

        let imported_install_dir = tmp.path().join("imported-skills");
        let imported_store = ManifestStore::new(tmp.path().join("imported-manifest.json"));
        let imported = import_repo_bundle_with_store(
            &exported.bundle_path,
            &imported_install_dir,
            &imported_store,
        )
        .await
        .unwrap();

        assert_eq!(imported.skills.len(), 1);
        assert_eq!(imported.skills[0].source, Some(SkillSource::Registry));

        let imported_manifest = imported_store.load().unwrap();
        let repo = imported_manifest.find_repo(&imported.source).unwrap();
        assert!(repo.quarantined);
        assert_eq!(
            repo.quarantine_reason.as_deref(),
            Some(IMPORT_QUARANTINE_REASON)
        );
        assert!(
            repo.skills
                .iter()
                .all(|skill| !skill.trusted && !skill.enabled)
        );
        assert_eq!(
            repo.provenance
                .as_ref()
                .map(|provenance| provenance.original_source.as_str()),
            Some("owner/demo")
        );
    }
}
