use {
    super::{api_error_response, configured_secret},
    axum::{Json, http::StatusCode, response::IntoResponse},
    moltis_config::{CoderEnvSources, MoltisConfig, schema::SandboxConfig, validate_coder_url},
    secrecy::{ExposeSecret, Secret},
    serde::Deserialize,
};

const INVALID_REMOTE_BACKEND_CONFIG: &str = "remote_backend_invalid_config";

enum RemoteBackendSaveError {
    Invalid(String),
    Save(moltis_config::Error),
}

impl From<moltis_config::Error> for RemoteBackendSaveError {
    fn from(error: moltis_config::Error) -> Self {
        Self::Save(error)
    }
}

#[derive(Debug, Deserialize)]
pub struct RemoteBackendUpdateRequest {
    /// Which backend: "vercel", "daytona", "coder", or "_global".
    backend: String,
    config: RemoteBackendConfigUpdate,
}

#[derive(Debug, Default, Deserialize)]
struct RemoteBackendConfigUpdate {
    backend: Option<String>,
    token: Option<Secret<String>>,
    api_key: Option<Secret<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    project_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    team_id: Option<Option<String>>,
    runtime: Option<String>,
    timeout_ms: Option<u64>,
    vcpus: Option<u64>,
    api_url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    target: Option<Option<String>>,
    url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    organization: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    user: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    template_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    template_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    workspace_prefix: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    ttl_ms: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    size: Option<Option<String>>,
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn nonempty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn normalized(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn configured_coder_token(token: &Option<Secret<String>>) -> bool {
    token
        .as_ref()
        .is_some_and(|token| !token.expose_secret().trim().is_empty())
}

fn coder_template_configured(sandbox: &SandboxConfig) -> bool {
    nonempty(sandbox.coder_template_id.as_deref())
        || nonempty(sandbox.coder_template_name.as_deref())
}

pub(super) fn coder_available(sandbox: &SandboxConfig) -> bool {
    nonempty(sandbox.coder_url.as_deref())
        && configured_coder_token(&sandbox.coder_token)
        && coder_template_configured(sandbox)
}

fn remote_backends_payload(
    config: &MoltisConfig,
    coder_env_sources: CoderEnvSources,
) -> serde_json::Value {
    let sb = &config.tools.exec.sandbox;
    let vercel_configured = configured_secret(&sb.vercel_token);
    let vercel_from_env =
        std::env::var("VERCEL_TOKEN").is_ok() || std::env::var("VERCEL_OIDC_TOKEN").is_ok();
    let daytona_configured = configured_secret(&sb.daytona_api_key);
    let daytona_from_env = std::env::var("DAYTONA_API_KEY").is_ok();
    let coder_token_configured = configured_coder_token(&sb.coder_token);
    let coder_url_configured = nonempty(sb.coder_url.as_deref());
    let coder_template_configured = coder_template_configured(sb);
    serde_json::json!({
        "backend": sb.backend,
        "vercel": {
            "configured": vercel_configured,
            "from_env": vercel_from_env,
            "project_id": sb.vercel_project_id,
            "team_id": sb.vercel_team_id,
            "runtime": sb.vercel_runtime.as_deref().unwrap_or("node24"),
            "timeout_ms": sb.vercel_timeout_ms.unwrap_or(300_000),
            "vcpus": sb.vercel_vcpus.unwrap_or(2),
        },
        "daytona": {
            "configured": daytona_configured,
            "from_env": daytona_from_env,
            "api_url": sb.daytona_api_url.as_deref().unwrap_or("https://app.daytona.io/api"),
            "target": sb.daytona_target,
        },
        "coder": {
            "configured": coder_available(sb),
            "url_configured": coder_url_configured,
            "url_from_env": coder_env_sources.url,
            "url": sb.coder_url,
            "token_configured": coder_token_configured,
            "token_from_env": coder_env_sources.token,
            "template_configured": coder_template_configured,
            "organization": sb.coder_organization,
            "user": sb.coder_user.as_deref().unwrap_or("me"),
            "template_id": sb.coder_template_id,
            "template_name": sb.coder_template_name,
            "workspace_prefix": sb.coder_workspace_prefix.as_deref().unwrap_or("moltis"),
            "ttl_ms": sb.coder_ttl_ms,
            "size": sb.coder_size,
        },
    })
}

fn apply_remote_backend_update(config: &mut MoltisConfig, body: &RemoteBackendUpdateRequest) {
    let sb = &mut config.tools.exec.sandbox;
    if let Some(value) = body.config.backend.as_deref() {
        sb.backend = value.to_string();
    }
    match body.backend.as_str() {
        "vercel" => {
            if let Some(value) = body.config.token.clone() {
                sb.vercel_token = Some(value);
            }
            if let Some(value) = body.config.project_id.clone() {
                sb.vercel_project_id = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.team_id.clone() {
                sb.vercel_team_id = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.runtime.as_deref() {
                sb.vercel_runtime = Some(value.to_string());
            }
            if let Some(value) = body.config.timeout_ms {
                sb.vercel_timeout_ms = Some(value);
            }
            if let Some(value) = body.config.vcpus {
                sb.vercel_vcpus = Some(value as u32);
            }
        },
        "daytona" => {
            if let Some(value) = body.config.api_key.clone() {
                sb.daytona_api_key = Some(value);
            }
            if let Some(value) = body.config.api_url.as_deref() {
                sb.daytona_api_url = Some(value.to_string());
            }
            if let Some(value) = body.config.target.clone() {
                sb.daytona_target = value.and_then(|value| normalized(Some(value)));
            }
        },
        "coder" => {
            if let Some(value) = body.config.token.clone() {
                sb.coder_token = Some(value);
            }
            if let Some(value) = body.config.url.as_deref() {
                sb.coder_url = Some(value.trim().to_string());
            }
            if let Some(value) = body.config.organization.clone() {
                sb.coder_organization = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.user.clone() {
                sb.coder_user = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.template_id.clone() {
                sb.coder_template_id = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.template_name.clone() {
                sb.coder_template_name = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.workspace_prefix.clone() {
                sb.coder_workspace_prefix = value.and_then(|value| normalized(Some(value)));
            }
            if let Some(value) = body.config.ttl_ms {
                sb.coder_ttl_ms = value;
            }
            if let Some(value) = body.config.size.clone() {
                sb.coder_size = value.and_then(|value| normalized(Some(value)));
            }
        },
        _ => {},
    }
}

fn validate_remote_backend_update(
    config: &MoltisConfig,
    body: &RemoteBackendUpdateRequest,
) -> Result<(), String> {
    if !matches!(
        body.backend.as_str(),
        "vercel" | "daytona" | "coder" | "_global"
    ) {
        return Err(format!("unsupported remote backend {:?}", body.backend));
    }

    let mut candidate = config.clone();
    apply_remote_backend_update(&mut candidate, body);
    if body.backend != "coder" && body.config.backend.as_deref() != Some("coder") {
        return Ok(());
    }

    let sandbox = &candidate.tools.exec.sandbox;
    let url = sandbox
        .coder_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Coder URL is required".to_string())?;
    validate_coder_url(url)?;
    if !configured_coder_token(&sandbox.coder_token) {
        return Err("Coder session token is required".into());
    }
    if !coder_template_configured(sandbox) {
        return Err("Coder template ID or template name is required".into());
    }
    if sandbox.coder_ttl_ms.is_some_and(|ttl_ms| ttl_ms < 0) {
        return Err("Coder TTL must be zero or a positive number of milliseconds".into());
    }
    Ok(())
}

pub async fn api_get_remote_backends_handler() -> impl IntoResponse {
    let (config, coder_env_sources) = moltis_config::discover_and_load_with_coder_env_sources();
    Json(remote_backends_payload(&config, coder_env_sources))
}

pub async fn api_set_remote_backend_handler(
    Json(body): Json<RemoteBackendUpdateRequest>,
) -> impl IntoResponse {
    let update_result = moltis_config::update_config_fallible(|config| {
        validate_remote_backend_update(config, &body).map_err(RemoteBackendSaveError::Invalid)?;
        apply_remote_backend_update(config, &body);
        Ok(())
    });
    match update_result {
        Ok(saved_path) => {
            let (config, coder_env_sources) =
                moltis_config::discover_and_load_with_coder_env_sources();
            Json(serde_json::json!({
                "ok": true,
                "restart_required": true,
                "config_path": saved_path.display().to_string(),
                "config": remote_backends_payload(&config, coder_env_sources),
            }))
            .into_response()
        },
        Err(RemoteBackendSaveError::Invalid(error)) => api_error_response(
            StatusCode::BAD_REQUEST,
            INVALID_REMOTE_BACKEND_CONFIG,
            error,
        ),
        Err(RemoteBackendSaveError::Save(error)) => api_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "remote_backend_save_failed",
            error.to_string(),
        ),
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret};

    fn coder_request(json: serde_json::Value) -> RemoteBackendUpdateRequest {
        serde_json::from_value(serde_json::json!({
            "backend": "coder",
            "config": json,
        }))
        .unwrap()
    }

    #[test]
    fn coder_update_persists_all_core_fields() {
        let request = coder_request(serde_json::json!({
            "url": "https://coder.example.com",
            "token": "secret",
            "organization": "engineering",
            "user": "me",
            "template_id": null,
            "template_name": "devbox",
            "workspace_prefix": "moltis",
            "ttl_ms": 600000,
            "size": "large",
        }));
        let mut config = MoltisConfig::default();
        apply_remote_backend_update(&mut config, &request);
        let sandbox = &config.tools.exec.sandbox;

        assert_eq!(
            sandbox.coder_url.as_deref(),
            Some("https://coder.example.com")
        );
        assert_eq!(sandbox.coder_organization.as_deref(), Some("engineering"));
        assert_eq!(sandbox.coder_user.as_deref(), Some("me"));
        assert!(sandbox.coder_template_id.is_none());
        assert_eq!(sandbox.coder_template_name.as_deref(), Some("devbox"));
        assert_eq!(sandbox.coder_workspace_prefix.as_deref(), Some("moltis"));
        assert_eq!(sandbox.coder_ttl_ms, Some(600_000));
        assert_eq!(sandbox.coder_size.as_deref(), Some("large"));
        assert_eq!(
            sandbox
                .coder_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("secret")
        );
    }

    #[test]
    fn coder_update_preserves_an_omitted_secret() {
        let mut config = MoltisConfig::default();
        config.tools.exec.sandbox.coder_token = Some(Secret::new("existing".into()));
        let request = coder_request(serde_json::json!({
            "url": "https://coder.example.com",
            "template_name": "devbox",
        }));

        apply_remote_backend_update(&mut config, &request);
        assert_eq!(
            config
                .tools
                .exec
                .sandbox
                .coder_token
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("existing")
        );
    }

    #[test]
    fn coder_update_rejects_insecure_url_before_persistence() {
        let request = coder_request(serde_json::json!({
            "url": "http://coder.example.com",
            "token": "secret",
            "template_name": "devbox",
        }));
        let config = MoltisConfig::default();

        let error = validate_remote_backend_update(&config, &request).unwrap_err();
        assert!(error.contains("HTTPS"));
        assert!(config.tools.exec.sandbox.coder_url.is_none());
    }

    #[test]
    fn validation_uses_the_fresh_config_candidate() {
        let request = coder_request(serde_json::json!({
            "template_id": null,
            "template_name": null,
        }));
        let mut config = MoltisConfig::default();
        let sandbox = &mut config.tools.exec.sandbox;
        sandbox.coder_url = Some("https://coder.example.com".into());
        sandbox.coder_token = Some(Secret::new("existing".into()));
        sandbox.coder_template_name = Some("devbox".into());

        let error = validate_remote_backend_update(&config, &request).unwrap_err();

        assert_eq!(error, "Coder template ID or template name is required");
        assert_eq!(
            config.tools.exec.sandbox.coder_template_name.as_deref(),
            Some("devbox")
        );
    }

    #[test]
    fn coder_availability_requires_url_token_and_template() {
        let mut sandbox = SandboxConfig::default();
        assert!(!coder_available(&sandbox));
        sandbox.coder_url = Some("https://coder.example.com".into());
        assert!(!coder_available(&sandbox));
        sandbox.coder_token = Some(Secret::new("token".into()));
        assert!(!coder_available(&sandbox));
        sandbox.coder_template_name = Some("devbox".into());
        assert!(coder_available(&sandbox));

        sandbox.coder_token = Some(Secret::new(" \t ".into()));
        assert!(!coder_available(&sandbox));
    }

    #[test]
    fn coder_payload_requires_template_and_reports_supplied_sources() {
        let mut config = MoltisConfig::default();
        let sandbox = &mut config.tools.exec.sandbox;
        sandbox.coder_url = Some("https://coder.example.com".into());
        sandbox.coder_token = Some(Secret::new("token".into()));

        let payload = remote_backends_payload(&config, CoderEnvSources {
            url: true,
            token: false,
        });
        assert_eq!(payload["coder"]["configured"], false);
        assert_eq!(payload["coder"]["template_configured"], false);
        assert_eq!(payload["coder"]["url_from_env"], true);
        assert_eq!(payload["coder"]["token_from_env"], false);
        assert!(payload["coder"].get("token").is_none());
    }

    #[test]
    fn coder_update_rejects_whitespace_token_before_persistence() {
        let request = coder_request(serde_json::json!({
            "url": "https://coder.example.com",
            "token": "   ",
            "template_name": "devbox",
        }));
        let config = MoltisConfig::default();

        let error = validate_remote_backend_update(&config, &request).unwrap_err();
        assert_eq!(error, "Coder session token is required");
        assert!(config.tools.exec.sandbox.coder_token.is_none());
    }

    #[test]
    fn coder_update_rejects_negative_ttl_and_accepts_zero() {
        let config = MoltisConfig::default();
        let negative = coder_request(serde_json::json!({
            "url": "https://coder.example.com",
            "token": "secret",
            "template_id": "template-id",
            "ttl_ms": -1,
        }));
        let error = validate_remote_backend_update(&config, &negative).unwrap_err();
        assert!(error.contains("zero or a positive"));
        assert!(config.tools.exec.sandbox.coder_ttl_ms.is_none());

        let zero = coder_request(serde_json::json!({
            "url": "https://coder.example.com",
            "token": "secret",
            "template_id": "template-id",
            "ttl_ms": 0,
        }));
        assert!(validate_remote_backend_update(&config, &zero).is_ok());
    }
}
