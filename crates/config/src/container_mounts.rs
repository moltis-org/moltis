//! Host path detection for containers launched from inside containers.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContainerMount {
    source: PathBuf,
    destination: PathBuf,
}

fn read_trimmed_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|contents| contents.trim().to_string())
        .filter(|contents| !contents.is_empty())
}

fn normalize_cgroup_container_ref(segment: &str) -> Option<String> {
    let mut value = segment.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(stripped) = value.strip_suffix(".scope") {
        value = stripped;
    }
    for prefix in ["docker-", "libpod-", "cri-containerd-"] {
        if let Some(stripped) = value.strip_prefix(prefix) {
            value = stripped;
            break;
        }
    }
    if value.len() < 12 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(value.to_string())
}

pub fn current_container_references() -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for candidate in [
        std::env::var("HOSTNAME").ok(),
        read_trimmed_file("/etc/hostname"),
    ]
    .into_iter()
    .flatten()
    {
        if seen.insert(candidate.clone()) {
            refs.push(candidate);
        }
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/self/cgroup") {
        for candidate in cgroup
            .lines()
            .flat_map(|line| line.split(['/', ':']))
            .filter_map(normalize_cgroup_container_ref)
        {
            if seen.insert(candidate.clone()) {
                refs.push(candidate);
            }
        }
    }
    refs
}

#[must_use]
fn parse_container_mounts_from_inspect(stdout: &str) -> Vec<ContainerMount> {
    let Ok(json): Result<serde_json::Value, _> = serde_json::from_str(stdout) else {
        return Vec::new();
    };
    let root = json
        .as_array()
        .and_then(|entries| entries.first())
        .unwrap_or(&json);
    root.get("Mounts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let source = entry.get("Source")?.as_str()?.trim();
            let destination = entry.get("Destination")?.as_str()?.trim();
            if source.is_empty() || destination.is_empty() {
                return None;
            }
            Some(ContainerMount {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
            })
        })
        .collect()
}

#[must_use]
fn resolve_host_path_from_mounts(guest_path: &Path, mounts: &[ContainerMount]) -> Option<PathBuf> {
    mounts
        .iter()
        .filter_map(|mount| {
            let relative = guest_path.strip_prefix(&mount.destination).ok()?;
            Some((
                mount.destination.components().count(),
                if relative.as_os_str().is_empty() {
                    mount.source.clone()
                } else {
                    mount.source.join(relative)
                },
            ))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, resolved)| resolved)
}

#[must_use]
fn inspect_container_mounts(cli: &str, reference: &str) -> Vec<ContainerMount> {
    let output = match Command::new(cli).args(["inspect", reference]).output() {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                cli,
                reference,
                stderr = %stderr.trim(),
                "container inspect failed while auto-detecting host data dir"
            );
            return Vec::new();
        },
        Err(error) => {
            debug!(
                cli,
                reference,
                %error,
                "could not inspect container while auto-detecting host data dir"
            );
            return Vec::new();
        },
    };
    parse_container_mounts_from_inspect(&String::from_utf8_lossy(&output.stdout))
}

#[must_use]
fn running_container_references(cli: &str) -> Vec<String> {
    let output = match Command::new(cli).args(["ps", "-q", "--no-trunc"]).output() {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                cli,
                stderr = %stderr.trim(),
                "container list failed while auto-detecting host data dir"
            );
            return Vec::new();
        },
        Err(error) => {
            debug!(
                cli,
                %error,
                "could not list containers while auto-detecting host data dir"
            );
            return Vec::new();
        },
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[must_use]
fn detect_host_data_dir_from_mount_sets<I>(guest_data_dir: &Path, mount_sets: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = Vec<ContainerMount>>,
{
    let mut detected: Option<PathBuf> = None;
    for mounts in mount_sets {
        if mounts.is_empty() {
            continue;
        }
        let Some(resolved) = resolve_host_path_from_mounts(guest_data_dir, &mounts) else {
            continue;
        };
        if let Some(existing) = &detected
            && existing != &resolved
        {
            debug!(
                guest_path = %guest_data_dir.display(),
                first_host_path = %existing.display(),
                other_host_path = %resolved.display(),
                "ambiguous host data dir from container mounts"
            );
            return None;
        }
        detected = Some(resolved);
    }
    detected
}

#[must_use]
pub fn detect_host_data_dir_with_references(
    cli: &str,
    guest_data_dir: &Path,
    references: &[String],
) -> Option<PathBuf> {
    let current_mount_sets = references
        .iter()
        .map(|reference| inspect_container_mounts(cli, reference));
    if let Some(resolved) = detect_host_data_dir_from_mount_sets(guest_data_dir, current_mount_sets)
    {
        debug!(
            cli,
            guest_path = %guest_data_dir.display(),
            host_path = %resolved.display(),
            "auto-detected host data dir from current container mounts"
        );
        return Some(resolved);
    }

    let running_mount_sets = running_container_references(cli)
        .into_iter()
        .map(|reference| inspect_container_mounts(cli, &reference));
    let resolved = detect_host_data_dir_from_mount_sets(guest_data_dir, running_mount_sets)?;
    debug!(
        cli,
        guest_path = %guest_data_dir.display(),
        host_path = %resolved.display(),
        "auto-detected host data dir by scanning running container mounts"
    );
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_cgroup_container_ref() {
        assert_eq!(
            normalize_cgroup_container_ref(
                "docker-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.scope"
            ),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into())
        );
        assert_eq!(
            normalize_cgroup_container_ref(
                "libpod-abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdef.scope"
            ),
            Some("abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdef".into())
        );
        assert!(normalize_cgroup_container_ref("user.slice").is_none());
    }

    #[test]
    fn parses_container_mounts_from_inspect() {
        let mounts = parse_container_mounts_from_inspect(
            r#"[{
            "Mounts": [
                {"Source": "/host/data", "Destination": "/home/moltis/.moltis"},
                {"Source": "/host/config", "Destination": "/home/moltis/.config/moltis"}
            ]
        }]"#,
        );
        assert_eq!(mounts, vec![
            ContainerMount {
                source: PathBuf::from("/host/data"),
                destination: PathBuf::from("/home/moltis/.moltis"),
            },
            ContainerMount {
                source: PathBuf::from("/host/config"),
                destination: PathBuf::from("/home/moltis/.config/moltis"),
            },
        ]);
    }

    #[test]
    fn resolves_host_path_from_mounts_prefers_longest_prefix() {
        let mounts = vec![
            ContainerMount {
                source: PathBuf::from("/host"),
                destination: PathBuf::from("/home"),
            },
            ContainerMount {
                source: PathBuf::from("/host/data"),
                destination: PathBuf::from("/home/moltis/.moltis"),
            },
        ];
        let resolved = resolve_host_path_from_mounts(
            &PathBuf::from("/home/moltis/.moltis/sandbox/home/shared"),
            &mounts,
        );
        assert_eq!(
            resolved,
            Some(PathBuf::from("/host/data/sandbox/home/shared"))
        );
    }

    #[test]
    fn detects_host_data_dir_from_mount_sets() {
        let guest_data_dir = PathBuf::from("/home/moltis/.moltis");
        let detected =
            detect_host_data_dir_from_mount_sets(&guest_data_dir, [vec![ContainerMount {
                source: PathBuf::from("/home/user/moltis/data"),
                destination: guest_data_dir.clone(),
            }]]);

        assert_eq!(detected, Some(PathBuf::from("/home/user/moltis/data")));
    }

    #[test]
    fn detects_ambiguous_mount_sets() {
        let guest_data_dir = PathBuf::from("/home/moltis/.moltis");
        let detected = detect_host_data_dir_from_mount_sets(&guest_data_dir, [
            vec![ContainerMount {
                source: PathBuf::from("/host/one"),
                destination: guest_data_dir.clone(),
            }],
            vec![ContainerMount {
                source: PathBuf::from("/host/two"),
                destination: guest_data_dir.clone(),
            }],
        ]);

        assert_eq!(detected, None);
    }
}
