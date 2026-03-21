use {
    anyhow::Result,
    async_trait::async_trait,
    base64::{Engine, engine::general_purpose::STANDARD},
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
};

/// Tool that collects witness logs and returns them as a base64-encoded tar.gz.
#[derive(Default)]
pub struct WitnessDownloadTool;

impl WitnessDownloadTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for WitnessDownloadTool {
    fn name(&self) -> &str { "witness_download" }

    fn description(&self) -> &str {
        "Download recent zkperf witness logs as a base64-encoded tar.gz archive. \
         Use this when the user asks for witness data, debug logs, or tool execution traces."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "last_n": {
                    "type": "integer",
                    "description": "Number of most recent witness files to include (default: all)"
                },
                "tool_filter": {
                    "type": "string",
                    "description": "Only include witnesses for this tool name"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let dir = moltis_config::data_dir().join("witness");
        if !dir.exists() {
            return Ok(json!({ "error": "no witness directory found", "path": dir.display().to_string() }));
        }

        let last_n = params.get("last_n").and_then(|v| v.as_u64()).map(|n| n as usize);
        let tool_filter = params.get("tool_filter").and_then(|v| v.as_str());

        let mut entries: Vec<_> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter(|e| {
                tool_filter.map_or(true, |f| {
                    e.file_name().to_string_lossy().contains(f)
                })
            })
            .collect();

        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        if let Some(n) = last_n {
            entries.truncate(n);
        }

        if entries.is_empty() {
            return Ok(json!({ "count": 0, "message": "no matching witness files" }));
        }

        // Build tar.gz in memory
        let mut archive = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut archive, flate2::Compression::fast());
            let mut tar = tar::Builder::new(enc);
            for entry in &entries {
                let path = entry.path();
                let data = std::fs::read(&path)?;
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(
                    &mut header,
                    entry.file_name(),
                    data.as_slice(),
                )?;
            }
            tar.into_inner()?.finish()?;
        }

        Ok(json!({
            "count": entries.len(),
            "format": "tar.gz",
            "encoding": "base64",
            "data": STANDARD.encode(&archive),
            "witness_dir": dir.display().to_string(),
        }))
    }
}
