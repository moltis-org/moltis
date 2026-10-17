//! One daemon connection: credentials, transport, reconnects, and the state
//! the views render (sessions, transcripts, models, approvals).

use std::{collections::HashMap, path::PathBuf, time::Duration};

use {
    gpui::{App, AsyncApp, Context, EventEmitter, Task, WeakEntity},
    moltis_client::{
        ClientIdentity, ClientUpdate, ConnectOptions, Endpoint, FailureKind, ManagedClient,
        SecretString, TlsTrust,
        api::Rpc,
        auth::{AuthClient, AuthStatus},
        protocol::{
            EventFrame, HelloOk,
            chat_event::{ChatEvent, ChatEventKind},
            client_api::{
                ApprovalDecision, ChatSendParams, ExecApprovalRequest, ModelEntry, SessionEvent,
                events,
            },
        },
        tls,
    },
    secrecy::ExposeSecret,
};

#[cfg(feature = "ssh")]
use moltis_client::ssh::SshTunnel;

use crate::{
    local_daemon, runtime,
    servers::{ServerKind, ServerProfile},
    sessions::{Sessions, new_session_key},
    transcript::{Change, Transcript},
};

/// Keychain account name for API keys.
const KEYCHAIN_USER: &str = "api-key";

/// Shown instead of sending credentials over plain HTTP to another machine.
const PLAINTEXT_REFUSED: &str = "This server uses plain http://, which would send your credentials unencrypted. Use https:// or an SSH tunnel.";

/// Events the desktop subscribes to.
const SUBSCRIPTIONS: &[&str] = &[
    events::CHAT,
    events::SESSION,
    events::EXEC_APPROVAL_REQUESTED,
    events::EXEC_APPROVAL_RESOLVED,
    events::MODELS_UPDATED,
];

/// Where the connection stands, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Connecting {
        attempt: u32,
    },
    Connected,
    Reconnecting {
        reason: String,
        retry_in: Duration,
    },
    /// The daemon wants credentials.
    NeedsLogin {
        reason: String,
        status: Option<AuthStatus>,
        busy: bool,
        error: Option<String>,
    },
    /// The certificate is unknown (or changed); the user must decide.
    UntrustedCertificate {
        fingerprint: String,
        changed: bool,
    },
    /// The local daemon is not running.
    NotRunning {
        moltis: Option<PathBuf>,
        starting: bool,
        error: Option<String>,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Phase,
    Sessions,
    Transcript {
        session_key: String,
        change: Change,
    },
    Approvals,
    Models,
    /// The profile changed (trusted certificate) and should be saved.
    ProfileChanged(ServerProfile),
    /// A transient message for the user.
    Toast(String),
}

pub struct Daemon {
    profile: ServerProfile,
    phase: Phase,
    connected_once: bool,
    client: Option<ManagedClient>,
    endpoint: Option<Endpoint>,
    #[cfg(feature = "ssh")]
    tunnel: Option<SshTunnel>,
    hello: Option<HelloOk>,
    sessions: Sessions,
    transcripts: HashMap<String, Transcript>,
    models: Vec<ModelEntry>,
    model: Option<String>,
    approvals: Vec<ExecApprovalRequest>,
    seq: u64,
    /// Bumped on every (re)connect; stale tasks check it and stop.
    generation: u64,
    task: Option<Task<()>>,
}

impl EventEmitter<DaemonEvent> for Daemon {}

impl Daemon {
    pub fn new(profile: ServerProfile, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            profile,
            phase: Phase::Connecting { attempt: 1 },
            connected_once: false,
            client: None,
            endpoint: None,
            #[cfg(feature = "ssh")]
            tunnel: None,
            hello: None,
            sessions: Sessions::default(),
            transcripts: HashMap::new(),
            models: Vec::new(),
            model: None,
            approvals: Vec::new(),
            seq: 0,
            generation: 0,
            task: None,
        };
        this.connect(cx);
        this
    }

    // ── Accessors ───────────────────────────────────────────────────────

    #[must_use]
    pub fn profile(&self) -> &ServerProfile {
        &self.profile
    }

    #[must_use]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    #[must_use]
    pub fn hello(&self) -> Option<&HelloOk> {
        self.hello.as_ref()
    }

    #[must_use]
    pub fn sessions(&self) -> &Sessions {
        &self.sessions
    }

    #[must_use]
    pub fn transcript(&self, session_key: &str) -> Option<&Transcript> {
        self.transcripts.get(session_key)
    }

    #[must_use]
    pub fn selected_transcript(&self) -> Option<&Transcript> {
        self.transcript(self.sessions.selected()?)
    }

    #[must_use]
    pub fn models(&self) -> &[ModelEntry] {
        &self.models
    }

    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    #[must_use]
    pub fn approvals(&self) -> &[ExecApprovalRequest] {
        &self.approvals
    }

    /// The web UI at `path`, while connected.
    #[must_use]
    pub fn web_url(&self, path: &str) -> Option<String> {
        let endpoint = self.endpoint.as_ref()?;
        Some(endpoint.http_url(path).to_string())
    }

    fn set_phase(&mut self, phase: Phase, cx: &mut Context<Self>) {
        if self.phase != phase {
            self.phase = phase;
            cx.emit(DaemonEvent::Phase);
            cx.notify();
        }
    }

    fn toast(&self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(DaemonEvent::Toast(message.into()));
    }

    // ── Connection lifecycle ────────────────────────────────────────────

    /// (Re)connect from scratch: read the key, open the transport, start the
    /// supervised client.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        self.stop();
        self.generation += 1;
        let generation = self.generation;
        self.set_phase(Phase::Connecting { attempt: 1 }, cx);
        let profile = self.profile.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = Self::run(this.clone(), profile, generation, cx).await;
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.set_phase(
                            Phase::Failed {
                                reason: format!("{error:#}"),
                            },
                            cx,
                        );
                    }
                });
            }
        }));
    }

    fn stop(&mut self) {
        if let Some(client) = self.client.take() {
            client.shutdown();
        }
        #[cfg(feature = "ssh")]
        {
            self.tunnel = None;
        }
        self.task = None;
    }

    /// Retry now instead of waiting out the backoff.
    pub fn retry_now(&mut self, cx: &mut Context<Self>) {
        match &self.client {
            Some(client) => client.reconnect_now(),
            None => self.connect(cx),
        }
    }

    async fn run(
        this: WeakEntity<Self>,
        profile: ServerProfile,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let api_key = read_api_key(&profile, cx).await;
        let endpoint = Self::open_transport(&this, &profile, generation, cx).await?;
        let options = ConnectOptions::new(endpoint.clone(), identity()).with_api_key(api_key);
        let subscriptions = SUBSCRIPTIONS.iter().map(ToString::to_string).collect();
        let (client, mut updates) = cx.update(|cx| {
            let _guard = runtime::Tokio::handle(cx).enter();
            ManagedClient::spawn(options, subscriptions)
        });
        this.update(cx, |this, _| {
            if this.generation == generation {
                this.client = Some(client.clone());
                this.endpoint = Some(endpoint);
            } else {
                client.shutdown();
            }
        })?;
        while let Some(update) = updates.recv().await {
            let current = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return false;
                }
                this.on_update(update, cx);
                true
            })?;
            if !current {
                break;
            }
        }
        Ok(())
    }

    #[cfg_attr(not(feature = "ssh"), allow(clippy::unused_async))]
    async fn open_transport(
        this: &WeakEntity<Self>,
        profile: &ServerProfile,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Endpoint> {
        match &profile.kind {
            ServerKind::Direct { endpoint } => {
                let _ = (this, generation);
                Ok(endpoint.clone())
            },
            #[cfg(feature = "ssh")]
            ServerKind::Ssh { target, tls } => {
                let target = target.clone();
                let tunnel = cx
                    .update(|cx| runtime::spawn(cx, async move { SshTunnel::open(&target).await }))
                    .await
                    .ok_or_else(|| anyhow::anyhow!("network runtime stopped"))??;
                let endpoint = tunnel.endpoint(tls.clone())?;
                this.update(cx, |this, _| {
                    if this.generation == generation {
                        this.tunnel = Some(tunnel);
                    }
                })?;
                Ok(endpoint)
            },
        }
    }

    fn on_update(&mut self, update: ClientUpdate, cx: &mut Context<Self>) {
        match update {
            // After the first connection, retries show as Reconnecting.
            ClientUpdate::Connecting { attempt } => {
                if !self.connected_once && !matches!(self.phase, Phase::NotRunning { .. }) {
                    self.set_phase(Phase::Connecting { attempt }, cx);
                }
            },
            ClientUpdate::Connected { hello } => {
                self.connected_once = true;
                self.hello = Some(*hello);
                self.set_phase(Phase::Connected, cx);
                self.refresh(cx);
            },
            ClientUpdate::Disconnected {
                reason,
                kind,
                retry_in,
            } => self.on_disconnected(reason, kind, retry_in, cx),
            ClientUpdate::AuthFailed { reason } => {
                self.client = None;
                self.set_phase(
                    Phase::NeedsLogin {
                        reason,
                        status: None,
                        busy: false,
                        error: None,
                    },
                    cx,
                );
                self.fetch_auth_status(cx);
            },
            ClientUpdate::Event(frame) => self.on_event(*frame, cx),
            ClientUpdate::Lagged(_) => {
                self.refresh(cx);
                if let Some(key) = self.sessions.selected().map(String::from) {
                    self.load_history(&key, cx);
                }
            },
        }
    }

    fn on_disconnected(
        &mut self,
        reason: String,
        kind: FailureKind,
        retry_in: Duration,
        cx: &mut Context<Self>,
    ) {
        match kind {
            FailureKind::Tls => {
                // Retrying cannot help until the user trusts the certificate.
                if let Some(client) = self.client.take() {
                    client.shutdown();
                }
                self.probe_certificate(cx);
            },
            FailureKind::Unreachable if self.profile.is_local() && !self.connected_once => {
                if !matches!(self.phase, Phase::NotRunning { .. }) {
                    let moltis = local_daemon::find_moltis(
                        std::env::var_os("PATH"),
                        moltis_config::home_dir().as_deref(),
                    );
                    self.set_phase(
                        Phase::NotRunning {
                            moltis,
                            starting: false,
                            error: None,
                        },
                        cx,
                    );
                }
            },
            _ => self.set_phase(Phase::Reconnecting { reason, retry_in }, cx),
        }
    }

    fn probe_certificate(&mut self, cx: &mut Context<Self>) {
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        // Only a server never verified before gets a first-use prompt; a
        // pinned or CA-verified one failing is a changed certificate.
        let changed = !matches!(self.profile.tls(), TlsTrust::System);
        let generation = self.generation;
        let probe = runtime::spawn(cx, async move { tls::probe_fingerprint(&endpoint).await });
        cx.spawn(async move |this, cx| {
            let result = probe.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let phase = match result {
                    Some(Ok(fingerprint)) => Phase::UntrustedCertificate {
                        fingerprint,
                        changed,
                    },
                    Some(Err(error)) => Phase::Failed {
                        reason: error.to_string(),
                    },
                    None => return,
                };
                this.set_phase(phase, cx);
            });
        })
        .detach();
    }

    /// Pin the certificate the user just confirmed and reconnect.
    pub fn trust_certificate(&mut self, cx: &mut Context<Self>) {
        let Phase::UntrustedCertificate { fingerprint, .. } = &self.phase else {
            return;
        };
        self.profile.set_tls(TlsTrust::Fingerprint {
            sha256: fingerprint.clone(),
        });
        cx.emit(DaemonEvent::ProfileChanged(self.profile.clone()));
        self.connect(cx);
    }

    fn fetch_auth_status(&mut self, cx: &mut Context<Self>) {
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        let status = runtime::spawn(
            cx,
            async move { AuthClient::new(endpoint)?.status(None).await },
        );
        cx.spawn(async move |this, cx| {
            let Some(Ok(status)) = status.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Phase::NeedsLogin {
                    reason,
                    busy,
                    error,
                    ..
                } = &this.phase
                {
                    let phase = Phase::NeedsLogin {
                        reason: reason.clone(),
                        status: Some(status),
                        busy: *busy,
                        error: error.clone(),
                    };
                    this.set_phase(phase, cx);
                }
            });
        })
        .detach();
    }

    fn set_login_state(&mut self, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        if let Phase::NeedsLogin { reason, status, .. } = &self.phase {
            let phase = Phase::NeedsLogin {
                reason: reason.clone(),
                status: status.clone(),
                busy,
                error,
            };
            self.set_phase(phase, cx);
        }
    }

    /// Exchange the daemon password for a scoped API key, store it, reconnect.
    pub fn login_with_password(&mut self, password: SecretString, cx: &mut Context<Self>) {
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        if !endpoint.protects_credentials() {
            self.set_login_state(false, Some(PLAINTEXT_REFUSED.into()), cx);
            return;
        }
        self.set_login_state(true, None, cx);
        let label = format!("Moltis Desktop ({})", hostname());
        let key = runtime::spawn(cx, async move {
            AuthClient::new(endpoint)?
                .exchange_password_for_key(&password, &label)
                .await
        });
        cx.spawn(async move |this, cx| match key.await {
            Some(Ok(key)) => {
                let _ = this.update(cx, |this, cx| this.store_key_and_connect(key, cx));
            },
            Some(Err(error)) => {
                let message = if error.is_auth() {
                    "Wrong password.".to_string()
                } else {
                    error.to_string()
                };
                let _ = this.update(cx, |this, cx| {
                    this.set_login_state(false, Some(message), cx)
                });
            },
            None => {},
        })
        .detach();
    }

    /// Use an API key created in the web UI (Settings → Security).
    pub fn use_api_key(&mut self, key: SecretString, cx: &mut Context<Self>) {
        if self
            .endpoint
            .as_ref()
            .is_some_and(|endpoint| !endpoint.protects_credentials())
        {
            self.set_login_state(false, Some(PLAINTEXT_REFUSED.into()), cx);
            return;
        }
        if !key.expose_secret().trim().starts_with("mk_") {
            self.set_login_state(false, Some("API keys start with mk_.".into()), cx);
            return;
        }
        let key = SecretString::new(key.expose_secret().trim().to_string());
        self.store_key_and_connect(key, cx);
    }

    fn store_key_and_connect(&mut self, key: SecretString, cx: &mut Context<Self>) {
        let url = self.profile.keychain_url();
        let write = cx.write_credentials(&url, KEYCHAIN_USER, key.expose_secret().as_bytes());
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.set_login_state(
                        false,
                        Some(format!("Could not save the key in the keychain: {error}")),
                        cx,
                    );
                    return;
                }
                this.connect(cx);
            });
        })
        .detach();
    }

    /// Delete the stored API key (sign out of this server).
    pub fn forget_credentials(&mut self, cx: &mut Context<Self>) {
        let delete = cx.delete_credentials(&self.profile.keychain_url());
        cx.spawn(async move |this, cx| {
            let _ = delete.await;
            let _ = this.update(cx, |this, cx| this.connect(cx));
        })
        .detach();
    }

    /// Install and start the local daemon as a user service.
    pub fn start_local_daemon(&mut self, cx: &mut Context<Self>) {
        let Phase::NotRunning {
            moltis: Some(moltis),
            ..
        } = &self.phase
        else {
            return;
        };
        let moltis = moltis.clone();
        self.set_phase(
            Phase::NotRunning {
                moltis: Some(moltis.clone()),
                starting: true,
                error: None,
            },
            cx,
        );
        let install = cx
            .background_executor()
            .spawn(async move { local_daemon::install_service(&moltis).map(|()| moltis) });
        cx.spawn(async move |this, cx| {
            let result = install.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    if let Some(client) = &this.client {
                        client.reconnect_now();
                    }
                },
                Err(error) => {
                    if let Phase::NotRunning { moltis, .. } = &this.phase {
                        let phase = Phase::NotRunning {
                            moltis: moltis.clone(),
                            starting: false,
                            error: Some(format!("{error:#}")),
                        };
                        this.set_phase(phase, cx);
                    }
                },
            });
        })
        .detach();
    }

    // ── Data ────────────────────────────────────────────────────────────

    /// Refetch sessions and models (after connecting or missing events).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let fetch = runtime::spawn(cx, async move {
            let (sessions, models) = tokio::join!(client.sessions_list(), client.models_list());
            (sessions, models)
        });
        cx.spawn(async move |this, cx| {
            let Some((sessions, models)) = fetch.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                match sessions {
                    Ok(sessions) => {
                        this.sessions.replace(sessions);
                        cx.emit(DaemonEvent::Sessions);
                        if let Some(key) = this.sessions.selected().map(String::from)
                            && !this.transcripts.contains_key(&key)
                        {
                            this.load_history(&key, cx);
                        }
                    },
                    Err(error) => this.toast(format!("Could not list sessions: {error}"), cx),
                }
                if let Ok(models) = models {
                    if this.model.is_none() {
                        this.model = models
                            .iter()
                            .find(|m| m.preferred)
                            .or_else(|| models.first())
                            .map(|m| m.id.clone());
                    }
                    this.models = models;
                    cx.emit(DaemonEvent::Models);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_history(&mut self, session_key: &str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let key = session_key.to_string();
        let fetch = runtime::spawn(cx, {
            let key = key.clone();
            async move { client.chat_history(&key).await }
        });
        cx.spawn(async move |this, cx| {
            let Some(result) = fetch.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| match result {
                Ok(history) => {
                    this.transcripts
                        .insert(key.clone(), Transcript::from_history(&history));
                    cx.emit(DaemonEvent::Transcript {
                        session_key: key,
                        change: Change::Reset,
                    });
                    cx.notify();
                },
                Err(error) => this.toast(format!("Could not load the conversation: {error}"), cx),
            });
        })
        .detach();
    }

    fn on_event(&mut self, frame: EventFrame, cx: &mut Context<Self>) {
        let payload = frame.payload.unwrap_or_default();
        match frame.event.as_str() {
            events::CHAT => {
                let Ok(event) = serde_json::from_value::<ChatEvent>(payload) else {
                    return;
                };
                self.on_chat_event(event, cx);
            },
            events::SESSION => {
                if let Ok(event) = serde_json::from_value::<SessionEvent>(payload)
                    && self.sessions.apply_session_event(&event)
                {
                    cx.emit(DaemonEvent::Sessions);
                    cx.notify();
                }
            },
            events::EXEC_APPROVAL_REQUESTED => {
                if let Ok(request) = serde_json::from_value::<ExecApprovalRequest>(payload)
                    && !self
                        .approvals
                        .iter()
                        .any(|a| a.request_id == request.request_id)
                {
                    self.approvals.push(request);
                    cx.emit(DaemonEvent::Approvals);
                    cx.notify();
                }
            },
            events::EXEC_APPROVAL_RESOLVED => {
                let id = payload
                    .get("requestId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let before = self.approvals.len();
                self.approvals.retain(|a| a.request_id != id);
                if self.approvals.len() != before {
                    cx.emit(DaemonEvent::Approvals);
                    cx.notify();
                }
            },
            events::MODELS_UPDATED => self.refresh(cx),
            _ => {},
        }
    }

    fn on_chat_event(&mut self, event: ChatEvent, cx: &mut Context<Self>) {
        let (row_changed, unknown) = self.sessions.apply_chat_event(&event);
        if unknown {
            self.refresh(cx);
        } else if row_changed {
            cx.emit(DaemonEvent::Sessions);
        }
        let Some(transcript) = self.transcripts.get_mut(&event.session_key) else {
            return;
        };
        let change = transcript.apply(&event.kind);
        if change != Change::None || matches!(event.kind, ChatEventKind::Thinking) {
            cx.emit(DaemonEvent::Transcript {
                session_key: event.session_key,
                change,
            });
        }
        cx.notify();
    }

    // ── Actions ─────────────────────────────────────────────────────────

    pub fn select_session(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.sessions.select(key) {
            return;
        }
        cx.emit(DaemonEvent::Sessions);
        if self.transcripts.contains_key(key) {
            cx.emit(DaemonEvent::Transcript {
                session_key: key.to_string(),
                change: Change::Reset,
            });
        } else {
            self.load_history(key, cx);
        }
        cx.notify();
    }

    pub fn new_session(&mut self, cx: &mut Context<Self>) {
        let key = new_session_key();
        self.transcripts.insert(key.clone(), Transcript::default());
        self.sessions.insert_draft(&key);
        cx.emit(DaemonEvent::Sessions);
        cx.emit(DaemonEvent::Transcript {
            session_key: key,
            change: Change::Reset,
        });
        cx.notify();
    }

    pub fn set_model(&mut self, model: &str, cx: &mut Context<Self>) {
        self.model = Some(model.to_string());
        cx.emit(DaemonEvent::Models);
        cx.notify();
    }

    pub fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let (Some(client), Some(session_key)) = (
            self.client.clone(),
            self.sessions.selected().map(String::from),
        ) else {
            self.toast("Not connected to the daemon.", cx);
            return;
        };
        self.transcripts.entry(session_key.clone()).or_default();
        self.seq += 1;
        let params = ChatSendParams {
            text,
            session_key: Some(session_key),
            model: self.model.clone(),
            seq: Some(self.seq),
        };
        let sent = runtime::spawn(cx, async move { client.chat_send(&params).await });
        cx.spawn(async move |this, cx| {
            if let Some(Err(error)) = sent.await {
                let _ = this.update(cx, |this, cx| {
                    this.toast(format!("Message not sent: {error}"), cx);
                });
            }
        })
        .detach();
    }

    pub fn abort(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(key)) = (
            self.client.clone(),
            self.sessions.selected().map(String::from),
        ) else {
            return;
        };
        let abort = runtime::spawn(cx, async move { client.chat_abort(&key).await });
        cx.spawn(async move |this, cx| {
            if let Some(Err(error)) = abort.await {
                let _ = this.update(cx, |this, cx| {
                    this.toast(format!("Could not stop: {error}"), cx)
                });
            }
        })
        .detach();
    }

    pub fn resolve_approval(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.approvals.retain(|a| a.request_id != request_id);
        cx.emit(DaemonEvent::Approvals);
        cx.notify();
        let id = request_id.to_string();
        let resolve = runtime::spawn(
            cx,
            async move { client.resolve_approval(&id, decision).await },
        );
        cx.spawn(async move |this, cx| {
            if let Some(Err(error)) = resolve.await {
                let _ = this.update(cx, |this, cx| {
                    this.toast(format!("Could not answer the approval: {error}"), cx);
                });
            }
        })
        .detach();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn read_api_key(profile: &ServerProfile, cx: &mut AsyncApp) -> Option<SecretString> {
    let url = profile.keychain_url();
    let read = cx.update(|cx: &mut App| cx.read_credentials(&url));
    let (_, bytes) = read.await.ok()??;
    String::from_utf8(bytes)
        .ok()
        .filter(|key| !key.is_empty())
        .map(SecretString::new)
}

fn identity() -> ClientIdentity {
    ClientIdentity {
        display_name: Some(format!("Moltis Desktop on {}", hostname())),
        ..ClientIdentity::new("moltis-desktop", env!("CARGO_PKG_VERSION"))
    }
}

fn hostname() -> String {
    gethostname::gethostname()
        .into_string()
        .ok()
        .map(|h| h.trim_end_matches(".local").to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "this computer".into())
}
