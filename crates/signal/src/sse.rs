//! Minimal Server-Sent Events parser for signal-cli event streams.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: Option<String>,
    pub id: Option<String>,
}

/// Incremental SSE parser.
///
/// Input is buffered as raw bytes and only complete lines are decoded. Network
/// chunks can split a multi-byte UTF-8 character (such as an emoji) in two, so
/// decoding each chunk on its own would corrupt it. A `\n` byte never occurs
/// inside a multi-byte UTF-8 sequence, so splitting on it is always safe.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    current: SseEvent,
}

impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        let mut consumed = 0;
        while let Some(offset) = self.buffer[consumed..].iter().position(|&b| b == b'\n') {
            let line_end = consumed + offset;
            let line = decode_line(&self.buffer[consumed..line_end]);
            consumed = line_end + 1;
            if let Some(event) = self.consume_line(&line) {
                events.push(event);
            }
        }
        self.buffer.drain(..consumed);

        events
    }

    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buffer.is_empty() {
            let bytes = std::mem::take(&mut self.buffer);
            let line = decode_line(&bytes);
            let _ = self.consume_line(&line);
        }
        self.flush()
    }

    fn consume_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.flush();
        }
        if line.starts_with(':') {
            return None;
        }

        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            let value = value.strip_prefix(' ').unwrap_or(value);
            (field, value)
        });

        match field {
            "event" => self.current.event = Some(value.to_string()),
            "data" => {
                if let Some(data) = &mut self.current.data {
                    data.push('\n');
                    data.push_str(value);
                } else {
                    self.current.data = Some(value.to_string());
                }
            },
            "id" => self.current.id = Some(value.to_string()),
            _ => {},
        }

        None
    }

    fn flush(&mut self) -> Option<SseEvent> {
        if self.current.event.is_none() && self.current.data.is_none() && self.current.id.is_none()
        {
            return None;
        }
        Some(std::mem::take(&mut self.current))
    }
}

/// Decode one complete line, dropping a trailing `\r` from CRLF endings.
fn decode_line(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use crate::sse::SseParser;

    #[test]
    fn parses_multiline_data_events() {
        let mut parser = SseParser::default();
        let events = parser.push(b"id: 1\nevent: message\ndata: {\"a\":\ndata: 1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("1"));
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data.as_deref(), Some("{\"a\":\n1}"));
    }

    #[test]
    fn ignores_comments_and_handles_chunks() {
        let mut parser = SseParser::default();
        assert!(parser.push(b": keepalive\ndata: hel").is_empty());
        let events = parser.push(b"lo\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.as_deref(), Some("hello"));
    }

    #[test]
    fn keeps_multibyte_characters_split_across_chunks() {
        // 👍 is four bytes in UTF-8. Split it between two network chunks.
        let payload = "data: {\"emoji\":\"\u{1F44D}\"}\n\n".as_bytes();
        let emoji_start = payload
            .iter()
            .position(|&b| b == 0xF0)
            .unwrap_or_default();
        let (first, second) = payload.split_at(emoji_start + 2);

        let mut parser = SseParser::default();
        assert!(parser.push(first).is_empty());
        let events = parser.push(second);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.as_deref(), Some("{\"emoji\":\"\u{1F44D}\"}"));
    }

    #[test]
    fn handles_one_byte_chunks() {
        let payload = "event: message\r\ndata: h\u{e9}llo \u{1F600}\r\n\r\n".as_bytes();
        let mut parser = SseParser::default();
        let events: Vec<_> = payload
            .iter()
            .flat_map(|byte| parser.push(std::slice::from_ref(byte)))
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data.as_deref(), Some("h\u{e9}llo \u{1F600}"));
    }

    #[test]
    fn replaces_invalid_utf8_without_dropping_the_event() {
        let mut parser = SseParser::default();
        let events = parser.push(b"data: ok\xFFok\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.as_deref(), Some("ok\u{FFFD}ok"));
    }

    #[test]
    fn finish_flushes_a_trailing_unterminated_line() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: tail\r").is_empty());
        let event = parser.finish();
        assert_eq!(event.and_then(|e| e.data).as_deref(), Some("tail"));
    }
}
