use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

// Keep embedded triple backticks visible without closing WhatsApp code markup.
const WORD_JOINER: char = '\u{2060}';

/// Convert common Markdown emitted by LLMs to WhatsApp's lightweight markup.
///
/// This is a one-way renderer: input is interpreted as Markdown, not as
/// already-formatted WhatsApp text.
pub(crate) fn markdown_to_whatsapp(markdown: &str) -> String {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    WhatsAppRenderer::new(markdown.len()).render(Parser::new_ext(markdown, options))
}

struct ListContext {
    next: Option<u64>,
}

struct DestinationContext {
    destination: String,
    output_start: usize,
    append_destination: bool,
}

struct HeadingContext {
    marker_position: usize,
    has_literal_asterisk: bool,
    suppressed_strong_markers: Vec<usize>,
}

struct WhatsAppRenderer {
    output: String,
    lists: Vec<ListContext>,
    destinations: Vec<DestinationContext>,
    heading: Option<HeadingContext>,
    quote_depth: usize,
    item_paragraphs: Vec<bool>,
    table_column: Option<usize>,
    code_block: Option<String>,
}

impl WhatsAppRenderer {
    fn new(capacity: usize) -> Self {
        Self {
            output: String::with_capacity(capacity),
            lists: Vec::new(),
            destinations: Vec::new(),
            heading: None,
            quote_depth: 0,
            item_paragraphs: Vec::new(),
            table_column: None,
            code_block: None,
        }
    }

    fn render<'a>(mut self, events: impl IntoIterator<Item = Event<'a>>) -> String {
        for event in events {
            match event {
                Event::Start(tag) => self.start(tag),
                Event::End(tag) => self.end(tag),
                Event::Text(text) => {
                    if let Some(code_block) = &mut self.code_block {
                        code_block.push_str(&text);
                    } else {
                        self.write_literal(&text);
                    }
                },
                Event::Code(code) => {
                    let marker = if code.contains('`') {
                        "```"
                    } else {
                        "`"
                    };
                    let code = code.replace("```", &format!("``{WORD_JOINER}`"));
                    self.write(marker);
                    self.write(&code);
                    self.write(marker);
                },
                Event::Html(html) | Event::InlineHtml(html) => self.write_literal(&html),
                Event::InlineMath(math) | Event::DisplayMath(math) => self.write_literal(&math),
                Event::FootnoteReference(label) => self.write_literal(&label),
                Event::SoftBreak | Event::HardBreak => self.ensure_newlines(1),
                Event::Rule => {
                    self.ensure_block_start();
                    self.write("---");
                    self.end_block();
                },
                Event::TaskListMarker(checked) => {
                    self.write(if checked {
                        "[x] "
                    } else {
                        "[ ] "
                    });
                },
            }
        }

        self.output
            .truncate(self.output.trim_end_matches('\n').len());
        self.output
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { .. } => {
                self.ensure_block_start();
                self.prepare_line();
                let marker_position = self.output.len();
                self.output.push('*');
                self.heading = Some(HeadingContext {
                    marker_position,
                    has_literal_asterisk: false,
                    suppressed_strong_markers: Vec::new(),
                });
            },
            Tag::BlockQuote(_) => {
                self.ensure_block_start();
                self.quote_depth += 1;
            },
            Tag::CodeBlock(_) => {
                self.ensure_block_start();
                self.code_block = Some(String::new());
            },
            Tag::HtmlBlock => self.ensure_block_start(),
            Tag::List(next) => {
                if !self.lists.is_empty() {
                    self.ensure_newlines(1);
                } else {
                    self.ensure_block_start();
                }
                self.lists.push(ListContext { next });
            },
            Tag::Item => self.start_item(),
            Tag::Table(_) => {
                self.ensure_block_start();
                self.table_column = Some(0);
            },
            Tag::TableHead | Tag::TableRow => self.table_column = Some(0),
            Tag::TableCell => {
                let column = self.table_column.unwrap_or_default();
                if column > 0 {
                    self.write(" · ");
                }
                self.table_column = Some(column + 1);
            },
            Tag::Emphasis => self.write("_"),
            Tag::Strong => {
                if let Some(heading) = &mut self.heading {
                    heading.suppressed_strong_markers.push(self.output.len());
                } else {
                    self.write("*");
                }
            },
            Tag::Strikethrough => self.write("~"),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => self.start_destination(dest_url.into_string(), link_type),
            Tag::Image {
                link_type,
                dest_url,
                ..
            } => self.start_destination(dest_url.into_string(), link_type),
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {},
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if !self.item_paragraphs.is_empty() || self.quote_depth > 0 {
                    self.ensure_newlines(1);
                } else {
                    self.end_block();
                }
            },
            TagEnd::Heading(_) => {
                if let Some(heading) = self.heading.take() {
                    if heading.has_literal_asterisk {
                        self.output.remove(heading.marker_position);
                        for marker in heading.suppressed_strong_markers.into_iter().rev() {
                            self.output.insert(marker.saturating_sub(1), '*');
                        }
                    } else {
                        self.output.push('*');
                    }
                }
                self.end_block();
            },
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.end_block();
            },
            TagEnd::CodeBlock => {
                let code_block = self.code_block.take().unwrap_or_default();
                let code_block = code_block.replace("```", &format!("``{WORD_JOINER}`"));
                self.write("```");
                self.ensure_newlines(1);
                self.write(&code_block);
                self.ensure_newlines(1);
                self.write("```");
                self.end_block();
            },
            TagEnd::HtmlBlock => self.end_block(),
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.end_block();
                } else {
                    self.ensure_newlines(1);
                }
            },
            TagEnd::Item => {
                self.item_paragraphs.pop();
                self.ensure_newlines(1);
            },
            TagEnd::TableHead | TagEnd::TableRow => self.ensure_newlines(1),
            TagEnd::Table => {
                self.table_column = None;
                self.end_block();
            },
            TagEnd::TableCell => {},
            TagEnd::Emphasis => self.write("_"),
            TagEnd::Strong => {
                if let Some(heading) = &mut self.heading {
                    heading.suppressed_strong_markers.push(self.output.len());
                } else {
                    self.write("*");
                }
            },
            TagEnd::Strikethrough => self.write("~"),
            TagEnd::Link | TagEnd::Image => self.end_destination(),
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {},
        }
    }

    fn start_item(&mut self) {
        if !self.output.is_empty() {
            self.ensure_newlines(1);
        }
        self.prepare_line();
        self.output.extend(std::iter::repeat_n(
            ' ',
            self.lists.len().saturating_sub(1) * 2,
        ));
        if let Some(list) = self.lists.last_mut() {
            if let Some(next) = &mut list.next {
                self.output.push_str(&format!("{next}. "));
                *next = next.saturating_add(1);
            } else {
                self.output.push_str("- ");
            }
        }
        self.item_paragraphs.push(false);
    }

    fn start_paragraph(&mut self) {
        let is_continuation = self.item_paragraphs.last().copied() == Some(true);
        if let Some(started) = self.item_paragraphs.last_mut() {
            *started = true;
        }
        if is_continuation {
            self.ensure_newlines(1);
            self.prepare_line();
            self.output
                .extend(std::iter::repeat_n(' ', self.lists.len() * 2));
        }
    }

    fn start_destination(&mut self, destination: String, link_type: LinkType) {
        self.destinations.push(DestinationContext {
            destination,
            output_start: self.output.len(),
            append_destination: !matches!(link_type, LinkType::Autolink | LinkType::Email),
        });
    }

    fn end_destination(&mut self) {
        let Some(context) = self.destinations.pop() else {
            return;
        };
        if !context.append_destination {
            return;
        }

        let label = self.output[context.output_start..].trim();
        if label == context.destination {
            return;
        }
        if !label.is_empty() {
            self.write(": ");
        }
        self.write_literal(&context.destination);
    }

    fn ensure_block_start(&mut self) {
        if !self.output.is_empty() {
            self.ensure_newlines(2);
        }
    }

    fn end_block(&mut self) {
        self.ensure_newlines(if self.quote_depth > 0 {
            1
        } else {
            2
        });
    }

    fn ensure_newlines(&mut self, count: usize) {
        let existing = self
            .output
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\n')
            .count();
        self.output
            .extend(std::iter::repeat_n('\n', count.saturating_sub(existing)));
    }

    fn prepare_line(&mut self) {
        if (self.output.is_empty() || self.output.ends_with('\n')) && self.quote_depth > 0 {
            self.output
                .extend(std::iter::repeat_n("> ", self.quote_depth));
        }
    }

    fn write(&mut self, text: &str) {
        for segment in text.split_inclusive('\n') {
            self.prepare_line();
            self.output.push_str(segment);
        }
    }

    fn write_literal(&mut self, text: &str) {
        if let Some(heading) = &mut self.heading {
            heading.has_literal_asterisk |= text.contains('*');
        }
        self.write(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_markdown_as_whatsapp_markup() {
        let input = "## Notícias\n\n1. **Robôs** e *agentes*\n2. [Reuters](https://reuters.com) · ~~antigo~~";
        let expected =
            "*Notícias*\n\n1. *Robôs* e _agentes_\n2. Reuters: https://reuters.com · ~antigo~";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn renders_nested_emphasis_without_overlapping_heading_markers() {
        assert_eq!(
            markdown_to_whatsapp("## This is ***very important***"),
            "*This is _very important_*"
        );
        assert_eq!(markdown_to_whatsapp("## **Title**"), "*Title*");
    }

    #[test]
    fn omits_heading_wrapper_around_literal_asterisks() {
        assert_eq!(markdown_to_whatsapp("## 2*3 = 6"), "2*3 = 6");
        assert_eq!(markdown_to_whatsapp("## `*` operator"), "*`*` operator*");
        assert_eq!(
            markdown_to_whatsapp("## **Important** 2*3"),
            "*Important* 2*3"
        );
        assert_eq!(
            markdown_to_whatsapp("## [docs](https://example.test/a*b)"),
            "docs: https://example.test/a*b"
        );
    }

    #[test]
    fn treats_asterisk_emphasis_as_markdown_not_native_whatsapp() {
        assert_eq!(markdown_to_whatsapp("*italic*"), "_italic_");
        assert_eq!(markdown_to_whatsapp("_also italic_"), "_also italic_");
    }

    #[test]
    fn preserves_code_content_and_removes_fence_language() {
        let input = "Use `**bold**` here.\n\n```json\n{\"value\": \"**literal**\"}\n```";
        let expected = "Use `**bold**` here.\n\n```\n{\"value\": \"**literal**\"}\n```";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn uses_longer_inline_marker_for_code_containing_backticks() {
        assert_eq!(markdown_to_whatsapp("``a ` b``"), "```a ` b```");
        assert_eq!(
            markdown_to_whatsapp("````a ``` b````"),
            "```a ``\u{2060}` b```"
        );
    }

    #[test]
    fn neutralizes_embedded_code_fences() {
        let input = "````text\nbefore\n```\nafter\n````";
        assert_eq!(
            markdown_to_whatsapp(input),
            "```\nbefore\n``\u{2060}`\nafter\n```"
        );
    }

    #[test]
    fn renders_gfm_tables_without_markdown_structure() {
        let input = "| Expression | Meaning |\n|---|---:|\n| `left \\| right` | choice |\n| A \\| B | escaped |";
        let expected = "Expression · Meaning\n`left | right` · choice\nA | B · escaped";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn leaves_non_table_pipe_content_visible() {
        let input = "Diagram:\n|---|---|";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn renders_links_images_and_autolinks_with_visible_destinations() {
        let input = "[Reference](https://example.com/wiki/Foo_(bar)) ![diagram](https://example.com/a.png) <https://example.com/docs>";
        let expected = "Reference: https://example.com/wiki/Foo_(bar) diagram: https://example.com/a.png https://example.com/docs";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn renders_lists_tasks_and_quotes() {
        let input = "> first\n> second\n\n- [x] done\n- [ ] next";
        let expected = "> first\n> second\n\n- [x] done\n- [ ] next";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn indents_loose_and_nested_list_content() {
        let input = "- first\n\n  second\n  1. nested";
        let expected = "- first\n  second\n  1. nested";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn preserves_malformed_markdown_without_inventing_delimiters() {
        for input in [
            "**incomplete",
            "~~incomplete",
            "[incomplete",
            "[label](https://example.com/incomplete",
            "#not a heading",
        ] {
            assert_eq!(markdown_to_whatsapp(input), input);
        }
    }

    #[test]
    fn preserves_unicode_bare_urls_and_normalizes_line_endings() {
        let input = "**Robôs 🤖** https://example.com/a_b?q=1\r\nInformação";
        let expected = "*Robôs 🤖* https://example.com/a_b?q=1\nInformação";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }
}
