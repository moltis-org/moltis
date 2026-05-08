use std::sync::Arc;

use {
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    tracing::{info, warn},
};

use crate::auth;

pub(super) struct CredentialEnvVarProvider {
    pub(super) store: Arc<auth::CredentialStore>,
    pub(super) gateway_url: Option<String>,
    pub(super) sandbox_api_key: Option<Secret<String>>,
}

#[async_trait]
impl moltis_tools::exec::EnvVarProvider for CredentialEnvVarProvider {
    async fn get_env_vars(&self) -> Vec<(String, Secret<String>)> {
        let mut vars = match self.store.get_all_env_values().await {
            Ok(values) => values
                .into_iter()
                .filter(|(key, _)| !key.starts_with("__MOLTIS_"))
                .map(|(key, value)| (key, Secret::new(value)))
                .collect(),
            Err(error) => {
                warn!(error = %error, "failed to load runtime env overrides for tools");
                Vec::new()
            },
        };

        if let Some(ref url) = self.gateway_url {
            vars.push(("MOLTIS_GATEWAY_URL".into(), Secret::new(url.clone())));
        }
        if let Some(ref key) = self.sandbox_api_key {
            vars.push((
                "MOLTIS_API_KEY".into(),
                Secret::new(key.expose_secret().clone()),
            ));
        }

        vars
    }
}

pub(super) async fn ensure_sandbox_api_key(store: &auth::CredentialStore) -> Option<String> {
    if let Ok(vals) = store.get_all_env_values().await
        && let Some((_, key)) = vals.iter().find(|(k, _)| k == "__MOLTIS_SANDBOX_API_KEY")
    {
        return Some(key.clone());
    }

    let scopes = vec!["operator.read".to_string(), "operator.write".to_string()];
    match store.create_api_key("sandbox-ctl", Some(&scopes)).await {
        Ok((_id, raw_key)) => {
            if let Err(e) = store
                .set_env_var("__MOLTIS_SANDBOX_API_KEY", &raw_key)
                .await
            {
                warn!(error = %e, "failed to persist sandbox API key");
            }
            info!("created sandbox-ctl API key for moltis-ctl");
            Some(raw_key)
        },
        Err(e) => {
            warn!(error = %e, "failed to create sandbox API key");
            None
        },
    }
}
