//! Text buffer, selection and cursor movement of a text input. Offsets are
//! UTF-8 byte offsets on grapheme boundaries; the platform input API's UTF-16
//! offsets are converted at the edge.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Editing {
    text: String,
    selection: Range<usize>,
    /// The cursor is at `selection.start` rather than `.end`.
    reversed: bool,
    /// IME composition range.
    marked: Option<Range<usize>>,
    multiline: bool,
}

impl Editing {
    #[must_use]
    pub fn new(multiline: bool) -> Self {
        Self {
            multiline,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    #[must_use]
    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    #[must_use]
    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    #[must_use]
    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = self.clean(text);
        self.selection = self.text.len()..self.text.len();
        self.reversed = false;
        self.marked = None;
    }

    fn clean(&self, text: &str) -> String {
        if self.multiline {
            text.replace("\r\n", "\n")
        } else {
            text.chars().filter(|c| !matches!(c, '\n' | '\r')).collect()
        }
    }

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selection = offset..offset;
        self.reversed = false;
    }

    /// Extend the selection so the cursor lands on `offset`.
    pub fn select_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
    }

    pub fn select_all(&mut self) {
        self.selection = 0..self.text.len();
        self.reversed = false;
    }

    /// Select `range` (from a double-click or the IME).
    pub fn select_range(&mut self, range: Range<usize>) {
        let start = self.clamp(range.start.min(range.end));
        let end = self.clamp(range.start.max(range.end));
        self.selection = start..end;
        self.reversed = false;
    }

    #[must_use]
    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    #[must_use]
    pub fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.text.len())
    }

    /// Start of the word at or before `offset`.
    #[must_use]
    pub fn word_start(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .map(|(i, _)| i)
            .rev()
            .find(|i| *i < offset)
            .unwrap_or(0)
    }

    /// End of the word at or after `offset`.
    #[must_use]
    pub fn word_end(&self, offset: usize) -> usize {
        self.text
            .unicode_word_indices()
            .map(|(i, word)| i + word.len())
            .find(|end| *end > offset)
            .unwrap_or(self.text.len())
    }

    /// The word range around `offset` (for double-click).
    #[must_use]
    pub fn word_range(&self, offset: usize) -> Range<usize> {
        self.text
            .unicode_word_indices()
            .map(|(i, word)| i..i + word.len())
            .find(|range| range.start <= offset && offset <= range.end)
            .unwrap_or(offset..offset)
    }

    #[must_use]
    pub fn line_start(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)]
            .rfind('\n')
            .map_or(0, |i| i + 1)
    }

    #[must_use]
    pub fn line_end(&self, offset: usize) -> usize {
        let offset = self.clamp(offset);
        self.text[offset..]
            .find('\n')
            .map_or(self.text.len(), |i| offset + i)
    }

    /// Replace `range` (or the marked text, or the selection) with `text`.
    pub fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = self.clamp(range.start.min(range.end));
        let end = self.clamp(range.start.max(range.end));
        let text = self.clean(text);
        self.text.replace_range(start..end, &text);
        let cursor = start + text.len();
        self.selection = cursor..cursor;
        self.reversed = false;
        self.marked = None;
    }

    /// IME: replace and mark `text`; `selected` is relative to `text`.
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = self.clamp(range.start.min(range.end));
        self.replace(Some(range), text);
        let inserted = self.clean(text);
        self.marked = (!inserted.is_empty()).then(|| start..start + inserted.len());
        if let Some(selected) = selected {
            let a = self.clamp(start + selected.start.min(inserted.len()));
            let b = self.clamp(start + selected.end.min(inserted.len()));
            self.selection = a.min(b)..a.max(b);
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn insert(&mut self, text: &str) {
        self.replace(None, text);
    }

    /// Delete the selection, or the grapheme before the cursor.
    /// Returns whether anything was deleted.
    pub fn backspace(&mut self) -> bool {
        if self.selection.is_empty() {
            let previous = self.previous_boundary(self.cursor());
            if previous == self.cursor() {
                return false;
            }
            self.select_to(previous);
        }
        self.insert("");
        true
    }

    /// Delete the selection, or the grapheme after the cursor.
    pub fn delete(&mut self) -> bool {
        if self.selection.is_empty() {
            let next = self.next_boundary(self.cursor());
            if next == self.cursor() {
                return false;
            }
            self.select_to(next);
        }
        self.insert("");
        true
    }

    pub fn delete_word_left(&mut self) -> bool {
        if self.selection.is_empty() {
            let start = self.word_start(self.cursor());
            if start == self.cursor() {
                return false;
            }
            self.select_to(start);
        }
        self.insert("");
        true
    }

    pub fn delete_to_line_start(&mut self) -> bool {
        if self.selection.is_empty() {
            let start = self.line_start(self.cursor());
            if start == self.cursor() {
                return self.backspace();
            }
            self.select_to(start);
        }
        self.insert("");
        true
    }

    pub fn left(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.previous_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.start);
        }
    }

    pub fn right(&mut self) {
        if self.selection.is_empty() {
            self.move_to(self.next_boundary(self.cursor()));
        } else {
            self.move_to(self.selection.end);
        }
    }

    #[must_use]
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    #[must_use]
    pub fn offset_from_utf16(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (i, c) in self.text.char_indices() {
            if units >= utf16 {
                return i;
            }
            units += c.len_utf16();
        }
        self.text.len()
    }

    #[must_use]
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    #[must_use]
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(text: &str, multiline: bool) -> Editing {
        let mut e = Editing::new(multiline);
        e.set_text(text);
        e
    }

    #[test]
    fn insert_and_backspace_graphemes() {
        let mut e = with("", true);
        e.insert("héllo 👨‍👩‍👧");
        assert!(e.backspace());
        assert_eq!(e.text(), "héllo ");
        e.move_to(0);
        assert!(!e.backspace());
        assert!(e.delete());
        assert_eq!(e.text(), "éllo ");
        e.move_to(e.text().len());
        assert!(!e.delete());
    }

    #[test]
    fn single_line_strips_newlines() {
        let mut e = with("a\nb", false);
        assert_eq!(e.text(), "ab");
        e.insert("\r\nc");
        assert_eq!(e.text(), "abc");
        let mut m = with("a\r\nb", true);
        assert_eq!(m.text(), "a\nb");
        m.insert("\n");
        assert_eq!(m.text(), "a\nb\n");
    }

    #[test]
    fn selection_extends_both_ways() {
        let mut e = with("abcdef", false);
        e.move_to(3);
        e.select_to(5);
        assert_eq!(e.selected_text(), "de");
        e.select_to(1);
        assert_eq!(e.selected_text(), "bc");
        assert!(e.is_reversed());
        assert_eq!(e.cursor(), 1);
        e.insert("X");
        assert_eq!(e.text(), "aXdef");
        e.select_all();
        assert_eq!(e.selected_text(), "aXdef");
        e.left();
        assert_eq!(e.cursor(), 0);
        e.select_all();
        e.right();
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    fn words_and_lines() {
        let mut e = with("one two\nthree four", true);
        assert_eq!(e.word_start(6), 4);
        assert_eq!(e.word_end(4), 7);
        assert_eq!(e.word_range(5), 4..7);
        assert_eq!(e.line_start(12), 8);
        assert_eq!(e.line_end(9), 18);
        assert_eq!(e.line_end(2), 7);
        e.move_to(18);
        assert!(e.delete_word_left());
        assert_eq!(e.text(), "one two\nthree ");
        assert!(e.delete_to_line_start());
        assert_eq!(e.text(), "one two\n");
        // At a line start, it joins with the previous line.
        assert!(e.delete_to_line_start());
        assert_eq!(e.text(), "one two");
        e.move_to(0);
        assert!(!e.delete_word_left());
    }

    #[test]
    fn utf16_conversion() {
        let e = with("a😀b", false);
        assert_eq!(e.offset_to_utf16(5), 3);
        assert_eq!(e.offset_from_utf16(3), 5);
        assert_eq!(e.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(e.range_to_utf16(&(0..6)), 0..4);
        assert_eq!(e.offset_from_utf16(99), 6);
    }

    #[test]
    fn ime_marking() {
        let mut e = with("ab", false);
        e.replace_and_mark(None, "にほ", Some(0..1));
        assert_eq!(e.text(), "abにほ");
        assert_eq!(e.marked(), Some(2..8));
        e.replace(None, "日本");
        assert_eq!(e.text(), "ab日本");
        assert_eq!(e.marked(), None);
        assert_eq!(e.cursor(), 8);
        e.replace_and_mark(None, "", None);
        assert_eq!(e.marked(), None);
        e.unmark();
    }

    #[test]
    fn offsets_are_clamped_to_char_boundaries() {
        let mut e = with("é", false);
        e.move_to(1);
        assert_eq!(e.cursor(), 0);
        let (start, end) = (5, 0);
        e.select_range(start..end);
        assert_eq!(e.selection(), 0..2);
        e.set_text("");
        assert_eq!(e.text(), "");
    }
}
