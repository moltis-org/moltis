//! System information detection for local LLM inference.
//!
//! Detects available RAM and GPU capabilities to suggest appropriate models.

use {
    crate::local_gguf::runtime_devices::{self, GgufRuntimeDevice, read_proc_meminfo},
    sysinfo::System,
};

/// System information for model selection.
#[derive(Debug, Clone)]
pub struct SystemInfo {
    /// Total system RAM in bytes.
    pub total_ram_bytes: u64,
    /// Available (free) RAM in bytes.
    pub available_ram_bytes: u64,
    /// GGUF devices discovered via llama.cpp runtime probing.
    pub gguf_devices: Vec<GgufRuntimeDevice>,
    /// Whether Metal GPU acceleration is available (macOS).
    pub has_metal: bool,
    /// Whether CUDA GPU acceleration is available (NVIDIA).
    pub has_cuda: bool,
    /// Whether Vulkan GPU acceleration is available.
    pub has_vulkan: bool,
    /// Whether running on Apple Silicon (M1/M2/M3/etc).
    pub is_apple_silicon: bool,
}

impl SystemInfo {
    /// Detect system information.
    #[must_use]
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
        let runtime = runtime_devices::detect_runtime_support();

        // Apple Silicon detection: macOS + aarch64
        let is_apple_silicon = cfg!(target_os = "macos") && cfg!(target_arch = "aarch64");

        Self {
            total_ram_bytes,
            available_ram_bytes,
            gguf_devices: runtime.devices,
            has_metal: runtime.has_metal,
            has_cuda: runtime.has_cuda,
            has_vulkan: runtime.has_vulkan,
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
        !self.gguf_devices.is_empty()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_device(backend: &str) -> GgufRuntimeDevice {
        GgufRuntimeDevice {
            index: 0,
            name: format!("{backend}0"),
            description: "GPU".into(),
            backend: backend.into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }
    }

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
            gguf_devices: vec![],
            has_metal: false,
            has_cuda: false,
            has_vulkan: false,
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
            gguf_devices: vec![],
            has_metal: false,
            has_cuda: false,
            has_vulkan: false,
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
            gguf_devices: vec![sample_device("Metal")],
            has_metal: true,
            has_cuda: false,
            has_vulkan: false,
            is_apple_silicon: true,
        };
        assert!(info.has_gpu());

        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gguf_devices: vec![sample_device("CUDA")],
            has_metal: false,
            has_cuda: true,
            has_vulkan: false,
            is_apple_silicon: false,
        };
        assert!(info.has_gpu());

        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gguf_devices: vec![sample_device("Vulkan")],
            has_metal: false,
            has_cuda: false,
            has_vulkan: true,
            is_apple_silicon: false,
        };
        assert!(info.has_gpu());

        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gguf_devices: vec![],
            has_metal: false,
            has_cuda: false,
            has_vulkan: false,
            is_apple_silicon: false,
        };
        assert!(!info.has_gpu());
    }

    #[test]
    fn test_is_apple_silicon_detection() {
        let info = SystemInfo::detect();
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(info.is_apple_silicon);
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert!(!info.is_apple_silicon);
    }

    #[test]
    fn test_has_metal_flag_can_be_set() {
        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gguf_devices: vec![sample_device("Metal")],
            has_metal: true,
            has_cuda: false,
            has_vulkan: false,
            is_apple_silicon: true,
        };
        assert!(info.has_metal);
    }

    #[test]
    fn test_has_vulkan_flag_can_be_set() {
        let info = SystemInfo {
            total_ram_bytes: 0,
            available_ram_bytes: 0,
            gguf_devices: vec![sample_device("Vulkan")],
            has_metal: false,
            has_cuda: false,
            has_vulkan: true,
            is_apple_silicon: false,
        };
        assert!(info.has_vulkan);
    }
}
