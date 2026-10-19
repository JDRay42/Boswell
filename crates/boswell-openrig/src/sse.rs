//! An incremental server-sent events parser, enough for OpenRig's
//! `/api/events`: `id:` and `data:` fields, comment lines, and frames split
//! across network reads at any byte.

/// One dispatched SSE frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The frame's `id:`, if it carried one.
    pub id: Option<String>,
    /// The `data:` lines, joined with `\n`.
    pub data: String,
}

/// Accumulates bytes and yields complete frames.
#[derive(Debug, Default)]
pub struct Parser {
    buffer: Vec<u8>,
    id: Option<String>,
    data: Vec<String>,
}

impl Parser {
    /// Feed a chunk of the response body, returning every frame it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=pos).collect();
            line.pop(); // the '\n'
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(frame) = self.line(&line) {
                frames.push(frame);
            }
        }
        frames
    }

    fn line(&mut self, line: &str) -> Option<Frame> {
        if line.is_empty() {
            // A blank line dispatches. A frame with no data is not dispatched.
            let id = self.id.take();
            if self.data.is_empty() {
                return None;
            }
            return Some(Frame {
                id,
                data: std::mem::take(&mut self.data).join("\n"),
            });
        }
        if line.starts_with(':') {
            return None; // comment / keepalive
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => self.data.push(value.to_string()),
            "id" => self.id = Some(value.to_string()),
            _ => {} // event:, retry: and unknown fields are not needed
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let mut p = Parser::default();
        assert!(p.push(b"id: 4").is_empty());
        assert!(p.push(b"2\nda").is_empty());
        let frames = p.push(b"ta: {\"type\":\"queue.claimed\"}\n\n");
        assert_eq!(
            frames,
            vec![Frame {
                id: Some("42".into()),
                data: "{\"type\":\"queue.claimed\"}".into()
            }]
        );
    }

    #[test]
    fn data_lines_join_and_comments_and_crlf_are_ignored() {
        let mut p = Parser::default();
        let frames = p.push(b": keepalive\r\ndata: a\r\ndata: b\r\n\r\ndata: c\n\n");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, "a\nb");
        assert_eq!(frames[0].id, None);
        assert_eq!(frames[1].data, "c");
    }

    #[test]
    fn a_frame_without_data_is_not_dispatched() {
        let mut p = Parser::default();
        assert!(p.push(b"id: 7\n\n").is_empty());
    }
}
