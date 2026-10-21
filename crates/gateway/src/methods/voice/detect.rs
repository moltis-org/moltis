//! Environment and service availability detection for voice providers.
//!
//! Two kinds of probe live here: host capability checks used to tell the user
//! whether a heavyweight local backend can run at all, and liveness checks for
//! the local servers the voice providers talk to.

use std::time::Duration;

/// Check if Python 3.10+ is available.
pub(crate) async fn check_python_version() -> serde_json::Value {
    // Try python3 first, then python
    for cmd in &["python3", "python"] {
        if let Ok(output) = tokio::process::Command::new(cmd)
            .arg("--version")
            .output()
            .await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Parse "Python 3.11.0" format
            if let Some(version) = version_str.strip_prefix("Python ") {
                let version = version.trim();
                // Check if version is 3.10+
                let parts: Vec<&str> = version.split('.').collect();
                if parts.len() >= 2
                    && let (Ok(major), Ok(minor)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                {
                    let sufficient = major > 3 || (major == 3 && minor >= 10);
                    return serde_json::json!({
                        "available": true,
                        "version": version,
                        "sufficient": sufficient,
                    });
                }
                return serde_json::json!({
                    "available": true,
                    "version": version,
                    "sufficient": false,
                });
            }
        }
    }
    serde_json::json!({
        "available": false,
        "version": null,
        "sufficient": false,
    })
}

/// Check CUDA availability via nvidia-smi.
pub(crate) async fn check_cuda_availability() -> serde_json::Value {
    // Check if nvidia-smi is available
    if let Ok(output) = tokio::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
        .await
        && output.status.success()
    {
        let info = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = info.trim().lines().collect();
        if let Some(first_gpu) = lines.first() {
            let parts: Vec<&str> = first_gpu.split(", ").collect();
            if parts.len() >= 2 {
                let gpu_name = parts[0].trim();
                let memory_mb: u64 = parts[1].trim().parse().unwrap_or(0);
                // vLLM needs ~9.5GB, recommend 10GB minimum
                let sufficient = memory_mb >= 10000;
                return serde_json::json!({
                    "available": true,
                    "gpu_name": gpu_name,
                    "memory_mb": memory_mb,
                    "sufficient": sufficient,
                });
            }
        }
        return serde_json::json!({
            "available": true,
            "gpu_name": null,
            "memory_mb": null,
            "sufficient": false,
        });
    }
    serde_json::json!({
        "available": false,
        "gpu_name": null,
        "memory_mb": null,
        "sufficient": false,
    })
}

/// Check if the system meets Voxtral Local requirements.
pub(crate) fn check_voxtral_compatibility(
    os: &str,
    arch: &str,
    python: &serde_json::Value,
    cuda: &serde_json::Value,
) -> (bool, Vec<String>) {
    let mut reasons = Vec::new();

    // vLLM primarily supports Linux
    let os_ok = os == "linux";
    if !os_ok {
        if os == "macos" {
            reasons.push("vLLM has limited macOS support. Linux is recommended.".into());
        } else if os == "windows" {
            reasons.push("vLLM requires WSL2 on Windows.".into());
        }
    }

    // Architecture check
    let arch_ok = arch == "x86_64";
    if !arch_ok && arch == "aarch64" {
        reasons.push("ARM64 has limited CUDA/vLLM support.".into());
    }

    // Python check
    let python_ok = python["sufficient"].as_bool().unwrap_or(false);
    if !python["available"].as_bool().unwrap_or(false) {
        reasons.push("Python is not installed. Install Python 3.10+.".into());
    } else if !python_ok {
        let ver = python["version"].as_str().unwrap_or("unknown");
        reasons.push(format!("Python {} is too old. Python 3.10+ required.", ver));
    }

    // CUDA check
    let cuda_ok = cuda["sufficient"].as_bool().unwrap_or(false);
    if !cuda["available"].as_bool().unwrap_or(false) {
        reasons.push("No NVIDIA GPU detected. CUDA GPU with 10GB+ VRAM required.".into());
    } else if !cuda_ok {
        let mem = cuda["memory_mb"].as_u64().unwrap_or(0);
        reasons.push(format!(
            "GPU has {}MB VRAM. 10GB+ recommended for Voxtral.",
            mem
        ));
    }

    // Overall compatibility
    let compatible = os_ok && arch_ok && python_ok && cuda_ok;

    (compatible, reasons)
}

pub(super) async fn check_binary_available(name: &str) -> Option<String> {
    // Try to find the binary in PATH
    if let Ok(output) = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

/// Check if Coqui TTS server is running.
pub(super) async fn check_coqui_server(endpoint: &str) -> bool {
    // Try to connect to the server's health endpoint
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // Coqui TTS server responds to GET /
    if let Ok(resp) = client.get(endpoint).send().await {
        return resp.status().is_success();
    }
    false
}

/// Check if a vLLM-Omni server is running (for VoxCPM).
///
/// The speech endpoint is an OpenAI-compatible base URL such as
/// `http://localhost:8000/v1`, while `/health` is served from the root.
pub(super) async fn check_voxcpm_server(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    let base = endpoint.trim_end_matches('/');
    let root = base.strip_suffix("/v1").unwrap_or(base);
    for url in [format!("{root}/health"), format!("{base}/audio/voices")] {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return true;
        }
    }
    false
}

/// Check if vLLM server is running (for Voxtral local).
pub(super) async fn check_vllm_server(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // vLLM exposes /health endpoint
    let health_url = format!("{}/health", endpoint.trim_end_matches('/'));
    if let Ok(resp) = client.get(&health_url).send().await {
        return resp.status().is_success();
    }
    false
}
