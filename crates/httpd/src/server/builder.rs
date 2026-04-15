//! Gateway router construction and finalization.
//!
//! Contains the builder functions that wire routes, auth, middleware, and
//! state into a complete `Router` ready for `axum::serve`.

use std::sync::Arc;

use {
    axum::{Router, routing::get},
    tower_http::set_header::SetResponseHeaderLayer,
};

use moltis_gateway::{
    auth_webauthn::SharedWebAuthnRegistry, methods::MethodRegistry, state::GatewayState,
};

use crate::auth_routes::{AuthState, auth_router};

use super::{
    AppState, GatewayBase,
    handlers::{health_handler, ws_upgrade_handler},
    middleware::{apply_middleware_stack, build_cors_layer},
};

#[cfg(feature = "ngrok")]
use super::ngrok::NgrokController;

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(feature = "push-notifications")]
pub(super) fn build_gateway_base_internal(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<moltis_gateway::push::PushService>>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> GatewayBase {
    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws/chat", get(ws_upgrade_handler))
        .route("/ws", get(ws_upgrade_handler));

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_registry: webauthn_registry.clone(),
            gateway_state: Arc::clone(&state),
            login_guard: crate::login_guard::LoginGuard::new(),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    #[cfg(feature = "graphql")]
    let graphql_schema = crate::graphql_routes::build_graphql_schema(Arc::clone(&state));
    #[cfg(feature = "ngrok")]
    let ngrok_runtime = Arc::new(tokio::sync::RwLock::new(None));
    #[cfg(feature = "ngrok")]
    let ngrok_controller = Arc::new(NgrokController::new(
        Arc::clone(&state),
        webauthn_registry.clone(),
        Arc::clone(&ngrok_runtime),
    ));

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
        webauthn_registry: webauthn_registry.clone(),
        #[cfg(feature = "ngrok")]
        ngrok_controller_owner: None,
        #[cfg(feature = "ngrok")]
        ngrok_controller: Arc::downgrade(&ngrok_controller),
        #[cfg(feature = "ngrok")]
        ngrok_runtime,
        push_service,
        #[cfg(feature = "graphql")]
        graphql_schema,
    };

    // GraphQL routes -- auth is handled by the global auth_gate in
    // finalize_gateway_app.
    #[cfg(feature = "graphql")]
    {
        router = router.route(
            "/graphql",
            get(crate::graphql_routes::graphql_get_handler)
                .post(crate::graphql_routes::graphql_handler),
        );
    }

    #[cfg(feature = "ngrok")]
    {
        (router, app_state, ngrok_controller)
    }
    #[cfg(not(feature = "ngrok"))]
    {
        (router, app_state)
    }
}

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(feature = "push-notifications")]
pub fn build_gateway_base(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<moltis_gateway::push::PushService>>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> (Router<AppState>, AppState) {
    #[cfg(feature = "ngrok")]
    let (router, mut app_state, ngrok_controller) =
        build_gateway_base_internal(state, methods, push_service, webauthn_registry);
    #[cfg(feature = "ngrok")]
    super::runtime::attach_ngrok_controller_owner(&mut app_state, &ngrok_controller);
    #[cfg(not(feature = "ngrok"))]
    let (router, app_state) =
        build_gateway_base_internal(state, methods, push_service, webauthn_registry);
    (router, app_state)
}

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(not(feature = "push-notifications"))]
pub(super) fn build_gateway_base_internal(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> GatewayBase {
    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws/chat", get(ws_upgrade_handler))
        .route("/ws", get(ws_upgrade_handler));

    // Add Prometheus metrics endpoint (unauthenticated for scraping).
    #[cfg(feature = "prometheus")]
    {
        router = router.route(
            "/metrics",
            get(crate::metrics_routes::prometheus_metrics_handler),
        );
    }

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_registry: webauthn_registry.clone(),
            gateway_state: Arc::clone(&state),
            login_guard: crate::login_guard::LoginGuard::new(),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    #[cfg(feature = "graphql")]
    let graphql_schema = crate::graphql_routes::build_graphql_schema(Arc::clone(&state));
    #[cfg(feature = "ngrok")]
    let ngrok_runtime = Arc::new(tokio::sync::RwLock::new(None));
    #[cfg(feature = "ngrok")]
    let ngrok_controller = Arc::new(NgrokController::new(
        Arc::clone(&state),
        webauthn_registry.clone(),
        Arc::clone(&ngrok_runtime),
    ));

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
        webauthn_registry: webauthn_registry.clone(),
        #[cfg(feature = "ngrok")]
        ngrok_controller_owner: None,
        #[cfg(feature = "ngrok")]
        ngrok_controller: Arc::downgrade(&ngrok_controller),
        #[cfg(feature = "ngrok")]
        ngrok_runtime,
        #[cfg(feature = "graphql")]
        graphql_schema,
    };

    // GraphQL routes -- auth is handled by the global auth_gate in
    // finalize_gateway_app.
    #[cfg(feature = "graphql")]
    {
        router = router.route(
            "/graphql",
            get(crate::graphql_routes::graphql_get_handler)
                .post(crate::graphql_routes::graphql_handler),
        );
    }

    #[cfg(feature = "ngrok")]
    {
        (router, app_state, ngrok_controller)
    }
    #[cfg(not(feature = "ngrok"))]
    {
        (router, app_state)
    }
}

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(not(feature = "push-notifications"))]
pub fn build_gateway_base(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> (Router<AppState>, AppState) {
    #[cfg(feature = "ngrok")]
    let (router, mut app_state, ngrok_controller) =
        build_gateway_base_internal(state, methods, webauthn_registry);
    #[cfg(feature = "ngrok")]
    super::runtime::attach_ngrok_controller_owner(&mut app_state, &ngrok_controller);
    #[cfg(not(feature = "ngrok"))]
    let (router, app_state) = build_gateway_base_internal(state, methods, webauthn_registry);
    (router, app_state)
}

/// Apply throttle, auth gate, middleware, and state to a base router,
/// producing the final `Router` ready for `axum::serve`.
pub fn finalize_gateway_app(
    router: Router<AppState>,
    app_state: AppState,
    http_request_logs: bool,
) -> Router {
    let cors = build_cors_layer();
    // Auth gate covers the entire router -- public paths are exempted inside
    // `is_public_path()`.  Only compiled when the web-ui feature is enabled
    // (matches the old architecture where auth_gate was global).
    #[cfg(feature = "web-ui")]
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::auth_middleware::auth_gate,
    ));
    // Vault guard blocks API requests when the vault is sealed (not
    // uninitialized). Applied after auth_gate so sealed state is checked
    // only for authenticated requests.
    #[cfg(feature = "vault")]
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::auth_middleware::vault_guard,
    ));
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::request_throttle::throttle_gate,
    ));
    // HSTS: instruct browsers to always use HTTPS once they've connected securely.
    let router = if app_state.gateway.is_secure() {
        use axum::http::{HeaderValue, header};
        router.layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
    } else {
        router
    };
    let router = apply_middleware_stack(router, cors, http_request_logs);
    router.with_state(app_state)
}

/// Convenience wrapper: build base + finalize in one call (used by tests).
#[cfg(feature = "push-notifications")]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<moltis_gateway::push::PushService>>,
    http_request_logs: bool,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> Router {
    let (router, app_state) = build_gateway_base(state, methods, push_service, webauthn_registry);
    finalize_gateway_app(router, app_state, http_request_logs)
}

/// Convenience wrapper: build base + finalize in one call (used by tests).
#[cfg(not(feature = "push-notifications"))]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    http_request_logs: bool,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> Router {
    let (router, app_state) = build_gateway_base(state, methods, webauthn_registry);
    finalize_gateway_app(router, app_state, http_request_logs)
}
