//! The comparison window: every variant framed side by side, with a pick
//! toggle and a note for each, and "Send to agent" for the lot.

use std::path::PathBuf;

use gpui::{
    AnyElement, AnyView, App, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    KeyDownEvent, Pixels, SharedString, Stateful, Window, actions, div, prelude::*, px,
};

use {
    super::{Body, Mockup, feedback},
    crate::{input::TextInput, theme::Theme, ui},
};

actions!(mockup, [Close, Send]);

const CONTEXT: &str = "MockupWindow";

/// The window's own keys. Note fields keep Escape, which hands focus back to
/// the window so the number keys choose a variant again.
pub fn key_bindings() -> [KeyBinding; 2] {
    let platform = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    [
        KeyBinding::new(&format!("{platform}-w"), Close, Some(CONTEXT)),
        KeyBinding::new(&format!("{platform}-enter"), Send, Some(CONTEXT)),
    ]
}

/// How wide each frame is drawn, relative to the width it was designed for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Width {
    Narrow,
    #[default]
    Design,
    Wide,
}

impl Width {
    const ALL: [Self; 3] = [Self::Narrow, Self::Design, Self::Wide];

    fn label(self) -> &'static str {
        match self {
            Self::Narrow => "Narrow",
            Self::Design => "Design",
            Self::Wide => "Wide",
        }
    }

    fn of(self, design: Pixels) -> Pixels {
        match self {
            Self::Narrow => (design * 0.66).round(),
            Self::Design => design,
            Self::Wide => (design * 1.5).round(),
        }
    }
}

/// The app follows the system appearance, so these are the looks it ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Light,
    Dark,
}

impl Look {
    const ALL: [Self; 2] = [Self::Light, Self::Dark];

    fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn theme(self) -> Theme {
        match self {
            Self::Light => Theme::light(),
            Self::Dark => Theme::dark(),
        }
    }
}

struct Card {
    picked: bool,
    note: Entity<TextInput>,
    /// Built once for `Body::View` variants, so their state is kept.
    view: Option<AnyView>,
}

pub struct MockupWindow {
    mockup: Mockup,
    look: Look,
    width: Width,
    /// One variant shown alone, or all of them.
    solo: Option<usize>,
    cards: Vec<Card>,
    overall: Entity<TextInput>,
    feedback: Option<PathBuf>,
    status: SharedString,
    focus: FocusHandle,
}

fn letter(index: usize) -> char {
    u8::try_from(index)
        .ok()
        .filter(|index| *index < 26)
        .map_or('?', |index| char::from(b'A' + index))
}

/// A toolbar toggle: accent-filled when on.
fn toggle(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    cx: &App,
) -> Stateful<Div> {
    let kind = if on {
        ui::ButtonKind::Primary
    } else {
        ui::ButtonKind::Secondary
    };
    ui::button(id, label, kind, cx).flex_none()
}

impl MockupWindow {
    pub fn new(
        mockup: Mockup,
        feedback: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let look = if Theme::global(cx).dark {
            Look::Dark
        } else {
            Look::Light
        };
        let cards = mockup
            .variants
            .iter()
            .enumerate()
            .map(|(index, variant)| Card {
                picked: false,
                note: cx
                    .new(|cx| TextInput::single_line(format!("Note on {}...", letter(index)), cx)),
                view: match variant.body {
                    Body::View(build) => Some(build(window, cx)),
                    Body::Element(_) => None,
                },
            })
            .collect();
        let overall = cx
            .new(|cx| TextInput::single_line("Overall note, e.g. B's header with C's list...", cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            mockup,
            look,
            width: Width::default(),
            solo: None,
            cards,
            overall,
            feedback,
            status: "Pick variants, add notes, then send them to the agent".into(),
            focus,
        }
    }

    fn inputs(&self) -> impl Iterator<Item = &Entity<TextInput>> {
        self.cards
            .iter()
            .map(|card| &card.note)
            .chain([&self.overall])
    }

    fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.look = look;
        cx.set_global(look.theme());
        cx.notify();
    }

    fn width_label(&self) -> String {
        format!(
            "{} ({} px)",
            self.width.label(),
            f32::from(self.width.of(self.mockup.frame.width))
        )
    }

    fn report(&self, cx: &App) -> String {
        feedback::Report {
            title: self.mockup.title,
            theme: self.look.label(),
            width: &self.width_label(),
            choices: self
                .mockup
                .variants
                .iter()
                .zip(&self.cards)
                .enumerate()
                .map(|(index, (variant, card))| feedback::Choice {
                    letter: letter(index),
                    name: variant.name,
                    picked: card.picked,
                    note: card.note.read(cx).text(),
                })
                .collect(),
            overall: self.overall.read(cx).text(),
        }
        .text()
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let text = self.report(cx);
        // The terminal copy is for an agent that reads the process output.
        println!("{text}");
        self.status = match &self.feedback {
            None => "Printed to the terminal (no --feedback file)".into(),
            Some(path) => match feedback::write(path, text.as_bytes()) {
                Ok(()) => format!("Sent to {}", path.display()).into(),
                Err(error) => {
                    eprintln!("mockup: unable to write {}: {error}", path.display());
                    format!("Not sent: {error}").into()
                },
            },
        };
        // The skill reads this line to know where the notes went.
        println!("mockup: sent: {}", self.status);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            // Escape belongs to the native input system during composition.
            if self.inputs().any(|input| {
                let input = input.read(cx);
                input.focus_handle(cx).is_focused(window) && input.is_composing()
            }) {
                return;
            }
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        // Digits belong to a note field while one has focus.
        if !self.focus.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }
        let Some(digit) = key.parse::<usize>().ok().filter(|_| key.len() == 1) else {
            return;
        };
        self.solo = digit
            .checked_sub(1)
            .filter(|index| *index < self.cards.len());
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let theme = Theme::global(cx);
        let label = |text: &'static str| {
            div()
                .flex_none()
                .text_sm()
                .text_color(theme.text_muted)
                .child(text)
        };
        let gap = || div().flex_none().w_3();
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                div()
                    .flex_none()
                    .mr_3()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.mockup.title),
            )
            .child(label("Theme"))
            .children(Look::ALL.into_iter().map(|look| {
                toggle(look.label(), look.label(), look == self.look, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_look(look, cx)))
            }))
            .child(gap())
            .child(label("Width"))
            .children(Width::ALL.into_iter().map(|width| {
                toggle(width.label(), width.label(), width == self.width, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.width = width;
                        cx.notify();
                    },
                ))
            }))
            .child(gap())
            .child(label("Show"))
            .child(
                toggle("show-all", "All", self.solo.is_none(), cx)
                    .debug_selector(|| "mockup-show-all".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.solo = None;
                        cx.notify();
                    })),
            )
            .children((0..self.cards.len()).map(|index| {
                toggle(
                    ("show", index),
                    letter(index).to_string(),
                    self.solo == Some(index),
                    cx,
                )
                .debug_selector(move || format!("mockup-show-{}", letter(index)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.solo = Some(index);
                    cx.notify();
                }))
            }))
    }

    fn render_card(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let variant = &self.mockup.variants[index];
        let card = &self.cards[index];
        let key = letter(index);
        let width = self.width.of(self.mockup.frame.width);
        let body = match (variant.body, &card.view) {
            (_, Some(view)) => view.clone().into_any_element(),
            (Body::Element(draw), None) => draw(window, cx),
            // A view is always built with its card.
            (Body::View(_), None) => div().into_any_element(),
        };
        let theme = Theme::global(cx);
        let pick = toggle(
            ("pick", index),
            if card.picked {
                "Picked"
            } else {
                "Pick"
            },
            card.picked,
            cx,
        )
        .debug_selector(move || format!("mockup-pick-{key}"))
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(card) = this.cards.get_mut(index) {
                card.picked = !card.picked;
                cx.notify();
            }
        }));
        div()
            .id(("variant", index))
            .debug_selector(move || format!("variant-{key}"))
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .p_3()
            .w(width + px(26.))
            .rounded_lg()
            .bg(theme.surface)
            .border_1()
            .border_color(if card.picked {
                theme.accent
            } else {
                theme.border
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(theme.accent)
                            .text_color(theme.accent_text)
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(key.to_string()),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(variant.name))
                    .child(pick),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(variant.note),
            )
            .child(
                // The frame the variant would sit in: the window background,
                // at the width it was designed for.
                div()
                    .debug_selector(move || format!("frame-{key}"))
                    .w(width)
                    .min_h(self.mockup.frame.height)
                    // A column, so a body's `.flex_1()` fills the least height.
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(body),
            )
            .child(ui::field(cx).bg(theme.background).child(card.note.clone()))
            .into_any_element()
    }
}

impl Render for MockupWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: Vec<_> = match self.solo {
            Some(index) => vec![index],
            None => (0..self.cards.len()).collect(),
        };
        let cards: Vec<_> = shown
            .into_iter()
            .map(|index| self.render_card(index, window, cx))
            .collect();
        let empty = cards.is_empty();
        let toolbar = self.render_toolbar(cx);
        let send = toggle("send", "Send to agent", true, cx)
            .debug_selector(|| "mockup-send".into())
            .on_click(cx.listener(|this, _, _, cx| this.send(cx)));
        let overall = ui::field(cx).child(self.overall.clone());
        let theme = Theme::global(cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &Close, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &Send, _, cx| this.send(cx)))
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.text)
            .font_family(theme.ui_font.clone())
            .child(toolbar)
            .child(
                div()
                    .id("mockup-grid")
                    .debug_selector(|| "mockup-grid".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_wrap()
                    .content_start()
                    .items_start()
                    .gap_4()
                    .when(empty, |el| {
                        el.child(
                            div()
                                .text_color(theme.text_muted)
                                .child("This mockup has no variants."),
                        )
                    })
                    .children(cards),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .child(div().flex_1().min_w_0().child(overall))
                    .child(send)
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(420.))
                            .truncate()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(self.status.clone()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_run_a_to_z_then_give_up() {
        assert_eq!(letter(0), 'A');
        assert_eq!(letter(25), 'Z');
        assert_eq!(letter(26), '?');
    }

    #[test]
    fn widths_scale_the_design_width() {
        assert_eq!(Width::Narrow.of(px(300.)), px(198.));
        assert_eq!(Width::Design.of(px(300.)), px(300.));
        assert_eq!(Width::Wide.of(px(300.)), px(450.));
    }
}
