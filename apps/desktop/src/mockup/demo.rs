// The built-in mockup, shown when no MOLTIS_MOCKUP_FILE was given. It is also
// the template for an agent's own file: the same imports, the same
// `pub(super) fn mockup() -> Mockup`, and both kinds of body. Each body's
// root takes `.flex_1()` to fill its frame. A file compiled
// in through `include!` may not start with inner attributes or `//!` docs.

use gpui::{prelude::*, *};

use {
    super::{Body, Mockup, Variant},
    crate::theme::Theme,
};

pub(super) fn mockup() -> Mockup {
    Mockup {
        title: "Sidebar server footer",
        // The sidebar's width, and the footer's least height.
        frame: size(px(260.), px(64.)),
        variants: vec![
            Variant {
                name: "Current",
                note: "Today's footer: dot, name, status and version, Switch.",
                body: Body::Element(current),
            },
            Variant {
                name: "Pill",
                note: "One line; status as a coloured pill, version on hover.",
                body: Body::Element(pill),
            },
            Variant {
                name: "Accent bar",
                note: "A status-coloured bar on the left; no dot.",
                body: Body::Element(accent_bar),
            },
            Variant {
                name: "Expandable",
                note: "Click to reveal the address and the other servers.",
                body: Body::View(|_, cx| cx.new(|_| Expandable::default()).into()),
            },
        ],
    }
}

/// Realistic content every variant shows, so they differ only in design.
const NAME: &str = "Studio (Mac mini, living room)";
const STATUS: &str = "Connecting";
const VERSION: &str = "20261015.02";
const ADDRESS: &str = "ssh penso@studio.local → 127.0.0.1:18789";
const OTHERS: [&str; 2] = ["This Mac", "hetzner-fsn1"];

fn status_color(theme: &Theme) -> Hsla {
    theme.warning
}

fn current(_: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::global(cx);
    div()
        .flex_1()
        .flex()
        .items_center()
        .gap_2()
        .p_3()
        .bg(theme.sidebar)
        .child(div().text_color(status_color(theme)).child("●"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_sm().truncate().child(NAME))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(format!("{STATUS} · v{VERSION}")),
                ),
        )
        .child(div().text_xs().text_color(theme.text_muted).child("Switch"))
        .into_any_element()
}

fn pill(_: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::global(cx);
    let color = status_color(theme);
    div()
        .flex_1()
        .flex()
        .items_center()
        .gap_2()
        .p_3()
        .bg(theme.sidebar)
        .child(div().flex_1().min_w_0().text_sm().truncate().child(NAME))
        .child(
            div()
                .flex_none()
                .px_2()
                .rounded_full()
                .bg(color.opacity(0.15))
                .text_xs()
                .text_color(color)
                .child(STATUS),
        )
        .into_any_element()
}

fn accent_bar(_: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::global(cx);
    div()
        .flex_1()
        .flex()
        .bg(theme.sidebar)
        .child(div().flex_none().w(px(3.)).bg(status_color(theme)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .p_3()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(NAME),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(format!("{STATUS} · v{VERSION}")),
                ),
        )
        .into_any_element()
}

/// A stateful variant: the card keeps its expanded state across theme and
/// width changes, because the window builds the view only once.
#[derive(Default)]
struct Expandable {
    open: bool,
}

impl Render for Expandable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx);
        let hover = theme.surface;
        div()
            .id("expandable")
            .flex_1()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .bg(theme.sidebar)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open = !this.open;
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(status_color(theme)).child("●"))
                    .child(div().flex_1().min_w_0().text_sm().truncate().child(NAME))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(if self.open {
                                "▴"
                            } else {
                                "▾"
                            }),
                    ),
            )
            .when(self.open, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .truncate()
                        .child(ADDRESS),
                )
                .children(OTHERS.map(|name| {
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme.surface)
                        .text_sm()
                        .child(name)
                }))
            })
    }
}
