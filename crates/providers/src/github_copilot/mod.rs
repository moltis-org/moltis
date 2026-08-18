mod auth_prefetch;
mod catalog;
mod diagnostics;
mod discovery;
mod endpoints;
mod provider;

pub use {
    auth_prefetch::prefetch_api_token_metadata,
    catalog::default_model_catalog,
    provider::{
        DeviceCodeResponse, GitHubCopilotProvider, available_models, has_stored_tokens,
        live_models, start_model_discovery,
    },
};
