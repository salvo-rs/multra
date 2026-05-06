//! An async parser for `multipart/form-data` content-type in Rust.
//!
//! It accepts a [`Stream`](futures_util::stream::Stream) of
//! [`Bytes`](bytes::Bytes), or with the `tokio-io` feature enabled, an
//! `AsyncRead` reader as a source, so that it can be plugged into any async
//! Rust environment e.g. any async server.
//!
//! To enable trace logging via the `log` crate, enable the `log` feature.
//!
//! # Examples
//!
//! ```no_run
//! use std::convert::Infallible;
//!
//! use bytes::Bytes;
//! // Import multra types.
//! use futures_util::stream::once;
//! use futures_util::stream::Stream;
//! use multra::{Constraints, Multipart, SizeLimit};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Generate a byte stream and the boundary from somewhere e.g. server request body.
//!     let (stream, boundary) = get_byte_stream_from_somewhere().await;
//!
//!     let constraints = Constraints::new().size_limit(
//!         SizeLimit::new()
//!             .whole_stream(15 * 1024 * 1024)
//!             .per_field(10 * 1024 * 1024)
//!             .for_field("my_text_field", 30 * 1024),
//!     );
//!
//!     // Create a constrained `Multipart` instance for untrusted input.
//!     let mut multipart = Multipart::with_constraints(stream, boundary, constraints);
//!
//!     // Iterate over the fields, use `next_field()` to get the next field.
//!     while let Some(mut field) = multipart.next_field().await? {
//!         // Get field name.
//!         let name = field.name();
//!         // Get the field's filename if provided in "Content-Disposition" header.
//!         let file_name = field.file_name();
//!
//!         println!("Name: {:?}, File Name: {:?}", name, file_name);
//!
//!         // Process the field data chunks e.g. store them in a file.
//!         while let Some(chunk) = field.chunk().await? {
//!             // Do something with field chunk.
//!             println!("Chunk: {:?}", chunk);
//!         }
//!     }
//!
//!     Ok(())
//! }
//!
//! // Generate a byte stream and the boundary from somewhere e.g. server request body.
//! async fn get_byte_stream_from_somewhere()
//! -> (impl Stream<Item = Result<Bytes, Infallible>>, &'static str) {
//!     let data = "--X-BOUNDARY\r\nContent-Disposition: form-data; \
//!         name=\"my_text_field\"\r\n\r\nabcd\r\n--X-BOUNDARY--\r\n";
//!
//!     let stream = once(async move { Result::<Bytes, Infallible>::Ok(Bytes::from(data)) });
//!     (stream, "X-BOUNDARY")
//! }
//! ```
//!
//! ## Prevent Denial of Service (DoS) Attack
//!
//! This crate provides APIs to prevent potential DoS attacks with fine grained
//! control. The default constructors leave stream and field size limits
//! unbounded, so it is recommended to add explicit constraints for untrusted
//! multipart bodies.
//!
//! An example:
//!
//! ```
//! use multra::{Constraints, Multipart, SizeLimit};
//! # use bytes::Bytes;
//! # use std::convert::Infallible;
//! # use futures_util::stream::once;
//!
//! # async fn run() {
//! # let data = "--X-BOUNDARY\r\nContent-Disposition: form-data; \
//! #   name=\"my_text_field\"\r\n\r\nabcd\r\n--X-BOUNDARY--\r\n";
//! # let some_stream = once(async move { Result::<Bytes, Infallible>::Ok(Bytes::from(data)) });
//! // Create some constraints to be applied to the fields to prevent DoS attack.
//! let constraints = Constraints::new()
//!     // We only accept `my_text_field` and `my_file_field` fields,
//!     // For any unknown field, we will throw an error.
//!     .allowed_fields(vec!["my_text_field", "my_file_field"])
//!     .size_limit(
//!         SizeLimit::new()
//!             // Set 15mb as size limit for the whole stream body.
//!             .whole_stream(15 * 1024 * 1024)
//!             // Set 10mb as size limit for all fields.
//!             .per_field(10 * 1024 * 1024)
//!             // Set 30kb as size limit for our text field only.
//!             .for_field("my_text_field", 30 * 1024),
//!     );
//!
//! // Create a `Multipart` instance from a stream and the constraints.
//! let mut multipart = Multipart::with_constraints(some_stream, "X-BOUNDARY", constraints);
//!
//! while let Some(field) = multipart.next_field().await.unwrap() {
//!     let content = field.text().await.unwrap();
//!     assert_eq!(content, "abcd");
//! }
//! # }
//! # tokio::runtime::Runtime::new().unwrap().block_on(run());
//! ```
//!
//! Please refer [`Constraints`] for more info.
//!
//! ## Usage with [hyper.rs](https://hyper.rs/) server
//!
//! An [example](https://github.com/salvo-rs/multra/blob/main/examples/hyper_server_example.rs) showing usage with [hyper.rs](https://hyper.rs/).
//!
//! For more examples, please visit [examples](https://github.com/salvo-rs/multra/tree/main/examples).

#![forbid(unsafe_code)]
#![warn(
    missing_debug_implementations,
    rust_2018_idioms,
    trivial_casts,
    unused_qualifications
)]
#![doc(test(attr(deny(rust_2018_idioms, warnings))))]
#![doc(test(attr(allow(unused_extern_crates, unused_variables))))]

pub use bytes;
pub use constraints::Constraints;
pub use error::Error;
pub use field::Field;
pub use multipart::Multipart;
pub use size_limit::SizeLimit;

#[cfg(feature = "log")]
macro_rules! trace {
    ($($t:tt)*) => (::log::trace!($($t)*););
}

#[cfg(not(feature = "log"))]
macro_rules! trace {
    ($($t:tt)*) => {};
}

mod buffer;
mod constants;
mod constraints;
mod content_disposition;
mod error;
mod field;
mod helpers;
mod multipart;
mod size_limit;

/// A Result type often returned from methods that can have `multra` errors.
pub type Result<T, E = Error> = std::result::Result<T, E>;

fn is_boundary_char_no_space(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'\'' | b'(' | b')' | b'+' | b'_' | b',' | b'-' | b'.' | b'/' | b':' | b'=' | b'?'
        )
}

fn is_boundary_char(byte: u8) -> bool {
    byte == b' ' || is_boundary_char_no_space(byte)
}

fn validate_boundary(boundary: &str) -> Result<()> {
    let bytes = boundary.as_bytes();

    if bytes.is_empty()
        || bytes.len() > 70
        || !bytes[..bytes.len() - 1]
            .iter()
            .copied()
            .all(is_boundary_char)
        || !is_boundary_char_no_space(bytes[bytes.len() - 1])
    {
        return Err(Error::InvalidBoundary {
            boundary: boundary.to_owned(),
        });
    }

    Ok(())
}

/// Parses the `Content-Type` header to extract the boundary value.
///
/// # Examples
///
/// ```
/// # fn run(){
/// let content_type = "multipart/form-data; boundary=ABCDEFG";
///
/// assert_eq!(
///     multra::parse_boundary(content_type),
///     Ok("ABCDEFG".to_owned())
/// );
/// # }
/// # run();
/// ```
pub fn parse_boundary<T: AsRef<str>>(content_type: T) -> Result<String> {
    let m = content_type
        .as_ref()
        .parse::<mime::Mime>()
        .map_err(Error::DecodeContentType)?;

    if !(m.type_() == mime::MULTIPART && m.subtype() == mime::FORM_DATA) {
        return Err(Error::NoMultipart);
    }

    let boundary = m.get_param(mime::BOUNDARY).ok_or(Error::NoBoundary)?;
    validate_boundary(boundary.as_str())?;

    Ok(boundary.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_boundary() {
        let content_type = "multipart/form-data; boundary=ABCDEFG";
        assert_eq!(parse_boundary(content_type), Ok("ABCDEFG".to_owned()));

        let content_type = "multipart/form-data; boundary=------ABCDEFG";
        assert_eq!(parse_boundary(content_type), Ok("------ABCDEFG".to_owned()));

        let content_type = "boundary=------ABCDEFG";
        assert!(parse_boundary(content_type).is_err());

        let content_type = "text/plain";
        assert!(parse_boundary(content_type).is_err());

        let content_type = "text/plain; boundary=------ABCDEFG";
        assert!(parse_boundary(content_type).is_err());

        let boundary = "a".repeat(70);
        let content_type = format!("multipart/form-data; boundary={boundary}");
        assert_eq!(parse_boundary(content_type), Ok(boundary));

        let boundary = "a".repeat(71);
        let content_type = format!("multipart/form-data; boundary={boundary}");
        assert!(matches!(
            parse_boundary(content_type),
            Err(Error::InvalidBoundary { .. })
        ));

        let content_type = "multipart/form-data; boundary=\"abc@def\"";
        assert!(matches!(
            parse_boundary(content_type),
            Err(Error::InvalidBoundary { .. })
        ));
    }
}
