//! GraphQL HTTP handlers for the gateway.
//!
//! These handlers bridge `AppState` to the `moltis-graphql` schema, providing
//! GraphiQL on GET `/graphql`, query/mutation execution on POST `/graphql`,
//! and WebSocket subscriptions on GET `/graphql`.

use std::sync::Arc;

use {
    async_graphql::http::GraphiQLSource,
    axum::{
        Json,
        extract::{FromRequestParts, Request, State, WebSocketUpgrade},
        http::{HeaderMap, StatusCode, header},
        response::{Html, IntoResponse, Response},
    },
    serde_json::Value,
};

use crate::{server::AppState, state::GatewayState};

/// `SystemInfoService` implementation backed by the gateway's live state.
///
/// Covers methods that read gateway-internal data (connections, nodes, hooks,
/// heartbeat) rather than delegating to a domain service crate.
pub struct GatewaySystemInfoService {
    pub state: Arc<GatewayState>,
}

#[async_trait::async_trait]
impl moltis_service_traits::SystemInfoService for GatewaySystemInfoService {
    async fn health(&self) -> Result<Value, String> {
        let count = self.state.client_count().await;
        Ok(serde_json::json!({
            "ok": true,
            "connections": count,
        }))
    }

    async fn status(&self) -> Result<Value, String> {
        let inner = self.state.inner.read().await;
        Ok(serde_json::json!({
            "hostname": self.state.hostname,
            "version": self.state.version,
            "connections": inner.clients.len(),
            "uptimeMs": self.state.uptime_ms(),
        }))
    }

    async fn system_presence(&self) -> Result<Value, String> {
        let inner = self.state.inner.read().await;
        let clients: Vec<_> = inner
            .clients
            .values()
            .map(|c| {
                serde_json::json!({
                    "connId": c.conn_id,
                    "role": c.role(),
                    "connectedAt": c.connected_at.elapsed().as_secs(),
                })
            })
            .collect();
        let nodes: Vec<_> = inner
            .nodes
            .list()
            .iter()
            .map(|n| {
                serde_json::json!({
                    "nodeId": n.node_id,
                    "connId": n.conn_id,
                    "displayName": n.display_name,
                    "platform": n.platform,
                    "version": n.version,
                })
            })
            .collect();
        Ok(serde_json::json!({ "clients": clients, "nodes": nodes }))
    }

    async fn node_list(&self) -> Result<Value, String> {
        let inner = self.state.inner.read().await;
        let nodes: Vec<_> = inner
            .nodes
            .list()
            .iter()
            .map(|n| {
                serde_json::json!({
                    "nodeId": n.node_id,
                    "connId": n.conn_id,
                    "displayName": n.display_name,
                    "platform": n.platform,
                    "version": n.version,
                })
            })
            .collect();
        Ok(serde_json::json!(nodes))
    }

    async fn node_describe(&self, params: Value) -> Result<Value, String> {
        let node_id = params
            .get("nodeId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing nodeId".to_string())?;
        let inner = self.state.inner.read().await;
        let node = inner
            .nodes
            .get(node_id)
            .ok_or_else(|| "node not found".to_string())?;
        Ok(serde_json::json!({
            "nodeId": node.node_id,
            "displayName": node.display_name,
            "platform": node.platform,
            "version": node.version,
            "capabilities": node.capabilities,
            "commands": node.commands,
            "permissions": node.permissions,
            "pathEnv": node.path_env,
            "remoteIp": node.remote_ip,
            "connectedAt": node.connected_at.elapsed().as_secs(),
        }))
    }

    async fn hooks_list(&self) -> Result<Value, String> {
        let inner = self.state.inner.read().await;
        let hooks: Vec<_> = inner
            .discovered_hooks
            .iter()
            .map(|h| {
                serde_json::json!({
                    "name": h.name,
                    "description": h.description,
                    "emoji": h.emoji,
                    "events": h.events,
                    "enabled": h.enabled,
                    "eligible": h.eligible,
                    "callCount": h.call_count,
                    "failureCount": h.failure_count,
                    "source": h.source,
                    "priority": h.priority,
                })
            })
            .collect();
        Ok(serde_json::json!(hooks))
    }

    async fn heartbeat_status(&self) -> Result<Value, String> {
        let inner = self.state.inner.read().await;
        Ok(serde_json::json!({ "config": inner.heartbeat_config }))
    }

    async fn heartbeat_runs(&self, _params: Value) -> Result<Value, String> {
        Ok(serde_json::json!([]))
    }
}

/// Handle GET `/graphql`:
///
/// - Standard HTTP GET: returns GraphiQL.
/// - WebSocket upgrade GET: upgrades to GraphQL subscriptions.
pub async fn graphql_get_handler(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    if !state.gateway.is_graphql_enabled() {
        return graphql_disabled_response();
    }

    let (mut parts, _body) = req.into_parts();

    if is_websocket_upgrade_request(&parts.headers) {
        let protocol =
            match async_graphql_axum::GraphQLProtocol::from_request_parts(&mut parts, &()).await {
                Ok(protocol) => protocol,
                Err(status) => return status.into_response(),
            };

        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(ws) => ws,
            Err(rejection) => return rejection.into_response(),
        };

        return graphql_ws_response(&state, protocol, ws);
    }

    graphiql_response()
}

/// Handle GraphQL queries and mutations.
pub async fn graphql_handler(
    State(state): State<AppState>,
    req: async_graphql_axum::GraphQLRequest,
) -> impl IntoResponse {
    if !state.gateway.is_graphql_enabled() {
        return graphql_disabled_response();
    }

    async_graphql_axum::GraphQLResponse::from(state.graphql_schema.execute(req.into_inner()).await)
        .into_response()
}

fn graphql_ws_response(
    state: &AppState,
    protocol: async_graphql_axum::GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    let schema = state.graphql_schema.clone();
    ws.protocols(["graphql-transport-ws", "graphql-ws"])
        .on_upgrade(move |socket| {
            let resp = async_graphql_axum::GraphQLWebSocket::new(socket, schema, protocol);
            async move {
                resp.serve().await;
            }
        })
        .into_response()
}

fn graphiql_response() -> Response {
    Html(
        GraphiQLSource::build()
            .endpoint("/graphql")
            .subscription_endpoint("/graphql")
            .finish(),
    )
    .into_response()
}

fn graphql_disabled_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": "graphql server is disabled" })),
    )
        .into_response()
}

fn is_websocket_upgrade_request(headers: &HeaderMap) -> bool {
    // A proper WS upgrade has Connection: Upgrade AND Upgrade: websocket,
    // but we also accept the presence of Sec-WebSocket-Key as a fallback
    // since some clients (e.g. graphql-ws) may omit the Connection header.
    let has_upgrade_header = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("websocket"))
        })
        .unwrap_or(false);

    has_upgrade_header || headers.contains_key(header::SEC_WEBSOCKET_KEY)
}
