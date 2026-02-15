//! Browser detection and install guidance.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::types::{BrowserKind, BrowserPreference};

/// Executable names mapped to known Chromium-family browser kinds.
#[derive(Debug, Clone, Copy)]
struct BrowserExecutable {
    kind: BrowserKind,
    name: &'static str,
}

/// Known Chromium-based browser executable names to search for.
/// All of these support CDP (Chrome DevTools Protocol).
const CHROMIUM_EXECUTABLES: &[BrowserExecutable] = &[
    // Chrome
    BrowserExecutable {
        kind: BrowserKind::Chrome,
        name: "chrome",
    },
    BrowserExecutable {
        kind: BrowserKind::Chrome,
        name: "chrome-browser",
    },
    BrowserExecutable {
        kind: BrowserKind::Chrome,
        name: "google-chrome",
    },
    BrowserExecutable {
        kind: BrowserKind::Chrome,
        name: "google-chrome-stable",
    },
    // Chromium
    BrowserExecutable {
        kind: BrowserKind::Chromium,
        name: "chromium",
    },
    BrowserExecutable {
        kind: BrowserKind::Chromium,
        name: "chromium-browser",
    },
    // Microsoft Edge
    BrowserExecutable {
        kind: BrowserKind::Edge,
        name: "msedge",
    },
    BrowserExecutable {
        kind: BrowserKind::Edge,
        name: "microsoft-edge",
    },
    BrowserExecutable {
        kind: BrowserKind::Edge,
        name: "microsoft-edge-stable",
    },
    // Brave
    BrowserExecutable {
        kind: BrowserKind::Brave,
        name: "brave",
    },
    BrowserExecutable {
        kind: BrowserKind::Brave,
        name: "brave-browser",
    },
    // Opera
    BrowserExecutable {
        kind: BrowserKind::Opera,
        name: "opera",
    },
    // Vivaldi
    BrowserExecutable {
        kind: BrowserKind::Vivaldi,
        name: "vivaldi",
    },
    BrowserExecutable {
        kind: BrowserKind::Vivaldi,
        name: "vivaldi-stable",
    },
];

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, Clone, Copy)]
struct BrowserPath {
    kind: BrowserKind,
    path: &'static str,
}

/// macOS app bundle paths for Chromium-based browsers.
#[cfg(target_os = "macos")]
const MACOS_APP_PATHS: &[BrowserPath] = &[
    BrowserPath {
        kind: BrowserKind::Chrome,
        path: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    },
    BrowserPath {
        kind: BrowserKind::Chromium,
        path: "/Applications/Chromium.app/Contents/MacOS/Chromium",
    },
    BrowserPath {
        kind: BrowserKind::Edge,
        path: "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    },
    BrowserPath {
        kind: BrowserKind::Brave,
        path: "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    },
    BrowserPath {
        kind: BrowserKind::Opera,
        path: "/Applications/Opera.app/Contents/MacOS/Opera",
    },
    BrowserPath {
        kind: BrowserKind::Vivaldi,
        path: "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
    },
    BrowserPath {
        kind: BrowserKind::Arc,
        path: "/Applications/Arc.app/Contents/MacOS/Arc",
    },
];

/// Windows installation paths for Chromium-based browsers.
#[cfg(target_os = "windows")]
const WINDOWS_PATHS: &[BrowserPath] = &[
    BrowserPath {
        kind: BrowserKind::Chrome,
        path: r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    },
    BrowserPath {
        kind: BrowserKind::Chrome,
        path: r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    },
    BrowserPath {
        kind: BrowserKind::Edge,
        path: r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    },
    BrowserPath {
        kind: BrowserKind::Brave,
        path: r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
    },
];

/// Where a detected browser path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionSource {
    CustomPath,
    EnvVar,
    PlatformPath,
    PathLookup,
}

/// One detected browser candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedBrowser {
    pub kind: BrowserKind,
    pub path: PathBuf,
    pub source: DetectionSource,
}

/// Result of browser detection.
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// All detected browser candidates in preference order.
    pub browsers: Vec<DetectedBrowser>,
    /// Platform-specific install instructions.
    pub install_hint: String,
}

impl DetectionResult {
    #[must_use]
    pub fn found(&self) -> bool {
        !self.browsers.is_empty()
    }
}

fn infer_kind_from_path(path: &Path) -> BrowserKind {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.contains("brave") {
        return BrowserKind::Brave;
    }
    if lower.contains("msedge") || lower.contains("microsoft-edge") || lower.contains(" edge") {
        return BrowserKind::Edge;
    }
    if lower.contains("chromium") {
        return BrowserKind::Chromium;
    }
    if lower.contains("opera") {
        return BrowserKind::Opera;
    }
    if lower.contains("vivaldi") {
        return BrowserKind::Vivaldi;
    }
    if lower.contains("arc.app") || lower.ends_with("/arc") || lower.ends_with("\\arc.exe") {
        return BrowserKind::Arc;
    }
    if lower.contains("chrome") {
        return BrowserKind::Chrome;
    }
    BrowserKind::Custom
}

fn push_browser(
    browsers: &mut Vec<DetectedBrowser>,
    seen_paths: &mut HashSet<PathBuf>,
    kind: BrowserKind,
    path: PathBuf,
    source: DetectionSource,
) {
    if !path.exists() {
        return;
    }
    if !seen_paths.insert(path.clone()) {
        return;
    }
    browsers.push(DetectedBrowser { kind, path, source });
}

/// Detect all Chromium-based browsers available on the system.
///
/// Checks (in order):
/// 1. Custom path from config (if provided)
/// 2. CHROME environment variable
/// 3. Platform-specific installation paths (macOS app bundles, Windows paths)
/// 4. Known executable names in PATH (fallback)
#[must_use]
pub fn detect_browsers(custom_path: Option<&str>) -> Vec<DetectedBrowser> {
    let mut browsers = Vec::new();
    let mut seen_paths = HashSet::new();

    if let Some(path) = custom_path {
        let p = PathBuf::from(path);
        push_browser(
            &mut browsers,
            &mut seen_paths,
            infer_kind_from_path(&p),
            p,
            DetectionSource::CustomPath,
        );
    }

    if let Ok(path) = std::env::var("CHROME") {
        let p = PathBuf::from(path);
        push_browser(
            &mut browsers,
            &mut seen_paths,
            infer_kind_from_path(&p),
            p,
            DetectionSource::EnvVar,
        );
    }

    #[cfg(target_os = "macos")]
    for entry in MACOS_APP_PATHS {
        let p = PathBuf::from(entry.path);
        push_browser(
            &mut browsers,
            &mut seen_paths,
            entry.kind,
            p,
            DetectionSource::PlatformPath,
        );
    }

    #[cfg(target_os = "windows")]
    for entry in WINDOWS_PATHS {
        let p = PathBuf::from(entry.path);
        push_browser(
            &mut browsers,
            &mut seen_paths,
            entry.kind,
            p,
            DetectionSource::PlatformPath,
        );
    }

    for entry in CHROMIUM_EXECUTABLES {
        if let Ok(path) = which::which(entry.name) {
            push_browser(
                &mut browsers,
                &mut seen_paths,
                entry.kind,
                path,
                DetectionSource::PathLookup,
            );
        }
    }

    browsers
}

/// Detect available browsers and include install guidance when empty.
#[must_use]
pub fn detect_browser(custom_path: Option<&str>) -> DetectionResult {
    let browsers = detect_browsers(custom_path);
    let install_hint = if browsers.is_empty() {
        install_instructions()
    } else {
        String::new()
    };

    DetectionResult {
        browsers,
        install_hint,
    }
}

/// Select a browser from detected candidates based on request preference.
#[must_use]
pub fn pick_browser(
    browsers: &[DetectedBrowser],
    preference: Option<BrowserPreference>,
) -> Option<DetectedBrowser> {
    let pref = preference.unwrap_or_default();
    match pref.preferred_kind() {
        None => browsers.first().cloned(),
        Some(kind) => browsers.iter().find(|entry| entry.kind == kind).cloned(),
    }
}

/// Convert detected browser kinds into a readable list.
#[must_use]
pub fn installed_browser_labels(browsers: &[DetectedBrowser]) -> Vec<String> {
    let mut kinds = Vec::new();
    let mut seen = HashSet::new();
    for entry in browsers {
        if seen.insert(entry.kind) {
            kinds.push(entry.kind.to_string());
        }
    }
    kinds
}

/// Result of a best-effort browser auto-install attempt.
#[derive(Debug, Clone)]
pub struct AutoInstallResult {
    pub attempted: bool,
    pub installed: bool,
    pub details: String,
}

#[derive(Debug, Clone)]
struct InstallCommand {
    program: &'static str,
    args: Vec<&'static str>,
}

impl InstallCommand {
    fn new(program: &'static str, args: &[&'static str]) -> Self {
        Self {
            program,
            args: args.to_vec(),
        }
    }

    fn display(&self) -> String {
        if self.args.is_empty() {
            return self.program.to_string();
        }
        format!("{} {}", self.program, self.args.join(" "))
    }
}

fn install_targets_for_preference(preference: BrowserPreference) -> Vec<BrowserKind> {
    match preference {
        BrowserPreference::Auto => vec![
            BrowserKind::Chrome,
            BrowserKind::Chromium,
            BrowserKind::Brave,
            BrowserKind::Edge,
        ],
        BrowserPreference::Chrome => vec![BrowserKind::Chrome],
        BrowserPreference::Chromium => vec![BrowserKind::Chromium],
        BrowserPreference::Edge => vec![BrowserKind::Edge],
        BrowserPreference::Brave => vec![BrowserKind::Brave],
        BrowserPreference::Opera => vec![BrowserKind::Opera],
        BrowserPreference::Vivaldi => vec![BrowserKind::Vivaldi],
        BrowserPreference::Arc => vec![BrowserKind::Arc],
    }
}

#[cfg(target_os = "macos")]
fn macos_install_commands(target: BrowserKind) -> Vec<InstallCommand> {
    let casks = match target {
        BrowserKind::Chrome => vec!["google-chrome"],
        BrowserKind::Chromium => vec!["chromium"],
        BrowserKind::Edge => vec!["microsoft-edge"],
        BrowserKind::Brave => vec!["brave-browser"],
        BrowserKind::Opera => vec!["opera"],
        BrowserKind::Vivaldi => vec!["vivaldi"],
        BrowserKind::Arc => vec!["arc"],
        BrowserKind::Custom => vec![],
    };

    casks
        .into_iter()
        .map(|cask| InstallCommand::new("brew", &["install", "--cask", cask]))
        .collect()
}

#[cfg(target_os = "linux")]
fn linux_package_candidates(target: BrowserKind) -> Vec<&'static str> {
    match target {
        BrowserKind::Chrome => vec!["google-chrome-stable", "google-chrome", "chromium"],
        BrowserKind::Chromium => vec!["chromium-browser", "chromium"],
        BrowserKind::Edge => vec!["microsoft-edge-stable", "microsoft-edge", "chromium"],
        BrowserKind::Brave => vec!["brave-browser", "chromium"],
        BrowserKind::Opera => vec!["opera-stable", "opera", "chromium"],
        BrowserKind::Vivaldi => vec!["vivaldi-stable", "vivaldi", "chromium"],
        BrowserKind::Arc => vec!["chromium"],
        BrowserKind::Custom => vec![],
    }
}

#[cfg(target_os = "windows")]
fn windows_package_ids(target: BrowserKind) -> Vec<&'static str> {
    match target {
        BrowserKind::Chrome => vec!["Google.Chrome"],
        BrowserKind::Chromium => vec!["eloston.ungoogled-chromium"],
        BrowserKind::Edge => vec!["Microsoft.Edge"],
        BrowserKind::Brave => vec!["Brave.Brave"],
        BrowserKind::Opera => vec!["Opera.Opera"],
        BrowserKind::Vivaldi => vec!["VivaldiTechnologies.Vivaldi"],
        BrowserKind::Arc => vec!["TheBrowserCompany.Arc"],
        BrowserKind::Custom => vec![],
    }
}

async fn run_command(command: &InstallCommand) -> Result<(), String> {
    let result = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new(command.program)
            .args(&command.args)
            .output(),
    )
    .await;

    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(format!(
                "failed to execute '{}': {error}",
                command.display()
            ));
        },
        Err(_) => {
            return Err(format!("timed out after 180s: {}", command.display()));
        },
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = if !stderr.is_empty() {
        stderr
    } else {
        stdout
    };
    Err(format!("{} failed: {}", command.display(), details))
}

#[cfg(target_os = "linux")]
async fn run_with_optional_sudo(command: InstallCommand) -> Result<(), String> {
    let first_error = match run_command(&command).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    if which::which("sudo").is_err() {
        return Err(first_error);
    }

    let mut sudo_args = Vec::with_capacity(command.args.len() + 2);
    sudo_args.push("-n");
    sudo_args.push(command.program);
    sudo_args.extend(command.args.iter().copied());

    let sudo_cmd = InstallCommand::new("sudo", &sudo_args);
    run_command(&sudo_cmd)
        .await
        .map_err(|sudo_error| format!("{first_error} | {sudo_error}"))
}

#[cfg(target_os = "macos")]
async fn auto_install_for_targets(targets: &[BrowserKind]) -> AutoInstallResult {
    if which::which("brew").is_err() {
        return AutoInstallResult {
            attempted: false,
            installed: false,
            details: "brew is not installed".to_string(),
        };
    }

    let mut errors = Vec::new();
    for target in targets {
        for command in macos_install_commands(*target) {
            match run_command(&command).await {
                Ok(()) => {
                    return AutoInstallResult {
                        attempted: true,
                        installed: true,
                        details: format!("installed browser via '{}'", command.display()),
                    };
                },
                Err(error) => errors.push(error),
            }
        }
    }

    AutoInstallResult {
        attempted: true,
        installed: false,
        details: errors.join(" | "),
    }
}

#[cfg(target_os = "linux")]
struct LinuxPkgManager {
    bin: &'static str,
    install_args: &'static [&'static str],
    /// Run this command before installing (e.g. `apt-get update`).
    pre_install: Option<&'static [&'static str]>,
}

#[cfg(target_os = "linux")]
const LINUX_PKG_MANAGERS: &[LinuxPkgManager] = &[
    LinuxPkgManager {
        bin: "apt-get",
        install_args: &["install", "-y", "-qq"],
        pre_install: Some(&["update", "-qq"]),
    },
    LinuxPkgManager {
        bin: "dnf",
        install_args: &["install", "-y"],
        pre_install: None,
    },
    LinuxPkgManager {
        bin: "pacman",
        install_args: &["-S", "--noconfirm"],
        pre_install: None,
    },
];

#[cfg(target_os = "linux")]
async fn auto_install_for_targets(targets: &[BrowserKind]) -> AutoInstallResult {
    let available: Vec<&LinuxPkgManager> = LINUX_PKG_MANAGERS
        .iter()
        .filter(|pm| which::which(pm.bin).is_ok())
        .collect();

    if available.is_empty() {
        return AutoInstallResult {
            attempted: false,
            installed: false,
            details: "no supported package manager found (apt-get/dnf/pacman)".to_string(),
        };
    }

    let mut errors = Vec::new();

    for pm in &available {
        if let Some(pre) = pm.pre_install {
            let mut args = Vec::with_capacity(pre.len());
            args.extend_from_slice(pre);
            let _ = run_with_optional_sudo(InstallCommand::new(pm.bin, &args)).await;
        }

        for target in targets {
            for pkg in linux_package_candidates(*target) {
                let mut args = Vec::with_capacity(pm.install_args.len() + 1);
                args.extend_from_slice(pm.install_args);
                args.push(pkg);
                let cmd = InstallCommand::new(pm.bin, &args);
                match run_with_optional_sudo(cmd).await {
                    Ok(()) => {
                        return AutoInstallResult {
                            attempted: true,
                            installed: true,
                            details: format!("installed browser package '{pkg}'"),
                        };
                    },
                    Err(error) => errors.push(error),
                }
            }
        }
    }

    AutoInstallResult {
        attempted: true,
        installed: false,
        details: errors.join(" | "),
    }
}

#[cfg(target_os = "windows")]
async fn auto_install_for_targets(targets: &[BrowserKind]) -> AutoInstallResult {
    if which::which("winget").is_err() {
        return AutoInstallResult {
            attempted: false,
            installed: false,
            details: "winget is not installed".to_string(),
        };
    }

    let mut errors = Vec::new();
    for target in targets {
        for id in windows_package_ids(*target) {
            let command = InstallCommand::new("winget", &[
                "install",
                "--id",
                id,
                "--accept-package-agreements",
                "--accept-source-agreements",
            ]);

            match run_command(&command).await {
                Ok(()) => {
                    return AutoInstallResult {
                        attempted: true,
                        installed: true,
                        details: format!("installed browser package '{}'", id),
                    };
                },
                Err(error) => errors.push(error),
            }
        }
    }

    AutoInstallResult {
        attempted: true,
        installed: false,
        details: errors.join(" | "),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn auto_install_for_targets(_targets: &[BrowserKind]) -> AutoInstallResult {
    AutoInstallResult {
        attempted: false,
        installed: false,
        details: "automatic browser install is not supported on this platform".to_string(),
    }
}

/// Attempt to auto-install a browser when none are detected.
///
/// This is best-effort only. On failure, callers should still return explicit
/// install instructions.
pub async fn auto_install_browser(preference: BrowserPreference) -> AutoInstallResult {
    let targets = install_targets_for_preference(preference);
    auto_install_for_targets(&targets).await
}

/// Get platform-specific install instructions.
#[must_use]
pub fn install_instructions() -> String {
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unknown"
    };

    let instructions = match platform {
        "macOS" => {
            "  brew install --cask google-chrome\n  \
             # Alternatives: chromium, brave-browser, microsoft-edge"
        },
        "Linux" => {
            "  Debian/Ubuntu: sudo apt install chromium-browser\n  \
             Fedora:         sudo dnf install chromium\n  \
             Arch:           sudo pacman -S chromium\n  \
             # Alternatives: brave-browser, microsoft-edge-stable"
        },
        "Windows" => {
            "  winget install Google.Chrome\n  \
             # Alternatives: Microsoft.Edge, Brave.Brave"
        },
        _ => "  Download from https://www.google.com/chrome/",
    };

    format!(
        "No Chromium-based browser found. Install one:\n\n\
         {instructions}\n\n\
         Any Chromium-based browser works (Chrome, Chromium, Edge, Brave, Opera, Vivaldi).\n\n\
         Or set the path manually:\n  \
         [tools.browser]\n  \
         chrome_path = \"/path/to/browser\"\n\n\
         Or set the CHROME environment variable."
    )
}

/// Check browser availability and warn if not found.
///
/// Call this at startup when browser is enabled. Prints a visible warning
/// to stderr and logs via tracing for log file capture.
pub fn check_and_warn(custom_path: Option<&str>) -> bool {
    let result = detect_browser(custom_path);

    if !result.found() {
        eprintln!("\n⚠️  Browser tool enabled but no compatible browser was found!");
        eprintln!("{}", result.install_hint);
        eprintln!();

        tracing::warn!(
            "Browser tool enabled but no compatible browser was found.\n{}",
            result.install_hint
        );
        return false;
    }

    let labels = installed_browser_labels(&result.browsers);
    tracing::info!(browsers = %labels.join(", "), "Host browsers detected");

    for browser in &result.browsers {
        tracing::debug!(
            browser = %browser.kind,
            path = %browser.path.display(),
            source = ?browser.source,
            "browser candidate"
        );
    }
    true
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_instructions_not_empty() {
        let hint = install_instructions();
        assert!(!hint.is_empty());
        assert!(hint.contains("Chrome"));
    }

    #[test]
    fn test_install_instructions_platform_specific() {
        let hint = install_instructions();

        #[cfg(target_os = "macos")]
        assert!(
            hint.contains("brew"),
            "macOS instructions should mention brew"
        );

        #[cfg(target_os = "linux")]
        assert!(
            hint.contains("apt") || hint.contains("dnf") || hint.contains("pacman"),
            "Linux instructions should mention package managers"
        );

        #[cfg(target_os = "windows")]
        assert!(
            hint.contains("winget"),
            "Windows instructions should mention winget"
        );
    }

    #[test]
    fn test_detect_with_invalid_custom_path() {
        let result = detect_browser(Some("/nonexistent/path/to/chrome"));
        assert!(!result.install_hint.is_empty() || result.found());
    }

    #[test]
    fn test_detect_custom_path_takes_precedence() {
        let temp_dir = std::env::temp_dir();
        let fake_browser = temp_dir.join("fake-chrome-for-test");
        std::fs::write(&fake_browser, "fake").unwrap();

        let result = detect_browser(Some(fake_browser.to_str().unwrap()));
        assert!(result.found());
        assert_eq!(result.browsers[0].path, fake_browser);

        std::fs::remove_file(&fake_browser).unwrap();
    }

    #[test]
    fn test_chromium_executables_list_not_empty() {
        assert!(
            !CHROMIUM_EXECUTABLES.is_empty(),
            "Should have executable names to search"
        );
        assert!(
            CHROMIUM_EXECUTABLES
                .iter()
                .any(|entry| entry.name == "chrome"),
            "Should include 'chrome'"
        );
        assert!(
            CHROMIUM_EXECUTABLES
                .iter()
                .any(|entry| entry.name == "chromium"),
            "Should include 'chromium'"
        );
    }

    #[test]
    fn test_pick_browser_auto_uses_first() {
        let browsers = vec![
            DetectedBrowser {
                kind: BrowserKind::Brave,
                path: PathBuf::from("/tmp/brave"),
                source: DetectionSource::PathLookup,
            },
            DetectedBrowser {
                kind: BrowserKind::Chrome,
                path: PathBuf::from("/tmp/chrome"),
                source: DetectionSource::PathLookup,
            },
        ];

        let selected = pick_browser(&browsers, Some(BrowserPreference::Auto)).unwrap();
        assert_eq!(selected.kind, BrowserKind::Brave);
    }

    #[test]
    fn test_pick_browser_specific() {
        let browsers = vec![
            DetectedBrowser {
                kind: BrowserKind::Brave,
                path: PathBuf::from("/tmp/brave"),
                source: DetectionSource::PathLookup,
            },
            DetectedBrowser {
                kind: BrowserKind::Chrome,
                path: PathBuf::from("/tmp/chrome"),
                source: DetectionSource::PathLookup,
            },
        ];

        let selected = pick_browser(&browsers, Some(BrowserPreference::Chrome)).unwrap();
        assert_eq!(selected.kind, BrowserKind::Chrome);
    }

    #[test]
    fn test_pick_browser_missing_specific_returns_none() {
        let browsers = vec![DetectedBrowser {
            kind: BrowserKind::Brave,
            path: PathBuf::from("/tmp/brave"),
            source: DetectionSource::PathLookup,
        }];

        let selected = pick_browser(&browsers, Some(BrowserPreference::Chrome));
        assert!(selected.is_none());
    }

    #[test]
    fn test_installed_browser_labels_dedupes_kinds() {
        let browsers = vec![
            DetectedBrowser {
                kind: BrowserKind::Chrome,
                path: PathBuf::from("/tmp/chrome"),
                source: DetectionSource::PathLookup,
            },
            DetectedBrowser {
                kind: BrowserKind::Chrome,
                path: PathBuf::from("/tmp/chrome2"),
                source: DetectionSource::PathLookup,
            },
            DetectedBrowser {
                kind: BrowserKind::Brave,
                path: PathBuf::from("/tmp/brave"),
                source: DetectionSource::PathLookup,
            },
        ];

        let labels = installed_browser_labels(&browsers);
        assert_eq!(labels, vec!["chrome".to_string(), "brave".to_string()]);
    }

    // --- infer_kind_from_path tests ---

    #[test]
    fn test_infer_chrome_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/google-chrome")),
            BrowserKind::Chrome,
        );
        assert_eq!(
            infer_kind_from_path(Path::new(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
            )),
            BrowserKind::Chrome,
        );
    }

    #[test]
    fn test_infer_chromium_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/chromium-browser")),
            BrowserKind::Chromium,
        );
    }

    #[test]
    fn test_infer_brave_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new(
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
            )),
            BrowserKind::Brave,
        );
    }

    #[test]
    fn test_infer_edge_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/msedge")),
            BrowserKind::Edge,
        );
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/microsoft-edge")),
            BrowserKind::Edge,
        );
        assert_eq!(
            infer_kind_from_path(Path::new(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
            )),
            BrowserKind::Edge,
        );
    }

    #[test]
    fn test_infer_opera_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/opera")),
            BrowserKind::Opera,
        );
    }

    #[test]
    fn test_infer_vivaldi_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/vivaldi-stable")),
            BrowserKind::Vivaldi,
        );
    }

    #[test]
    fn test_infer_arc_from_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/Applications/Arc.app/Contents/MacOS/Arc")),
            BrowserKind::Arc,
        );
        // Trailing `/arc` (case insensitive)
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/local/bin/arc")),
            BrowserKind::Arc,
        );
    }

    #[test]
    fn test_infer_custom_from_unknown_path() {
        assert_eq!(
            infer_kind_from_path(Path::new("/usr/bin/my-browser")),
            BrowserKind::Custom,
        );
    }

    #[test]
    fn test_infer_brave_not_confused_with_chrome() {
        // "Brave Browser" contains "Bra" but not "chrome" — should be Brave, not Chrome.
        assert_eq!(
            infer_kind_from_path(Path::new("/opt/brave-browser/brave")),
            BrowserKind::Brave,
        );
    }

    #[test]
    fn test_infer_chromium_before_chrome() {
        // "chromium" contains "chrome" — should be Chromium, not Chrome.
        assert_eq!(
            infer_kind_from_path(Path::new("/snap/bin/chromium")),
            BrowserKind::Chromium,
        );
    }

    // --- install_targets_for_preference tests ---

    #[test]
    fn test_install_targets_auto_returns_multiple() {
        let targets = install_targets_for_preference(BrowserPreference::Auto);
        assert!(targets.len() > 1, "Auto should try multiple browsers");
        assert!(targets.contains(&BrowserKind::Chrome));
        assert!(targets.contains(&BrowserKind::Chromium));
    }

    #[test]
    fn test_install_targets_specific_returns_single() {
        let targets = install_targets_for_preference(BrowserPreference::Brave);
        assert_eq!(targets, vec![BrowserKind::Brave]);
    }

    // --- platform install command mapping tests ---

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_install_commands_chrome() {
        let cmds = macos_install_commands(BrowserKind::Chrome);
        assert!(!cmds.is_empty());
        assert_eq!(cmds[0].program, "brew");
        assert!(cmds[0].args.contains(&"google-chrome"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_install_commands_custom_is_empty() {
        let cmds = macos_install_commands(BrowserKind::Custom);
        assert!(cmds.is_empty(), "Custom should not attempt any install");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_package_candidates_chrome() {
        let pkgs = linux_package_candidates(BrowserKind::Chrome);
        assert!(!pkgs.is_empty());
        assert!(pkgs.iter().any(|p| p.contains("chrome")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_package_candidates_custom_is_empty() {
        let pkgs = linux_package_candidates(BrowserKind::Custom);
        assert!(pkgs.is_empty(), "Custom should not attempt any install");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_package_ids_chrome() {
        let ids = windows_package_ids(BrowserKind::Chrome);
        assert!(!ids.is_empty());
        assert!(ids.contains(&"Google.Chrome"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_package_ids_custom_is_empty() {
        let ids = windows_package_ids(BrowserKind::Custom);
        assert!(ids.is_empty(), "Custom should not attempt any install");
    }
}
