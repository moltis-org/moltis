use {
    serde_json::Value,
    std::{collections::HashSet, path::Path},
};

// Pull in ServiceResult, ServiceError, security_audit, etc. from services module
use super::super::*;

pub(super) fn local_repo_head_sha(repo_dir: &Path) -> Option<String> {
    let repo = gix::open(repo_dir).ok()?;
    let obj = repo.rev_parse_single("HEAD").ok()?;
    Some(obj.detach().to_hex().to_string())
}

pub(super) fn detect_and_mark_repo_drift(
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
            tracing::warn!(
                source = %repo.source,
                expected_sha = %expected_sha,
                current_sha = %current_sha,
                "detect_and_mark_repo_drift: drift detected, resetting all skills"
            );
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
pub(super) fn delete_discovered_skill(source_type: &str, params: &Value) -> ServiceResult {
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    if is_protected_discovered_skill(skill_name) {
        return Err(ServiceError::forbidden(format!(
            "skill '{skill_name}' is protected and cannot be deleted from the UI"
        )));
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
pub(super) fn skill_detail_discovered(source_type: &str, skill_name: &str) -> ServiceResult {
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

/// Load skill detail for a bundled skill by name.
#[cfg(feature = "bundled-skills")]
pub(super) fn skill_detail_bundled(skill_name: &str) -> ServiceResult {
    use moltis_skills::requirements::check_requirements;

    let store = moltis_skills::bundled::BundledSkillStore::new();
    let skills = store.discover();
    let meta = skills
        .iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| format!("bundled skill '{skill_name}' not found"))?;

    let body = store
        .read_skill(skill_name)
        .ok_or_else(|| format!("bundled skill '{skill_name}' body not readable"))?;

    let elig = check_requirements(meta);

    let config = moltis_config::discover_and_load();
    let enabled = meta.category.as_deref().is_none_or(|cat| {
        !config
            .skills
            .disabled_bundled_categories
            .iter()
            .any(|c| c == cat)
    });

    Ok(serde_json::json!({
        "name": meta.name,
        "description": meta.description,
        "category": meta.category,
        "license": meta.license,
        "compatibility": meta.compatibility,
        "allowed_tools": meta.allowed_tools,
        "requires": meta.requires,
        "origin": meta.origin,
        "eligible": elig.eligible,
        "missing_bins": elig.missing_bins,
        "install_options": elig.install_options,
        "trusted": true,
        "enabled": enabled,
        "protected": true,
        "body": body,
        "body_html": markdown_to_html(&body),
        "source": "bundled",
    }))
}

/// Toggle a bundled skill by adding/removing its category from
/// `disabled_bundled_categories` in config. Bundled skills are not tracked
/// in the manifest, so `toggle_skill()` cannot handle them.
pub(super) fn toggle_bundled_skill(params: &Value, enabled: bool) -> ServiceResult {
    let skill_name = params
        .get("skill")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'skill' parameter".to_string())?;

    // Find the category for this bundled skill.
    #[cfg(feature = "bundled-skills")]
    {
        let store = moltis_skills::bundled::BundledSkillStore::new();
        let skills = store.discover();
        let skill = skills
            .iter()
            .find(|s| s.name == skill_name)
            .ok_or_else(|| format!("bundled skill '{skill_name}' not found"))?;
        let category = skill
            .category
            .clone()
            .ok_or_else(|| format!("bundled skill '{skill_name}' has no category"))?;

        let cat_clone = category.clone();
        if let Err(e) = moltis_config::update_config(|cfg| {
            if enabled {
                cfg.skills
                    .disabled_bundled_categories
                    .retain(|c| c != &cat_clone);
            } else if !cfg
                .skills
                .disabled_bundled_categories
                .iter()
                .any(|c| c == &cat_clone)
            {
                cfg.skills.disabled_bundled_categories.push(cat_clone);
            }
        }) {
            return Err(format!("failed to save config: {e}").into());
        }

        security_audit(
            "skills.skill.toggle",
            serde_json::json!({
                "source": "bundled",
                "skill": skill_name,
                "category": category,
                "enabled": enabled,
            }),
        );

        Ok(
            serde_json::json!({ "source": "bundled", "skill": skill_name, "category": category, "enabled": enabled }),
        )
    }
    #[cfg(not(feature = "bundled-skills"))]
    {
        let _ = (skill_name, enabled);
        Err("bundled skills are not available in this build".into())
    }
}

pub(super) fn toggle_skill(params: &Value, enabled: bool) -> ServiceResult {
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

    let mut auto_trusted = false;
    if enabled {
        let quarantined = manifest
            .find_repo(source)
            .map(|repo| repo.quarantined)
            .ok_or_else(|| format!("repo '{source}' not found"))?;
        if quarantined {
            return Err(format!(
                "repo '{source}' is quarantined. Review it and run skills.repos.unquarantine before enabling"
            )
            .into());
        }

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
            if !manifest.set_skill_trusted(source, skill_name, true) {
                return Err(format!("skill '{skill_name}' not found in repo '{source}'").into());
            }
            auto_trusted = true;
        }
    }

    if !manifest.set_skill_enabled(source, skill_name, enabled) {
        return Err(format!("skill '{skill_name}' not found in repo '{source}'").into());
    }
    store.save(&manifest).map_err(ServiceError::message)?;

    if auto_trusted {
        security_audit(
            "skills.skill.trust",
            serde_json::json!({
                "source": source,
                "skill": skill_name,
                "trusted": true,
                "auto": true,
            }),
        );
    }
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

pub(super) fn set_skill_trusted(params: &Value, trusted: bool) -> ServiceResult {
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
