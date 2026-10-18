//! One daemon connection: credentials, transport, reconnects, and the state
//! the views render (sessions, transcripts, models, approvals).

mod history;

use std::{collections::HashMap, path::PathBuf, time::Duration};

use {
    gpui::{App, AsyncApp, Context, EventEmitter, Task, WeakEntity},
    moltis_client::{
        ClientIdentity, ClientUpdate, ConnectOptions, Endpoint, FailureKind, ManagedClient,
        SecretString, TlsTrust,
        api::Rpc,
        auth::{AuthClient, AuthStatus},
        backoff::Backoff,
        protocol::{
            EventFrame, HelloOk,
            chat_event::{ChatEvent, ChatEventKind},
            client_api::{
                AgentIdentity, ApprovalDecision, ChatSendParams, ContextTokenUsage,
                ExecApprovalRequest, ModelEntry, SessionEvent, events,
            },
        },
        tls,
    },
    secrecy::ExposeSecret,
};

#[cfg(feature = "ssh")]
use moltis_client::ssh::SshTunnel;

use crate::{
    local_daemon::{self, AutoStart, ServiceCheck},
    runtime,
    servers::{ServerKind, ServerProfile},
    sessions::{Sessions, new_session_key},
    transcript::{Change, Transcript},
};

use self::history::TranscriptCache;

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
    /// The local daemon was found stopped and restarted once already; a
    /// second failure waits for the user instead of reinstalling the service
    /// in a loop. Only a daemon already installed as a service is started
    /// unasked: otherwise the user chooses between running moltis here and
    /// connecting to another computer.
    auto_started: bool,
    /// The installed service was checked against the bundled daemon.
    service_checked: bool,
    /// The running local daemon is older than the bundled one: `(running,
    /// bundled)` versions, until the user restarts it.
    outdated: Option<(String, String)>,
    client: Option<ManagedClient>,
    endpoint: Option<Endpoint>,
    #[cfg(feature = "ssh")]
    tunnel: Option<SshTunnel>,
    hello: Option<HelloOk>,
    sessions: Sessions,
    transcripts: TranscriptCache,
    /// Invalidates data requests across socket reconnects as well as transports.
    data_epoch: u64,
    models: Vec<ModelEntry>,
    /// Token usage per session, from `chat.context`.
    contexts: HashMap<String, ContextTokenUsage>,
    identity: Option<AgentIdentity>,
    model: Option<String>,
    approvals: Vec<ExecApprovalRequest>,
    seq: u64,
    /// Bumped on every (re)connect; stale tasks check it and stop.
    generation: u64,
    task: Option<Task<()>>,
}

impl EventEmitter<DaemonEvent> for Daemon {}

impl Daemon {
    #[cfg(test)]
    pub(crate) fn disconnected_for_test(profile: ServerProfile, cx: &mut Context<Self>) -> Self {
        let mut daemon = Self::new(profile, cx);
        daemon.stop();
        daemon
    }

    pub fn new(profile: ServerProfile, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            profile,
            phase: Phase::Connecting { attempt: 1 },
            connected_once: false,
            auto_started: false,
            service_checked: false,
            outdated: None,
            client: None,
            endpoint: None,
            #[cfg(feature = "ssh")]
            tunnel: None,
            hello: None,
            sessions: Sessions::default(),
            transcripts: TranscriptCache::default(),
            data_epoch: 0,
            models: Vec::new(),
            contexts: HashMap::new(),
            identity: None,
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
    /// Token usage of the selected session, once fetched.
    pub fn selected_context(&self) -> Option<ContextTokenUsage> {
        let key = self.sessions.selected()?;
        self.contexts.get(key).copied()
    }

    /// The agent's identity and the owner's name, once fetched.
    pub fn identity(&self) -> Option<&AgentIdentity> {
        self.identity.as_ref()
    }

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
        self.invalidate_data();
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
        if self.transport_dead() {
            self.connect(cx);
            return;
        }
        match &self.client {
            Some(client) => client.reconnect_now(),
            None => self.connect(cx),
        }
    }

    fn transport_dead(&mut self) -> bool {
        #[cfg(feature = "ssh")]
        if matches!(self.profile.kind, ServerKind::Ssh { .. }) {
            return self.tunnel.as_mut().is_none_or(|tunnel| !tunnel.is_alive());
        }
        false
    }

    async fn run(
        this: WeakEntity<Self>,
        profile: ServerProfile,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let api_key = read_api_key(&profile, cx).await;
        let mut backoff = Backoff::default();
        loop {
            let endpoint = match Self::open_transport(&this, &profile, generation, cx).await {
                Ok(endpoint) => endpoint,
                Err(error) => {
                    let retry_in = backoff.next_delay();
                    let current = this.update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        this.set_phase(
                            Phase::Reconnecting {
                                reason: format!("{error:#}"),
                                retry_in,
                            },
                            cx,
                        );
                        true
                    })?;
                    if !current {
                        return Ok(());
                    }
                    cx.background_executor().timer(retry_in).await;
                    continue;
                },
            };
            backoff.reset();
            if !Self::run_client(&this, endpoint, api_key.clone(), generation, cx).await? {
                return Ok(());
            }
            // The SSH process exited. Drop the old forward/client before
            // opening a replacement, which may choose a different local port.
            this.update(cx, |this, _| {
                if let Some(client) = this.client.take() {
                    client.shutdown();
                }
                this.endpoint = None;
                #[cfg(feature = "ssh")]
                {
                    this.tunnel = None;
                }
            })?;
        }
    }

    /// Returns true when the underlying transport needs to be recreated.
    async fn run_client(
        this: &WeakEntity<Self>,
        endpoint: Endpoint,
        api_key: Option<SecretString>,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<bool> {
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
            let (current, reopen) = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return (false, false);
                }
                let reopen = matches!(
                    update,
                    ClientUpdate::Connecting { .. } | ClientUpdate::Disconnected { .. }
                ) && this.transport_dead();
                if reopen {
                    this.invalidate_data();
                    this.set_phase(
                        Phase::Reconnecting {
                            reason: "SSH tunnel closed. Reopening the connection.".into(),
                            retry_in: Duration::ZERO,
                        },
                        cx,
                    );
                } else {
                    this.on_update(update, cx);
                }
                (true, reopen)
            })?;
            if reopen {
                return Ok(true);
            }
            if !current {
                break;
            }
        }
        Ok(false)
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
                let _ = (this, generation, cx);
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
                self.invalidate_data();
                self.connected_once = true;
                self.hello = Some(*hello);
                self.set_phase(Phase::Connected, cx);
                self.refresh(cx);
                if let Some(key) = self.sessions.selected().map(String::from) {
                    self.load_history(&key, cx);
                }
                self.check_service(cx);
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
                self.invalidate_data();
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
        self.invalidate_data();
        match kind {
            FailureKind::Tls => {
                // Retrying cannot help until the user trusts the certificate.
                if let Some(client) = self.client.take() {
                    client.shutdown();
                }
                // The local daemon may have written its CA since the profile
                // was read: trust that before asking about a fingerprint.
                if let Some(local) = self.profile.reloaded_local() {
                    self.profile = local;
                    cx.emit(DaemonEvent::ProfileChanged(self.profile.clone()));
                    cx.spawn(async move |this, cx| {
                        let _ = this.update(cx, |this, cx| this.connect(cx));
                    })
                    .detach();
                    return;
                }
                self.probe_certificate(cx);
            },
            FailureKind::Unreachable if self.profile.is_local() && !self.connected_once => {
                if !matches!(self.phase, Phase::NotRunning { .. }) {
                    let moltis = local_daemon::find_moltis(
                        local_daemon::bundled_dir().as_deref(),
                        std::env::var_os("PATH"),
                        moltis_config::home_dir().as_deref(),
                    );
                    let auto_start = if self.auto_started {
                        None
                    } else {
                        installed_auto_start()
                    };
                    self.set_phase(
                        Phase::NotRunning {
                            moltis,
                            starting: false,
                            error: None,
                        },
                        cx,
                    );
                    if let Some(auto_start) = auto_start {
                        self.auto_started = true;
                        match auto_start {
                            AutoStart::Install => self.start_local_daemon(cx),
                            AutoStart::Start(installed) => {
                                self.start_installed_service(installed, cx);
                            },
                        }
                    }
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
        self.run_service_job(move || local_daemon::install_service(&moltis), cx);
    }

    /// Start the service already installed here, as installed.
    fn start_installed_service(&mut self, installed: PathBuf, cx: &mut Context<Self>) {
        self.run_service_job(
            move || local_daemon::start_installed_service(&installed),
            cx,
        );
    }

    /// Show "starting" on the not-running screen while `job` runs, then wait
    /// for the daemon or show the job's error there.
    fn run_service_job(
        &mut self,
        job: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Phase::NotRunning { moltis, .. } = &self.phase else {
            return;
        };
        self.set_phase(
            Phase::NotRunning {
                moltis: moltis.clone(),
                starting: true,
                error: None,
            },
            cx,
        );
        let job = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.poll_started_daemon(cx),
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

    /// The local daemon is older than the one bundled with this app:
    /// `(running, bundled)` versions.
    pub fn outdated(&self) -> Option<&(String, String)> {
        self.outdated.as_ref()
    }

    /// Once per connection to this computer: make sure the user service runs
    /// this app's daemon. A service left pointing at a moved or deleted app
    /// is reinstalled; an older daemon after an in-place update is offered a
    /// restart (it may be busy, so never restarted unasked).
    fn check_service(&mut self, cx: &mut Context<Self>) {
        if self.service_checked || !self.profile.is_local() {
            return;
        }
        self.service_checked = true;
        let bundled = local_daemon::bundled_daemon();
        let service_path = bundled.as_deref().map(local_daemon::service_path);
        let installed = moltis_config::home_dir().and_then(|home| {
            local_daemon::installed_service_binary(&home, &moltis_config::data_dir())
        });
        let running = self
            .hello
            .as_ref()
            .map(|hello| hello.server.version.clone());
        match local_daemon::check_service(
            installed.as_deref(),
            service_path.as_deref(),
            running.as_deref(),
            local_daemon::bundled_version(),
        ) {
            ServiceCheck::Fine => {},
            ServiceCheck::Reinstall => {
                let Some(moltis) = bundled else {
                    return;
                };
                crate::logging::info!(?installed, "moltis service points elsewhere; reinstalling");
                let install = cx
                    .background_executor()
                    .spawn(async move { local_daemon::install_service(&moltis) });
                cx.spawn(async move |this, cx| {
                    let result = install.await;
                    let _ = this.update(cx, |this, cx| match result {
                        Ok(()) => this.poll_started_daemon(cx),
                        Err(error) => this.toast(format!("{error:#}"), cx),
                    });
                })
                .detach();
            },
            ServiceCheck::Restart { running, bundled } => {
                self.outdated = Some((running, bundled));
                cx.notify();
            },
        }
    }

    /// Restart the user service so it runs the bundled (newer) daemon.
    pub fn restart_local_daemon(&mut self, cx: &mut Context<Self>) {
        let Some(moltis) = local_daemon::bundled_daemon() else {
            return;
        };
        self.outdated = None;
        cx.notify();
        let restart = cx
            .background_executor()
            .spawn(async move { local_daemon::restart_service(&moltis) });
        cx.spawn(async move |this, cx| {
            let result = restart.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.poll_started_daemon(cx),
                Err(error) => this.toast(format!("{error:#}"), cx),
            });
        })
        .detach();
    }

    /// The service was (re)installed or restarted; the daemon needs a few
    /// seconds to listen. Retry every half second instead of waiting out the
    /// reconnect backoff, until the connection has dropped (a restart takes
    /// down the one we have) and come back.
    fn poll_started_daemon(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let mut went_down = false;
            for _ in 0..120 {
                let waiting = this
                    .update(cx, |this, _| {
                        let connected = matches!(this.phase, Phase::Connected);
                        went_down |= !connected;
                        let waiting = this.generation == generation && !(went_down && connected);
                        if waiting
                            && !connected
                            && let Some(client) = &this.client
                        {
                            client.reconnect_now();
                        }
                        waiting
                    })
                    .unwrap_or(false);
                if !waiting {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
        })
        .detach();
    }

    // ── Data ────────────────────────────────────────────────────────────

    fn invalidate_data(&mut self) {
        self.data_epoch += 1;
        self.transcripts.invalidate();
        self.contexts.clear();
    }

    /// Refetch sessions and models (after connecting or missing events).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let epoch = self.data_epoch;
        let fetch = runtime::spawn(cx, async move {
            tokio::join!(
                client.sessions_list(),
                client.models_list(),
                client.agent_identity()
            )
        });
        cx.spawn(async move |this, cx| {
            let Some((sessions, models, identity)) = fetch.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.data_epoch != epoch {
                    return;
                }
                match sessions {
                    Ok(sessions) => {
                        this.sessions.replace(sessions);
                        cx.emit(DaemonEvent::Sessions);
                        if let Some(key) = this.sessions.selected().map(String::from)
                            && this.transcripts.needs_load(&key)
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
                match identity {
                    Ok(identity) => this.identity = Some(identity),
                    Err(error) => crate::logging::debug!(%error, "agent identity unavailable"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_history(&mut self, session_key: &str, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Connected) {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let Some(request) = self.transcripts.begin_load(session_key) else {
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
                    if !this.transcripts.finish_load(&key, request, &history) {
                        return;
                    }
                    this.fetch_context(&key, cx);
                    cx.emit(DaemonEvent::Transcript {
                        session_key: key,
                        change: Change::Reset,
                    });
                    cx.notify();
                },
                Err(error) => {
                    if this.transcripts.fail_load(&key, request) {
                        this.toast(format!("Could not load the conversation: {error}"), cx);
                    }
                },
            });
        })
        .detach();
    }

    /// Refetch a session's token usage (after loading it or a finished turn).
    fn fetch_context(&mut self, session_key: &str, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let epoch = self.data_epoch;
        let key = session_key.to_string();
        let fetch = runtime::spawn(cx, {
            let key = key.clone();
            async move { client.chat_context(&key).await }
        });
        cx.spawn(async move |this, cx| {
            let Some(result) = fetch.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.data_epoch != epoch {
                    return;
                }
                match result {
                    Ok(context) => {
                        this.contexts.insert(key, context.token_usage);
                        cx.notify();
                    },
                    Err(error) => crate::logging::debug!(%error, "session context unavailable"),
                }
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
        if matches!(
            event.kind,
            ChatEventKind::Final(_) | ChatEventKind::Error { .. } | ChatEventKind::Aborted { .. }
        ) {
            self.fetch_context(&event.session_key, cx);
        }
        let (row_changed, unknown) = self.sessions.apply_chat_event(&event);
        if unknown {
            self.refresh(cx);
        } else if row_changed {
            cx.emit(DaemonEvent::Sessions);
        }
        let change = self.transcripts.apply(&event);
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
            if self.transcripts.needs_load(key) {
                self.load_history(key, cx);
            }
            return;
        }
        cx.emit(DaemonEvent::Sessions);
        if self.transcripts.get(key).is_some() {
            cx.emit(DaemonEvent::Transcript {
                session_key: key.to_string(),
                change: Change::Reset,
            });
        }
        if self.transcripts.needs_load(key) {
            self.load_history(key, cx);
        }
        cx.notify();
    }

    pub fn new_session(&mut self, cx: &mut Context<Self>) {
        let key = new_session_key();
        self.transcripts.insert_draft(&key);
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

    pub fn send(&mut self, text: String, cx: &mut Context<Self>) -> Option<Task<bool>> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        if !matches!(self.phase, Phase::Connected) {
            self.toast("Not connected to the daemon. Your draft has been kept.", cx);
            return None;
        }
        let (Some(client), Some(session_key)) = (
            self.client.clone(),
            self.sessions.selected().map(String::from),
        ) else {
            self.toast("Not connected to the daemon.", cx);
            return None;
        };
        self.seq += 1;
        let params = ChatSendParams {
            text,
            session_key: Some(session_key),
            model: self.model.clone(),
            seq: Some(self.seq),
        };
        let sent = runtime::spawn(cx, async move { client.chat_send(&params).await });
        Some(cx.spawn(async move |this, cx| match sent.await {
            Some(Ok(_)) => true,
            result => {
                let message = match result {
                    Some(Err(error)) => format!("Message not sent: {error}"),
                    _ => "Message not sent: network runtime stopped.".into(),
                };
                let _ = this.update(cx, |this, cx| this.toast(message, cx));
                false
            },
        }))
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

/// A service installed on this computer means the user chose to run moltis
/// here, so a stopped daemon is started without asking.
fn installed_auto_start() -> Option<AutoStart> {
    let home = moltis_config::home_dir()?;
    let installed = local_daemon::installed_service_binary(&home, &moltis_config::data_dir());
    let bundled = local_daemon::bundled_daemon().map(|moltis| local_daemon::service_path(&moltis));
    local_daemon::auto_start(installed.as_deref(), bundled.as_deref())
}

#[cfg(test)]
#[path = "daemon/tests.rs"]
mod tests;
