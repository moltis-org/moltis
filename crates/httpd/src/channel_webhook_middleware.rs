use {
    axum::{
        http::StatusCode,
        response::{IntoResponse as _, Response},
    },
    moltis_channels::channel_webhook_middleware::ChannelWebhookRejection,
};

/// Convert a webhook rejection into an HTTP response for channel routes.
pub fn rejection_into_response(rejection: ChannelWebhookRejection) -> Response {
    match rejection {
        ChannelWebhookRejection::BadSignature(ref msg) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "ok": false, "error": msg })),
        )
            .into_response(),
        ChannelWebhookRejection::StaleTimestamp {
            age_seconds,
            max_seconds,
        } => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": format!(
                    "request timestamp too old ({age_seconds}s > {max_seconds}s max)"
                )
            })),
        )
            .into_response(),
        ChannelWebhookRejection::MissingHeaders(ref header) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": format!("missing required header: {header}")
            })),
        )
            .into_response(),
        ChannelWebhookRejection::Duplicate => (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "ok": true, "deduplicated": true })),
        )
            .into_response(),
        ChannelWebhookRejection::RateLimited { retry_after } => {
            let secs = retry_after.as_secs().max(1);
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": "rate limited",
                    "retry_after_seconds": secs
                })),
            )
                .into_response();
            if let Ok(val) = secs.to_string().parse() {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, val);
            }
            resp
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rejection_into_response_status_codes() {
        let bad_sig = rejection_into_response(ChannelWebhookRejection::BadSignature("test".into()));
        assert_eq!(bad_sig.status(), StatusCode::UNAUTHORIZED);

        let stale = rejection_into_response(ChannelWebhookRejection::StaleTimestamp {
            age_seconds: 400,
            max_seconds: 300,
        });
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

        let missing =
            rejection_into_response(ChannelWebhookRejection::MissingHeaders("x-sig".into()));
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

        let dup = rejection_into_response(ChannelWebhookRejection::Duplicate);
        assert_eq!(dup.status(), StatusCode::OK);

        let rate = rejection_into_response(ChannelWebhookRejection::RateLimited {
            retry_after: std::time::Duration::from_secs(30),
        });
        assert_eq!(rate.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            rate.headers().get(axum::http::header::RETRY_AFTER).unwrap(),
            "30"
        );
    }
}
