//! Shared SSE parsing utilities for the cloud streaming adapters (T061,
//! dev/06 §4).
//!
//! The OpenAI and Anthropic adapters both speak `text/event-stream`, but with
//! different event vocabularies: OpenAI emits bare `data:` lines terminated by
//! `data: [DONE]`, while Anthropic pairs `event: <type>` with `data: <json>`.
//! This module owns everything they share:
//!
//! * [`SseParser`] — incremental, allocation-light reassembly of SSE events
//!   from arbitrary byte-chunk boundaries (`reqwest::Response::bytes_stream`
//!   chunks can split a line — or even the `data:` prefix — anywhere).
//! * [`delta_stream`] — adapts a raw response-body byte stream plus a
//!   per-event handler into the [`ChatDeltaStream`] contract via
//!   `futures::stream::unfold` (the proven construction from the Ollama
//!   adapter). Cancellation needs no token: dropping the returned stream drops
//!   the `reqwest` body, which closes the HTTP connection and discards the
//!   partial (dev/06 §4).
//!
//! Log safety (dev/09 §5): no SSE payload text ever enters a
//! [`ProviderError`] produced here — errors carry fixed technical tags only.

use std::collections::VecDeque;

use futures::stream::{self, BoxStream};
use futures::StreamExt;

use super::provider::{ChatDeltaStream, ProviderError};
use super::types::ChatDelta;

/// Hard cap on a single SSE line (card §6). A well-formed provider delta is a
/// few hundred bytes; anything beyond this is a misbehaving endpoint and the
/// stream fails fast instead of buffering unboundedly.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// One reassembled SSE event: the optional `event:` type and the `data:`
/// payload (multiple `data:` lines are joined with `\n` per the SSE spec).
/// Events without any `data:` line are never dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// What the adapter's event handler decided about one [`SseEvent`].
pub enum SseAction {
    /// Emit this text as the next [`ChatDelta`] (ordinal assigned here).
    Delta(String),
    /// Nothing to emit (role-only chunk, ping, ignorable event type).
    Skip,
    /// Terminate the stream gracefully (`[DONE]`, `message_stop`).
    End,
    /// Push this error and terminate. Implementors must keep payload text out
    /// of the error value (dev/09 §5).
    Fail(ProviderError),
}

/// Incremental SSE event parser.
///
/// Feed raw body chunks with [`SseParser::push_chunk`]; each call returns the
/// events *completed* by that chunk. Call [`SseParser::finish`] at end of body
/// to flush an event whose terminating blank line never arrived (providers
/// commonly end the body right after the last `data:` line).
#[derive(Default)]
pub struct SseParser {
    /// Bytes of the line currently being assembled (no `\n` seen yet).
    line_buf: Vec<u8>,
    /// `event:` field of the event currently being assembled.
    event_type: Option<String>,
    /// `data:` lines of the event currently being assembled.
    data_lines: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw body chunk; get back every event completed by it.
    ///
    /// Errors with a fixed-tag [`ProviderError::BadResponse`] when a single
    /// line exceeds [`MAX_LINE_BYTES`]; the parser is then poisoned and the
    /// caller must stop feeding it.
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, ProviderError> {
        self.line_buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(pos) = self.line_buf.iter().position(|&b| b == b'\n') {
            if pos > MAX_LINE_BYTES {
                return Err(line_too_long());
            }
            let mut line: Vec<u8> = self.line_buf.drain(..=pos).collect();
            line.pop(); // trailing \n
            if line.last() == Some(&b'\r') {
                line.pop(); // CRLF tolerance
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(event) = self.consume_line(&line) {
                events.push(event);
            }
        }
        // A partial line still waiting for its newline must also respect the cap.
        if self.line_buf.len() > MAX_LINE_BYTES {
            return Err(line_too_long());
        }
        Ok(events)
    }

    /// Flush at end of body: process a final unterminated line, then dispatch
    /// the in-flight event if it accumulated any data.
    pub fn finish(&mut self) -> Result<Vec<SseEvent>, ProviderError> {
        let mut events = Vec::new();
        if !self.line_buf.is_empty() {
            if self.line_buf.len() > MAX_LINE_BYTES {
                return Err(line_too_long());
            }
            let line = String::from_utf8_lossy(&self.line_buf).into_owned();
            self.line_buf.clear();
            if let Some(event) = self.consume_line(&line) {
                events.push(event);
            }
        }
        if let Some(event) = self.dispatch() {
            events.push(event);
        }
        Ok(events)
    }

    /// Process one complete line; returns an event when the line was the
    /// blank-line event terminator.
    fn consume_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None; // SSE comment (keep-alive)
        }
        if let Some(value) = field_value(line, "event") {
            self.event_type = Some(value.to_string());
        } else if let Some(value) = field_value(line, "data") {
            self.data_lines.push(value.to_string());
        }
        // Other fields (`id:`, `retry:`) and malformed lines are ignored —
        // the SSE contract treats unknown fields as no-ops.
        None
    }

    /// Close out the in-flight event. Per the SSE spec an event with no data
    /// lines is not dispatched (its type is still reset).
    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event_type.take();
        if self.data_lines.is_empty() {
            return None;
        }
        let data = self.data_lines.join("\n");
        self.data_lines.clear();
        Some(SseEvent { event, data })
    }
}

/// Extract the value of `<field>: <value>` (one optional leading space after
/// the colon is stripped, per the SSE spec).
fn field_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// Fixed-tag oversize error — never carries any buffered bytes (dev/09 §5).
fn line_too_long() -> ProviderError {
    ProviderError::BadResponse("sse line exceeds the 64 KiB limit".into())
}

// ── ChatDelta stream construction ────────────────────────────────────────────

/// Raw body chunks as the adapters hand them over
/// (`resp.bytes_stream().map(|c| c.map(|b| b.to_vec())).boxed()`).
pub type ByteChunkStream = BoxStream<'static, Result<Vec<u8>, reqwest::Error>>;

struct StreamState<F> {
    body: ByteChunkStream,
    parser: SseParser,
    on_event: F,
    pending: VecDeque<ChatDelta>,
    error: Option<ProviderError>,
    next_index: usize,
    finished: bool,
}

impl<F: FnMut(&SseEvent) -> SseAction> StreamState<F> {
    fn consume_events(&mut self, events: Vec<SseEvent>) {
        for event in events {
            if self.finished || self.error.is_some() {
                return;
            }
            match (self.on_event)(&event) {
                SseAction::Delta(text) => {
                    self.pending.push_back(ChatDelta {
                        text,
                        index: self.next_index,
                    });
                    self.next_index += 1;
                }
                SseAction::Skip => {}
                SseAction::End => self.finished = true,
                SseAction::Fail(e) => self.error = Some(e),
            }
        }
    }
}

/// Adapt a raw SSE response body into a [`ChatDeltaStream`].
///
/// `on_event` is the provider-specific vocabulary (OpenAI `[DONE]`/delta
/// chunks, Anthropic typed events); everything transport- and framing-level is
/// handled here. Delta ordinals are assigned 0-based in arrival order.
///
/// Semantics (dev/06 §4, card §3):
/// * a [`SseAction::Fail`] or transport/framing error is yielded as one final
///   `Err` item, then the stream terminates;
/// * after [`SseAction::End`] the stream yields `None` (later body bytes are
///   never read);
/// * dropping the returned stream drops `body` — for a `reqwest` byte stream
///   that closes the HTTP connection, which *is* the cancellation path.
pub fn delta_stream<F>(
    body: ByteChunkStream,
    map_transport: fn(reqwest::Error) -> ProviderError,
    on_event: F,
) -> ChatDeltaStream
where
    F: FnMut(&SseEvent) -> SseAction + Send + 'static,
{
    let state = StreamState {
        body,
        parser: SseParser::new(),
        on_event,
        pending: VecDeque::new(),
        error: None,
        next_index: 0,
        finished: false,
    };

    stream::unfold(state, move |mut st| async move {
        loop {
            // Drain parsed deltas first, then a deferred error, then end.
            if let Some(delta) = st.pending.pop_front() {
                return Some((Ok(delta), st));
            }
            if let Some(err) = st.error.take() {
                st.finished = true;
                return Some((Err(err), st));
            }
            if st.finished {
                return None;
            }
            match st.body.next().await {
                Some(Ok(chunk)) => match st.parser.push_chunk(&chunk) {
                    Ok(events) => st.consume_events(events),
                    Err(e) => st.error = Some(e),
                },
                Some(Err(e)) => st.error = Some(map_transport(e)),
                None => {
                    // End of body without an explicit terminator: flush any
                    // unterminated tail, then finish gracefully.
                    match st.parser.finish() {
                        Ok(events) => st.consume_events(events),
                        Err(e) => st.error = Some(e),
                    }
                    st.finished = true;
                }
            }
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_all(parser: &mut SseParser, chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(parser.push_chunk(chunk).unwrap());
        }
        events.extend(parser.finish().unwrap());
        events
    }

    // ── SseParser: framing ──────────────────────────────────────────────────

    #[test]
    fn parses_openai_style_data_events() {
        let mut parser = SseParser::new();
        let events = push_all(
            &mut parser,
            &[b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n\n"],
        );
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            SseEvent {
                event: None,
                data: "{\"a\":1}".into()
            }
        );
        assert_eq!(
            events[1],
            SseEvent {
                event: None,
                data: "{\"b\":2}".into()
            }
        );
        assert_eq!(
            events[2],
            SseEvent {
                event: None,
                data: "[DONE]".into()
            }
        );
    }

    #[test]
    fn parses_anthropic_style_typed_events() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\"}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n",
            "\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n",
            "\n",
        );
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &[body.as_bytes()]);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event.as_deref(), Some("message_start"));
        assert_eq!(events[1].event.as_deref(), Some("content_block_delta"));
        assert_eq!(
            events[1].data,
            "{\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}"
        );
        assert_eq!(events[2].event.as_deref(), Some("message_stop"));
    }

    /// Chunks split mid-line, mid-`data:` prefix, and with CRLF endings must
    /// reassemble into the same event sequence.
    #[test]
    fn reassembles_chunks_split_at_awkward_boundaries() {
        let chunks: [&[u8]; 5] = [
            b"da",
            b"ta: {\"text\":\"Hel\"}\r\n\r\nev",
            b"ent: content_block_delta\nda",
            b"ta: {\"text\":\"lo\"}\n",
            b"\n",
        ];
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &chunks);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            SseEvent {
                event: None,
                data: "{\"text\":\"Hel\"}".into()
            }
        );
        assert_eq!(
            events[1],
            SseEvent {
                event: Some("content_block_delta".into()),
                data: "{\"text\":\"lo\"}".into(),
            }
        );
    }

    #[test]
    fn finish_flushes_unterminated_tail_line() {
        let mut parser = SseParser::new();
        assert!(parser.push_chunk(b"data: [DONE]").unwrap().is_empty());
        let events = parser.finish().unwrap();
        assert_eq!(
            events,
            vec![SseEvent {
                event: None,
                data: "[DONE]".into()
            }]
        );
        // A second finish has nothing left to release.
        assert!(parser.finish().unwrap().is_empty());
    }

    #[test]
    fn joins_multiple_data_lines_with_newline() {
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &[b"data: first\ndata: second\n\n"]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "first\nsecond");
    }

    #[test]
    fn ignores_comments_unknown_fields_and_dataless_events() {
        let body = concat!(
            ": keep-alive comment\n",
            "id: 42\n",
            "retry: 3000\n",
            "event: ping\n",
            "\n",
            "data: payload\n",
            "\n",
        );
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &[body.as_bytes()]);
        // The dataless `ping` event is never dispatched; only the data event is.
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            SseEvent {
                event: None,
                data: "payload".into()
            }
        );
    }

    #[test]
    fn data_without_space_after_colon_is_accepted() {
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &[b"data:{\"x\":1}\n\n"]);
        assert_eq!(events[0].data, "{\"x\":1}");
    }

    #[test]
    fn event_type_resets_between_events() {
        let body = b"event: content_block_delta\ndata: a\n\ndata: b\n\n";
        let mut parser = SseParser::new();
        let events = push_all(&mut parser, &[body.as_slice()]);
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(events[1].event, None);
    }

    // ── SseParser: oversize guard (card §6) ─────────────────────────────────

    #[test]
    fn oversize_line_fails_fast_without_echoing_bytes() {
        let mut parser = SseParser::new();
        // 'q' never appears in the fixed oversize message, so any hit proves
        // buffered payload bytes leaked into the error rendering.
        let huge = vec![b'q'; MAX_LINE_BYTES + 1];
        let err = parser.push_chunk(&huge).unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        assert!(!format!("{err}").contains('q'));
    }

    #[test]
    fn oversize_line_completed_in_one_chunk_is_rejected() {
        let mut parser = SseParser::new();
        let mut huge = vec![b'y'; MAX_LINE_BYTES + 1];
        huge.push(b'\n');
        assert!(parser.push_chunk(&huge).is_err());
    }

    // ── delta_stream: end-to-end on byte chunks ─────────────────────────────

    fn body_of(chunks: Vec<&'static [u8]>) -> ByteChunkStream {
        stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<_, reqwest::Error>(c.to_vec())),
        )
        .boxed()
    }

    fn fixed_transport_error(_e: reqwest::Error) -> ProviderError {
        ProviderError::Unreachable("transport error".into())
    }

    /// Pass-through handler: any data event becomes a delta, `[DONE]` ends.
    fn passthrough(event: &SseEvent) -> SseAction {
        if event.data == "[DONE]" {
            SseAction::End
        } else {
            SseAction::Delta(event.data.clone())
        }
    }

    #[tokio::test]
    async fn delta_stream_assigns_zero_based_ordinals() {
        let body = body_of(vec![
            b"data: tok1\n\nda",
            b"ta: tok2\n\ndata: tok3\n\ndata: [DONE]\n\n",
        ]);
        let stream = delta_stream(body, fixed_transport_error, passthrough);
        let collected: Vec<_> = stream.collect().await;
        let deltas: Vec<ChatDelta> = collected.into_iter().map(|d| d.unwrap()).collect();
        assert_eq!(
            deltas,
            vec![
                ChatDelta {
                    text: "tok1".into(),
                    index: 0
                },
                ChatDelta {
                    text: "tok2".into(),
                    index: 1
                },
                ChatDelta {
                    text: "tok3".into(),
                    index: 2
                },
            ]
        );
    }

    #[tokio::test]
    async fn delta_stream_ignores_events_after_end() {
        let body = body_of(vec![b"data: tok1\n\ndata: [DONE]\n\ndata: late\n\n"]);
        let stream = delta_stream(body, fixed_transport_error, passthrough);
        let collected: Vec<_> = stream.collect().await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].as_ref().unwrap().text, "tok1");
    }

    #[tokio::test]
    async fn delta_stream_yields_handler_error_then_terminates() {
        let body = body_of(vec![b"data: ok\n\ndata: poison\n\ndata: never\n\n"]);
        let handler = |event: &SseEvent| {
            if event.data == "poison" {
                SseAction::Fail(ProviderError::BadResponse(
                    "unparseable sse delta chunk".into(),
                ))
            } else {
                SseAction::Delta(event.data.clone())
            }
        };
        let mut stream = delta_stream(body, fixed_transport_error, handler);
        assert_eq!(stream.next().await.unwrap().unwrap().text, "ok");
        let err = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        assert!(!format!("{err}").contains("poison"));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn delta_stream_finishes_gracefully_without_terminator() {
        // Body ends mid-event (no [DONE], no trailing blank line).
        let body = body_of(vec![b"data: only\n"]);
        let stream = delta_stream(body, fixed_transport_error, passthrough);
        let collected: Vec<_> = stream.collect().await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].as_ref().unwrap().text, "only");
    }

    #[tokio::test]
    async fn delta_stream_oversize_line_surfaces_as_error() {
        let huge = Box::leak(vec![b'z'; MAX_LINE_BYTES + 2].into_boxed_slice());
        let body = body_of(vec![b"data: fine\n\n", huge]);
        let mut stream = delta_stream(body, fixed_transport_error, passthrough);
        assert_eq!(stream.next().await.unwrap().unwrap().text, "fine");
        assert!(stream.next().await.unwrap().is_err());
        assert!(stream.next().await.is_none());
    }
}
