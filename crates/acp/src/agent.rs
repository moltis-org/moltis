//! `impl acp::Agent` — the agent side of the Agent Client Protocol.
//!
//! Everything here runs on the [`tokio::task::LocalSet`] that owns the
//! connection, so the types are `Rc`-based and never cross a thread boundary.
//! Real work is delegated to a [`AcpBackend`], which is `Send + Sync` and runs
//! on the ordinary multi-threaded runtime.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::Arc,
};

use {
    agent_client_protocol::{self as acp, Client as _},
    async_trait::async_trait,
};

use crate::{
    backend::{AcpBackend, MAX_TURN_UPDATE_BYTES, TurnUpdates, validate_history},
    session::{ACP_SESSION_NAMESPACE, SessionKey, SessionRegistry},
    setup::SessionSetup,
};

const MAX_PROMPT_BLOCKS: usize = 256;
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// Protocol handler bridging an ACP client to a Moltis backend.
pub struct MoltisAgent {
    backend: Arc<dyn AcpBackend>,
    sessions: SessionRegistry,
    /// Weak on purpose: the connection owns the handler, so a strong reference
    /// here would form a cycle and leak both for the life of the process.
    connection: RefCell<Weak<acp::AgentSideConnection>>,
}

impl MoltisAgent {
    #[must_use]
    pub fn new(backend: Arc<dyn AcpBackend>) -> Self {
        Self {
            backend,
            sessions: SessionRegistry::new(),
            connection: RefCell::new(Weak::new()),
        }
    }

    /// Attaches the connection used to send `session/update` notifications.
    ///
    /// `AgentSideConnection::new` consumes the handler, so the handler cannot
    /// hold the connection at construction time. Build the agent first, pass a
    /// clone into the connection, then call this. The caller must keep the
    /// `Rc` alive for as long as the connection is served.
    pub fn set_connection(&self, connection: &Rc<acp::AgentSideConnection>) {
        *self.connection.borrow_mut() = Rc::downgrade(connection);
    }

    #[must_use]
    pub fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    /// Clones the connection out of its `RefCell` so no borrow is held across
    /// an await point.
    fn connection(&self) -> acp::Result<Rc<acp::AgentSideConnection>> {
        self.connection
            .borrow()
            .upgrade()
            .ok_or_else(|| acp::Error::internal_error().data("ACP connection not attached"))
    }

    async fn notify(
        connection: &Rc<acp::AgentSideConnection>,
        session_id: &acp::SessionId,
        update: acp::SessionUpdate,
    ) -> bool {
        connection
            .session_notification(acp::SessionNotification::new(session_id.clone(), update))
            .await
            .is_ok()
    }
}

/// Flattens the supported prompt blocks into the plain text Moltis consumes.
/// Blocks outside the advertised capabilities are rejected instead of being
/// silently replaced or dropped.
pub fn prompt_text(blocks: &[acp::ContentBlock]) -> acp::Result<String> {
    if blocks.len() > MAX_PROMPT_BLOCKS {
        return Err(acp::Error::invalid_params().data(format!(
            "prompt has more than {MAX_PROMPT_BLOCKS} content blocks"
        )));
    }
    let text = blocks
        .iter()
        .map(|block| match block {
            acp::ContentBlock::Text(text) => Ok(text.text.clone()),
            acp::ContentBlock::ResourceLink(link) => Ok(link.uri.to_string()),
            _ => Err(acp::Error::invalid_params()
                .data("prompt contains content the agent did not advertise")),
        })
        .collect::<acp::Result<Vec<_>>>()?
        .join("\n");
    if text.len() > MAX_PROMPT_BYTES {
        return Err(
            acp::Error::invalid_params().data(format!("prompt exceeds {MAX_PROMPT_BYTES} bytes"))
        );
    }
    Ok(text)
}

#[async_trait(?Send)]
impl acp::Agent for MoltisAgent {
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, args)))]
    async fn initialize(
        &self,
        args: acp::InitializeRequest,
    ) -> acp::Result<acp::InitializeResponse> {
        let version = if args.protocol_version == acp::ProtocolVersion::V1 {
            acp::ProtocolVersion::V1
        } else {
            acp::ProtocolVersion::LATEST
        };
        let capabilities = self.backend.capabilities();
        Ok(acp::InitializeResponse::new(version)
            .agent_capabilities(
                acp::AgentCapabilities::new().load_session(capabilities.load_session),
            )
            .agent_info(
                acp::Implementation::new(
                    "moltis",
                    option_env!("MOLTIS_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
                )
                .title("Moltis"),
            ))
    }

    async fn authenticate(
        &self,
        _args: acp::AuthenticateRequest,
    ) -> acp::Result<acp::AuthenticateResponse> {
        // The client is the local parent process that spawned us; there is no
        // separate identity to establish.
        Ok(acp::AuthenticateResponse::new())
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, args)))]
    async fn new_session(
        &self,
        args: acp::NewSessionRequest,
    ) -> acp::Result<acp::NewSessionResponse> {
        self.sessions.ensure_capacity()?;
        let setup_key = SessionKey::namespaced(format!("setup-{}", self.sessions.next_local_id()));
        let _setup_guard = self.sessions.begin_setup(&setup_key)?;
        let setup = SessionSetup::new(args.cwd, args.mcp_servers).await?;
        let key = self.backend.create_session(&setup).await.map_err(|error| {
            acp::Error::internal_error().data(format!("failed to create session: {error}"))
        })?;
        // A backend minting keys outside the namespace would quietly hand the
        // client an id that `load_session` must then refuse. That is our bug,
        // not the client's, so it is an internal error rather than bad input.
        if !key.is_namespaced() {
            let _ = self.backend.discard_session(&key).await;
            return Err(acp::Error::internal_error().data(format!(
                "backend created session {key} outside the `{ACP_SESSION_NAMESPACE}:` namespace"
            )));
        }
        if let Err(error) = self.sessions.insert(key.clone()) {
            let _ = self.backend.discard_session(&key).await;
            return Err(error);
        }
        Ok(acp::NewSessionResponse::new(acp::SessionId::from(key)))
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, args)))]
    async fn load_session(
        &self,
        args: acp::LoadSessionRequest,
    ) -> acp::Result<acp::LoadSessionResponse> {
        if !self.backend.capabilities().load_session {
            return Err(acp::Error::method_not_found());
        }
        // `session_id` is arbitrary client input, and unlike `prompt`/`cancel`
        // there is no registry entry to check it against — resuming a session
        // this connection never opened is the whole point. The `acp:` namespace
        // is therefore the only thing keeping a client from naming a Web UI or
        // channel session here and driving it with subsequent prompts. Enforce
        // it before the backend sees the key, so no backend has to re-derive
        // the invariant to stay isolated.
        let key = SessionKey::from(&args.session_id);
        if !key.is_namespaced() {
            return Err(acp::Error::invalid_params().data(format!(
                "session id {key} is outside the `{ACP_SESSION_NAMESPACE}:` namespace"
            )));
        }
        self.sessions.ensure_capacity_for(&key)?;
        let _setup_guard = self.sessions.begin_setup(&key)?;
        let setup = SessionSetup::new(args.cwd, args.mcp_servers).await?;
        let history = self
            .backend
            .load_session(&key, &setup)
            .await
            .map_err(|error| {
                let detail = format!("failed to load session {key}: {error}");
                if error.is::<crate::backend::SessionNotFound>() {
                    acp::Error::invalid_params().data(detail)
                } else {
                    acp::Error::internal_error().data(detail)
                }
            })?;

        // The spec asks the agent to stream the whole conversation back before
        // resolving the request.
        let connection = self.connection()?;
        if let Err(error) = validate_history(&history) {
            let _ = self.backend.discard_session(&key).await;
            return Err(acp::Error::internal_error()
                .data(format!("cannot replay session history: {error}")));
        }
        for update in history {
            if !Self::notify(&connection, &args.session_id, update).await {
                // The closed connection immediately drives backend shutdown,
                // which owns cleanup. Removing the runtime here would race that
                // shutdown while its MCP process is still being stopped.
                return Err(acp::Error::internal_error()
                    .data("ACP client disconnected during session replay"));
            }
        }
        if let Err(error) = self.sessions.insert(key.clone()) {
            let _ = self.backend.discard_session(&key).await;
            return Err(error);
        }
        Ok(acp::LoadSessionResponse::new())
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, args)))]
    async fn prompt(&self, args: acp::PromptRequest) -> acp::Result<acp::PromptResponse> {
        let key = self.sessions.resolve(&args.session_id)?;
        let text = prompt_text(&args.prompt)?;
        let _prompt = self.sessions.begin_prompt(&key)?;
        let connection = self.connection()?;
        self.sessions.clear_cancelled(&key);

        // Memory is bounded by `TurnUpdates`' byte budget. An unbounded item
        // queue avoids rejecting a valid burst made up of many tiny deltas.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        let update_status = updates.clone();
        let turn = self.backend.prompt(&key, text, updates);
        let mut turn = std::pin::pin!(turn);

        // Forward updates while the turn runs, on this same task. Doing it here
        // rather than in a spawned forwarder means a backend that leaks a
        // `TurnUpdates` clone cannot wedge the response: we stop draining the
        // moment the turn resolves.
        let mut update_bytes = 0usize;
        let mut forwarding = true;
        let mut update_limit_exceeded = false;
        let outcome = loop {
            tokio::select! {
                biased;
                Some(update) = rx.recv() => {
                    if !forwarding {
                        continue;
                    }
                    update_bytes = update_bytes.saturating_add(
                        serde_json::to_vec(&update).map_or(MAX_TURN_UPDATE_BYTES + 1, |value| value.len())
                    );
                    if update_bytes > MAX_TURN_UPDATE_BYTES {
                        update_limit_exceeded = true;
                        forwarding = false;
                        let _ = self.backend.cancel(&key).await;
                    } else if !Self::notify(&connection, &args.session_id, update).await {
                        forwarding = false;
                        let _ = self.backend.cancel(&key).await;
                    }
                },
                result = &mut turn => break result,
            }
        };

        // Flush anything already queued so no delta lands after the response.
        while forwarding && let Ok(update) = rx.try_recv() {
            update_bytes = update_bytes.saturating_add(
                serde_json::to_vec(&update).map_or(MAX_TURN_UPDATE_BYTES + 1, |value| value.len()),
            );
            if update_bytes > MAX_TURN_UPDATE_BYTES {
                update_limit_exceeded = true;
                break;
            }
            if !Self::notify(&connection, &args.session_id, update).await {
                break;
            }
        }

        if update_limit_exceeded || update_status.limit_exceeded() {
            return Err(acp::Error::internal_error()
                .data(format!("turn updates exceed {MAX_TURN_UPDATE_BYTES} bytes")));
        }

        let cancelled = self.sessions.take_cancelled(&key);
        let stop_reason = match outcome {
            // A cancel that raced the turn's own completion still has to be
            // reported as cancelled: the client is waiting for that signal.
            Ok(_) if cancelled => acp::StopReason::Cancelled,
            Ok(reason) => reason,
            Err(_) if cancelled => acp::StopReason::Cancelled,
            Err(error) => {
                return Err(acp::Error::internal_error().data(format!("turn failed: {error}")));
            },
        };
        Ok(acp::PromptResponse::new(stop_reason))
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, args)))]
    async fn cancel(&self, args: acp::CancelNotification) -> acp::Result<()> {
        // Arrives out-of-band while `prompt` is still pending. Flag first so the
        // pending turn reports `Cancelled` even if the backend finishes first.
        let key = self.sessions.resolve(&args.session_id)?;
        self.sessions.mark_cancelled(&key);
        self.backend.cancel(&key).await.map_err(|error| {
            acp::Error::internal_error().data(format!("failed to cancel turn: {error}"))
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn prompt_text_joins_supported_text_blocks() {
        let blocks = vec![
            acp::ContentBlock::from("first"),
            acp::ContentBlock::from("second"),
        ];
        assert_eq!(
            prompt_text(&blocks).expect("supported prompt"),
            "first\nsecond"
        );
    }

    #[test]
    fn prompt_text_rejects_unadvertised_content() {
        let image = serde_json::from_value::<acp::ContentBlock>(serde_json::json!({
            "type": "image",
            "data": "AA==",
            "mimeType": "image/png"
        }))
        .expect("valid ACP image block");
        assert!(prompt_text(&[image]).is_err());
    }

    #[test]
    fn prompt_text_rejects_oversized_input() {
        let oversized = "x".repeat(MAX_PROMPT_BYTES + 1);
        assert!(prompt_text(&[acp::ContentBlock::from(oversized)]).is_err());
    }
}
