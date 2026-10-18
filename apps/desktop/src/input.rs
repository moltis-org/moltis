//! A native, IME-aware text input: single-line fields and the multi-line,
//! wrapping chat composer.

mod editing;
mod layout;

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, div, fill, point,
    prelude::*, px, relative, size,
};

use {
    self::{editing::Editing, layout::TextLayout},
    crate::{actions::*, theme::Theme},
};

/// What a [`TextInput`] reports to its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    /// Enter was pressed (outside IME composition).
    Submit,
}

const MASK: char = '•';

pub struct TextInput {
    focus: FocusHandle,
    edit: Editing,
    placeholder: SharedString,
    masked: bool,
    max_lines: usize,
    layout: Option<TextLayout>,
    bounds: Option<Bounds<Pixels>>,
    scroll_y: Pixels,
    selecting: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    /// A single-line field.
    pub fn single_line(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::new(placeholder, 1, cx)
    }

    /// A wrapping field growing up to `max_lines` before it scrolls.
    pub fn multi_line(
        placeholder: impl Into<SharedString>,
        max_lines: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(placeholder, max_lines.max(2), cx)
    }

    fn new(placeholder: impl Into<SharedString>, max_lines: usize, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            edit: Editing::new(max_lines > 1),
            placeholder: placeholder.into(),
            masked: false,
            max_lines,
            layout: None,
            bounds: None,
            scroll_y: px(0.),
            selecting: false,
        }
    }

    /// Draw dots instead of the text, and keep it off the clipboard.
    #[must_use]
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    #[must_use]
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit.set_text(text);
        self.scroll_y = px(0.);
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Whether an IME composition is in progress, which owns Escape and Enter.
    #[cfg(feature = "mockup")]
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.edit.marked().is_some()
    }

    fn is_multiline(&self) -> bool {
        self.max_lines > 1
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    // ── Masking: the shaped text may be dots ────────────────────────────

    fn display_text(&self) -> String {
        if self.masked {
            std::iter::repeat_n(MASK, self.edit.text().chars().count()).collect()
        } else {
            self.edit.text().to_string()
        }
    }

    fn display_offset(&self, offset: usize) -> usize {
        if self.masked {
            self.edit.text()[..offset].chars().count() * MASK.len_utf8()
        } else {
            offset
        }
    }

    /// Text the platform asks for (IME, dictation, accessibility). A masked
    /// field answers nothing, so its secret never leaves through native
    /// text queries.
    fn queried_text(
        &self,
        range_utf16: &Range<usize>,
        actual: &mut Option<Range<usize>>,
    ) -> Option<String> {
        if self.masked {
            return None;
        }
        let range = self.edit.range_from_utf16(range_utf16);
        *actual = Some(self.edit.range_to_utf16(&range));
        Some(self.edit.text()[range].to_string())
    }

    fn real_offset(&self, offset: usize) -> usize {
        if self.masked {
            self.edit
                .text()
                .char_indices()
                .nth(offset / MASK.len_utf8())
                .map_or(self.edit.text().len(), |(i, _)| i)
        } else {
            offset
        }
    }

    // ── Positions ───────────────────────────────────────────────────────

    fn index_for_mouse(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(layout)) = (self.bounds, self.layout.as_ref()) else {
            return 0;
        };
        let local = point(
            position.x - bounds.left(),
            position.y - bounds.top() + self.scroll_y,
        );
        self.real_offset(layout.index_for(local))
    }

    /// The offset one visual row above (`-1`) or below (`1`) the cursor.
    fn vertical(&self, direction: f32) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let here = layout.position_for(self.display_offset(self.edit.cursor()));
        let target = point(here.x, here.y + layout.line_height * (direction + 0.5));
        if target.y < px(0.) || target.y > layout.height {
            return None;
        }
        Some(self.real_offset(layout.index_for(target)))
    }

    // ── Actions ─────────────────────────────────────────────────────────

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.backspace() {
            self.edited(cx);
        } else {
            window.play_system_bell();
        }
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.delete() {
            self.edited(cx);
        } else {
            window.play_system_bell();
        }
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.edit.delete_word_left() {
            self.edited(cx);
        }
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edit.delete_to_line_start() {
            self.edited(cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.left();
        cx.notify();
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.right();
        cx.notify();
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical(-1.).unwrap_or(0);
        self.edit.move_to(target);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical(1.).unwrap_or(self.edit.text().len());
        self.edit.move_to(target);
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical(-1.).unwrap_or(0);
        self.edit.select_to(target);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical(1.).unwrap_or(self.edit.text().len());
        self.edit.select_to(target);
        cx.notify();
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.move_to(self.edit.word_start(self.edit.cursor()));
        cx.notify();
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.move_to(self.edit.word_end(self.edit.cursor()));
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.edit
            .select_to(self.edit.previous_boundary(self.edit.cursor()));
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.edit
            .select_to(self.edit.next_boundary(self.edit.cursor()));
        cx.notify();
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.edit
            .select_to(self.edit.word_start(self.edit.cursor()));
        cx.notify();
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.select_to(self.edit.word_end(self.edit.cursor()));
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.select_all();
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.move_to(self.edit.line_start(self.edit.cursor()));
        cx.notify();
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.move_to(self.edit.line_end(self.edit.cursor()));
        cx.notify();
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.edit
            .select_to(self.edit.line_start(self.edit.cursor()));
        cx.notify();
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.edit.select_to(self.edit.line_end(self.edit.cursor()));
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit.insert(&text);
            self.edited(cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.masked && !self.edit.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.edit.selected_text().to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.masked || self.edit.selection().is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.edit.selected_text().to_string(),
        ));
        self.edit.insert("");
        self.edited(cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if self.edit.marked().is_some() {
            self.edit.unmark();
            cx.notify();
            return;
        }
        cx.emit(InputEvent::Submit);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_multiline() {
            self.edit.insert("\n");
            self.edited(cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let index = self.index_for_mouse(event.position);
        match event.click_count {
            2 => self.edit.select_range(self.edit.word_range(index)),
            n if n >= 3 => self
                .edit
                .select_range(self.edit.line_start(index)..self.edit.line_end(index)),
            _ if event.modifiers.shift => self.edit.select_to(index),
            _ => {
                self.selecting = true;
                self.edit.move_to(index);
            },
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.edit.select_to(self.index_for_mouse(event.position));
            cx.notify();
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        self.queried_text(&range_utf16, actual)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.edit.range_to_utf16(&self.edit.selection()),
            reversed: self.edit.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.edit
            .marked()
            .map(|range| self.edit.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.edit.unmark();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.edit.range_from_utf16(&range));
        self.edit.replace(range, text);
        self.edited(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.edit.range_from_utf16(&range));
        // The new selection is in UTF-16 units of `text` itself.
        let selected = selected_utf16.map(|range| {
            let to_bytes = |units: usize| {
                let mut count = 0;
                text.char_indices()
                    .find(|(_, c)| {
                        let found = count >= units;
                        count += c.len_utf16();
                        found
                    })
                    .map_or(text.len(), |(i, _)| i)
            };
            to_bytes(range.start)..to_bytes(range.end)
        });
        self.edit.replace_and_mark(range, text, selected);
        self.edited(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.edit.range_from_utf16(&range_utf16);
        let start = layout.position_for(self.display_offset(range.start));
        let end = layout.position_for(self.display_offset(range.end));
        let origin = point(bounds.left(), bounds.top() - self.scroll_y);
        Some(Bounds::from_corners(
            origin + start,
            origin + point(end.x, end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.edit.offset_to_utf16(self.index_for_mouse(position)))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(INPUT_CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .child(TextElement { input: cx.entity() })
    }
}

/// Paints a [`TextInput`]'s text, selection and caret.
struct TextElement {
    input: Entity<TextInput>,
}

struct Prepaint {
    layout: Option<TextLayout>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    scroll_y: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Text and runs to shape for the current state.
fn shaping(input: &TextInput, window: &Window, theme: &Theme) -> (SharedString, Vec<TextRun>) {
    let style = window.text_style();
    let (text, color) = if input.edit.text().is_empty() {
        (input.placeholder.to_string(), theme.text_muted)
    } else {
        (input.display_text(), style.color)
    };
    let base = TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = match input.edit.marked() {
        Some(marked) if !input.edit.text().is_empty() && !input.masked => [
            (0..marked.start, None),
            (
                marked.clone(),
                Some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.),
                    wavy: false,
                }),
            ),
            (marked.end..text.len(), None),
        ]
        .into_iter()
        .filter(|(range, _)| !range.is_empty())
        .map(|(range, underline)| TextRun {
            len: range.len(),
            underline,
            ..base.clone()
        })
        .collect(),
        _ => vec![base],
    };
    (text.into(), runs)
}

impl Element for TextElement {
    type PrepaintState = Prepaint;
    type RequestLayoutState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let input = self.input.read(cx);
        let max_lines = input.max_lines;
        let (text, runs) = shaping(input, window, Theme::global(cx));
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if max_lines == 1 {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let width = known.width.or(match available.width {
                    gpui::AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let layout = TextLayout::shape(&text, &runs, font_size, line_height, width, window);
                let height = layout.height.min(line_height * max_lines as f32);
                size(width.unwrap_or(px(0.)), height)
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme = Theme::global(cx).clone();
        let input = self.input.read(cx);
        let (text, runs) = shaping(input, window, &theme);
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let wrap = (input.max_lines > 1).then_some(bounds.size.width);
        let layout = TextLayout::shape(&text, &runs, font_size, line_height, wrap, window);

        let showing_text = !input.edit.text().is_empty();
        let selection = input.edit.selection();
        let cursor_at = if showing_text {
            layout.position_for(input.display_offset(input.edit.cursor()))
        } else {
            point(px(0.), px(0.))
        };

        // Keep the caret visible inside the clipped area.
        let mut scroll_y = input.scroll_y;
        if cursor_at.y < scroll_y {
            scroll_y = cursor_at.y;
        } else if cursor_at.y + line_height > scroll_y + bounds.size.height {
            scroll_y = cursor_at.y + line_height - bounds.size.height;
        }
        scroll_y = scroll_y.clamp(px(0.), (layout.height - bounds.size.height).max(px(0.)));
        let origin = point(bounds.left(), bounds.top() - scroll_y);

        let selection_quads = if showing_text && !selection.is_empty() {
            let start = layout.position_for(input.display_offset(selection.start));
            let end = layout.position_for(input.display_offset(selection.end));
            selection_rects(start, end, line_height, bounds.size.width)
                .into_iter()
                .map(|rect| fill(rect + origin, theme.selection))
                .collect()
        } else {
            Vec::new()
        };
        let cursor = selection.is_empty().then(|| {
            fill(
                Bounds::new(origin + cursor_at, size(px(1.5), line_height)),
                theme.accent,
            )
        });
        Prepaint {
            layout: Some(layout),
            selection: selection_quads,
            cursor,
            scroll_y,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let Some(layout) = prepaint.layout.take() else {
            return;
        };
        let origin = point(bounds.left(), bounds.top() - prepaint.scroll_y);
        let line_height = layout.line_height;
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            for (line, top) in layout.lines() {
                let _ = line.paint(
                    origin + point(px(0.), top),
                    line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        let scroll_y = prepaint.scroll_y;
        self.input.update(cx, |input, _| {
            input.layout = Some(layout);
            input.bounds = Some(bounds);
            input.scroll_y = scroll_y;
        });
    }
}

/// Rectangles covering a selection from `start` to `end` (caret positions,
/// relative to the text origin) across wrapped rows.
fn selection_rects(
    start: Point<Pixels>,
    end: Point<Pixels>,
    line_height: Pixels,
    width: Pixels,
) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(
            start,
            point(end.x, end.y + line_height),
        )];
    }
    let mut rects = vec![Bounds::from_corners(
        start,
        point(width, start.y + line_height),
    )];
    if end.y > start.y + line_height {
        rects.push(Bounds::from_corners(
            point(px(0.), start.y + line_height),
            point(width, end.y),
        ));
    }
    rects.push(Bounds::from_corners(
        point(px(0.), end.y),
        point(end.x, end.y + line_height),
    ));
    rects
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, TestAppContext};

    use super::*;

    #[gpui::test]
    fn masked_fields_answer_no_text_queries(cx: &mut TestAppContext) {
        let plain = cx.new(|cx| TextInput::single_line("Name", cx));
        let secret = cx.new(|cx| TextInput::single_line("Password", cx).masked());
        for input in [&plain, &secret] {
            input.update(cx, |input, cx| input.set_text("hunter2", cx));
        }
        let mut actual = None;
        let text = plain.read_with(cx, |input, _| input.queried_text(&(0..7), &mut actual));
        assert_eq!(text.as_deref(), Some("hunter2"));
        assert_eq!(actual, Some(0..7));

        let mut actual = None;
        let text = secret.read_with(cx, |input, _| input.queried_text(&(0..7), &mut actual));
        assert_eq!(text, None);
        assert_eq!(actual, None);
    }

    #[test]
    fn selection_rects_span_rows() {
        let lh = px(10.);
        let one = selection_rects(point(px(5.), px(0.)), point(px(20.), px(0.)), lh, px(100.));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].size.width, px(15.));
        let two = selection_rects(point(px(5.), px(0.)), point(px(20.), px(10.)), lh, px(100.));
        assert_eq!(two.len(), 2);
        let three = selection_rects(point(px(5.), px(0.)), point(px(20.), px(30.)), lh, px(100.));
        assert_eq!(three.len(), 3);
        assert_eq!(three[1].size.height, px(20.));
    }
}
