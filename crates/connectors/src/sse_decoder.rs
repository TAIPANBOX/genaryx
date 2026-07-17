//! SSE (Server-Sent Events) frame decoder: pure byte-to-event reassembly, no
//! I/O. Phase-0 spike #6 (06 §7, 07 §4.2): TokenFuse Cloud's `/v1/stream` is
//! `text/event-stream`, framed as lines, events separated by a blank line,
//! `data:` lines carrying the payload (one JSON object per event for our
//! purposes), comments starting with `:`. [`SseDecoder::feed`] is the whole
//! contract: hand it whatever bytes the transport happened to hand you, in
//! whatever chunking the OS/network gave you, and it emits only complete
//! events, buffering everything else until it is complete. That buffering is
//! what makes the client resilient to chunk-split frames -- a `data:` line,
//! or even a bare `\r` half of a `\r\n`, landing at the boundary between two
//! reads.
//!
//! Deliberately transport-agnostic and dependency-free: this module knows
//! nothing about HTTP, reqwest, or tokio, which is what makes it directly
//! unit-testable without a network (every test below feeds it byte slices by
//! hand; see `cloud_sse.rs` for how a real connection drives it).

/// One decoded SSE event: the payload plus the two optional framing fields
/// the spec defines alongside `data:` (07 §4.2 only requires `data`, but
/// `id`/`event` are cheap to carry, and `id` is what feeds `Last-Event-ID` on
/// reconnect -- see `cloud_sse.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The event id in effect when this event was dispatched. Sticky across
    /// events per the SSE spec: an event with no `id:` field of its own still
    /// carries forward whatever `id:` was last seen on the stream.
    pub id: Option<String>,
    /// The `event:` field, if the server sent one for this event. `None`
    /// means the default (unnamed) event type; unlike `id`, this does not
    /// persist between events.
    pub event: Option<String>,
    /// The event's payload: every `data:` line for this event joined by
    /// `\n` (matching the SSE spec's multi-line-data behavior). TokenFuse
    /// Cloud sends exactly one `data:` line per event (one JSON object), so
    /// in practice this is just that line's value verbatim.
    pub data: String,
}

/// Incremental SSE decoder. Owns only the small amount of state the format
/// needs across chunk boundaries: an in-progress line, an in-progress
/// event's accumulated fields, and the last-seen event id.
///
/// Not tied to any I/O type: feed it bytes from anywhere (a real socket, a
/// test harness, a replay log) via [`SseDecoder::feed`].
#[derive(Debug, Default)]
pub struct SseDecoder {
    /// Bytes received but not yet resolved into a complete line. A line is
    /// only "complete" once its terminator is unambiguous; see
    /// [`find_line_end`] for why a trailing lone `\r` is deliberately held
    /// back rather than treated as a terminator immediately.
    buffer: Vec<u8>,
    /// `data:` lines accumulated for the event currently being built, each
    /// with a trailing `\n` already appended (the final one is trimmed off
    /// on dispatch).
    data_buffer: String,
    /// The `event:` field for the event currently being built.
    event_type: Option<String>,
    /// The most recently seen `id:` value. Persists across dispatches (spec
    /// semantics): a server that sends `id:` once and then a run of bare
    /// `data:` events still lets every one of them carry that id.
    last_id: Option<String>,
}

/// Where the next complete line ends, or why we can't tell yet.
enum LineEnd {
    /// `buffer[..line_len]` is the line's content (terminator excluded);
    /// `consumed` bytes (including the terminator) should be drained.
    Found { line_len: usize, consumed: usize },
    /// No terminator anywhere in the buffer yet.
    Incomplete,
    /// The buffer ends in a lone `\r`: it might be the start of a `\r\n`
    /// pair whose `\n` just hasn't arrived yet. Treating it as a terminator
    /// now would risk splitting one CRLF line ending into two lines -- an
    /// extra spurious blank line, which would flush a premature or empty
    /// event. Wait for at least one more byte before deciding.
    NeedsMoreData,
}

/// Scan for the next line terminator (`\n`, `\r\n`, or a bare `\r`), per the
/// SSE spec's line-splitting rule. Kept free of `SseDecoder` so it is
/// trivial to reason about (and test) in isolation.
fn find_line_end(buffer: &[u8]) -> LineEnd {
    for (i, &b) in buffer.iter().enumerate() {
        if b == b'\n' {
            return LineEnd::Found {
                line_len: i,
                consumed: i + 1,
            };
        }
        if b == b'\r' {
            return match buffer.get(i + 1) {
                Some(b'\n') => LineEnd::Found {
                    line_len: i,
                    consumed: i + 2,
                },
                Some(_) => LineEnd::Found {
                    line_len: i,
                    consumed: i + 1,
                },
                None => LineEnd::NeedsMoreData,
            };
        }
    }
    LineEnd::Incomplete
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recently seen `id:` field value, if any. `cloud_sse.rs`
    /// reads this after each `feed` to send `Last-Event-ID` on reconnect.
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// Feed one chunk of bytes, exactly as received from the transport (any
    /// size, any split point, including mid-line or mid-UTF8-character).
    /// Returns every event completed as a result, in order -- zero, one, or
    /// many. Bytes that do not yet complete a line stay buffered for the
    /// next call; that buffering is the entire mechanism behind chunk-split
    /// resilience (07 §4.2).
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        loop {
            match find_line_end(&self.buffer) {
                LineEnd::Incomplete | LineEnd::NeedsMoreData => break,
                LineEnd::Found { line_len, consumed } => {
                    let line = String::from_utf8_lossy(&self.buffer[..line_len]).into_owned();
                    self.buffer.drain(..consumed);
                    if let Some(event) = self.process_line(&line) {
                        events.push(event);
                    }
                }
            }
        }
        events
    }

    /// Apply one already-split line to the decoder's field state, dispatching
    /// (and returning) an event on a blank line if there is data to send.
    /// Mirrors the WHATWG EventSource "process the field" / "dispatch the
    /// event" algorithm, restricted to the fields TokenFuse Cloud uses.
    fn process_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None; // Comment / keepalive ping: ignored by spec.
        }
        let (field, value) = match line.find(':') {
            Some(idx) => {
                let field = &line[..idx];
                let raw_value = &line[idx + 1..];
                let value = raw_value.strip_prefix(' ').unwrap_or(raw_value);
                (field, value)
            }
            // A line with no colon at all is a field name with an empty value.
            None => (line, ""),
        };
        match field {
            "data" => {
                self.data_buffer.push_str(value);
                self.data_buffer.push('\n');
            }
            "event" => self.event_type = Some(value.to_string()),
            // Per spec, an id containing NUL is ignored entirely (left as
            // whatever it was before); not a real-world concern for
            // TokenFuse Cloud, but cheap to honor via a match guard.
            "id" if !value.contains('\0') => {
                self.last_id = Some(value.to_string());
            }
            // `retry:` and any unrecognized field name are ignored: no
            // reconnection-hint plumbing in this spike (fixed backoff
            // policy instead, see `cloud_sse.rs`).
            _ => {}
        }
        None
    }

    /// Blank line reached: emit the accumulated event if it has data, per
    /// spec (an empty data buffer means no dispatch at all). Always resets
    /// the per-event buffers either way.
    fn dispatch(&mut self) -> Option<SseEvent> {
        let event_type = self.event_type.take();
        if self.data_buffer.is_empty() {
            return None;
        }
        // Every appended data line added a trailing '\n'; the spec's
        // algorithm drops just the final one before dispatch.
        let mut data = std::mem::take(&mut self.data_buffer);
        if data.ends_with('\n') {
            data.pop();
        }
        Some(SseEvent {
            id: self.last_id.clone(),
            event: event_type,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_split_across_two_chunks_reassembles() {
        let mut d = SseDecoder::new();
        assert!(
            d.feed(b"data: {\"a\":1").is_empty(),
            "no complete event yet: the closing brace and terminator haven't arrived"
        );
        let events = d.feed(b"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, r#"{"a":1}"#);
    }

    #[test]
    fn crlf_split_exactly_at_the_boundary_does_not_spawn_a_spurious_event() {
        // The chunk boundary lands between the \r and \n of the blank-line
        // terminator. If a lone trailing \r were (wrongly) treated as its
        // own terminator immediately, this would flush two events (one
        // real, one spurious-empty) instead of one.
        let mut d = SseDecoder::new();
        assert!(d.feed(b"data: x\r").is_empty());
        let events = d.feed(b"\n\r\n");
        assert_eq!(
            events.len(),
            1,
            "exactly one event, no phantom blank-line event"
        );
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn multibyte_utf8_character_split_across_chunks_decodes_correctly() {
        // "caf" + the two UTF-8 bytes of 'e' with an acute accent (U+00E9),
        // split so the chunk boundary lands between those two bytes: proof
        // that chunk-split resilience holds at the byte level, not just at
        // line boundaries.
        let mut d = SseDecoder::new();
        let mut chunk1 = b"data: caf".to_vec();
        chunk1.push(0xC3);
        assert!(d.feed(&chunk1).is_empty());
        let events = d.feed(&[0xA9, b'\n', b'\n']);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "caf\u{e9}");
    }

    #[test]
    fn two_frames_in_one_chunk_both_emit() {
        let mut d = SseDecoder::new();
        let events = d.feed(b"data: one\n\ndata: two\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "one");
        assert_eq!(events[1].data, "two");
    }

    #[test]
    fn comments_and_keepalives_are_ignored() {
        let mut d = SseDecoder::new();
        let events = d.feed(b": keepalive\ndata: x\n: another comment\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn blank_line_terminator_flushes_only_once_data_is_present() {
        let mut d = SseDecoder::new();
        assert!(d.feed(b"data: partial").is_empty(), "no newline at all yet");
        assert!(
            d.feed(b"\n").is_empty(),
            "line completed, but no blank-line terminator yet"
        );
        let events = d.feed(b"\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "partial");
    }

    #[test]
    fn multi_line_data_is_joined_with_newlines() {
        let mut d = SseDecoder::new();
        let events = d.feed(b"data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn event_id_is_sticky_across_events_until_overwritten() {
        let mut d = SseDecoder::new();
        let events = d.feed(b"id: 42\ndata: first\n\ndata: second\n\nid: 43\ndata: third\n\n");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].id.as_deref(), Some("42"));
        assert_eq!(
            events[1].id.as_deref(),
            Some("42"),
            "id persists without its own id: line"
        );
        assert_eq!(events[2].id.as_deref(), Some("43"));
        assert_eq!(d.last_event_id(), Some("43"));
    }

    #[test]
    fn event_type_field_is_captured_and_does_not_persist() {
        let mut d = SseDecoder::new();
        let events = d.feed(b"event: alert\ndata: x\n\ndata: y\n\n");
        assert_eq!(events[0].event.as_deref(), Some("alert"));
        assert_eq!(
            events[1].event, None,
            "event type does not persist like id does"
        );
    }

    #[test]
    fn dispatch_with_no_data_emits_nothing() {
        let mut d = SseDecoder::new();
        let events = d.feed(b"\n\n\n");
        assert!(events.is_empty());
    }

    #[test]
    fn byte_at_a_time_feed_still_reassembles_correctly() {
        // The extreme case of chunk-splitting: one byte per `feed` call.
        let mut d = SseDecoder::new();
        let mut events = Vec::new();
        for byte in b"data: {\"n\":7}\n\n" {
            events.extend(d.feed(&[*byte]));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, r#"{"n":7}"#);
    }
}
