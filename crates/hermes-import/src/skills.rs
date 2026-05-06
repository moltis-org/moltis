//! Import skills from Hermes.
//!
//! Hermes skills use the same SKILL.md format as Moltis, so this is a
//! straightforward directory copy.

use std::path::Path;

use moltis_import_core::{report::CategoryReport, skills::copy_skill_dirs};

use crate::detect::HermesDetection;

/// Discover skill directories in the Hermes skills folder.
pub fn discover_skills(detection: &HermesDetection) -> Vec<(String, std::path::PathBuf)> {
    let Some(ref skills_dir) = detection.skills_dir else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|entry| entry.path().is_dir() && entry.path().join("SKILL.md").is_file())
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| (name.to_string(), entry.path()))
        })
        .collect()
}

/// Import skills from Hermes into Moltis.
pub fn import_skills(detection: &HermesDetection, dest_skills_dir: &Path) -> CategoryReport {
    let skills = discover_skills(detection);
    let sources: Vec<(String, &Path)> = skills
        .iter()
        .map(|(name, path)| (name.clone(), path.as_path()))
        .collect();

    copy_skill_dirs(&sources, dest_skills_dir)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, moltis_import_core::report::ImportStatus};

    fn make_detection(home: &Path) -> HermesDetection {
        HermesDetection {
            home_dir: home.to_path_buf(),
            config_path: None,
            env_path: None,
            skills_dir: None,
            soul_path: None,
            agents_path: None,
            memory_path: None,
            user_path: None,
            has_data: true,
        }
    }

    #[test]
    fn discover_skills_finds_valid_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("good-skill")).unwrap();
        std::fs::write(
            skills_dir.join("good-skill").join("SKILL.md"),
            "---\nname: test\n---\nContent",
        )
        .unwrap();
        std::fs::create_dir_all(skills_dir.join("not-a-skill")).unwrap();

        let mut detection = make_detection(tmp.path());
        detection.skills_dir = Some(skills_dir);

        let skills = discover_skills(&detection);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].0, "good-skill");
    }

    #[test]
    fn import_skills_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("my-skill")).unwrap();
        std::fs::write(
            skills_dir.join("my-skill").join("SKILL.md"),
            "---\nname: test\n---\nDo stuff.",
        )
        .unwrap();

        let dest = tmp.path().join("dest-skills");
        let mut detection = make_detection(tmp.path());
        detection.skills_dir = Some(skills_dir);

        let report = import_skills(&detection, &dest);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        assert!(dest.join("my-skill").join("SKILL.md").is_file());
    }

    #[test]
    fn import_skills_no_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let detection = make_detection(tmp.path());
        let report = import_skills(&detection, &tmp.path().join("dest"));
        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
