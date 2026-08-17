use {anyhow::Result, clap::Subcommand};

#[derive(Subcommand)]
pub enum ProviderAction {
    /// Inspect non-secret provider model metadata.
    Inspect {
        /// Provider name, currently "github-copilot".
        provider: String,
    },
}

pub async fn handle_providers(action: ProviderAction) -> Result<()> {
    match action {
        ProviderAction::Inspect { provider } => inspect_provider(&provider).await,
    }
}

async fn inspect_provider(provider: &str) -> Result<()> {
    match provider {
        "github-copilot" => inspect_github_copilot().await,
        other => anyhow::bail!("unsupported provider inspection target: {other}"),
    }
}

fn provider_inspection_payload(
    models: Vec<moltis_providers::DiscoveredModel>,
) -> Vec<serde_json::Value> {
    models
        .into_iter()
        .map(|model| {
            let capabilities = model
                .capabilities
                .unwrap_or_else(|| moltis_providers::ModelCapabilities::infer(&model.id));
            serde_json::json!({
                "id": model.id,
                "displayName": model.display_name,
                "createdAt": model.created_at,
                "recommended": model.recommended,
                "capabilities": capabilities,
            })
        })
        .collect()
}

#[cfg(feature = "provider-github-copilot")]
async fn inspect_github_copilot() -> Result<()> {
    let models =
        tokio::task::spawn_blocking(moltis_providers::github_copilot::live_models).await??;
    let payload = provider_inspection_payload(models);
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

#[cfg(not(feature = "provider-github-copilot"))]
async fn inspect_github_copilot() -> Result<()> {
    anyhow::bail!("this build does not include GitHub Copilot support")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn provider_inspection_payload_infers_missing_capabilities() {
        let payload = provider_inspection_payload(vec![
            moltis_providers::DiscoveredModel::new("gpt-5.4", "GPT-5.4")
                .with_created_at(Some(123))
                .with_recommended(true),
        ]);

        assert_eq!(payload[0]["id"], "gpt-5.4");
        assert_eq!(payload[0]["displayName"], "GPT-5.4");
        assert_eq!(payload[0]["createdAt"], 123);
        assert_eq!(payload[0]["recommended"], true);
        assert_eq!(payload[0]["capabilities"]["context_window"], 128_000);
    }
}
