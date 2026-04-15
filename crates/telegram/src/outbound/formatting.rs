//! HTML-to-plain-text conversion for Telegram fallback rendering.
//!
//! When Telegram rejects HTML-formatted messages, we strip tags and decode
//! entities to produce a readable plain-text fallback.

pub(crate) fn telegram_html_to_plain_text(html: &str) -> String {
    let mut plain = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(ch) = remaining.chars().next() {
        if ch == '<' {
            if let Some((tag_name, consumed_len)) = consume_plain_text_html_tag(remaining) {
                if is_plain_text_line_break_tag(&tag_name) && !plain.ends_with('\n') {
                    plain.push('\n');
                }
                remaining = &remaining[consumed_len..];
                continue;
            }
        } else if ch == '&'
            && let Some((decoded, consumed_len)) = consume_html_entity(remaining)
        {
            plain.push_str(&decoded);
            remaining = &remaining[consumed_len..];
            continue;
        }

        plain.push(ch);
        remaining = &remaining[ch.len_utf8()..];
    }

    plain.trim_matches('\n').to_string()
}

fn consume_plain_text_html_tag(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    if bytes.first().copied()? != b'<' {
        return None;
    }

    let mut index = 1usize;
    if bytes.get(index).copied() == Some(b'/') {
        index += 1;
    }

    let name_start = index;
    let first = bytes.get(index).copied()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }

    while let Some(next) = bytes.get(index).copied() {
        if next.is_ascii_alphanumeric() || next == b'-' {
            index += 1;
            continue;
        }
        break;
    }

    let tag_name = input[name_start..index].to_ascii_lowercase();
    if !is_plain_text_html_tag_name(&tag_name) {
        return None;
    }

    let mut quote = None;
    while let Some(next) = bytes.get(index).copied() {
        match quote {
            Some(delimiter) if next == delimiter => quote = None,
            Some(_) => {},
            None if next == b'\'' || next == b'"' => quote = Some(next),
            None if next == b'>' => return Some((tag_name, index + 1)),
            None => {},
        }
        index += 1;
    }

    None
}

fn consume_html_entity(input: &str) -> Option<(String, usize)> {
    if !input.starts_with('&') {
        return None;
    }

    let mut entity = String::new();
    for (index, ch) in input.char_indices() {
        entity.push(ch);
        let consumed_len = index + ch.len_utf8();
        if ch == ';' {
            return decode_html_entity(&entity).map(|decoded| (decoded, consumed_len));
        }
        if entity.len() > 12 {
            return None;
        }
    }

    None
}

fn is_plain_text_html_tag_name(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "a" | "b"
            | "blockquote"
            | "br"
            | "code"
            | "del"
            | "div"
            | "em"
            | "i"
            | "ins"
            | "li"
            | "p"
            | "pre"
            | "s"
            | "span"
            | "strike"
            | "strong"
            | "tg-emoji"
            | "tg-spoiler"
            | "u"
    )
}

fn is_plain_text_line_break_tag(tag_name: &str) -> bool {
    matches!(tag_name, "blockquote" | "br" | "div" | "li" | "p" | "pre")
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "&amp;" => Some("&".to_string()),
        "&lt;" => Some("<".to_string()),
        "&gt;" => Some(">".to_string()),
        "&quot;" => Some("\"".to_string()),
        "&apos;" | "&#39;" => Some("'".to_string()),
        "&nbsp;" | "&#160;" => Some(" ".to_string()),
        _ => decode_numeric_html_entity(entity),
    }
}

fn decode_numeric_html_entity(entity: &str) -> Option<String> {
    let value = entity
        .strip_prefix("&#x")
        .or_else(|| entity.strip_prefix("&#X"))
        .and_then(|hex| hex.strip_suffix(';'))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| {
            entity
                .strip_prefix("&#")
                .and_then(|decimal| decimal.strip_suffix(';'))
                .and_then(|decimal| decimal.parse::<u32>().ok())
        })?;

    char::from_u32(value).map(|ch| ch.to_string())
}
