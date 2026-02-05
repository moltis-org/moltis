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
/// Checks:
/// 1. Custom path from config (if provided)
/// 2. CHROME environment variable
/// 3. Known executable names in PATH
/// 4. Platform-specific installation paths (macOS, Windows)
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

    // Check known executable names in PATH
    for name in CHROMIUM_EXECUTABLES {
        if let Ok(path) = which::which(name) {
            return DetectionResult {
                found: true,
                path: Some(path),
                install_hint: String::new(),
            };
        }
    }

    // Check platform-specific installation paths
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

    // Not found - return with install instructions
    DetectionResult {
        found: false,
        path: None,
        install_hint: install_instructions(),
    }
}

/// Get platform-specific install instructions.
fn install_instructions() -> String {
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
        tracing::info!(path = %path.display(), "Browser detected");
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
    fn test_detect_with_invalid_custom_path() {
        let result = detect_browser(Some("/nonexistent/path/to/chrome"));
        // Should fall through to other detection methods
        // The result depends on whether Chrome is installed on the test system
        assert!(!result.install_hint.is_empty() || result.found);
    }
}
