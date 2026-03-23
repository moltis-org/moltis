use std::{fs::File, net::SocketAddr, sync::Arc};

use {
    a2::{
        DefaultNotificationBuilder, Endpoint, NotificationBuilder, NotificationOptions,
        client::ClientConfig,
        request::notification::{Priority, PushType},
    },
    anyhow::{Context, Result},
    axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    },
    clap::Parser,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
};

/// Privacy-preserving APNS push relay for Moltis gateways.
///
/// Holds a single Apple .p8 key and forwards opaque "wake up" silent
/// pushes on behalf of self-hosted gateways. The relay never sees
/// message text or metadata.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Address to bind the HTTP server to.
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    /// Port to listen on.
    #[arg(long, default_value_t = 8090)]
    port: u16,

    /// Path to the Apple .p8 private key file.
    #[arg(long)]
    key_path: String,

    /// Apple key identifier (from App Store Connect).
    #[arg(long)]
    key_id: String,

    /// Apple team identifier.
    #[arg(long)]
    team_id: String,

    /// iOS app bundle identifier (apns-topic).
    #[arg(long)]
    bundle_id: String,

    /// Optional shared secret that gateways must send in the
    /// Authorization header. If omitted, the relay accepts all requests.
    #[arg(long)]
    auth_token: Option<String>,
}

struct AppState {
    apns: a2::Client,
    bundle_id: String,
    auth_token: Option<String>,
}

#[derive(Deserialize)]
struct PushRequest {
    device_token: String,
    #[serde(default)]
    environment: ApnsEnvironment,
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
enum ApnsEnvironment {
    #[default]
    Production,
    Sandbox,
}

#[derive(Serialize)]
struct PushResponse {
    status: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let mut key_file = File::open(&args.key_path).context("failed to open .p8 key file")?;

    let client_config = ClientConfig::new(Endpoint::Production);
    let apns = a2::Client::token(&mut key_file, &args.key_id, &args.team_id, client_config)
        .context("failed to create APNS client")?;

    let state = Arc::new(AppState {
        apns,
        bundle_id: args.bundle_id,
        auth_token: args.auth_token,
    });

    let app = Router::new()
        .route("/push", post(handle_push))
        .route("/health", get(handle_health))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", args.bind, args.port)
        .parse()
        .context("invalid bind address")?;

    tracing::info!(%addr, "courier listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_health() -> StatusCode {
    StatusCode::OK
}

/// Extract bearer token from Authorization header and validate against expected.
///
/// Uses SHA-256 hash comparison to avoid timing side-channels.
/// Returns `true` if no auth is configured (open relay) or if the token matches.
fn check_auth(expected: &Option<String>, headers: &HeaderMap) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    provided.is_some_and(|token| {
        let expected_hash = Sha256::digest(expected.as_bytes());
        let provided_hash = Sha256::digest(token.as_bytes());
        expected_hash == provided_hash
    })
}

async fn handle_push(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !check_auth(&state.auth_token, &headers) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "unauthorized".to_string(),
            }),
        ));
    }

    if req.device_token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "missing device_token".to_string(),
            }),
        ));
    }

    // Build silent notification: content-available: 1, no alert/badge/sound.
    let builder = DefaultNotificationBuilder::new().set_content_available();

    let options = NotificationOptions {
        apns_topic: Some(&state.bundle_id),
        apns_push_type: Some(PushType::Background),
        apns_priority: Some(Priority::Normal),
        ..Default::default()
    };

    let payload = builder.build(&req.device_token, options);

    if matches!(req.environment, ApnsEnvironment::Sandbox) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse {
                error: "sandbox environment is not supported by this relay".to_string(),
            }),
        ));
    }

    match state.apns.send(payload).await {
        Ok(response) => {
            tracing::debug!(code = response.code, "apns response");
            Ok(Json(PushResponse {
                status: "sent".to_string(),
            }))
        },
        Err(e) => {
            tracing::error!("apns error: {e}");
            Err((
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: "push delivery failed".to_string(),
                }),
            ))
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── Serde tests ─────────────────────────────────────────────────────────

    #[test]
    fn parse_push_request_production() {
        let json = r#"{"device_token": "abc123"}"#;
        let req: PushRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.device_token, "abc123");
        assert!(matches!(req.environment, ApnsEnvironment::Production));
    }

    #[test]
    fn parse_push_request_sandbox() {
        let json = r#"{"device_token": "abc123", "environment": "sandbox"}"#;
        let req: PushRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.device_token, "abc123");
        assert!(matches!(req.environment, ApnsEnvironment::Sandbox));
    }

    #[test]
    fn parse_push_request_missing_token() {
        let json = r#"{}"#;
        let result: Result<PushRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn error_response_serializes() {
        let resp = ErrorResponse {
            error: "test error".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("test error"));
    }

    #[test]
    fn push_response_serializes() {
        let resp = PushResponse {
            status: "sent".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("sent"));
    }

    #[test]
    fn apns_environment_default_is_production() {
        assert!(matches!(
            ApnsEnvironment::default(),
            ApnsEnvironment::Production
        ));
    }

    // ── Auth validation tests ───────────────────────────────────────────────

    #[test]
    fn check_auth_no_token_configured_accepts_all() {
        let headers = HeaderMap::new();
        assert!(check_auth(&None, &headers));
    }

    #[test]
    fn check_auth_valid_bearer_token() {
        let expected = Some("my-secret".to_string());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer my-secret".parse().unwrap());
        assert!(check_auth(&expected, &headers));
    }

    #[test]
    fn check_auth_wrong_bearer_token() {
        let expected = Some("my-secret".to_string());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer wrong-token".parse().unwrap());
        assert!(!check_auth(&expected, &headers));
    }

    #[test]
    fn check_auth_missing_authorization_header() {
        let expected = Some("my-secret".to_string());
        let headers = HeaderMap::new();
        assert!(!check_auth(&expected, &headers));
    }

    #[test]
    fn check_auth_non_bearer_scheme() {
        let expected = Some("my-secret".to_string());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());
        assert!(!check_auth(&expected, &headers));
    }

    #[test]
    fn check_auth_empty_bearer_value() {
        let expected = Some("my-secret".to_string());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer ".parse().unwrap());
        assert!(!check_auth(&expected, &headers));
    }

    #[test]
    fn check_auth_bearer_only_no_space() {
        let expected = Some("my-secret".to_string());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer".parse().unwrap());
        assert!(!check_auth(&expected, &headers));
    }
}
