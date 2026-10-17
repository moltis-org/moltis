//! The left pane: shortcuts, the sessions of the connected daemon grouped by
//! day, and a footer with the owner and the server switcher.

use gpui::{
    Action, AnyElement, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};

use {
    crate::{
        actions::{NewSession, OpenWebCrons, OpenWebSettings, OpenWebSkills, ShowServers},
        assets::{Icon, icon},
        daemon::{Daemon, Phase},
        sessions::{Recency, today_start_ms},
        theme::Theme,
    },
    moltis_client::protocol::client_api::SessionEntry,
};

pub const WIDTH: f32 = 260.;

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

    fn render_row(
        &self,
        index: usize,
        entry: &SessionEntry,
        selected: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = entry.key.clone();
        let unread = entry.has_unread() && !selected;
        let (bullet, bullet_color) = if entry.replying {
            ("●", theme.brand)
        } else {
            ("○", theme.text_faint)
        };
        let hover = theme.surface_hover;
        div()
            .id(("session", index))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py(px(5.))
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.selected))
            .when(!selected, |this| this.hover(move |style| style.bg(hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.daemon
                    .update(cx, |daemon, cx| daemon.select_session(&key, cx));
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(14.))
                    .text_xs()
                    .text_color(bullet_color)
                    .child(bullet),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(unread, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(entry.title().to_string()),
            )
            .into_any_element()
    }
}

fn nav_item(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    action: impl Action,
    theme: &Theme,
) -> impl IntoElement {
    let hover = theme.surface_hover;
    let action = action.boxed_clone();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(5.))
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .hover(move |style| style.bg(hover))
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .child(icon(glyph, 16.).text_color(theme.text_muted))
        .child(label)
}

fn section_label(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_xs()
        .text_color(theme.text_faint)
        .child(text)
}

/// Up to two initials for the avatar: "Fabien Penso" → "FP".
#[must_use]
pub fn initials(name: &str) -> String {
    let initials: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if initials.is_empty() {
        "?".into()
    } else {
        initials
    }
}

/// The colour of the server's status dot.
fn status_color(phase: &Phase, theme: &Theme) -> Hsla {
    match phase {
        Phase::Connected => theme.success,
        Phase::Reconnecting { .. } | Phase::Connecting { .. } | Phase::NeedsLogin { .. } => {
            theme.warning
        },
        _ => theme.danger,
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let daemon = self.daemon.read(cx);
        let selected = daemon.sessions().selected().map(String::from);
        let entries = daemon.sessions().entries().to_vec();
        let server: SharedString = daemon.profile().name.clone().into();
        let status_color = status_color(daemon.phase(), &theme);
        let owner = daemon
            .identity()
            .and_then(|identity| identity.user_name.clone())
            .filter(|name| !name.trim().is_empty());

        let today_start = today_start_ms();
        let mut today = Vec::new();
        let mut earlier = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let is_selected = selected.as_deref() == Some(entry.key.as_str());
            let row = self.render_row(index, entry, is_selected, &theme, cx);
            match Recency::of(entry.updated_at, today_start) {
                Recency::Today => today.push(row),
                Recency::Earlier => earlier.push(row),
            }
        }

        let hover = theme.surface_hover;
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("server-switcher")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowServers), cx))
                    .child(
                        div()
                            .flex_none()
                            .size(px(22.))
                            .rounded_full()
                            .bg(theme.selected)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(initials(owner.as_deref().unwrap_or(&server))),
                    )
                    .child(match &owner {
                        Some(owner) => div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().flex_none().text_sm().child(owner.clone()))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(format!("· {server}")),
                            ),
                        None => div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(server.clone()),
                    })
                    .child(icon(Icon::ChevronDown, 10.).text_color(theme.text_muted))
                    .child(
                        div()
                            .flex_none()
                            .mx_1()
                            .size(px(8.))
                            .rounded_full()
                            .bg(status_color),
                    ),
            )
            .child(
                div()
                    .id("settings")
                    .flex_none()
                    .size(px(26.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(theme.text_muted)
                    .hover(move |style| style.bg(hover))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenWebSettings), cx);
                    })
                    .child(icon(Icon::Settings, 17.).text_color(theme.text_muted)),
            );

        div()
            .w(px(WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(crate::titlebar::movable(
                div()
                    .id("sidebar-titlebar")
                    .flex_none()
                    .h(px(crate::titlebar::INSET)),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_2()
                    .child(nav_item(
                        "nav-new",
                        Icon::Plus,
                        "New chat",
                        NewSession,
                        &theme,
                    ))
                    .child(nav_item(
                        "nav-scheduled",
                        Icon::Clock,
                        "Scheduled",
                        OpenWebCrons,
                        &theme,
                    ))
                    .child(nav_item(
                        "nav-skills",
                        Icon::Sparkles,
                        "Skills",
                        OpenWebSkills,
                        &theme,
                    )),
            )
            .child(
                div()
                    .id("sessions")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .when(!today.is_empty(), |this| {
                        this.child(section_label("Today", &theme)).children(today)
                    })
                    .when(!earlier.is_empty(), |this| {
                        this.child(section_label("Earlier", &theme))
                            .children(earlier)
                    }),
            )
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_the_first_two_words() {
        assert_eq!(initials("Fabien Penso"), "FP");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("Jean Luc Picard"), "JL");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("élodie durand"), "ÉD");
    }
}
