//! Minimal Server-Sent Events parser.

/// One SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental SSE parser: push bytes, pop complete events.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: String,
    pending_bytes: Vec<u8>,
}

impl SseParser {
    /// A new parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns every complete event.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.pending_bytes.extend_from_slice(bytes);
        // Only decode up to the last complete UTF-8 boundary.
        let valid = match std::str::from_utf8(&self.pending_bytes) {
            Ok(s) => s.len(),
            Err(e) => e.valid_up_to(),
        };
        let chunk: Vec<u8> = self.pending_bytes.drain(..valid).collect();
        self.buf
            .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..pos + 2).collect();
            if let Some(ev) = parse_block(&block) {
                out.push(ev);
            }
        }
        out
    }

    /// Flush a trailing event without a blank line.
    pub fn finish(&mut self) -> Option<SseEvent> {
        let block = std::mem::take(&mut self.buf);
        parse_block(&block)
    }
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if line.starts_with(':') || line.is_empty() {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => event = Some(value.to_string()),
            "data" => data.push(value),
            _ => {}
        }
    }
    if event.is_none() && data.is_empty() {
        return None;
    }
    Some(SseEvent {
        event,
        data: data.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_events() {
        let mut p = SseParser::new();
        assert!(p.push(b"event: ping\r\ndata: {\"a\"").is_empty());
        let evs = p.push(b":1}\r\n\r\n: comment\n\ndata: x\ndata: y\n\n");
        assert_eq!(
            evs,
            vec![
                SseEvent {
                    event: Some("ping".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "x\ny".into()
                }
            ]
        );
        // Multi-byte char split across chunks.
        let bytes = "data: é\n\n".as_bytes();
        assert!(p.push(&bytes[..7]).is_empty());
        assert_eq!(p.push(&bytes[7..])[0].data, "é");
    }
}
