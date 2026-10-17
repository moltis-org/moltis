//! Full-window states while not chatting: connecting, login, certificate
//! trust, choosing where moltis runs, failures.

use gpui::{
    Context, Div, Entity, FontWeight, IntoElement, ParentElement, Render, Stateful, Styled,
    Subscription, Window, div, prelude::*,
};

use {
    moltis_client::{SecretString, tls::display_fingerprint},
    std::time::Duration,
};

use crate::{
    actions::AddServer,
    assets::{Icon, icon},
    daemon::{Daemon, Phase},
    input::{InputEvent, TextInput},
    local_daemon,
    theme::Theme,
    ui::{ButtonKind, button, card, centered, field, label, muted},
};

pub struct StatusView {
    daemon: Entity<Daemon>,
    password: Entity<TextInput>,
    api_key: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

impl StatusView {
    pub fn new(daemon: Entity<Daemon>, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| TextInput::single_line("Password", cx).masked());
        let api_key = cx.new(|cx| TextInput::single_line("mk_…", cx).masked());
        let subscriptions = vec![
            cx.observe(&daemon, |_, _, cx| cx.notify()),
            cx.subscribe(&password, |this, _, event, cx| {
                if *event == InputEvent::Submit {
                    this.login(cx);
                }
            }),
            cx.subscribe(&api_key, |this, _, event, cx| {
                if *event == InputEvent::Submit {
                    this.use_key(cx);
                }
            }),
        ];
        Self {
            daemon,
            password,
            api_key,
            _subscriptions: subscriptions,
        }
    }

    /// Whether this view has something to show (otherwise the chat shows).
    pub fn is_blocking(&self, cx: &gpui::App) -> bool {
        !matches!(
            self.daemon.read(cx).phase(),
            Phase::Connected | Phase::Reconnecting { .. }
        )
    }

    pub fn password_focus(&self, cx: &gpui::App) -> gpui::FocusHandle {
        gpui::Focusable::focus_handle(self.password.read(cx), cx)
    }

    fn login(&mut self, cx: &mut Context<Self>) {
        let password = self.password.read(cx).text().to_string();
        if password.is_empty() {
            return;
        }
        self.password.update(cx, |input, cx| input.clear(cx));
        self.daemon.update(cx, |daemon, cx| {
            daemon.login_with_password(SecretString::new(password), cx);
        });
    }

    fn use_key(&mut self, cx: &mut Context<Self>) {
        let key = self.api_key.read(cx).text().to_string();
        if key.is_empty() {
            return;
        }
        self.api_key.update(cx, |input, cx| input.clear(cx));
        self.daemon.update(cx, |daemon, cx| {
            daemon.use_api_key(SecretString::new(key), cx)
        });
    }

    fn render_login(
        &self,
        reason: &str,
        can_password: Option<bool>,
        busy: bool,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global(cx).clone();
        let name = self.daemon.read(cx).profile().name.clone();
        let mut panel = card(cx)
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(format!("Sign in to {name}")))
            .child(muted(
                "Moltis Desktop signs in once with the daemon password and keeps a scoped API key in your keychain.",
                cx,
            ));
        if can_password != Some(false) {
            panel = panel
                .child(label("Password", cx))
                .child(field(cx).child(self.password.clone()))
                .child(
                    div().flex().gap_2().child(
                        button(
                            "login",
                            if busy {
                                "Signing in…"
                            } else {
                                "Sign in"
                            },
                            ButtonKind::Primary,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.login(cx))),
                    ),
                );
        } else {
            panel = panel.child(muted(
                "This daemon has no password (passkeys only). Create an API key in the web UI under Settings → Security and paste it below.",
                cx,
            ));
        }
        panel
            .child(div().h_px().bg(theme.border))
            .child(label("Or paste an API key", cx))
            .child(field(cx).child(self.api_key.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("use-key", "Use key", ButtonKind::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.use_key(cx))),
                    )
                    .child(
                        button("open-web", "Open web UI", ButtonKind::Ghost, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(url) =
                                    this.daemon.read(cx).web_url("/settings/security")
                                {
                                    cx.open_url(&url);
                                }
                            }),
                        ),
                    ),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(error.to_string()),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(reason.to_string()),
            )
            .into_any_element()
    }
}

impl StatusView {
    /// No daemon answers on this computer: run one here, or connect to
    /// another computer instead (nothing gets installed then).
    fn render_not_running(
        &self,
        moltis: Option<&std::path::Path>,
        starting: bool,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::global(cx).clone();
        if starting {
            return card(cx)
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Starting Moltis on this computer…"),
                )
                .child(muted(
                    "Installing the daemon as a background service. This window connects as soon as it is up.",
                    cx,
                ))
                .into_any_element();
        }
        let found = moltis.is_some();
        let bundled =
            moltis.and_then(std::path::Path::parent) == local_daemon::bundled_dir().as_deref();
        let local_detail = match (found, bundled) {
            (true, true) => {
                "Run the daemon included with this app as a background service. It keeps running when the app quits and starts at login."
            },
            (true, false) => {
                "Run your installed moltis as a background service. It keeps running when the app quits and starts at login."
            },
            (false, _) => {
                "The moltis command was not found. Install moltis, or use an app download that includes it."
            },
        };
        card(cx)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Where should Moltis run?"),
            )
            .child(muted(
                "Moltis Desktop is a window onto a moltis daemon, which answers your channels and runs your agents.",
                cx,
            ))
            .child(
                choice("run-here", Icon::Monitor, "On this computer", local_detail, found, &theme)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.daemon
                            .update(cx, |daemon, cx| daemon.start_local_daemon(cx));
                    })),
            )
            .child(
                choice(
                    "connect-remote",
                    Icon::Globe,
                    "On another computer",
                    "Connect to a moltis daemon over HTTPS, or through an SSH tunnel. Nothing is installed here.",
                    true,
                    &theme,
                )
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(AddServer), cx);
                }),
            )
            .child(muted(
                "Starting moltis yourself? This window connects as soon as it is up.",
                cx,
            ))
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(error.to_string()),
                )
            })
            .into_any_element()
    }
}

/// One clickable option: icon, title, explanation, and a chevron.
fn choice(
    id: &'static str,
    glyph: Icon,
    title: &'static str,
    detail: &'static str,
    enabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let hover = theme.surface_hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_3()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .when(enabled, |this| {
            this.cursor_pointer().hover(move |style| style.bg(hover))
        })
        .when(!enabled, |this| this.opacity(0.55))
        .child(icon(glyph, 20.).text_color(theme.text_muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_xs().text_color(theme.text_muted).child(detail)),
        )
        .when(enabled, |this| {
            this.child(icon(Icon::ChevronRight, 14.).text_color(theme.text_faint))
        })
}

impl Render for StatusView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let phase = self.daemon.read(cx).phase().clone();
        let profile = self.daemon.read(cx).profile().clone();
        let content = match phase {
            Phase::Connected | Phase::Reconnecting { .. } => div(),
            Phase::Connecting { attempt } => card(cx)
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(format!("Connecting to {}…", profile.name)))
                .child(muted(profile.address(), cx))
                .when(attempt > 1, |this| this.child(muted(format!("Attempt {attempt}"), cx))),
            Phase::NeedsLogin {
                reason,
                status,
                busy,
                error,
            } => div().child(self.render_login(
                &reason,
                status.as_ref().map(moltis_client::auth::AuthStatus::can_login_with_password),
                busy,
                error.as_deref(),
                cx,
            )),
            Phase::UntrustedCertificate {
                fingerprint,
                changed,
            } => card(cx)
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(if changed {
                    "The server certificate changed"
                } else {
                    "Trust this server?"
                }))
                .child(muted(
                    if changed {
                        "The certificate no longer matches the one you trusted. This happens when the daemon regenerated its certificate, or when someone intercepts the connection. Only continue if you expected it."
                    } else {
                        "Moltis uses its own certificate authority. Compare this fingerprint with the one printed by the daemon (or shown in its web UI) before trusting it."
                    },
                    cx,
                ))
                .child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.code_background)
                        .font_family(theme.mono_font.clone())
                        .text_xs()
                        .child(display_fingerprint(&fingerprint)),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            button("trust", "Trust and connect", if changed { ButtonKind::Danger } else { ButtonKind::Primary }, cx)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.daemon.update(cx, |daemon, cx| daemon.trust_certificate(cx));
                                })),
                        )
                        .child(
                            button("cancel-trust", "Cancel", ButtonKind::Ghost, cx).on_click(
                                cx.listener(|_, _, window, cx| {
                                    window.dispatch_action(Box::new(crate::actions::ShowServers), cx);
                                }),
                            ),
                        ),
                ),
            Phase::NotRunning {
                moltis,
                starting,
                error,
            } => div().child(self.render_not_running(moltis.as_deref(), starting, error.as_deref(), cx)),
            Phase::Failed { reason } => card(cx)
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(format!("Could not connect to {}", profile.name)))
                .child(div().text_sm().text_color(theme.danger).child(reason))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            button("retry", "Retry", ButtonKind::Primary, cx).on_click(cx.listener(
                                |this, _, _, cx| this.daemon.update(cx, |daemon, cx| daemon.connect(cx)),
                            )),
                        )
                        .child(
                            button("servers", "Servers", ButtonKind::Ghost, cx).on_click(cx.listener(
                                |_, _, window, cx| {
                                    window.dispatch_action(Box::new(crate::actions::ShowServers), cx);
                                },
                            )),
                        ),
                ),
        };
        centered(content)
    }
}

/// "Reconnecting in 4s" for the chat banner.
#[must_use]
pub fn reconnect_text(reason: &str, retry_in: Duration) -> String {
    let seconds = retry_in.as_secs_f32().ceil() as u64;
    if seconds == 0 {
        format!("Connection lost ({reason}). Reconnecting…")
    } else {
        format!("Connection lost ({reason}). Reconnecting in {seconds}s…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_text_rounds_up() {
        assert_eq!(
            reconnect_text("closed", Duration::from_millis(1500)),
            "Connection lost (closed). Reconnecting in 2s…"
        );
        assert_eq!(
            reconnect_text("x", Duration::ZERO),
            "Connection lost (x). Reconnecting…"
        );
    }
}
