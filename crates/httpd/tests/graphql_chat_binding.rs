#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "graphql")]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use {async_trait::async_trait, tokio::net::TcpListener};

use {
    moltis_gateway::{
        auth,
        methods::MethodRegistry,
        services::{ChatService, GatewayServices, ServiceResult},
        state::GatewayState,
    },
    moltis_httpd::server::{build_gateway_base, finalize_gateway_app},
    serde_json::{Value, json},
};

#[derive(Default)]
struct RecordingChatService {
    calls: Mutex<Vec<String>>,
}

impl RecordingChatService {
    fn record(&self, method: &str) {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(method.to_string());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
impl ChatService for RecordingChatService {
    async fn send(&self, params: Value) -> ServiceResult {
        self.record("send");
        if params["message"] != "Hello" || params["sessionKey"] != "sess1" {
            return Err(format!("unexpected send params: {params}").into());
        }
        Ok(json!({ "ok": true }))
    }

    async fn abort(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "ok": true }))
    }

    async fn cancel_queued(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "cleared": 0 }))
    }

    async fn history(&self, params: Value) -> ServiceResult {
        self.record("history");
        if params["sessionKey"] != "sess1" {
            return Err(format!("unexpected history params: {params}").into());
        }
        Ok(json!([{
            "role": "assistant",
            "content": "History",
        }]))
    }

    async fn inject(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "ok": true }))
    }

    async fn clear(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "ok": true }))
    }

    async fn compact(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "ok": true }))
    }

    async fn context(&self, _params: Value) -> ServiceResult {
        Ok(json!({}))
    }

    async fn raw_prompt(&self, _params: Value) -> ServiceResult {
        Ok(json!({ "text": "prompt" }))
    }

    async fn full_context(&self, _params: Value) -> ServiceResult {
        Ok(json!([]))
    }

    async fn active(&self, params: Value) -> ServiceResult {
        self.record("active");
        if params["sessionKey"] != "sess1" {
            return Err(format!("unexpected active params: {params}").into());
        }
        Ok(json!({ "active": true }))
    }
}

async fn start_graphql_server() -> (SocketAddr, Arc<GatewayState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    moltis_config::set_config_dir(tmp.path().to_path_buf());
    moltis_config::set_data_dir(tmp.path().to_path_buf());

    let state = GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop());
    let state_clone = Arc::clone(&state);
    let methods = Arc::new(MethodRegistry::new());

    #[cfg(feature = "push-notifications")]
    let (router, app_state) = build_gateway_base(state, methods, None, None);
    #[cfg(not(feature = "push-notifications"))]
    let (router, app_state) = build_gateway_base(state, methods, None);

    let app = finalize_gateway_app(router, app_state, false);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    (addr, state_clone, tmp)
}

#[tokio::test]
async fn graphql_chat_uses_late_bound_override_after_schema_build() {
    let (addr, state, _tmp) = start_graphql_server().await;

    let chat = Arc::new(RecordingChatService::default());
    state
        .set_chat(Arc::clone(&chat) as Arc<dyn ChatService>)
        .await;

    let client = reqwest::Client::new();

    let send_response: Value = client
        .post(format!("http://{addr}/graphql"))
        .json(&json!({
            "query": r#"mutation { chat { send(message: "Hello", sessionKey: "sess1") { ok } } }"#,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(send_response["data"]["chat"]["send"]["ok"], true);

    let history_response: Value = client
        .post(format!("http://{addr}/graphql"))
        .json(&json!({
            "query": r#"query { chat { history(sessionKey: "sess1") } }"#,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        history_response["data"]["chat"]["history"][0]["content"],
        "History"
    );

    let active_response: Value = client
        .post(format!("http://{addr}/graphql"))
        .json(&json!({
            "query": r#"query { sessions { active(sessionKey: "sess1") { active } } }"#,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        active_response["data"]["sessions"]["active"]["active"],
        true
    );
    assert_eq!(chat.calls(), vec!["send", "history", "active"]);
}
