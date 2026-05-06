//! Runtime GGUF device probing via llama.cpp.

use {
    llama_cpp_2::{LlamaCppError, list_llama_ggml_backend_devices, llama_backend::LlamaBackend},
    tracing::debug,
};

/// A GGUF-capable runtime device exposed by llama.cpp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufRuntimeDevice {
    pub index: usize,
    pub name: String,
    pub description: String,
    pub backend: String,
    pub memory_total_bytes: u64,
    pub memory_free_bytes: u64,
}

/// Summary of GPU backends actually available to GGUF inference at runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GgufRuntimeSupport {
    pub devices: Vec<GgufRuntimeDevice>,
    pub has_metal: bool,
    pub has_cuda: bool,
    pub has_vulkan: bool,
}

impl GgufRuntimeSupport {
    #[must_use]
    pub fn from_devices(devices: Vec<GgufRuntimeDevice>) -> Self {
        let has_metal = devices
            .iter()
            .any(|device| device.backend.eq_ignore_ascii_case("metal"));
        let has_cuda = devices
            .iter()
            .any(|device| device.backend.eq_ignore_ascii_case("cuda"));
        let has_vulkan = devices
            .iter()
            .any(|device| device.backend.eq_ignore_ascii_case("vulkan"));

        Self {
            devices,
            has_metal,
            has_cuda,
            has_vulkan,
        }
    }
}

/// Detect GGUF runtime devices and summarize available acceleration backends.
#[must_use]
pub fn detect_runtime_support() -> GgufRuntimeSupport {
    let backend_guard = match LlamaBackend::init() {
        Ok(backend) => Some(backend),
        Err(LlamaCppError::BackendAlreadyInitialized) => {
            debug!("llama backend already initialized; reusing for GGUF device probe");
            None
        },
        Err(error) => {
            debug!(%error, "failed to initialize llama backend for GGUF device probe");
            return GgufRuntimeSupport::default();
        },
    };

    let devices = list_llama_ggml_backend_devices()
        .into_iter()
        .filter(|device| !device.backend.eq_ignore_ascii_case("cpu"))
        .map(|device| GgufRuntimeDevice {
            index: device.index,
            name: device.name,
            description: device.description,
            backend: device.backend,
            memory_total_bytes: device.memory_total as u64,
            memory_free_bytes: device.memory_free as u64,
        })
        .collect();

    drop(backend_guard);

    GgufRuntimeSupport::from_devices(devices)
}

/// Parse `/proc/meminfo` as a fallback when `sysinfo` returns 0 (common in
/// Docker containers with restrictive cgroup settings).
///
/// Returns `(total_bytes, available_bytes)` or `None` if the file is absent or
/// unparseable.
pub(crate) fn read_proc_meminfo() -> Option<(u64, u64)> {
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
pub(crate) fn parse_meminfo_kb(value: &str) -> Option<u64> {
    value.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::{GgufRuntimeDevice, GgufRuntimeSupport};

    #[test]
    fn test_from_devices_detects_known_backends() {
        let support = GgufRuntimeSupport::from_devices(vec![
            GgufRuntimeDevice {
                index: 0,
                name: "Metal0".into(),
                description: "Apple GPU".into(),
                backend: "Metal".into(),
                memory_total_bytes: 1,
                memory_free_bytes: 1,
            },
            GgufRuntimeDevice {
                index: 1,
                name: "CUDA0".into(),
                description: "NVIDIA".into(),
                backend: "CUDA".into(),
                memory_total_bytes: 1,
                memory_free_bytes: 1,
            },
            GgufRuntimeDevice {
                index: 2,
                name: "Vulkan0".into(),
                description: "Intel".into(),
                backend: "Vulkan".into(),
                memory_total_bytes: 1,
                memory_free_bytes: 1,
            },
        ]);

        assert!(support.has_metal);
        assert!(support.has_cuda);
        assert!(support.has_vulkan);
        assert!(!support.devices.is_empty());
    }

    #[test]
    fn test_from_devices_is_case_insensitive() {
        let support = GgufRuntimeSupport::from_devices(vec![GgufRuntimeDevice {
            index: 0,
            name: "vk0".into(),
            description: "GPU".into(),
            backend: "vUlKaN".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }]);

        assert!(support.has_vulkan);
        assert!(!support.devices.is_empty());
    }

    #[test]
    fn test_from_devices_handles_unknown_gpu_backends() {
        let support = GgufRuntimeSupport::from_devices(vec![GgufRuntimeDevice {
            index: 0,
            name: "ROCm0".into(),
            description: "AMD".into(),
            backend: "ROCm".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }]);

        assert!(!support.has_metal);
        assert!(!support.has_cuda);
        assert!(!support.has_vulkan);
        assert!(!support.devices.is_empty());
    }
}
