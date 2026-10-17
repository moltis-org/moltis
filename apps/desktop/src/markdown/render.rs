//! Markdown blocks to native gpui elements.

use std::ops::Range;

use gpui::{
    AnyElement, App, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveText,
    IntoElement, ParentElement, SharedString, StrikethroughStyle, Styled, StyledText,
    UnderlineStyle, div, prelude::FluentBuilder, px,
};

use crate::{
    markdown::{
        highlight::highlight,
        parse::{Block, Inline},
    },
    theme::Theme,
};

/// Render `blocks`; `id` must be unique per message so link handlers and
/// element ids do not collide.
pub fn render(id: &str, blocks: &[Block], cx: &App) -> AnyElement {
    let theme = Theme::global(cx);
    div()
        .flex()
        .flex_col()
        .gap_3()
        .line_height(px(23.))
        .children(
            blocks
                .iter()
                .enumerate()
                .map(|(i, block)| render_block(&format!("{id}-{i}"), block, theme, 0)),
        )
        .into_any_element()
}

/// Deeper nesting (from model output) is not rendered, bounding recursion.
const MAX_DEPTH: usize = 12;

fn render_block(id: &str, block: &Block, theme: &Theme, depth: usize) -> AnyElement {
    if depth > MAX_DEPTH {
        return div()
            .text_color(theme.text_muted)
            .child("…")
            .into_any_element();
    }
    match block {
        Block::Paragraph(inline) => inline_element(id, inline, theme),
        Block::Heading { level, content } => {
            let size = match level {
                1 => px(22.),
                2 => px(19.),
                3 => px(17.),
                _ => px(15.),
            };
            div()
                .pt_1()
                .text_size(size)
                .font_weight(FontWeight::SEMIBOLD)
                .child(inline_element(id, content, theme))
                .into_any_element()
        },
        Block::Code { lang, code } => code_block(code, lang.as_deref(), theme),
        Block::List { start, items } => div()
            .flex()
            .flex_col()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, item)| {
                let marker = match start {
                    Some(first) => format!("{}.", first + i as u64),
                    None => "•".into(),
                };
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .min_w(px(18.))
                            .text_color(theme.text_muted)
                            .child(marker),
                    )
                    .child(div().flex_1().flex().flex_col().gap_1().children(
                        item.iter().enumerate().map(|(j, block)| {
                            render_block(&format!("{id}-{i}-{j}"), block, theme, depth + 1)
                        }),
                    ))
            }))
            .into_any_element(),
        Block::Quote(blocks) => div()
            .pl_3()
            .border_l_2()
            .border_color(theme.border)
            .text_color(theme.text_muted)
            .flex()
            .flex_col()
            .gap_2()
            .children(
                blocks
                    .iter()
                    .enumerate()
                    .map(|(i, block)| render_block(&format!("{id}-{i}"), block, theme, depth + 1)),
            )
            .into_any_element(),
        Block::Rule => div()
            .my_1()
            .h(px(1.))
            .w_full()
            .bg(theme.border)
            .into_any_element(),
        Block::Table { header, rows } => table(id, header, rows, theme),
    }
}

fn code_block(code: &str, lang: Option<&str>, theme: &Theme) -> AnyElement {
    let highlights = highlight(code, lang)
        .into_iter()
        .map(|(range, kind)| {
            (range, HighlightStyle {
                color: Some(theme.syntax(kind)),
                ..HighlightStyle::default()
            })
        })
        .collect::<Vec<_>>();
    div()
        .w_full()
        .rounded_lg()
        .overflow_hidden()
        .bg(theme.code_background)
        .border_1()
        .border_color(theme.border)
        .when_some(lang.filter(|lang| !lang.is_empty()), |this, lang| {
            this.child(
                div()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .line_height(px(18.))
                    .text_color(theme.text_muted)
                    .child(lang.to_string()),
            )
        })
        .child(
            div()
                .px_3()
                .py_2()
                .font_family(theme.mono_font.clone())
                .text_size(px(12.5))
                .line_height(px(19.))
                .child(
                    StyledText::new(SharedString::from(code.to_string()))
                        .with_highlights(highlights),
                ),
        )
        .into_any_element()
}

fn table(id: &str, header: &[Inline], rows: &[Vec<Inline>], theme: &Theme) -> AnyElement {
    let columns = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let row = |cells: &[Inline], row_id: String, bold: bool| {
        div()
            .flex()
            .border_b_1()
            .border_color(theme.border)
            .children((0..columns).map(|c| {
                let cell = cells.get(c).cloned().unwrap_or_default();
                div()
                    .flex_1()
                    .px_2()
                    .py_1()
                    .when(bold, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(inline_element(&format!("{row_id}-{c}"), &cell, theme))
            }))
    };
    div()
        .flex()
        .flex_col()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .child(row(header, format!("{id}-h"), true))
        .children(
            rows.iter()
                .enumerate()
                .map(|(i, cells)| row(cells, format!("{id}-{i}"), false)),
        )
        .into_any_element()
}

/// Styled text, clickable where it links.
fn inline_element(id: &str, inline: &Inline, theme: &Theme) -> AnyElement {
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut fonts: Vec<(Range<usize>, SharedString)> = Vec::new();
    for span in &inline.spans {
        let style = &span.style;
        let mut highlight = HighlightStyle::default();
        if style.bold {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if style.italic {
            highlight.font_style = Some(FontStyle::Italic);
        }
        if style.strike {
            highlight.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if style.code {
            highlight.background_color = Some(theme.code_background);
            fonts.push((span.range.clone(), theme.mono_font.clone()));
        }
        if style.link.is_some() {
            highlight.color = Some(theme.link);
            highlight.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(theme.link),
                wavy: false,
            });
        }
        highlights.push((span.range.clone(), highlight));
    }
    let text = StyledText::new(SharedString::from(inline.text.clone()))
        .with_highlights(highlights)
        .with_font_family_overrides(fonts);
    let links: Vec<(Range<usize>, String)> = inline
        .links()
        .into_iter()
        .filter(|(_, url)| is_safe_link(url))
        .map(|(range, url)| (range, url.to_string()))
        .collect();
    if links.is_empty() {
        return text.into_any_element();
    }
    let ranges = links.iter().map(|(range, _)| range.clone()).collect();
    InteractiveText::new(ElementId::Name(SharedString::from(id.to_string())), text)
        .on_click(ranges, move |index, _, cx| {
            if let Some((_, url)) = links.get(index) {
                cx.open_url(url);
            }
        })
        .into_any_element()
}

/// Only web and mail links open; `file:`, `javascript:` and custom schemes
/// from model output are not followed.
fn is_safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_open() {
        assert!(is_safe_link("https://moltis.org"));
        assert!(is_safe_link(" HTTP://x"));
        assert!(is_safe_link("mailto:a@b.c"));
        assert!(!is_safe_link("file:///etc/passwd"));
        assert!(!is_safe_link("javascript:alert(1)"));
        assert!(!is_safe_link("x-apple.systempreferences:"));
        assert!(!is_safe_link("/relative"));
    }
}
