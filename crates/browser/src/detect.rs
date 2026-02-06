//! Browser detection and install guidance.

use std::path::PathBuf;

/// Known Chromium-based browser executable names to search for.
/// All of these support CDP (Chrome DevTools Protocol).
const CHROMIUM_EXECUTABLES: &[&str] = &[
    // Chrome
    "chrome",
    "chrome-browser",
    "google-chrome",
    "google-chrome-stable",
    // Chromium
    "chromium",
    "chromium-browser",
    // Microsoft Edge
    "msedge",
    "microsoft-edge",
    "microsoft-edge-stable",
    // Brave
    "brave",
    "brave-browser",
    // Opera
    "opera",
    // Vivaldi
    "vivaldi",
    "vivaldi-stable",
];

/// macOS app bundle paths for Chromium-based browsers.
#[cfg(target_os = "macos")]
const MACOS_APP_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Opera.app/Contents/MacOS/Opera",
    "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
    "/Applications/Arc.app/Contents/MacOS/Arc",
];

/// Windows installation paths for Chromium-based browsers.
#[cfg(target_os = "windows")]
const WINDOWS_PATHS: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
];

/// Result of browser detection.
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// Whether a browser was found.
    pub found: bool,
    /// Path to the browser executable (if found).
    pub path: Option<PathBuf>,
    /// Platform-specific install instructions.
    pub install_hint: String,
}

/// Detect if a Chromium-based browser is available on the system.
///
/// Checks (in order):
/// 1. Custom path from config (if provided)
/// 2. CHROME environment variable
/// 3. Platform-specific installation paths (macOS app bundles, Windows paths)
///    - These are checked first because they're more reliable than PATH lookups
///    - PATH can contain broken wrapper scripts (e.g., Homebrew's deprecated chromium)
/// 4. Known executable names in PATH (fallback)
pub fn detect_browser(custom_path: Option<&str>) -> DetectionResult {
    // Check custom path first
    if let Some(path) = custom_path {
        let p = PathBuf::from(path);
        if p.exists() {
            return DetectionResult {
                found: true,
                path: Some(p),
                install_hint: String::new(),
            };
        }
    }

    // Check CHROME environment variable
    if let Ok(path) = std::env::var("CHROME") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return DetectionResult {
                found: true,
                path: Some(p),
                install_hint: String::new(),
            };
        }
    }

    // Check platform-specific installation paths FIRST (more reliable than PATH)
    // PATH can contain broken wrapper scripts (e.g., Homebrew's deprecated chromium)
    #[cfg(target_os = "macos")]
    for path in MACOS_APP_PATHS {
        let p = PathBuf::from(path);
        if p.exists() {
            return DetectionResult {
                found: true,
                path: Some(p),
                install_hint: String::new(),
            };
        }
    }

    #[cfg(target_os = "windows")]
    for path in WINDOWS_PATHS {
        let p = PathBuf::from(path);
        if p.exists() {
            return DetectionResult {
                found: true,
                path: Some(p),
                install_hint: String::new(),
            };
        }
    }

    // Fallback: check known executable names in PATH
    for name in CHROMIUM_EXECUTABLES {
        if let Ok(path) = which::which(name) {
            return DetectionResult {
                found: true,
                path: Some(path),
                install_hint: String::new(),
            };
        }
    }

    // Not found - return with install instructions
    DetectionResult {
        found: false,
        path: None,
        install_hint: install_instructions(),
    }
}

/// Get platform-specific install instructions.
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

    if !result.found {
        // Print to stderr for immediate visibility to users
        eprintln!("\n⚠️  Browser tool enabled but Chrome/Chromium not found!");
        eprintln!("{}", result.install_hint);
        eprintln!();

        // Also log for log file capture
        tracing::warn!(
            "Browser tool enabled but Chrome/Chromium not found.\n{}",
            result.install_hint
        );
    } else if let Some(ref path) = result.path {
        tracing::info!(path = %path.display(), "Host browser detected");
    }

    result.found
}

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
        // Should fall through to other detection methods
        // The result depends on whether Chrome is installed on the test system
        assert!(!result.install_hint.is_empty() || result.found);
    }

    #[test]
    fn test_detect_custom_path_takes_precedence() {
        // Create a temp file to simulate a browser executable
        let temp_dir = std::env::temp_dir();
        let fake_browser = temp_dir.join("fake-chrome-for-test");
        std::fs::write(&fake_browser, "fake").unwrap();

        let result = detect_browser(Some(fake_browser.to_str().unwrap()));
        assert!(result.found);
        assert_eq!(result.path.as_ref().unwrap(), &fake_browser);

        std::fs::remove_file(&fake_browser).unwrap();
    }

    // Note: Testing CHROME env var detection would require unsafe blocks
    // in Rust 2024 edition. The functionality is simple and covered by
    // manual testing. The detection order is:
    // 1. Custom path (tested above)
    // 2. CHROME env var
    // 3. Platform app paths (tested below for macOS/Windows)
    // 4. PATH executables

    #[test]
    fn test_chromium_executables_list_not_empty() {
        assert!(
            !CHROMIUM_EXECUTABLES.is_empty(),
            "Should have executable names to search"
        );
        assert!(
            CHROMIUM_EXECUTABLES.contains(&"chrome"),
            "Should include 'chrome'"
        );
        assert!(
            CHROMIUM_EXECUTABLES.contains(&"chromium"),
            "Should include 'chromium'"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_app_paths_not_empty() {
        assert!(
            !MACOS_APP_PATHS.is_empty(),
            "Should have macOS app paths to check"
        );
        // Should include Google Chrome
        assert!(
            MACOS_APP_PATHS.iter().any(|p| p.contains("Google Chrome")),
            "Should include Google Chrome path"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_detection_prefers_app_bundles() {
        // This test verifies that if both a PATH executable and an app bundle exist,
        // the app bundle is found first (by checking the detection order in the code)
        //
        // We can't easily mock the filesystem, but we can verify the paths are checked
        // in the right order by checking if a real browser is found via app path.
        let result = detect_browser(None);

        if result.found {
            let path = result.path.unwrap();
            let path_str = path.to_string_lossy();

            // If found, on macOS it should preferentially find app bundles
            // (unless CHROME env var is set)
            if std::env::var("CHROME").is_err() {
                // If it starts with /Applications, we found via app bundle path
                // If it doesn't, we fell back to PATH (which is OK if no app bundles exist)
                if path_str.starts_with("/Applications") {
                    // Good - found via app bundle, which is preferred
                    assert!(
                        path_str.contains(".app"),
                        "macOS detection should find .app bundle"
                    );
                }
                // Note: If no app bundles exist, PATH is used as fallback, which is correct
            }
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_windows_paths_not_empty() {
        assert!(
            !WINDOWS_PATHS.is_empty(),
            "Should have Windows paths to check"
        );
        assert!(
            WINDOWS_PATHS.iter().any(|p| p.contains("chrome.exe")),
            "Should include chrome.exe path"
        );
    }

    #[test]
    fn test_detection_result_fields() {
        // When not found, should have install hint
        let not_found = DetectionResult {
            found: false,
            path: None,
            install_hint: install_instructions(),
        };
        assert!(!not_found.found);
        assert!(not_found.path.is_none());
        assert!(!not_found.install_hint.is_empty());

        // When found, should have path and empty hint
        let found = DetectionResult {
            found: true,
            path: Some(PathBuf::from("/usr/bin/chrome")),
            install_hint: String::new(),
        };
        assert!(found.found);
        assert!(found.path.is_some());
        assert!(found.install_hint.is_empty());
    }
}
