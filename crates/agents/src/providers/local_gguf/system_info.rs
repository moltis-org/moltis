//! System information detection for local LLM inference.
//!
//! Detects available RAM and GPU capabilities to suggest appropriate models.

use sysinfo::System;

/// System information for model selection.
#[derive(Debug, Clone)]
pub struct SystemInfo {
    /// Total system RAM in bytes.
    pub total_ram_bytes: u64,
    /// Available (free) RAM in bytes.
    pub available_ram_bytes: u64,
    /// Whether Metal GPU acceleration is available (macOS).
    pub has_metal: bool,
    /// Whether CUDA GPU acceleration is available (NVIDIA).
    pub has_cuda: bool,
    /// Whether running on Apple Silicon (M1/M2/M3/etc).
    pub is_apple_silicon: bool,
}

impl SystemInfo {
    /// Detect system information.
    pub fn detect() -> Self {
        let sys = System::new_all();

        let (total_ram_bytes, available_ram_bytes) = {
            let total = sys.total_memory();
            let available = sys.available_memory();
            if total > 0 {
                (total, available)
            } else if let Some((fallback_total, fallback_available)) = read_proc_meminfo() {
                (fallback_total, fallback_available)
            } else {
                (total, available)
            }
        };

        // Metal detection: compile-time check for macOS + runtime check
        let has_metal = cfg!(target_os = "macos") && cfg!(feature = "local-llm-metal");

        // CUDA detection: compile-time feature check
        let has_cuda = cfg!(feature = "local-llm-cuda");

        // Apple Silicon detection: macOS + ARM architecture
        let is_apple_silicon = cfg!(target_os = "macos") && cfg!(target_arch = "aarch64");

        Self {
            total_ram_bytes,
            available_ram_bytes,
            has_metal,
            has_cuda,
            is_apple_silicon,
        }
    }

    /// Total RAM in gigabytes.
    #[must_use]
    pub fn total_ram_gb(&self) -> u32 {
        (self.total_ram_bytes / (1024 * 1024 * 1024)) as u32
    }

    /// Available RAM in gigabytes.
    #[must_use]
    pub fn available_ram_gb(&self) -> u32 {
        (self.available_ram_bytes / (1024 * 1024 * 1024)) as u32
    }

    /// Memory tier for model suggestions.
    #[must_use]
    pub fn memory_tier(&self) -> MemoryTier {
        let gb = self.total_ram_gb();
        if gb >= 32 {
            MemoryTier::Large
        } else if gb >= 16 {
            MemoryTier::Medium
        } else if gb >= 8 {
            MemoryTier::Small
        } else {
            MemoryTier::Tiny
        }
    }

    /// Whether GPU acceleration is available.
    #[must_use]
    pub fn has_gpu(&self) -> bool {
        self.has_metal || self.has_cuda
    }
}

/// Memory tier for model recommendations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    /// 4GB or less — only very small models
    Tiny,
    /// 8GB — small 1-3B models
    Small,
    /// 16GB — medium 7-14B models
    Medium,
    /// 32GB+ — larger 14B+ models
    Large,
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryTier::Tiny => write!(f, "tiny (4GB)"),
            MemoryTier::Small => write!(f, "small (8GB)"),
            MemoryTier::Medium => write!(f, "medium (16GB)"),
            MemoryTier::Large => write!(f, "large (32GB+)"),
        }
    }
}

/// Parse `/proc/meminfo` as a fallback when `sysinfo` returns 0 (common in
/// Docker containers with restrictive cgroup settings).
///
/// Returns `(total_bytes, available_bytes)` or `None` if the file is absent or
/// unparseable.
fn read_proc_meminfo() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_kb: Option<u64> = None;
    let mut available_kb: Option<u64> = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = parse_meminfo_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_meminfo_kb(rest);
        }
        if total_kb.is_some() && available_kb.is_some() {
            break;
        }
    }

    let total = total_kb? * 1024;
    let available = available_kb.unwrap_or(0) * 1024;
    Some((total, available))
}

/// Parse a `/proc/meminfo` value line like `"   16384 kB"` into kilobytes.
fn parse_meminfo_kb(value: &str) -> Option<u64> {
    value.split_whitespace().next()?.parse::<u64>().ok()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_does_not_panic() {
        let info = SystemInfo::detect();
        assert!(info.total_ram_bytes > 0);
    }

    #[test]
    fn test_ram_gb_conversion() {
        let info = SystemInfo {
            total_ram_bytes: 16 * 1024 * 1024 * 1024, // 16 GB
            available_ram_bytes: 8 * 1024 * 1024 * 1024,
            has_metal: false,
            has_cuda: false,
            is_apple_silicon: false,
        };
        assert_eq!(info.total_ram_gb(), 16);
        assert_eq!(info.available_ram_gb(), 8);
    }

    #[test]
    fn test_memory_tier() {
        let make_info = |gb: u64| SystemInfo {
            total_ram_bytes: gb * 1024 * 1024 * 1024,
            available_ram_bytes: 0,
            has_metal: false,
            has_cuda: false,
            is_apple_silicon: false,
        };

        assert_eq!(make_info(2).memory_tier(), MemoryTier::Tiny);
        assert_eq!(make_info(4).memory_tier(), MemoryTier::Tiny);
        assert_eq!(make_info(8).memory_tier(), MemoryTier::Small);
        assert_eq!(make_info(15).memory_tier(), MemoryTier::Small);
        assert_eq!(make_info(16).memory_tier(), MemoryTier::Medium);
        assert_eq!(make_info(24).memory_tier(), MemoryTier::Medium);
        assert_eq!(make_info(32).memory_tier(), MemoryTier::Large);
        assert_eq!(make_info(64).memory_tier(), MemoryTier::Large);
    }

    #[test]
    fn test_has_gpu() {
        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            has_metal: true,
            has_cuda: false,
            is_apple_silicon: true,
        };
        assert!(info.has_gpu());

        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            has_metal: false,
            has_cuda: true,
            is_apple_silicon: false,
        };
        assert!(info.has_gpu());

        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            has_metal: false,
            has_cuda: false,
            is_apple_silicon: false,
        };
        assert!(!info.has_gpu());
    }

    #[test]
    fn test_parse_meminfo_kb() {
        assert_eq!(parse_meminfo_kb("   16384 kB"), Some(16384));
        assert_eq!(parse_meminfo_kb("1024 kB"), Some(1024));
        assert_eq!(parse_meminfo_kb(""), None);
        assert_eq!(parse_meminfo_kb("   not_a_number kB"), None);
    }

    #[test]
    fn test_read_proc_meminfo_on_host() {
        // On Linux this will parse /proc/meminfo; on macOS it will return None.
        let result = read_proc_meminfo();
        if cfg!(target_os = "linux") {
            // Should succeed on any Linux host
            let (total, _available) = result.expect("/proc/meminfo should be readable on Linux");
            assert!(total > 0, "total RAM should be > 0");
        } else {
            // /proc/meminfo doesn't exist on macOS/Windows
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_apple_silicon_detection() {
        // On Apple Silicon, is_apple_silicon should be true
        let info = SystemInfo::detect();
        // We can't assert the value since it depends on the machine,
        // but we can verify the field exists and is consistent
        if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            assert!(info.is_apple_silicon);
        }
    }
}
