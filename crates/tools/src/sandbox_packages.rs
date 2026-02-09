//! `sandbox_packages` tool — lists packages pre-installed in the sandbox
//! container, grouped by category.
//!
//! The LLM calls this tool before running commands that need specific tools
//! (image processing, audio/video, document conversion, GIS, etc.) to check
//! what's available, instead of bloating every system prompt with the full
//! package list.
//!
//! **Hybrid mode**: the tool always returns the configured package list
//! (categorized).  When a sandbox container is already running for the
//! current session it also queries `dpkg-query` inside the container and
//! reports any extra packages that were installed at runtime.

use std::{collections::HashSet, sync::Arc};

use {
    anyhow::Result,
    async_trait::async_trait,
    serde_json::{Value, json},
    tracing::debug,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram};

use moltis_agents::tool_registry::AgentTool;

use crate::{exec::ExecOpts, sandbox::SandboxRouter};

// ── Category mapping ────────────────────────────────────────────────────────

/// Static mapping of package names to categories.
///
/// Order matters: first match wins. Packages not matching any entry end up
/// in "Other". Library/dev/font packages are filtered out before
/// categorization (see [`is_infrastructure_package`]).
const CATEGORY_MAP: &[(&str, &[&str])] = &[
    ("Networking", &[
        "curl",
        "wget",
        "ca-certificates",
        "dnsutils",
        "netcat-openbsd",
        "openssh-client",
        "iproute2",
        "net-tools",
    ]),
    ("Languages", &[
        "python3",
        "python3-pip",
        "python3-venv",
        "python-is-python3",
        "nodejs",
        "npm",
        "ruby",
    ]),
    ("Build tools", &[
        "build-essential",
        "clang",
        "pkg-config",
        "autoconf",
        "automake",
        "libtool",
        "bison",
        "flex",
        "dpkg-dev",
        "fakeroot",
    ]),
    ("Compression", &[
        "zip",
        "unzip",
        "bzip2",
        "xz-utils",
        "p7zip-full",
        "tar",
        "zstd",
        "lz4",
        "pigz",
    ]),
    ("CLI utilities", &[
        "git",
        "gnupg2",
        "jq",
        "rsync",
        "file",
        "tree",
        "sqlite3",
        "sudo",
        "locales",
        "tzdata",
        "shellcheck",
        "patchelf",
        "tmux",
    ]),
    ("Text processing", &["ripgrep", "fd-find", "yq"]),
    ("Browser automation", &["chromium"]),
    ("Image processing", &[
        "imagemagick",
        "graphicsmagick",
        "libvips-tools",
        "pngquant",
        "optipng",
        "jpegoptim",
        "webp",
        "libimage-exiftool-perl",
    ]),
    ("Audio/video", &[
        "ffmpeg",
        "sox",
        "lame",
        "flac",
        "vorbis-tools",
        "opus-tools",
        "mediainfo",
    ]),
    ("Documents", &[
        "pandoc",
        "poppler-utils",
        "ghostscript",
        "texlive-latex-base",
        "texlive-latex-extra",
        "texlive-fonts-recommended",
        "antiword",
        "catdoc",
        "unrtf",
        "libreoffice-core",
        "libreoffice-writer",
    ]),
    ("Data processing", &[
        "csvtool",
        "xmlstarlet",
        "html2text",
        "dos2unix",
        "miller",
        "datamash",
    ]),
    ("GIS/maps", &[
        "gdal-bin",
        "mapnik-utils",
        "osm2pgsql",
        "osmium-tool",
        "osmctools",
        "python3-mapnik",
    ]),
    ("CalDAV/CardDAV", &["vdirsyncer", "khal", "python3-caldav"]),
    ("Email", &[
        "isync",
        "offlineimap3",
        "notmuch",
        "notmuch-mutt",
        "aerc",
        "mutt",
        "neomutt",
    ]),
    ("Newsgroups (NNTP)", &["tin", "slrn"]),
    ("Messaging APIs", &["python3-discord"]),
];

/// Returns `true` for packages that are infrastructure/library deps and should
/// be hidden from the LLM (they're not directly useful as CLI tools).
fn is_infrastructure_package(pkg: &str) -> bool {
    // lib*-dev, lib* (shared libs), *-dev (header packages), fonts-*
    pkg.starts_with("lib")
        || pkg.ends_with("-dev")
        || pkg.starts_with("fonts-")
        // Python dev package
        || pkg == "python3-dev"
        // Ruby dev package
        || pkg == "ruby-dev"
        // LLVM dev
        || pkg == "llvm-dev"
        // Browser automation support libs (not the browser itself)
        || pkg.starts_with("libx")
        || pkg.starts_with("libn")
        || pkg.starts_with("liba")
}

/// Well-known base OS packages that are always present and not useful to list.
const BASE_OS_PACKAGES: &[&str] = &[
    "adduser",
    "apt",
    "base-files",
    "base-passwd",
    "bash",
    "coreutils",
    "dash",
    "debconf",
    "debianutils",
    "diffutils",
    "dpkg",
    "e2fsprogs",
    "findutils",
    "grep",
    "gzip",
    "hostname",
    "init-system-helpers",
    "login",
    "mawk",
    "mount",
    "ncurses-base",
    "ncurses-bin",
    "passwd",
    "perl-base",
    "procps",
    "sed",
    "sensible-utils",
    "sysvinit-utils",
    "tar",
    "ubuntu-keyring",
    "util-linux",
];

/// Categorize a list of packages, filtering out infrastructure deps.
///
/// Returns a sorted list of `(category, packages)` pairs. Packages not
/// matching any known category are grouped under "Other".
fn categorize_packages(packages: &[String]) -> Vec<(&'static str, Vec<&str>)> {
    use std::collections::BTreeMap;

    let mut categories: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for pkg in packages {
        if is_infrastructure_package(pkg) {
            continue;
        }

        let category = CATEGORY_MAP
            .iter()
            .find_map(|(cat, members)| {
                if members.contains(&pkg.as_str()) {
                    Some(*cat)
                } else {
                    None
                }
            })
            .unwrap_or("Other");

        categories.entry(category).or_default().push(pkg);
    }

    categories.into_iter().collect()
}

/// Query the sandbox container for installed packages via `dpkg-query`.
///
/// Returns `None` if the container is not reachable (not running, etc.).
async fn query_sandbox_packages(router: &SandboxRouter, session_key: &str) -> Option<Vec<String>> {
    let id = router.sandbox_id_for(session_key);
    let opts = ExecOpts {
        timeout: std::time::Duration::from_secs(5),
        ..Default::default()
    };

    // dpkg-query with a format string to get one package name per line.
    let cmd = "dpkg-query -W -f='${Package}\n'";

    match router.backend().exec(&id, cmd, &opts).await {
        Ok(result) if result.exit_code == 0 => {
            let packages: Vec<String> = result
                .stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            debug!(
                count = packages.len(),
                "sandbox dpkg-query returned packages"
            );
            Some(packages)
        },
        Ok(result) => {
            debug!(
                exit_code = result.exit_code,
                "sandbox dpkg-query failed (container may not be running)"
            );
            None
        },
        Err(e) => {
            debug!(error = %e, "sandbox dpkg-query error (container not reachable)");
            None
        },
    }
}

/// Filter packages from the sandbox query that are not already in the config
/// list and are not base OS / infrastructure packages.
fn extra_packages(sandbox_pkgs: &[String], config_pkgs: &[String]) -> Vec<String> {
    let config_set: HashSet<&str> = config_pkgs.iter().map(String::as_str).collect();
    let base_set: HashSet<&str> = BASE_OS_PACKAGES.iter().copied().collect();

    let mut extras: Vec<String> = sandbox_pkgs
        .iter()
        .filter(|pkg| {
            !config_set.contains(pkg.as_str())
                && !base_set.contains(pkg.as_str())
                && !is_infrastructure_package(pkg)
        })
        .cloned()
        .collect();
    extras.sort();
    extras
}

// ── Tool ────────────────────────────────────────────────────────────────────

/// LLM-callable tool that lists sandbox packages grouped by category.
pub struct SandboxPackagesTool {
    sandbox_router: Option<Arc<SandboxRouter>>,
}

impl SandboxPackagesTool {
    pub fn new() -> Self {
        Self {
            sandbox_router: None,
        }
    }

    /// Attach a sandbox router to read the configured packages.
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }
}

impl Default for SandboxPackagesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentTool for SandboxPackagesTool {
    fn name(&self) -> &str {
        "sandbox_packages"
    }

    fn description(&self) -> &str {
        "List packages pre-installed in the sandbox container, grouped by category. \
         Call this before running commands that need specific tools (image processing, \
         audio/video, document conversion, GIS, email, CalDAV, etc.) to check what's \
         available. Also reports extra packages installed at runtime if a container is running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();

        let router = match &self.sandbox_router {
            Some(r) => r,
            None => {
                return Ok(json!({
                    "error": "Sandbox is not enabled"
                }));
            },
        };

        let packages = &router.config().packages;

        if packages.is_empty() {
            #[cfg(feature = "metrics")]
            {
                counter!("tools_sandbox_packages_total").increment(1);
                histogram!("tools_sandbox_packages_duration_seconds")
                    .record(start.elapsed().as_secs_f64());
            }
            return Ok(json!({
                "total": 0,
                "categories": {}
            }));
        }

        // Build categorized config list.
        let grouped = categorize_packages(packages);

        let mut categories = serde_json::Map::new();
        let mut visible_total: usize = 0;
        for (category, pkgs) in &grouped {
            visible_total += pkgs.len();
            categories.insert(
                (*category).to_string(),
                Value::Array(
                    pkgs.iter()
                        .map(|p| Value::String((*p).to_string()))
                        .collect(),
                ),
            );
        }

        let mut result = json!({
            "total": visible_total,
            "categories": categories
        });

        // Hybrid: try querying the running sandbox container for extra packages.
        let session_key = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        if router.is_sandboxed(session_key).await
            && let Some(sandbox_pkgs) = query_sandbox_packages(router, session_key).await
        {
            let extras = extra_packages(&sandbox_pkgs, packages);
            if !extras.is_empty() {
                result["additional_installed"] = json!(extras);
                result["additional_installed_count"] = json!(extras.len());
            }
        }

        #[cfg(feature = "metrics")]
        {
            counter!("tools_sandbox_packages_total").increment(1);
            histogram!("tools_sandbox_packages_duration_seconds")
                .record(start.elapsed().as_secs_f64());
        }

        Ok(result)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::sandbox::{SandboxConfig, SandboxRouter},
        moltis_agents::tool_registry::AgentTool,
    };

    fn make_tool(packages: Vec<String>) -> SandboxPackagesTool {
        let config = SandboxConfig {
            packages,
            ..Default::default()
        };
        let router = Arc::new(SandboxRouter::new(config));
        SandboxPackagesTool::new().with_sandbox_router(router)
    }

    #[tokio::test]
    async fn test_list_returns_categorized_packages() {
        let tool = make_tool(vec![
            "curl".into(),
            "wget".into(),
            "ffmpeg".into(),
            "pandoc".into(),
            "imagemagick".into(),
        ]);

        let result = tool.execute(json!({})).await.unwrap();

        assert_eq!(result["total"], 5);

        let cats = result["categories"].as_object().unwrap();
        assert!(cats.contains_key("Networking"));
        assert!(cats.contains_key("Audio/video"));
        assert!(cats.contains_key("Documents"));
        assert!(cats.contains_key("Image processing"));

        let networking = cats["Networking"].as_array().unwrap();
        assert!(networking.contains(&Value::String("curl".into())));
        assert!(networking.contains(&Value::String("wget".into())));

        let audio = cats["Audio/video"].as_array().unwrap();
        assert!(audio.contains(&Value::String("ffmpeg".into())));
    }

    #[tokio::test]
    async fn test_filters_library_deps() {
        let tool = make_tool(vec![
            "libssl-dev".into(),
            "fonts-liberation".into(),
            "libvips-tools".into(),
            "curl".into(),
            "libxss1".into(),
            "libnss3".into(),
            "python3-dev".into(),
        ]);

        let result = tool.execute(json!({})).await.unwrap();

        // Only curl should remain (libvips-tools is filtered by is_infrastructure_package
        // because it starts with "lib")
        let cats = result["categories"].as_object().unwrap();
        assert_eq!(result["total"], 1);
        assert!(cats.contains_key("Networking"));
        assert!(!cats.contains_key("Image processing"));
    }

    #[tokio::test]
    async fn test_custom_packages_in_other() {
        let tool = make_tool(vec![
            "curl".into(),
            "my-custom-tool".into(),
            "another-tool".into(),
        ]);

        let result = tool.execute(json!({})).await.unwrap();

        assert_eq!(result["total"], 3);
        let cats = result["categories"].as_object().unwrap();
        assert!(cats.contains_key("Other"));

        let other = cats["Other"].as_array().unwrap();
        assert!(other.contains(&Value::String("my-custom-tool".into())));
        assert!(other.contains(&Value::String("another-tool".into())));
    }

    #[tokio::test]
    async fn test_no_sandbox_returns_error() {
        let tool = SandboxPackagesTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert_eq!(result["error"], "Sandbox is not enabled");
    }

    #[tokio::test]
    async fn test_empty_packages() {
        let tool = make_tool(vec![]);
        let result = tool.execute(json!({})).await.unwrap();

        assert_eq!(result["total"], 0);
        let cats = result["categories"].as_object().unwrap();
        assert!(cats.is_empty());
    }

    #[test]
    fn test_tool_metadata() {
        let tool = SandboxPackagesTool::new();
        assert_eq!(tool.name(), "sandbox_packages");
        assert!(tool.description().contains("sandbox"));
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_categorize_preserves_order_within_category() {
        let packages = vec![
            "wget".to_string(),
            "curl".to_string(),
            "dnsutils".to_string(),
        ];
        let grouped = categorize_packages(&packages);
        let (cat, pkgs) = &grouped[0];
        assert_eq!(*cat, "Networking");
        assert_eq!(pkgs, &["wget", "curl", "dnsutils"]);
    }

    #[test]
    fn test_is_infrastructure_package() {
        assert!(is_infrastructure_package("libssl-dev"));
        assert!(is_infrastructure_package("libxss1"));
        assert!(is_infrastructure_package("libnss3"));
        assert!(is_infrastructure_package("fonts-liberation"));
        assert!(is_infrastructure_package("python3-dev"));
        assert!(is_infrastructure_package("ruby-dev"));
        assert!(is_infrastructure_package("llvm-dev"));
        assert!(is_infrastructure_package("libatk1.0-0t64"));

        assert!(!is_infrastructure_package("curl"));
        assert!(!is_infrastructure_package("ffmpeg"));
        assert!(!is_infrastructure_package("pandoc"));
        assert!(!is_infrastructure_package("imagemagick"));
    }

    #[test]
    fn test_new_categories_exist() {
        let packages = vec![
            "vdirsyncer".into(),
            "khal".into(),
            "isync".into(),
            "aerc".into(),
            "notmuch".into(),
            "tin".into(),
            "slrn".into(),
            "python3-discord".into(),
        ];
        let grouped = categorize_packages(&packages);
        let cat_names: Vec<&str> = grouped.iter().map(|(c, _)| *c).collect();

        assert!(cat_names.contains(&"CalDAV/CardDAV"));
        assert!(cat_names.contains(&"Email"));
        assert!(cat_names.contains(&"Newsgroups (NNTP)"));
        assert!(cat_names.contains(&"Messaging APIs"));
    }

    #[test]
    fn test_extra_packages_filters_config_and_base() {
        let config = vec!["curl".into(), "wget".into(), "ffmpeg".into()];
        let sandbox = vec![
            "curl".into(),
            "wget".into(),
            "ffmpeg".into(),
            "bash".into(),       // base OS
            "coreutils".into(),  // base OS
            "libssl3t64".into(), // infrastructure
            "htop".into(),       // extra
            "neovim".into(),     // extra
            "fonts-noto".into(), // infrastructure (fonts-*)
        ];

        let extras = extra_packages(&sandbox, &config);
        assert_eq!(extras, vec!["htop", "neovim"]);
    }

    #[test]
    fn test_extra_packages_empty_when_same() {
        let config = vec!["curl".into(), "wget".into()];
        let sandbox = vec!["curl".into(), "wget".into(), "bash".into()];

        let extras = extra_packages(&sandbox, &config);
        assert!(extras.is_empty());
    }
}
