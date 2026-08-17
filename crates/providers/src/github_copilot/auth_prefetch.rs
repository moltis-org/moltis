use {moltis_oauth::TokenStore, tracing::warn};

use super::provider::fetch_copilot_auth;

pub async fn prefetch_api_token_metadata(
    client: &reqwest::Client,
    token_store: &TokenStore,
) -> anyhow::Result<()> {
    if let Err(error) = token_store.delete("github-copilot-api") {
        warn!(%error, "failed to delete cached github-copilot API token before prefetch");
    }
    fetch_copilot_auth(client, token_store).await.map(|_| ())
}
