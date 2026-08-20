/// Convert common Markdown emitted by LLMs to WhatsApp's lightweight markup.
///
/// WhatsApp understands `*bold*`, `_italic_`, `~strike~`, fenced/inline code,
/// block quotes, and plain URLs. It does not understand Markdown headings,
/// double-delimited emphasis, or `[label](url)` links.
pub(crate) fn markdown_to_whatsapp(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut in_code_fence = false;
    let segments: Vec<_> = markdown.split_inclusive('\n').map(split_segment).collect();
    let mut index = 0;

    while index < segments.len() {
        let (line, has_newline) = segments[index];
        let trimmed = line.trim_start();

        if !in_code_fence
            && !trimmed.starts_with("```")
            && let Some(header_cells) = table_header_cells(&segments, index)
        {
            let column_count = header_cells.len();
            render_table_row(&mut output, &header_cells, has_newline);
            index += 2; // The validated separator row is structural, not content.

            while let Some(&(row, row_has_newline)) = segments.get(index) {
                let row = row.trim_start();
                let Some(cells) = table_cells(row) else {
                    break;
                };
                if cells.len() != column_count {
                    break;
                }
                render_table_row(&mut output, &cells, row_has_newline);
                index += 1;
            }
            continue;
        }

        if trimmed.starts_with("```") {
            output.push_str("```");
            in_code_fence = !in_code_fence;
        } else if in_code_fence {
            output.push_str(line);
        } else if let Some((indent, heading)) = markdown_heading(line) {
            output.push_str(indent);
            let heading = convert_inline(heading);
            // Preserve an existing bold run instead of creating overlapping
            // WhatsApp delimiters around only part of the heading.
            if heading.contains('*') {
                output.push_str(&heading);
            } else {
                output.push('*');
                output.push_str(&heading);
                output.push('*');
            }
        } else {
            output.push_str(&convert_inline(line));
        }

        if has_newline {
            output.push('\n');
        }
        index += 1;
    }

    output
}

fn split_segment(segment: &str) -> (&str, bool) {
    match segment.strip_suffix('\n') {
        Some(line) => (line.strip_suffix('\r').unwrap_or(line), true),
        None => (segment, false),
    }
}

fn markdown_heading(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) || trimmed.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    Some((&line[..indent_len], trimmed[hashes + 1..].trim()))
}

fn table_cells(line: &str) -> Option<Vec<String>> {
    let mut chars = line.trim_end().strip_prefix('|')?.chars().peekable();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut code_ticks = None;

    while let Some(ch) = chars.next() {
        if ch == '\\' && code_ticks.is_none() && chars.peek() == Some(&'|') {
            chars.next();
            cell.push('|');
            continue;
        }

        if ch == '`' {
            let mut ticks = 1;
            while chars.peek() == Some(&'`') {
                chars.next();
                ticks += 1;
            }
            cell.extend(std::iter::repeat_n('`', ticks));
            match code_ticks {
                None => code_ticks = Some(ticks),
                Some(opening_ticks) if opening_ticks == ticks => code_ticks = None,
                Some(_) => {},
            }
            continue;
        }

        if ch == '|' && code_ticks.is_none() {
            cells.push(cell.trim().to_owned());
            cell.clear();
            if chars.peek().is_none() {
                return Some(cells);
            }
            continue;
        }

        cell.push(ch);
    }

    None
}

fn is_table_separator(cells: &[String]) -> bool {
    cells.iter().all(|cell| {
        let cell = cell.trim_matches(':');
        cell.len() >= 3 && cell.bytes().all(|byte| byte == b'-')
    })
}

fn table_header_cells(segments: &[(&str, bool)], index: usize) -> Option<Vec<String>> {
    let &(header, header_has_newline) = segments.get(index)?;
    let &(separator, _) = segments.get(index + 1)?;
    if !header_has_newline {
        return None;
    }

    let header = header.trim_start();
    let separator = separator.trim_start();
    let header_cells = table_cells(header)?;
    let separator_cells = table_cells(separator)?;
    if !is_table_separator(&separator_cells) || header_cells.len() != separator_cells.len() {
        return None;
    }

    Some(header_cells)
}

fn render_table_row(output: &mut String, cells: &[String], has_newline: bool) {
    output.push_str(&convert_inline(&cells.join(" · ")));
    if has_newline {
        output.push('\n');
    }
}

fn convert_inline(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_inline_code = false;
    let mut in_bold = false;
    let mut in_underscore_bold = false;
    let mut in_strike = false;
    let mut in_bold_italic = false;

    while let Some(ch) = chars.next() {
        if ch == '`' {
            in_inline_code = !in_inline_code;
            output.push(ch);
            continue;
        }
        if in_inline_code {
            output.push(ch);
            continue;
        }

        match ch {
            '\\' => {
                output.push(ch);
                if let Some(next) = chars.next() {
                    output.push(next);
                }
            },
            '*' if chars.peek() == Some(&'*') => {
                let is_triple = {
                    let mut lookahead = chars.clone();
                    lookahead.next();
                    lookahead.peek() == Some(&'*')
                };
                if is_triple {
                    if in_bold_italic {
                        chars.next();
                        chars.next();
                        output.push_str("*_");
                        in_bold_italic = false;
                    } else if has_closing_delimiter(&chars, "***") {
                        chars.next();
                        chars.next();
                        output.push_str("_*");
                        in_bold_italic = true;
                    } else {
                        chars.next();
                        chars.next();
                        output.push_str("***");
                    }
                } else if in_bold {
                    chars.next();
                    output.push('*');
                    in_bold = false;
                } else if has_closing_delimiter(&chars, "**") {
                    chars.next();
                    output.push('*');
                    in_bold = true;
                } else {
                    chars.next();
                    output.push_str("**");
                }
            },
            '_' if chars.peek() == Some(&'_') => {
                if in_underscore_bold {
                    chars.next();
                    output.push('*');
                    in_underscore_bold = false;
                } else if has_closing_delimiter(&chars, "__") {
                    chars.next();
                    output.push('*');
                    in_underscore_bold = true;
                } else {
                    chars.next();
                    output.push_str("__");
                }
            },
            '~' if chars.peek() == Some(&'~') => {
                if in_strike {
                    chars.next();
                    output.push('~');
                    in_strike = false;
                } else if has_closing_delimiter(&chars, "~~") {
                    chars.next();
                    output.push('~');
                    in_strike = true;
                } else {
                    chars.next();
                    output.push_str("~~");
                }
            },
            '!' if chars.peek() == Some(&'[') => {
                chars.next();
                if !convert_link(&mut chars, &mut output) {
                    output.push_str("![");
                }
            },
            '[' => {
                if !convert_link(&mut chars, &mut output) {
                    output.push('[');
                }
            },
            '<' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(&next) = chars.peek() {
                    if next == '>' {
                        chars.next();
                        closed = true;
                        break;
                    }
                    value.push(next);
                    chars.next();
                }
                if closed && (value.starts_with("http://") || value.starts_with("https://")) {
                    output.push_str(&value);
                } else {
                    output.push('<');
                    output.push_str(&value);
                    if closed {
                        output.push('>');
                    }
                }
            },
            _ => output.push(ch),
        }
    }

    output
}

fn has_closing_delimiter<I>(chars: &std::iter::Peekable<I>, delimiter: &str) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    let remaining: String = chars.clone().collect();
    let Some(rest) = remaining.strip_prefix(&delimiter[1..]) else {
        return false;
    };
    let bytes = rest.as_bytes();
    let delimiter = delimiter.as_bytes();
    let mut in_code = false;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => {
                in_code = !in_code;
                index += 1;
            },
            _ if !in_code && bytes[index..].starts_with(delimiter) => return true,
            _ => index += 1,
        }
    }
    false
}

fn convert_link<I>(chars: &mut std::iter::Peekable<I>, output: &mut String) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    let original = chars.clone();
    let mut label = String::new();
    for ch in chars.by_ref() {
        if ch == ']' {
            break;
        }
        label.push(ch);
    }
    if chars.next() != Some('(') {
        *chars = original;
        return false;
    }

    let mut url = String::new();
    let mut depth = 1usize;
    for ch in chars.by_ref() {
        if ch == '(' {
            depth += 1;
            url.push(ch);
            continue;
        }
        if ch == ')' {
            depth -= 1;
            if depth > 0 {
                url.push(ch);
                continue;
            }
            if label.is_empty() || label == url {
                output.push_str(&url);
            } else {
                output.push_str(&convert_inline(&label));
                output.push_str(": ");
                output.push_str(&url);
            }
            return true;
        }
        url.push(ch);
    }

    *chars = original;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_llm_markdown_to_whatsapp_markup() {
        let input =
            "## Notícias\n\n1. **Robôs** e *agentes*\n[Reuters](https://reuters.com) · ~~antigo~~";
        let expected =
            "*Notícias*\n\n1. *Robôs* e *agentes*\nReuters: https://reuters.com · ~antigo~";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn preserves_code_content_and_removes_fence_language() {
        let input = "```json\n{\"value\": \"**literal**\"}\n```";
        let expected = "```\n{\"value\": \"**literal**\"}\n```";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn renders_simple_tables_without_markdown_pipes() {
        let input = "| Time | Placar |\n|---|---:|\n| Flamengo | 2 x 1 |";
        let expected = "Time · Placar\nFlamengo · 2 x 1";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn preserves_isolated_pipe_rows_and_ascii_borders() {
        let input = "Diagram:\n|---|---|\n| left | right |\nEnd";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn requires_matching_header_and_separator_columns_for_tables() {
        let input = "| First | Second |\n|---|\n| value | another |";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn preserves_pipes_inside_table_code_spans() {
        let input = "| Expression | Meaning |\n|---|---|\n| `left | right` | choice |";
        let expected = "Expression · Meaning\n`left | right` · choice";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn preserves_escaped_pipes_as_table_cell_content() {
        let input = "| A \\| B | Meaning |\n|---|---|\n| left \\| right | choice |";
        let expected = "A | B · Meaning\nleft | right · choice";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn leaves_bare_urls_and_native_whatsapp_markup_unchanged() {
        let input = "*Atenção* https://example.com/a_b?q=1";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn conversion_is_idempotent() {
        let input = "## Título\n\n**forte** [fonte](https://example.com) ~~removido~~";
        let once = markdown_to_whatsapp(input);
        assert_eq!(markdown_to_whatsapp(&once), once);
    }

    #[test]
    fn preserves_malformed_markdown_without_inventing_delimiters() {
        let cases = [
            "**negrito incompleto",
            "__negrito incompleto",
            "~~riscado incompleto",
            "***ênfase incompleta",
            "[link incompleto",
            "[rótulo] sem URL",
            "[rótulo](https://example.com/incompleto",
            "<tag sem fechamento",
            "#não é título",
            "####### também não é título",
        ];
        for input in cases {
            assert_eq!(markdown_to_whatsapp(input), input);
        }
    }

    #[test]
    fn handles_nested_parentheses_in_link_destinations() {
        let input = "[Referência](https://example.com/wiki/Foo_(bar)?a=(b))";
        let expected = "Referência: https://example.com/wiki/Foo_(bar)?a=(b)";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn converts_images_and_autolinks_to_visible_urls() {
        let input = "![**diagrama**](https://example.com/a.png) <https://example.com/docs>";
        let expected = "*diagrama*: https://example.com/a.png https://example.com/docs";
        assert_eq!(markdown_to_whatsapp(input), expected);
    }

    #[test]
    fn converts_combined_bold_italic_markup() {
        assert_eq!(markdown_to_whatsapp("***importante***"), "_*importante*_");
    }

    #[test]
    fn does_not_rewrite_markdown_inside_inline_code() {
        let input = "Use `**bold**`, `[link](url)` e `~~strike~~`.";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn headings_with_inline_emphasis_do_not_overlap_markers() {
        assert_eq!(markdown_to_whatsapp("## **Título**"), "*Título*");
        assert_eq!(
            markdown_to_whatsapp("## This is *important*"),
            "This is *important*"
        );
        assert_eq!(
            markdown_to_whatsapp("## This is **important**"),
            "This is *important*"
        );
        assert_eq!(markdown_to_whatsapp("  ### Título"), "  *Título*");
    }

    #[test]
    fn preserves_unicode_and_whatsapp_list_syntax() {
        let input = "- [x] Robôs 🤖\n- [ ] Ações em português: informação";
        assert_eq!(markdown_to_whatsapp(input), input);
    }

    #[test]
    fn retains_trailing_newlines_and_normalizes_crlf() {
        assert_eq!(markdown_to_whatsapp("**fim**\n"), "*fim*\n");
        assert_eq!(markdown_to_whatsapp("**fim**\r\n"), "*fim*\n");
    }
}
