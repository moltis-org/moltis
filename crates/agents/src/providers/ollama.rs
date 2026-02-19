use super::DiscoveredModel;

fn normalize_ollama_api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagEntry {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsPayload {
    #[serde(default)]
    models: Vec<OllamaTagEntry>,
}

async fn discover_ollama_models_from_api(base_url: String) -> anyhow::Result<Vec<DiscoveredModel>> {
    let api_base = normalize_ollama_api_base_url(&base_url);
    let endpoint = format!("{}/api/tags", api_base.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?
        .get(&endpoint)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("ollama model discovery failed HTTP {status}");
    }

    let payload: OllamaTagsPayload = response.json().await?;
    let mut models: Vec<DiscoveredModel> = payload
        .models
        .into_iter()
        .map(|entry| entry.name.trim().to_string())
        .filter(|model| !model.is_empty())
        .map(|model| DiscoveredModel::new(model.clone(), model))
        .collect();
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    Ok(models)
}

pub(super) fn discover_ollama_models(base_url: &str) -> anyhow::Result<Vec<DiscoveredModel>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let base_url = base_url.to_string();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(discover_ollama_models_from_api(base_url)));
        let _ = tx.send(result);
    });

    rx.recv()
        .map_err(|err| anyhow::anyhow!("ollama model discovery worker failed: {err}"))?
}
