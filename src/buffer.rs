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

fn match_padding_and_crlf(
    bytes: &[u8],
    eof: bool,
    allow_eof: bool,
    scanned: &mut usize,
    limit: u64,
) -> crate::Result<BoundaryMatch> {
    if *scanned as u64 > limit {
        return Err(crate::Error::BoundaryPaddingSizeExceeded { limit });
    }
    // A partial delimiter stays in the buffer, but its padding must only be
    // scanned once even when the source supplies one byte per poll.
    while matches!(bytes.get(*scanned), Some(b' ' | b'\t')) {
        *scanned += 1;
        if *scanned as u64 > limit {
            return Err(crate::Error::BoundaryPaddingSizeExceeded { limit });
        }
    }

    let bytes = &bytes[*scanned..];
    if bytes.is_empty() {
        return Ok(if eof && allow_eof {
            BoundaryMatch::Valid
        } else if eof {
            BoundaryMatch::Invalid
        } else {
            BoundaryMatch::Partial
        });
    }

    Ok(match bytes {
        [b'\r', b'\n', ..] => BoundaryMatch::Valid,
        [b'\r'] if !eof => BoundaryMatch::Partial,
        _ => BoundaryMatch::Invalid,
    })
}

fn match_boundary_suffix(
    buf: &[u8],
    suffix_start: usize,
    eof: bool,
    scanned: &mut usize,
    limit: u64,
) -> crate::Result<BoundaryMatch> {
    let suffix = &buf[suffix_start..];
    match suffix {
        [] | [b'-'] if !eof => Ok(BoundaryMatch::Partial),
        [b'-', b'-', rest @ ..] => match_padding_and_crlf(rest, eof, true, scanned, limit),
        [] | [b'-', ..] => Ok(BoundaryMatch::Invalid),
        _ => match_padding_and_crlf(suffix, eof, false, scanned, limit),
    }
}

pub struct StreamBuffer<'r> {
    pub(crate) eof: bool,
    pub(crate) buf: BytesMut,
    pub(crate) stream: Pin<Box<dyn Stream<Item = Result<Bytes, crate::Error>> + Send + 'r>>,
    pub(crate) whole_stream_size_limit: u64,
    pub(crate) stream_size_counter: u64,
    boundary_padding_limit: u64,
    boundary_padding_scanned: usize,
    transport_padding_counter: u64,
    first_boundary_search_start: usize,
    header_search_start: usize,
    #[cfg(test)]
    headers_scanned_bytes: usize,
}

impl<'r> StreamBuffer<'r> {
    pub fn new<S>(stream: S, whole_stream_size_limit: u64, boundary_padding_limit: u64) -> Self
    where
        S: Stream<Item = Result<Bytes, crate::Error>> + Send + 'r,
    {
        StreamBuffer {
            eof: false,
            buf: BytesMut::new(),
            stream: Box::pin(stream),
            whole_stream_size_limit,
            stream_size_counter: 0,
            boundary_padding_limit,
            boundary_padding_scanned: 0,
            transport_padding_counter: 0,
            first_boundary_search_start: 0,
            header_search_start: 0,
            #[cfg(test)]
            headers_scanned_bytes: 0,
        }
    }

    pub fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), crate::Error> {
        if self.eof {
            return Ok(());
        }

        loop {
            match self.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    // Saturating add prevents an attacker from overflowing
                    // the counter back to zero to bypass the size check.
                    self.stream_size_counter =
                        self.stream_size_counter.saturating_add(data.len() as u64);

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
            self.header_search_start = 0;
            Some(self.buf.split_to(size).freeze())
        } else {
            None
        }
    }

    pub fn peek_exact(&self, size: usize) -> Option<&[u8]> {
        self.buf.get(..size)
    }

    pub fn read_headers(&mut self) -> Option<Bytes> {
        let pattern = constants::CRLF_CRLF.as_bytes();
        let haystack = &self.buf[self.header_search_start..];
        #[cfg(test)]
        {
            self.headers_scanned_bytes += haystack.len();
        }

        if let Some(idx) = memchr::memmem::find(haystack, pattern) {
            let end = self.header_search_start + idx + pattern.len();
            self.header_search_start = 0;
            Some(self.buf.split_to(end).freeze())
        } else {
            // A delimiter can straddle input chunks. Already searched bytes
            // need not be scanned again, except for this three-byte overlap.
            self.header_search_start = self.buf.len().saturating_sub(pattern.len() - 1);
            None
        }
    }

    pub fn find_first_boundary(&mut self, pattern: &[u8]) -> Option<usize> {
        let mut start = self.first_boundary_search_start;
        while let Some(relative) = memchr::memmem::find(&self.buf[start..], pattern) {
            let index = start + relative;
            if index == 0 || (index >= 2 && &self.buf[index - 2..index] == b"\r\n") {
                return Some(index);
            }
            start = index + 1;
        }

        // Retain enough overlap for a marker split across source chunks. The
        // preamble remains buffered, so preceding CRLF bytes are still visible.
        self.first_boundary_search_start = self.buf.len().saturating_sub(pattern.len() - 1);
        None
    }

    pub fn partial_pattern_suffix_len(&self, pattern: &[u8]) -> usize {
        partial_boundary_suffix_len(&self.buf, pattern)
    }

    pub fn advance(&mut self, size: usize) {
        self.header_search_start = 0;
        self.buf.advance(size);
    }

    pub fn advance_past_transport_padding(&mut self) -> crate::Result<bool> {
        let mut padding_len = 0;
        for byte in &self.buf {
            if *byte != b' ' && *byte != b'\t' {
                break;
            }
            padding_len += 1;
            self.transport_padding_counter = self.transport_padding_counter.saturating_add(1);
            if self.transport_padding_counter > self.boundary_padding_limit {
                return Err(crate::Error::BoundaryPaddingSizeExceeded {
                    limit: self.boundary_padding_limit,
                });
            }
        }
        self.advance(padding_len);
        let done = !self.buf.is_empty();
        if done {
            self.transport_padding_counter = 0;
        }
        Ok(done)
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
                field_name: field_name.map(ToOwned::to_owned),
            });
        } else if self.buf.is_empty() {
            return Ok(None);
        }

        let b_len = boundary.len();

        if let Some(idx) = memchr::memmem::find(&self.buf, boundary) {
            match match_boundary_suffix(
                &self.buf,
                idx + b_len,
                self.eof,
                &mut self.boundary_padding_scanned,
                self.boundary_padding_limit,
            )? {
                BoundaryMatch::Valid => {
                    self.boundary_padding_scanned = 0;
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
                field_name: field_name.map(ToOwned::to_owned),
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

#[cfg(test)]
mod tests {
    use super::StreamBuffer;

    fn buffer() -> StreamBuffer<'static> {
        StreamBuffer::new(
            futures_util::stream::empty(),
            u64::MAX,
            crate::constants::DEFAULT_BOUNDARY_PADDING_SIZE_LIMIT,
        )
    }

    #[test]
    fn headers_are_found_at_every_input_split() {
        let headers = b"X-Test: value\r\n\r\n";
        for split in 0..=headers.len() {
            let mut buffer = buffer();
            buffer.buf.extend_from_slice(&headers[..split]);
            let first = buffer.read_headers();
            if split == headers.len() {
                assert_eq!(first.unwrap().as_ref(), headers);
            } else {
                assert!(first.is_none());
                buffer.buf.extend_from_slice(&headers[split..]);
                buffer.buf.extend_from_slice(b"body");
                assert_eq!(buffer.read_headers().unwrap().as_ref(), headers);
                assert_eq!(buffer.buf.as_ref(), b"body");
            }
        }
    }

    #[test]
    fn byte_fragmented_header_scanning_is_linear() {
        let headers = format!("X-Long: {}\r\n\r\n", "a".repeat(16 * 1024));
        let mut buffer = buffer();
        for (index, byte) in headers.bytes().enumerate() {
            buffer.buf.extend_from_slice(&[byte]);
            let result = buffer.read_headers();
            if index + 1 == headers.len() {
                assert_eq!(result.unwrap().as_ref(), headers.as_bytes());
            } else {
                assert!(result.is_none());
            }
        }
        assert!(buffer.headers_scanned_bytes <= 4 * headers.len());
    }

    #[test]
    fn header_search_resets_after_each_header_block() {
        let mut buffer = buffer();
        for headers in [
            "X-Long: several bytes before the delimiter\r\n\r\n",
            "X: y\r\n\r\n",
        ] {
            for byte in headers.bytes() {
                buffer.buf.extend_from_slice(&[byte]);
                if buffer.buf.len() == headers.len() {
                    assert_eq!(buffer.read_headers().unwrap().as_ref(), headers.as_bytes());
                } else {
                    assert!(buffer.read_headers().is_none());
                }
            }
        }
    }

    #[test]
    fn header_search_resets_when_buffered_bytes_are_consumed() {
        for consumption in 0..3 {
            let mut buffer = buffer();
            buffer.buf.extend_from_slice(b"          ");
            assert!(buffer.read_headers().is_none());
            match consumption {
                0 => assert_eq!(buffer.read_exact(10).unwrap().len(), 10),
                1 => buffer.advance(10),
                _ => assert!(!buffer.advance_past_transport_padding().unwrap()),
            }
            buffer.buf.extend_from_slice(b"X: y\r\n\r\n");
            assert_eq!(buffer.read_headers().unwrap().as_ref(), b"X: y\r\n\r\n");
        }
    }
}
