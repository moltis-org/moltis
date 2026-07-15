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
        ProviderAction::Inspect { provider } => inspect_provider(&provider),
    }
}

fn inspect_provider(provider: &str) -> Result<()> {
    match provider {
        "github-copilot" => inspect_github_copilot(),
        other => anyhow::bail!("unsupported provider inspection target: {other}"),
    }
}

#[cfg(feature = "provider-github-copilot")]
fn inspect_github_copilot() -> Result<()> {
    let models = moltis_providers::github_copilot::live_models()?;
    let payload: Vec<serde_json::Value> = models
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
        .collect();
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

#[cfg(not(feature = "provider-github-copilot"))]
fn inspect_github_copilot() -> Result<()> {
    anyhow::bail!("this build does not include GitHub Copilot support")
}
