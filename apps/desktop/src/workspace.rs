//! The window's root: which daemon is connected and which screen shows.

use std::{collections::HashSet, time::Duration};

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Subscription, SystemNotification, SystemNotificationAction, Task, Window, div,
    prelude::*, px,
};

use moltis_client::protocol::client_api::ApprovalDecision;

use crate::{
    actions::*,
    chat_view::ChatView,
    daemon::{Daemon, DaemonEvent, Phase},
    servers::{LOCAL_ID, ServerProfile, ServerStore},
    servers_view::{ServersEvent, ServersView},
    sidebar::Sidebar,
    status_view::StatusView,
    theme::Theme,
};

/// How long a toast stays up.
const TOAST_DURATION: Duration = Duration::from_secs(5);
/// Longest command shown (and approvable) in a notification.
const NOTIFICATION_COMMAND_MAX: usize = 120;
/// Tag prefix of approval notifications.
const APPROVAL_TAG: &str = "approval:";

struct Connected {
    daemon: Entity<Daemon>,
    sidebar: Entity<Sidebar>,
    chat: Entity<ChatView>,
    status: Entity<StatusView>,
    _subscription: Subscription,
}

pub struct Workspace {
    store: ServerStore,
    local: Option<ServerProfile>,
    connected: Option<Connected>,
    servers: Entity<ServersView>,
    show_servers: bool,
    toast: Option<String>,
    toast_task: Option<Task<()>>,
    notified: HashSet<String>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ServerStore::load(ServerStore::default_path());
        let local = ServerProfile::local();
        let servers = cx.new(ServersView::new);
        let subscription = cx.subscribe_in(&servers, window, Self::on_servers_event);
        let mut this = Self {
            store,
            local,
            connected: None,
            servers,
            show_servers: false,
            toast: None,
            toast_task: None,
            notified: HashSet::new(),
            focus: cx.focus_handle(),
            _subscription: subscription,
        };
        let initial = this
            .store
            .last_used()
            .and_then(|id| this.profile(id))
            .or_else(|| this.local.clone())
            .or_else(|| this.store.servers().first().cloned());
        match initial {
            Some(profile) => this.connect(profile, window, cx),
            None => {
                this.show_servers = true;
                #[cfg(feature = "mdns")]
                this.servers.update(cx, |view, cx| view.browse(cx));
            },
        }
        this.sync_servers(cx);
        this
    }

    fn profile(&self, id: &str) -> Option<ServerProfile> {
        if id == LOCAL_ID {
            return self.local.clone();
        }
        self.store.get(id).cloned()
    }

    fn profiles(&self) -> Vec<ServerProfile> {
        self.local
            .iter()
            .cloned()
            .chain(self.store.servers().iter().cloned())
            .collect()
    }

    fn sync_servers(&mut self, cx: &mut Context<Self>) {
        let profiles = self.profiles();
        let active = self
            .connected
            .as_ref()
            .map(|c| c.daemon.read(cx).profile().id.clone());
        self.servers
            .update(cx, |view, cx| view.set_profiles(profiles, active, cx));
    }

    fn connect(&mut self, profile: ServerProfile, window: &mut Window, cx: &mut Context<Self>) {
        self.show_servers = false;
        if self
            .connected
            .as_ref()
            .is_some_and(|c| c.daemon.read(cx).profile().id == profile.id)
        {
            cx.notify();
            return;
        }
        self.store.set_last_used(&profile.id);
        self.save_store();
        let daemon = cx.new(|cx| Daemon::new(profile, cx));
        let subscription = cx.subscribe_in(&daemon, window, Self::on_daemon_event);
        let sidebar = cx.new(|cx| Sidebar::new(daemon.clone(), cx));
        let chat = cx.new(|cx| ChatView::new(daemon.clone(), cx));
        let status = cx.new(|cx| StatusView::new(daemon.clone(), cx));
        let focus = chat.read(cx).composer_focus(cx);
        window.focus(&focus, cx);
        self.connected = Some(Connected {
            daemon,
            sidebar,
            chat,
            status,
            _subscription: subscription,
        });
        self.notified.clear();
        self.sync_servers(cx);
        cx.notify();
    }

    fn save_store(&self) {
        if let Err(error) = self.store.save() {
            crate::logging::warn!(%error, "could not save servers.json");
        }
    }

    fn show_toast(&mut self, message: String, cx: &mut Context<Self>) {
        self.toast = Some(message);
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            let _ = this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn on_servers_event(
        &mut self,
        _: &Entity<ServersView>,
        event: &ServersEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ServersEvent::Connect(profile) => self.connect(profile.clone(), window, cx),
            ServersEvent::Save(profile) => {
                self.store.upsert(profile.clone());
                self.save_store();
                self.sync_servers(cx);
            },
            ServersEvent::Remove(id) => {
                if self
                    .connected
                    .as_ref()
                    .is_some_and(|c| c.daemon.read(cx).profile().id == *id)
                {
                    self.connected = None;
                }
                let url = self.store.get(id).map(ServerProfile::keychain_url);
                self.store.remove(id);
                self.save_store();
                if let Some(url) = url {
                    cx.delete_credentials(&url).detach();
                }
                self.sync_servers(cx);
                cx.notify();
            },
        }
    }

    fn on_daemon_event(
        &mut self,
        daemon: &Entity<Daemon>,
        event: &DaemonEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DaemonEvent::ProfileChanged(profile) => {
                if !profile.is_local() {
                    self.store.upsert(profile.clone());
                    self.save_store();
                }
                if profile.is_local() {
                    self.local = Some(profile.clone());
                }
                self.sync_servers(cx);
            },
            DaemonEvent::Toast(message) => self.show_toast(message.clone(), cx),
            DaemonEvent::Approvals => self.notify_approvals(daemon, window, cx),
            DaemonEvent::Phase => {
                if let Some(connected) = &self.connected
                    && matches!(daemon.read(cx).phase(), Phase::NeedsLogin { .. })
                {
                    let focus = connected.status.read(cx).password_focus(cx);
                    window.focus(&focus, cx);
                }
                cx.notify();
            },
            _ => {},
        }
    }

    /// Post a system notification for approvals the user has not seen,
    /// unless the window is in front.
    fn notify_approvals(
        &mut self,
        daemon: &Entity<Daemon>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let pending: Vec<_> = daemon.read(cx).approvals().to_vec();
        self.notified
            .retain(|id| pending.iter().any(|a| a.request_id == *id));
        for request in pending {
            if !self.notified.insert(request.request_id.clone()) || window.is_window_active() {
                continue;
            }
            // Approving from the notification is offered only when it shows
            // the whole command; otherwise clicking opens the app.
            let fully_shown = approvable_in_notification(&request.command);
            let body = if fully_shown {
                request.command.clone()
            } else {
                format!(
                    "{}… (open Moltis to review the full command)",
                    request
                        .command
                        .chars()
                        .map(|c| if c.is_control() {
                            ' '
                        } else {
                            c
                        })
                        .take(NOTIFICATION_COMMAND_MAX)
                        .collect::<String>()
                )
            };
            let actions = if fully_shown {
                vec![
                    SystemNotificationAction {
                        id: "approve".into(),
                        label: "Approve".into(),
                    },
                    SystemNotificationAction {
                        id: "deny".into(),
                        label: "Deny".into(),
                    },
                ]
            } else {
                Vec::new()
            };
            cx.show_system_notification(SystemNotification {
                tag: format!("{APPROVAL_TAG}{}", request.request_id).into(),
                title: "Moltis wants to run a command".into(),
                body: body.into(),
                actions,
            });
        }
    }

    /// A notification button or body was clicked.
    pub fn on_notification(&mut self, tag: &str, action: Option<&str>, cx: &mut Context<Self>) {
        let Some(request_id) = tag.strip_prefix(APPROVAL_TAG) else {
            return;
        };
        let decision = match action {
            Some("approve") => ApprovalDecision::Approved,
            Some("deny") => ApprovalDecision::Denied,
            _ => {
                cx.activate(true);
                return;
            },
        };
        if let Some(connected) = &self.connected {
            connected.daemon.update(cx, |daemon, cx| {
                daemon.resolve_approval(request_id, decision, cx)
            });
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────

    fn with_daemon(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Daemon, &mut Context<Daemon>),
    ) {
        if let Some(connected) = &self.connected {
            connected.daemon.update(cx, f);
        }
    }

    fn new_session(&mut self, _: &NewSession, window: &mut Window, cx: &mut Context<Self>) {
        self.show_servers = false;
        self.with_daemon(cx, |daemon, cx| daemon.new_session(cx));
        if let Some(connected) = &self.connected {
            let focus = connected.chat.read(cx).composer_focus(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn show_servers_action(&mut self, _: &ShowServers, _: &mut Window, cx: &mut Context<Self>) {
        self.show_servers = !self.show_servers || self.connected.is_none();
        self.sync_servers(cx);
        #[cfg(feature = "mdns")]
        if self.show_servers {
            self.servers.update(cx, |view, cx| view.browse(cx));
        }
        cx.notify();
    }

    fn add_server(&mut self, _: &AddServer, _: &mut Window, cx: &mut Context<Self>) {
        self.show_servers = true;
        self.servers.update(cx, |view, cx| view.start_adding(cx));
        self.sync_servers(cx);
        #[cfg(feature = "mdns")]
        self.servers.update(cx, |view, cx| view.browse(cx));
        cx.notify();
    }

    fn reconnect(&mut self, _: &ReconnectNow, _: &mut Window, cx: &mut Context<Self>) {
        self.with_daemon(cx, |daemon, cx| daemon.retry_now(cx));
    }

    fn sign_out(&mut self, _: &SignOut, _: &mut Window, cx: &mut Context<Self>) {
        self.with_daemon(cx, |daemon, cx| daemon.forget_credentials(cx));
    }

    fn open_web(&mut self, path: &str, cx: &mut Context<Self>) {
        let url = self
            .connected
            .as_ref()
            .and_then(|c| c.daemon.read(cx).web_url(path));
        match url {
            Some(url) => cx.open_url(&url),
            None => self.show_toast("Connect to a server first.".into(), cx),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let body = match (&self.connected, self.show_servers) {
            (Some(connected), false) => {
                let blocking = connected.status.read(cx).is_blocking(cx);
                div()
                    .size_full()
                    .flex()
                    .child(connected.sidebar.clone())
                    .child(if blocking {
                        // The status card has no header of its own.
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(titlebar_band())
                            .child(div().flex_1().min_h_0().child(connected.status.clone()))
                            .into_any_element()
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(connected.chat.clone())
                            .into_any_element()
                    })
                    .into_any_element()
            },
            _ => div()
                .size_full()
                .flex()
                .flex_col()
                .child(titlebar_band())
                .child(div().flex_1().min_h_0().child(self.servers.clone()))
                .into_any_element(),
        };
        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::new_session))
            .on_action(cx.listener(Self::show_servers_action))
            .on_action(cx.listener(Self::add_server))
            .on_action(cx.listener(Self::reconnect))
            .on_action(cx.listener(Self::sign_out))
            .on_action(cx.listener(|this, _: &OpenWebUi, _, cx| this.open_web("/", cx)))
            .on_action(
                cx.listener(|this, _: &OpenWebSettings, _, cx| this.open_web("/settings", cx)),
            )
            .on_action(cx.listener(|this, _: &OpenWebProviders, _, cx| {
                this.open_web("/settings/providers", cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenWebCrons, _, cx| this.open_web("/settings/crons", cx)),
            )
            .on_action(cx.listener(|this, _: &OpenWebSkills, _, cx| this.open_web("/skills", cx)))
            .on_action(
                cx.listener(|this, _: &OpenWebLogs, _, cx| this.open_web("/settings/logs", cx)),
            )
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.text)
            .font_family(theme.ui_font.clone())
            .text_size(px(14.))
            .child(body)
            .when_some(self.toast.clone(), |this, toast| {
                this.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .right_4()
                        .max_w(px(420.))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .shadow_lg()
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_sm()
                        .child(toast),
                )
            })
    }
}

/// An empty band that moves the window, where macOS draws the traffic lights.
fn titlebar_band() -> impl IntoElement {
    crate::titlebar::movable(
        div()
            .id("titlebar-band")
            .flex_none()
            .w_full()
            .h(px(crate::titlebar::INSET)),
    )
}

/// A command can be approved from a notification only if it is shown whole
/// and exactly as it runs: short, one line, printable ASCII. Anything else
/// (control, bidi or other format characters, look-alike Unicode) must be
/// reviewed in the app.
fn approvable_in_notification(command: &str) -> bool {
    command.len() <= NOTIFICATION_COMMAND_MAX
        && !command.trim().is_empty()
        && command.chars().all(|c| matches!(c, ' '..='~'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_commands_are_approvable_from_notifications() {
        assert!(approvable_in_notification("ls -la"));
        assert!(!approvable_in_notification("echo ok\nrm -rf ~"));
        assert!(!approvable_in_notification("echo \u{1b}[2Khidden"));
        assert!(!approvable_in_notification(
            &"a".repeat(NOTIFICATION_COMMAND_MAX + 1)
        ));
        assert!(!approvable_in_notification("   "));
        assert!(!approvable_in_notification("echo safe \u{202e}fr- mr"));
        assert!(!approvable_in_notification("ls \u{200b}-la"));
        assert!(!approvable_in_notification("cat ｆile"));
    }
}
