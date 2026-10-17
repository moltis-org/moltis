//! Shaped, wrapped text of an input, with offset ↔ position mapping.

use gpui::{Pixels, Point, SharedString, TextRun, Window, WrappedLine, point, px};

/// One shaped logical line per `\n`-separated segment.
pub struct TextLayout {
    lines: Vec<WrappedLine>,
    /// Byte offset where each logical line starts in the shaped text.
    starts: Vec<usize>,
    /// Top of each logical line, relative to the text origin.
    tops: Vec<Pixels>,
    pub line_height: Pixels,
    pub height: Pixels,
}

impl TextLayout {
    pub fn shape(
        text: &SharedString,
        runs: &[TextRun],
        font_size: Pixels,
        line_height: Pixels,
        wrap_width: Option<Pixels>,
        window: &mut Window,
    ) -> Self {
        let lines = window
            .text_system()
            .shape_text(text.clone(), font_size, runs, wrap_width, None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
        let mut starts = Vec::with_capacity(lines.len());
        let mut tops = Vec::with_capacity(lines.len());
        let mut start = 0;
        let mut top = px(0.);
        for line in &lines {
            starts.push(start);
            tops.push(top);
            start += line.len() + 1;
            top += line.size(line_height).height;
        }
        Self {
            lines,
            starts,
            tops,
            line_height,
            height: top.max(line_height),
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = (&WrappedLine, Pixels)> {
        self.lines.iter().zip(self.tops.iter().copied())
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts
            .iter()
            .rposition(|start| *start <= offset)
            .unwrap_or(0)
    }

    /// Top-left of the caret at `offset`, relative to the text origin.
    pub fn position_for(&self, offset: usize) -> Point<Pixels> {
        let index = self.line_of(offset);
        let (Some(line), Some(start), Some(top)) = (
            self.lines.get(index),
            self.starts.get(index),
            self.tops.get(index),
        ) else {
            return point(px(0.), px(0.));
        };
        let local = offset.saturating_sub(*start).min(line.len());
        let position = line
            .position_for_index(local, self.line_height)
            .unwrap_or_else(|| point(line.width(), px(0.)));
        point(position.x, *top + position.y)
    }

    /// The offset closest to `position` (relative to the text origin).
    pub fn index_for(&self, position: Point<Pixels>) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let index = self
            .tops
            .iter()
            .rposition(|top| *top <= position.y)
            .unwrap_or(0);
        let line = &self.lines[index];
        let local = point(position.x, position.y - self.tops[index]);
        let offset = match line.closest_index_for_position(local, self.line_height) {
            Ok(offset) | Err(offset) => offset,
        };
        self.starts[index] + offset.min(line.len())
    }
}
