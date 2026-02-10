//! Binary requirement checking and dependency installation.

use std::path::Path;

use anyhow::{Context, bail};

use crate::types::{InstallKind, InstallSpec, SkillEligibility, SkillMetadata};

/// Resolve install command program + args from an install spec.
pub fn install_program_and_args(spec: &InstallSpec) -> anyhow::Result<(&'static str, Vec<&str>)> {
    let (program, args) = match &spec.kind {
        InstallKind::Brew => {
            let formula = spec
                .formula
                .as_deref()
                .context("brew install requires 'formula'")?;
            ("brew", vec!["install", formula])
        },
        InstallKind::Npm => {
            let package = spec
                .package
                .as_deref()
                .context("npm install requires 'package'")?;
            ("npm", vec!["install", "-g", "--ignore-scripts", package])
        },
        InstallKind::Go => {
            let module = spec
                .module
                .as_deref()
                .context("go install requires 'module'")?;
            ("go", vec!["install", module])
        },
        InstallKind::Cargo => {
            let package = spec
                .package
                .as_deref()
                .context("cargo install requires 'package'")?;
            ("cargo", vec!["install", package])
        },
        InstallKind::Uv => {
            let package = spec
                .package
                .as_deref()
                .context("uv install requires 'package'")?;
            ("uv", vec!["tool", "install", package])
        },
        InstallKind::Download => {
            bail!("download install kind is not yet supported for automatic installation");
        },
    };

    Ok((program, args))
}

/// Render an install spec to a user-visible command preview.
pub fn install_command_preview(spec: &InstallSpec) -> anyhow::Result<String> {
    let (program, args) = install_program_and_args(spec)?;
    Ok(std::iter::once(program)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" "))
}

/// Returns the current OS identifier used for platform filtering.
pub fn current_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

/// Check whether a binary exists in PATH.
pub fn check_bin(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() && is_executable(&candidate) {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

/// Check all requirements for a skill and return eligibility info.
pub fn check_requirements(meta: &SkillMetadata) -> SkillEligibility {
    let req = &meta.requires;

    // If no requirements declared, skill is eligible
    if req.bins.is_empty() && req.any_bins.is_empty() {
        return SkillEligibility {
            eligible: true,
            missing_bins: Vec::new(),
            install_options: Vec::new(),
        };
    }

    let mut missing = Vec::new();

    // All bins must exist
    for bin in &req.bins {
        if !check_bin(bin) {
            missing.push(bin.clone());
        }
    }

    // At least one of any_bins must exist
    if !req.any_bins.is_empty() && !req.any_bins.iter().any(|b| check_bin(b)) {
        // All are missing — report all of them
        for bin in &req.any_bins {
            missing.push(bin.clone());
        }
    }

    let os = current_os();
    let install_options: Vec<InstallSpec> = req
        .install
        .iter()
        .filter(|spec| spec.os.is_empty() || spec.os.iter().any(|o| o == os))
        .cloned()
        .collect();

    SkillEligibility {
        eligible: missing.is_empty(),
        missing_bins: missing,
        install_options,
    }
}

/// Result of running an install command.
#[derive(Debug)]
pub struct InstallResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run an install spec command (e.g. `brew install <formula>`).
pub async fn run_install(spec: &InstallSpec) -> anyhow::Result<InstallResult> {
    let (program, args) = install_program_and_args(spec)?;

    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let output = tokio::process::Command::new(program)
        .args(&args_owned)
        .output()
        .await
        .with_context(|| format!("failed to run {program}"))?;

    Ok(InstallResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::types::SkillRequirements};

    #[test]
    fn test_current_os() {
        let os = current_os();
        assert!(["darwin", "linux", "windows", "unknown"].contains(&os));
    }

    #[test]
    fn test_check_bin_exists() {
        // `ls` should exist on any unix system
        #[cfg(unix)]
        assert!(check_bin("ls"));
    }

    #[test]
    fn test_check_bin_missing() {
        assert!(!check_bin("__nonexistent_binary_xyz__"));
    }

    #[test]
    fn test_no_requirements_is_eligible() {
        let meta = SkillMetadata {
            name: "test".into(),
            description: String::new(),
            homepage: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            dockerfile: None,
            requires: SkillRequirements::default(),
            path: Default::default(),
            source: None,
        };
        let elig = check_requirements(&meta);
        assert!(elig.eligible);
        assert!(elig.missing_bins.is_empty());
    }

    #[test]
    fn test_missing_bin_is_blocked() {
        let meta = SkillMetadata {
            name: "test".into(),
            description: String::new(),
            homepage: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            dockerfile: None,
            requires: SkillRequirements {
                bins: vec!["__nonexistent_binary_xyz__".into()],
                any_bins: Vec::new(),
                install: Vec::new(),
            },
            path: Default::default(),
            source: None,
        };
        let elig = check_requirements(&meta);
        assert!(!elig.eligible);
        assert_eq!(elig.missing_bins, vec!["__nonexistent_binary_xyz__"]);
    }

    #[test]
    fn test_any_bins_one_present() {
        let meta = SkillMetadata {
            name: "test".into(),
            description: String::new(),
            homepage: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            dockerfile: None,
            requires: SkillRequirements {
                bins: Vec::new(),
                any_bins: vec!["ls".into(), "__nonexistent__".into()],
                install: Vec::new(),
            },
            path: Default::default(),
            source: None,
        };
        #[cfg(unix)]
        {
            let elig = check_requirements(&meta);
            assert!(elig.eligible);
        }
    }

    #[test]
    fn test_any_bins_none_present() {
        let meta = SkillMetadata {
            name: "test".into(),
            description: String::new(),
            homepage: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            dockerfile: None,
            requires: SkillRequirements {
                bins: Vec::new(),
                any_bins: vec!["__nope1__".into(), "__nope2__".into()],
                install: Vec::new(),
            },
            path: Default::default(),
            source: None,
        };
        let elig = check_requirements(&meta);
        assert!(!elig.eligible);
        assert_eq!(elig.missing_bins.len(), 2);
    }

    #[test]
    fn test_install_options_filtered_by_os() {
        let meta = SkillMetadata {
            name: "test".into(),
            description: String::new(),
            homepage: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            dockerfile: None,
            requires: SkillRequirements {
                bins: vec!["__missing__".into()],
                any_bins: Vec::new(),
                install: vec![
                    InstallSpec {
                        kind: InstallKind::Brew,
                        formula: Some("test".into()),
                        package: None,
                        module: None,
                        url: None,
                        bins: vec!["__missing__".into()],
                        os: vec!["darwin".into()],
                        label: None,
                    },
                    InstallSpec {
                        kind: InstallKind::Npm,
                        formula: None,
                        package: Some("test".into()),
                        module: None,
                        url: None,
                        bins: vec!["__missing__".into()],
                        os: vec!["linux".into()],
                        label: None,
                    },
                    InstallSpec {
                        kind: InstallKind::Cargo,
                        formula: None,
                        package: Some("test".into()),
                        module: None,
                        url: None,
                        bins: vec!["__missing__".into()],
                        os: Vec::new(), // all platforms
                        label: None,
                    },
                ],
            },
            path: Default::default(),
            source: None,
        };
        let elig = check_requirements(&meta);
        assert!(!elig.eligible);
        // Should include the cargo (all platforms) + the one matching current OS
        let os = current_os();
        for opt in &elig.install_options {
            assert!(opt.os.is_empty() || opt.os.contains(&os.to_string()));
        }
        // At minimum the cargo one (os=[]) should always be present
        assert!(
            elig.install_options
                .iter()
                .any(|o| o.kind == InstallKind::Cargo)
        );
    }

    #[test]
    fn test_install_command_preview() {
        let spec = InstallSpec {
            kind: InstallKind::Cargo,
            formula: None,
            package: Some("ripgrep".into()),
            module: None,
            url: None,
            bins: vec!["rg".into()],
            os: Vec::new(),
            label: None,
        };

        let preview = install_command_preview(&spec).unwrap();
        assert_eq!(preview, "cargo install ripgrep");
    }

    #[test]
    fn test_npm_install_includes_ignore_scripts() {
        let spec = InstallSpec {
            kind: InstallKind::Npm,
            formula: None,
            package: Some("@anthropic/qmd".into()),
            module: None,
            url: None,
            bins: vec!["qmd".into()],
            os: Vec::new(),
            label: None,
        };

        let (program, args) = install_program_and_args(&spec).unwrap();
        assert_eq!(program, "npm");
        assert!(
            args.contains(&"--ignore-scripts"),
            "npm install must include --ignore-scripts to prevent supply chain attacks"
        );

        let preview = install_command_preview(&spec).unwrap();
        assert_eq!(preview, "npm install -g --ignore-scripts @anthropic/qmd");
    }
}
