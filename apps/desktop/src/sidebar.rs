//! Sessions of the connected daemon, plus the server switcher.

use gpui::{
    Context, Entity, FontWeight, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, prelude::*, px,
};

use crate::{
    actions::{NewSession, ShowServers},
    daemon::{Daemon, Phase},
    theme::Theme,
    ui::{ButtonKind, button},
};

pub struct Sidebar {
    daemon: Entity<Daemon>,
    _subscription: Subscription,
}

impl Sidebar {
    pub fn new(daemon: Entity<Daemon>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&daemon, |_, _, cx| cx.notify());
        Self {
            daemon,
            _subscription: subscription,
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let daemon = self.daemon.read(cx);
        let selected = daemon.sessions().selected().map(String::from);
        let entries = daemon.sessions().entries().to_vec();
        let profile = daemon.profile().clone();
        let (status, status_color) = match daemon.phase() {
            Phase::Connected => ("Connected", theme.success),
            Phase::Reconnecting { .. } | Phase::Connecting { .. } => ("Connecting", theme.warning),
            Phase::NeedsLogin { .. } => ("Sign in required", theme.warning),
            _ => ("Offline", theme.danger),
        };
        let version = daemon.hello().map(|h| h.server.version.clone());

        div()
            .w(px(260.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div().p_3().child(
                    button("new-chat", "+  New chat", ButtonKind::Secondary, cx)
                        .w_full()
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(NewSession), cx);
                        }),
                ),
            )
            .child(
                div()
                    .id("sessions")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_px()
                    .children(entries.into_iter().enumerate().map(|(i, entry)| {
                        let is_selected = selected.as_deref() == Some(entry.key.as_str());
                        let key = entry.key.clone();
                        let unread = entry.has_unread() && !is_selected;
                        div()
                            .id(("session", i))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .when(is_selected, |this| this.bg(theme.surface_hover))
                            .hover(|style| style.bg(theme.surface))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.daemon
                                    .update(cx, |daemon, cx| daemon.select_session(&key, cx));
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_sm()
                                            .when(unread, |this| {
                                                this.font_weight(FontWeight::SEMIBOLD)
                                            })
                                            .child(entry.title().to_string()),
                                    )
                                    .when(entry.replying, |this| {
                                        this.child(
                                            div().text_xs().text_color(theme.accent).child("●"),
                                        )
                                    }),
                            )
                            .when(entry.label.is_some(), |this| {
                                this.when_some(entry.preview.clone(), |this, preview| {
                                    this.child(
                                        div()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_xs()
                                            .text_color(theme.text_muted)
                                            .child(preview),
                                    )
                                })
                            })
                    })),
            )
            .child(
                div()
                    .id("server-footer")
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowServers), cx))
                    .child(div().text_color(status_color).child("●"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(profile.name.clone()),
                            )
                            .child(div().text_xs().text_color(theme.text_muted).child(
                                match version {
                                    Some(version) => format!("{status} · v{version}"),
                                    None => status.to_string(),
                                },
                            )),
                    )
                    .child(div().text_xs().text_color(theme.text_muted).child("Switch")),
            )
    }
}
