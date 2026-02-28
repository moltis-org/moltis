//! System information detection for local LLM inference.
//!
//! Detects available RAM and GPU capabilities to suggest appropriate models.

use sysinfo::System;

#[cfg(target_os = "macos")]
#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut std::ffi::c_void;
}

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
    #[must_use]
    pub fn detect() -> Self {
        let sys = System::new_all();

        let total_ram_bytes = sys.total_memory();
        let available_ram_bytes = sys.available_memory();

        // Apple Silicon detection: macOS + aarch64
        let is_apple_silicon = cfg!(target_os = "macos") && cfg!(target_arch = "aarch64");

        // Metal requires both compile-time backend support and a runtime device.
        let has_metal_compile_support =
            cfg!(target_os = "macos") && cfg!(feature = "local-llm-metal");
        let has_metal = has_metal_compile_support && metal_runtime_available();

        // CUDA detection: compile-time feature check
        let has_cuda = cfg!(feature = "local-llm-cuda");

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

#[cfg(target_os = "macos")]
#[must_use]
fn metal_runtime_available() -> bool {
    // SAFETY: Calling a pure system probe from Apple's Metal framework.
    // Returns null when no default Metal device is available.
    let device = unsafe { MTLCreateSystemDefaultDevice() };
    !device.is_null()
}

#[cfg(not(target_os = "macos"))]
#[must_use]
const fn metal_runtime_available() -> bool {
    false
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
    fn test_is_apple_silicon_detection() {
        let info = SystemInfo::detect();
        // On macOS aarch64, this should be true; otherwise false
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(info.is_apple_silicon);
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert!(!info.is_apple_silicon);
    }

    #[test]
    fn test_has_metal_detection_formula() {
        let info = SystemInfo::detect();
        let expected = cfg!(target_os = "macos")
            && cfg!(feature = "local-llm-metal")
            && metal_runtime_available();
        assert_eq!(info.has_metal, expected);
    }
}
