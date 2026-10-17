//! Rendering one transcript row.

use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, IntoElement, ParentElement, Styled, Window,
    div, prelude::*, px,
};

use crate::{
    chat_view::ChatView,
    markdown::render::render as render_markdown,
    theme::Theme,
    transcript::{AssistantMessage, ItemKind, ToolCall, ToolStatus, Usage},
};

/// Messages are centred in a readable column.
pub(super) const COLUMN: f32 = 720.;

impl ChatView {
    pub(super) fn render_item(
        &mut self,
        index: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self
            .daemon
            .read(cx)
            .selected_transcript()
            .and_then(|t| t.items().get(index).cloned())
        else {
            return div().into_any_element();
        };
        let theme = Theme::global(cx).clone();
        let body = match &item.kind {
            ItemKind::User { text } => div()
                .flex()
                .justify_end()
                .child(
                    div()
                        .max_w(px(COLUMN * 0.8))
                        .px_4()
                        .py_2()
                        .rounded_2xl()
                        .line_height(px(22.))
                        .bg(theme.user_bubble)
                        .child(text.clone()),
                )
                .into_any_element(),
            ItemKind::Assistant(message) => self.render_assistant(item.id, message, cx),
            ItemKind::Tool(tool) => self.render_tool(item.id, tool, cx),
            ItemKind::Notice { title, message } => div()
                .flex()
                .justify_center()
                .text_xs()
                .text_color(theme.text_muted)
                .child(if title.is_empty() {
                    message.clone()
                } else {
                    format!("{title} · {message}")
                })
                .into_any_element(),
            ItemKind::Error { message } => div()
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(theme.danger)
                .bg(theme.danger.opacity(0.08))
                .text_sm()
                .text_color(theme.danger)
                .child(message.clone())
                .into_any_element(),
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px_4()
            .py_2()
            .child(div().w_full().max_w(px(COLUMN)).child(body))
            .into_any_element()
    }

    fn render_assistant(
        &mut self,
        id: u64,
        message: &AssistantMessage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let blocks = self.blocks(id, &message.text);
        let expanded = self.expanded.contains(&id);
        let reasoning = message.reasoning.clone().filter(|r| !r.trim().is_empty());
        let text = message.text.clone();
        // The daemon reports the model id; show the name the picker shows.
        let model_name = message.model.as_ref().map(|id| {
            self.daemon
                .read(cx)
                .models()
                .iter()
                .find(|model| &model.id == id)
                .map_or_else(|| id.clone(), |model| model.name().to_string())
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(reasoning, |this, reasoning| {
                this.child(
                    div()
                        .id(("reasoning", id))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&id) {
                                this.expanded.insert(id);
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(if expanded {
                                    "▾ Thinking"
                                } else {
                                    "▸ Thinking"
                                }),
                        )
                        .when(expanded, |this| {
                            this.child(
                                div()
                                    .pl_3()
                                    .border_l_2()
                                    .border_color(theme.border)
                                    .text_sm()
                                    .text_color(theme.text_muted)
                                    .child(reasoning),
                            )
                        }),
                )
            })
            .child(render_markdown(&format!("md-{id}"), &blocks, cx))
            .when(message.streaming, |this| {
                this.child(div().text_color(theme.brand).child("▍"))
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .text_xs()
                    .text_color(theme.text_faint)
                    .when_some(model_name, |this, model| this.child(model))
                    .when_some(message.usage.as_ref().map(usage_text), |this, usage| {
                        this.child(usage)
                    })
                    .when(message.aborted, |this| this.child("stopped"))
                    .when(!message.streaming && !text.is_empty(), |this| {
                        this.child(
                            div()
                                .id(("copy", id))
                                .cursor_pointer()
                                .hover(|style| style.text_color(theme.text))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                })
                                .child("Copy"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_tool(&mut self, id: u64, tool: &ToolCall, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let expanded = self.expanded.contains(&id);
        let (icon, color) = match tool.status {
            ToolStatus::Running => ("⋯", theme.text_muted),
            ToolStatus::Succeeded => ("✓", theme.success),
            ToolStatus::Failed => ("✗", theme.danger),
        };
        let has_output = tool.output.as_ref().is_some_and(|o| !o.is_empty());
        div()
            .id(("tool", id))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .when(has_output, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&id) {
                            this.expanded.insert(id);
                        }
                        cx.notify();
                    }))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child(div().text_color(color).child(icon))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(tool.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .font_family(theme.mono_font.clone())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(tool.summary.clone()),
                    )
                    .when(has_output, |this| {
                        this.child(div().text_xs().text_color(theme.text_muted).child(
                            if expanded {
                                "▾"
                            } else {
                                "▸"
                            },
                        ))
                    }),
            )
            .when(expanded, |this| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .font_family(theme.mono_font.clone())
                        .text_xs()
                        .child(tool.output.clone().unwrap_or_default()),
                )
            })
            .into_any_element()
    }
}

fn usage_text(usage: &Usage) -> String {
    let mut text = format!("{} tokens", usage.output_tokens);
    if let Some(rate) = usage.tokens_per_second() {
        text.push_str(&format!(" · {rate:.0} tok/s"));
    }
    if usage.duration_ms > 0 {
        text.push_str(&format!(" · {:.1}s", usage.duration_ms as f64 / 1000.0));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_formats() {
        assert_eq!(
            usage_text(&Usage {
                input_tokens: 1,
                output_tokens: 50,
                duration_ms: 2000
            }),
            "50 tokens · 25 tok/s · 2.0s"
        );
        assert_eq!(
            usage_text(&Usage {
                input_tokens: 0,
                output_tokens: 3,
                duration_ms: 0
            }),
            "3 tokens"
        );
    }
}
