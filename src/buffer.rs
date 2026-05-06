use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes, BytesMut};
use futures_util::stream::Stream;

use crate::constants;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundaryMatch {
    Valid,
    Partial,
    Invalid,
}

fn partial_boundary_suffix_len(buf: &[u8], boundary: &[u8]) -> usize {
    let max_len = buf.len().min(boundary.len().saturating_sub(1));

    (1..=max_len)
        .rev()
        .find(|len| buf[buf.len() - len..] == boundary[..*len])
        .unwrap_or(0)
}

fn match_padding_and_crlf(bytes: &[u8], eof: bool, allow_eof: bool) -> BoundaryMatch {
    let mut idx = 0;
    while matches!(bytes.get(idx), Some(b' ' | b'\t')) {
        idx += 1;
    }

    let bytes = &bytes[idx..];
    if bytes.is_empty() {
        return if eof && allow_eof {
            BoundaryMatch::Valid
        } else if eof {
            BoundaryMatch::Invalid
        } else {
            BoundaryMatch::Partial
        };
    }

    match bytes {
        [b'\r', b'\n', ..] => BoundaryMatch::Valid,
        [b'\r'] if !eof => BoundaryMatch::Partial,
        _ => BoundaryMatch::Invalid,
    }
}

fn match_boundary_suffix(buf: &[u8], suffix_start: usize, eof: bool) -> BoundaryMatch {
    let suffix = &buf[suffix_start..];
    match suffix {
        [] if !eof => BoundaryMatch::Partial,
        [] => BoundaryMatch::Invalid,
        [b'-'] if !eof => BoundaryMatch::Partial,
        [b'-', b'-', rest @ ..] => match_padding_and_crlf(rest, eof, true),
        [b'-', ..] => BoundaryMatch::Invalid,
        _ => match_padding_and_crlf(suffix, eof, false),
    }
}

pub(crate) struct StreamBuffer<'r> {
    pub(crate) eof: bool,
    pub(crate) buf: BytesMut,
    pub(crate) stream: Pin<Box<dyn Stream<Item = Result<Bytes, crate::Error>> + Send + 'r>>,
    pub(crate) whole_stream_size_limit: u64,
    pub(crate) stream_size_counter: u64,
}

impl<'r> StreamBuffer<'r> {
    pub fn new<S>(stream: S, whole_stream_size_limit: u64) -> Self
    where
        S: Stream<Item = Result<Bytes, crate::Error>> + Send + 'r,
    {
        StreamBuffer {
            eof: false,
            buf: BytesMut::new(),
            stream: Box::pin(stream),
            whole_stream_size_limit,
            stream_size_counter: 0,
        }
    }

    pub fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), crate::Error> {
        if self.eof {
            return Ok(());
        }

        loop {
            match self.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    self.stream_size_counter += data.len() as u64;

                    if self.stream_size_counter > self.whole_stream_size_limit {
                        return Err(crate::Error::StreamSizeExceeded {
                            limit: self.whole_stream_size_limit,
                        });
                    }

                    if data.is_empty() {
                        continue;
                    }

                    self.buf.extend_from_slice(&data);
                    cx.waker().wake_by_ref();
                    return Ok(());
                }
                Poll::Ready(Some(Err(err))) => return Err(err),
                Poll::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Pending => return Ok(()),
            }
        }
    }

    pub fn read_exact(&mut self, size: usize) -> Option<Bytes> {
        if size <= self.buf.len() {
            Some(self.buf.split_to(size).freeze())
        } else {
            None
        }
    }

    pub fn peek_exact(&mut self, size: usize) -> Option<&[u8]> {
        self.buf.get(..size)
    }

    pub fn read_until(&mut self, pattern: &[u8]) -> Option<Bytes> {
        memchr::memmem::find(&self.buf, pattern)
            .map(|idx| self.buf.split_to(idx + pattern.len()).freeze())
    }

    pub fn read_to(&mut self, pattern: &[u8]) -> Option<Bytes> {
        memchr::memmem::find(&self.buf, pattern).map(|idx| self.buf.split_to(idx).freeze())
    }

    pub fn advance_past_transport_padding(&mut self) -> bool {
        match self.buf.iter().position(|b| *b != b' ' && *b != b'\t') {
            Some(pos) => {
                self.buf.advance(pos);
                true
            }
            None => {
                self.buf.clear();
                false
            }
        }
    }

    pub fn read_field_data(
        &mut self,
        boundary: &[u8],
        field_name: Option<&str>,
    ) -> crate::Result<Option<(bool, Bytes)>> {
        trace!("finding next field: {:?}", field_name);
        if self.buf.is_empty() && self.eof {
            trace!("empty buffer && EOF");
            return Err(crate::Error::IncompleteFieldData {
                field_name: field_name.map(|s| s.to_owned()),
            });
        } else if self.buf.is_empty() {
            return Ok(None);
        }

        let b_len = boundary.len();

        if let Some(idx) = memchr::memmem::find(&self.buf, boundary) {
            match match_boundary_suffix(&self.buf, idx + b_len, self.eof) {
                BoundaryMatch::Valid => {
                    trace!("new field found at {}", idx);
                    let bytes = self.buf.split_to(idx).freeze();

                    // discard \r\n.
                    self.buf.advance(constants::CRLF.len());

                    return Ok(Some((true, bytes)));
                }
                BoundaryMatch::Partial => {
                    if idx == 0 {
                        return Ok(None);
                    }

                    return Ok(Some((false, self.buf.split_to(idx).freeze())));
                }
                BoundaryMatch::Invalid => {
                    return Err(crate::Error::IncompleteStream);
                }
            }
        }

        if self.eof {
            trace!("no new field found: EOF. terminating");
            return Err(crate::Error::IncompleteFieldData {
                field_name: field_name.map(|s| s.to_owned()),
            });
        }

        let preserve_len = partial_boundary_suffix_len(&self.buf, boundary);
        let readable_len = self.buf.len().saturating_sub(preserve_len);

        if readable_len == 0 {
            Ok(None)
        } else {
            Ok(Some((false, self.buf.split_to(readable_len).freeze())))
        }
    }
}

impl fmt::Debug for StreamBuffer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamBuffer").finish()
    }
}
