//! Small shared building blocks: buttons, cards, labels.

use gpui::{
    App, Div, ElementId, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Stateful,
    Styled, div, prelude::*, px,
};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

/// A clickable label; attach `.on_click(...)`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    cx: &App,
) -> Stateful<Div> {
    let theme = Theme::global(cx);
    let (background, hover, text, border) = match kind {
        ButtonKind::Primary => (
            theme.accent,
            theme.accent.opacity(0.85),
            theme.accent_text,
            theme.accent,
        ),
        ButtonKind::Secondary => (theme.surface, theme.surface_hover, theme.text, theme.border),
        ButtonKind::Danger => (
            theme.danger,
            theme.danger.opacity(0.85),
            theme.accent_text,
            theme.danger,
        ),
        ButtonKind::Ghost => (
            transparent(),
            theme.surface_hover,
            theme.text_muted,
            transparent(),
        ),
    };
    div()
        .id(id.into())
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(background)
        .text_color(text)
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label.into())
}

fn transparent() -> Hsla {
    gpui::transparent_black()
}

/// A bordered panel.
pub fn card(cx: &App) -> Div {
    let theme = Theme::global(cx);
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
}

/// A field label above an input.
pub fn label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(Theme::global(cx).text_muted)
        .child(text.into())
}

/// The frame around a text input.
pub fn field(cx: &App) -> Div {
    let theme = Theme::global(cx);
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .min_h(px(28.))
}

/// Muted secondary text.
pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(Theme::global(cx).text_muted)
        .child(text.into())
}

/// A centred column for full-window states (connecting, login...).
pub fn centered(content: impl IntoElement) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(div().w(px(460.)).max_w_full().child(content))
}
