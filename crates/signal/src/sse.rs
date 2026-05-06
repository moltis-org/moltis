//! Minimal Server-Sent Events parser for signal-cli event streams.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
    current: SseEvent,
}

impl SseParser {
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        while let Some(line_end) = self.buffer.find('\n') {
            let mut line = self.buffer[..line_end].to_string();
            self.buffer.drain(..=line_end);
            if line.ends_with('\r') {
                line.pop();
            }
            if let Some(event) = self.consume_line(&line) {
                events.push(event);
            }
        }

        events
    }

    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
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

#[cfg(test)]
mod tests {
    use crate::sse::SseParser;

    #[test]
    fn parses_multiline_data_events() {
        let mut parser = SseParser::default();
        let events = parser.push("id: 1\nevent: message\ndata: {\"a\":\ndata: 1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("1"));
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data.as_deref(), Some("{\"a\":\n1}"));
    }

    #[test]
    fn ignores_comments_and_handles_chunks() {
        let mut parser = SseParser::default();
        assert!(parser.push(": keepalive\ndata: hel").is_empty());
        let events = parser.push("lo\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.as_deref(), Some("hello"));
    }
}
