//! Server-sent-events parsing for streaming provider responses.

use serde::{Deserialize, Serialize};

/// A single parsed SSE frame (`event:`/`data:` lines up to a blank line).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

/// Incrementally parses raw bytes from an SSE stream into `SseEvent`s.
/// Callers feed chunks as they arrive over the wire and drain completed
/// events; partial frames are buffered until a terminating blank line.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a raw chunk of bytes (assumed UTF-8) and returns any complete
    /// events found so far. Malformed UTF-8 is lossily replaced rather than
    /// erroring, since a parse failure here should never crash a stream.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut events = Vec::new();
        while let Some(pos) = self.buffer.find("\n\n") {
            let frame: String = self.buffer.drain(..pos + 2).collect();
            if let Some(event) = parse_frame(&frame) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_frame(frame: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut data_lines = Vec::new();
    let mut saw_any = false;
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
            saw_any = true;
        } else if let Some(rest) = line.strip_prefix("event:") {
            event.event = Some(rest.trim_start().to_string());
            saw_any = true;
        } else if let Some(rest) = line.strip_prefix("id:") {
            event.id = Some(rest.trim_start().to_string());
            saw_any = true;
        }
    }
    if !saw_any {
        return None;
    }
    event.data = data_lines.join("\n");
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_complete_event() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"event: message\ndata: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
        assert_eq!(events[0].event.as_deref(), Some("message"));
    }

    #[test]
    fn buffers_partial_frame_until_blank_line() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: partial");
        assert!(events.is_empty());
        let events = parser.feed(b"\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "partial");
    }
}
