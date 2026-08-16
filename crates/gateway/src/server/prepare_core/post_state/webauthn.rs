use {
    std::sync::Arc,
    tracing::{info, warn},
};

use crate::server::helpers::instance_slug;

pub(super) async fn build_webauthn_registry(
    config: &moltis_config::MoltisConfig,
    port: u16,
) -> anyhow::Result<Option<crate::auth_webauthn::SharedWebAuthnRegistry>> {
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };

    let (external_rp_id, external_origin) = if let Some(ref ext_url) =
        config.server.effective_external_url()
    {
        match url::Url::parse(ext_url) {
            Ok(parsed) => {
                let host = parsed.host_str().unwrap_or_default().to_string();
                if host.is_empty() {
                    warn!(
                        "server.external_url '{ext_url}' parsed successfully but has no hostname; ignoring"
                    );
                    (None, None)
                } else {
                    (Some(host), Some(ext_url.clone()))
                }
            },
            Err(e) => {
                warn!("invalid server.external_url '{ext_url}': {e}");
                (None, None)
            },
        }
    } else {
        (None, None)
    };

    let explicit_rp_id = external_rp_id
        .or_else(|| std::env::var("MOLTIS_WEBAUTHN_RP_ID").ok())
        .or_else(|| std::env::var("APP_DOMAIN").ok())
        .or_else(|| std::env::var("RENDER_EXTERNAL_HOSTNAME").ok())
        .or_else(|| {
            std::env::var("FLY_APP_NAME")
                .ok()
                .map(|name| format!("{name}.fly.dev"))
        })
        .or_else(|| std::env::var("RAILWAY_PUBLIC_DOMAIN").ok());
    let explicit_origin = external_origin
        .or_else(|| std::env::var("MOLTIS_WEBAUTHN_ORIGIN").ok())
        .or_else(|| std::env::var("APP_URL").ok())
        .or_else(|| std::env::var("RENDER_EXTERNAL_URL").ok());

    let mut wa_registry = crate::auth_webauthn::WebAuthnRegistry::new();
    let mut any_ok = false;

    let mut try_add = |rp_id: &str, origin_str: &str, extras: &[webauthn_rs::prelude::Url]| {
        let rp_id = crate::auth_webauthn::normalize_host(rp_id);
        if rp_id.is_empty() || wa_registry.contains_host(&rp_id) {
            return;
        }
        let Ok(origin_url) = webauthn_rs::prelude::Url::parse(origin_str) else {
            warn!("invalid WebAuthn origin URL '{origin_str}'");
            return;
        };
        match crate::auth_webauthn::WebAuthnState::new(&rp_id, &origin_url, extras) {
            Ok(wa) => {
                info!(rp_id = %rp_id, origins = ?wa.get_allowed_origins(), "WebAuthn RP registered");
                wa_registry.add(rp_id.clone(), wa);
                any_ok = true;
            },
            Err(e) => warn!(rp_id = %rp_id, "failed to init WebAuthn: {e}"),
        }
    };

    if let Some(ref rp_id) = explicit_rp_id {
        let origin = explicit_origin
            .clone()
            .unwrap_or_else(|| format!("https://{rp_id}"));
        try_add(rp_id, &origin, &[]);
    } else {
        let localhost_origin = format!("{default_scheme}://localhost:{port}");
        let moltis_localhost: Vec<webauthn_rs::prelude::Url> = webauthn_rs::prelude::Url::parse(
            &format!("{default_scheme}://moltis.localhost:{port}"),
        )
        .into_iter()
        .collect();
        try_add("localhost", &localhost_origin, &moltis_localhost);

        let instance_slug_value = instance_slug(config);
        if instance_slug_value != "localhost" {
            let bot_origin = format!("{default_scheme}://{instance_slug_value}:{port}");
            try_add(&instance_slug_value, &bot_origin, &[]);

            let bot_local = format!("{instance_slug_value}.local");
            let bot_local_origin = format!("{default_scheme}://{bot_local}:{port}");
            try_add(&bot_local, &bot_local_origin, &[]);
        }

        if let Ok(hn) = hostname::get() {
            let hn_str = hn.to_string_lossy();
            if hn_str != "localhost" {
                let local_name = if hn_str.ends_with(".local") {
                    hn_str.to_string()
                } else {
                    format!("{hn_str}.local")
                };
                let local_origin = format!("{default_scheme}://{local_name}:{port}");
                try_add(&local_name, &local_origin, &[]);

                let bare = hn_str.strip_suffix(".local").unwrap_or(&hn_str);
                if bare != local_name {
                    let bare_origin = format!("{default_scheme}://{bare}:{port}");
                    try_add(bare, &bare_origin, &[]);
                }
            }
        }
    }

    if any_ok {
        info!(origins = ?wa_registry.get_all_origins(), "WebAuthn passkeys enabled");
        Ok(Some(Arc::new(tokio::sync::RwLock::new(wa_registry))))
    } else {
        Ok(None)
    }
}
