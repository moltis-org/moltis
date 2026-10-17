//! CommonMark (plus tables, strikethrough, task lists) to a small block model
//! that the renderer maps to native gpui elements.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Inline style of a span of text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Byte range into [`Inline::text`].
    pub range: Range<usize>,
    pub style: SpanStyle,
}

/// Text with styled spans. Unstyled stretches have no span.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub spans: Vec<Span>,
}

impl Inline {
    fn push(&mut self, text: &str, style: &SpanStyle) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        if *style == SpanStyle::default() {
            return;
        }
        let end = self.text.len();
        // Merge with the previous span when the style continues.
        if let Some(last) = self.spans.last_mut()
            && last.range.end == start
            && last.style == *style
        {
            last.range.end = end;
            return;
        }
        self.spans.push(Span {
            range: start..end,
            style: style.clone(),
        });
    }

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Links in this text, with their byte ranges.
    #[must_use]
    pub fn links(&self) -> Vec<(Range<usize>, &str)> {
        self.spans
            .iter()
            .filter_map(|span| Some((span.range.clone(), span.style.link.as_deref()?)))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Inline),
    Heading {
        level: u8,
        content: Inline,
    },
    Code {
        lang: Option<String>,
        code: String,
    },
    List {
        /// First number of an ordered list.
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Rule,
    Table {
        header: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
}

enum Container {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Item(Vec<Block>),
}

impl Container {
    fn blocks(&mut self) -> Option<&mut Vec<Block>> {
        match self {
            Self::Root(blocks) | Self::Quote(blocks) | Self::Item(blocks) => Some(blocks),
            Self::List { .. } => None,
        }
    }
}

enum InlineKind {
    Paragraph,
    Heading(u8),
    Cell,
}

#[derive(Default)]
struct Table {
    header: Vec<Inline>,
    rows: Vec<Vec<Inline>>,
    row: Vec<Inline>,
    in_head: bool,
}

/// Containers nested deeper than this are flattened into their parent, so
/// hostile input cannot build an arbitrarily deep tree.
const MAX_NESTING: usize = 24;

struct Builder {
    stack: Vec<Container>,
    /// Containers opened past `MAX_NESTING` and not pushed.
    flattened: usize,
    inline: Option<(InlineKind, Inline)>,
    code: Option<(Option<String>, String)>,
    table: Option<Table>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<String>,
}

impl Builder {
    fn style(&self, code: bool) -> SpanStyle {
        SpanStyle {
            bold: self.bold > 0,
            italic: self.italic > 0,
            code,
            strike: self.strike > 0,
            link: self.links.last().cloned(),
        }
    }

    fn push_block(&mut self, block: Block) {
        if let Some(blocks) = self.stack.last_mut().and_then(Container::blocks) {
            blocks.push(block);
        }
    }

    /// Text outside any paragraph (tight list items) opens one implicitly.
    fn inline_mut(&mut self) -> &mut Inline {
        &mut self
            .inline
            .get_or_insert_with(|| (InlineKind::Paragraph, Inline::default()))
            .1
    }

    fn text(&mut self, text: &str, code: bool) {
        if let Some((_, buffer)) = &mut self.code {
            buffer.push_str(text);
            return;
        }
        let style = self.style(code);
        self.inline_mut().push(text, &style);
    }

    fn flush_inline(&mut self) {
        let Some((kind, inline)) = self.inline.take() else {
            return;
        };
        match kind {
            InlineKind::Cell => {
                if let Some(table) = &mut self.table {
                    table.row.push(inline);
                }
            },
            InlineKind::Heading(level) => self.push_block(Block::Heading {
                level,
                content: inline,
            }),
            InlineKind::Paragraph if inline.is_blank() => {},
            InlineKind::Paragraph => self.push_block(Block::Paragraph(inline)),
        }
    }

    /// Whether a new container may be pushed; otherwise it is flattened.
    fn open_container(&mut self) -> bool {
        self.flush_inline();
        if self.stack.len() > MAX_NESTING {
            self.flattened += 1;
            return false;
        }
        true
    }

    /// Whether the closing container was flattened (nothing to pop).
    fn close_flattened(&mut self) -> bool {
        self.flush_inline();
        if self.flattened > 0 {
            self.flattened -= 1;
            return true;
        }
        false
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_inline();
                self.inline = Some((InlineKind::Paragraph, Inline::default()));
            },
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.inline = Some((InlineKind::Heading(heading_level(level)), Inline::default()));
            },
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|lang| !lang.is_empty())
                        .map(String::from),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((lang, String::new()));
            },
            Tag::BlockQuote(_) => {
                if self.open_container() {
                    self.stack.push(Container::Quote(Vec::new()));
                }
            },
            Tag::List(start) => {
                if self.open_container() {
                    self.stack.push(Container::List {
                        start,
                        items: Vec::new(),
                    });
                }
            },
            Tag::Item => {
                if self.open_container() {
                    self.stack.push(Container::Item(Vec::new()));
                }
            },
            Tag::Table(_) => {
                self.flush_inline();
                self.table = Some(Table::default());
            },
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                }
            },
            Tag::TableCell => self.inline = Some((InlineKind::Cell, Inline::default())),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.links.push(dest_url.to_string());
            },
            _ => {},
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => self.flush_inline(),
            TagEnd::CodeBlock => {
                if let Some((lang, mut code)) = self.code.take() {
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    self.push_block(Block::Code { lang, code });
                }
            },
            TagEnd::BlockQuote(_) => {
                if self.close_flattened() {
                    return;
                }
                if let Some(Container::Quote(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            },
            TagEnd::Item => {
                if self.close_flattened() {
                    return;
                }
                if let Some(Container::Item(blocks)) = self.stack.pop()
                    && let Some(Container::List { items, .. }) = self.stack.last_mut()
                {
                    items.push(blocks);
                }
            },
            TagEnd::List(_) => {
                if self.close_flattened() {
                    return;
                }
                if let Some(Container::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            },
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.header = std::mem::take(&mut table.row);
                    table.in_head = false;
                }
            },
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table
                    && !table.in_head
                {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            },
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.push_block(Block::Table {
                        header: table.header,
                        rows: table.rows,
                    });
                }
            },
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {
                self.links.pop();
            },
            _ => {},
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Parse markdown into blocks. Never fails; unknown constructs become text.
#[must_use]
pub fn parse(markdown: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        stack: vec![Container::Root(Vec::new())],
        flattened: 0,
        inline: None,
        code: None,
        table: None,
        bold: 0,
        italic: 0,
        strike: 0,
        links: Vec::new(),
    };
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(text) => builder.text(&text, false),
            Event::Code(text) => builder.text(&text, true),
            Event::Html(html) | Event::InlineHtml(html) => builder.text(&html, false),
            Event::SoftBreak => builder.text(" ", false),
            Event::HardBreak => builder.text("\n", false),
            Event::Rule => {
                builder.flush_inline();
                builder.push_block(Block::Rule);
            },
            Event::TaskListMarker(done) => builder.text(
                if done {
                    "☑ "
                } else {
                    "☐ "
                },
                false,
            ),
            Event::FootnoteReference(name) => builder.text(&format!("[{name}]"), false),
            _ => {},
        }
    }
    builder.flush_inline();
    // An unterminated fence (still streaming) has no End event.
    if let Some((lang, code)) = builder.code.take() {
        builder.push_block(Block::Code { lang, code });
    }
    // Close containers left open by truncated input.
    while builder.stack.len() > 1 {
        match builder.stack.pop() {
            Some(Container::Quote(blocks)) => builder.push_block(Block::Quote(blocks)),
            Some(Container::Item(blocks)) => {
                if let Some(Container::List { items, .. }) = builder.stack.last_mut() {
                    items.push(blocks);
                }
            },
            Some(Container::List { start, items }) => {
                builder.push_block(Block::List { start, items });
            },
            Some(Container::Root(_)) | None => {},
        }
    }
    match builder.stack.pop() {
        Some(Container::Root(blocks)) => blocks,
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
