use std::fmt::{self, Display, Formatter};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A set of errors that can occur during parsing multipart stream and in other
/// operations.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// An unknown field is detected when multipart
    /// [`constraints`](crate::Constraints::allowed_fields) are added.
    UnknownField { field_name: Option<String> },

    /// The field data is found incomplete.
    IncompleteFieldData { field_name: Option<String> },

    /// Couldn't read the field headers completely.
    IncompleteHeaders,

    /// Failed to read headers.
    ReadHeaderFailed(httparse::Error),

    /// Failed to decode the field's raw header name to
    /// [`HeaderName`](http::header::HeaderName) type.
    DecodeHeaderName { name: String, cause: BoxError },

    /// Failed to decode the field's raw header value to
    /// [`HeaderValue`](http::header::HeaderValue) type.
    DecodeHeaderValue { value: Vec<u8>, cause: BoxError },

    /// Multipart stream is incomplete.
    IncompleteStream,

    /// The incoming field size exceeded the maximum limit.
    FieldSizeExceeded {
        limit: u64,
        field_name: Option<String>,
    },

    /// The incoming stream size exceeded the maximum limit.
    StreamSizeExceeded { limit: u64 },

    /// The data before the first multipart boundary exceeded the maximum
    /// limit.
    PreambleSizeExceeded { limit: u64 },

    /// The incoming field headers exceeded the maximum limit.
    HeadersSizeExceeded { limit: u64 },

    /// Stream read failed.
    StreamReadFailed(BoxError),

    /// Failed to lock the multipart shared state for any changes.
    LockFailure,

    /// The `Content-Type` header is not `multipart/form-data`.
    NoMultipart,

    /// Failed to convert the `Content-Type` to [`mime::Mime`] type.
    DecodeContentType(mime::FromStrError),

    /// No boundary found in `Content-Type` header.
    NoBoundary,

    /// The boundary parameter is outside the allowed multipart boundary syntax.
    InvalidBoundary { boundary: String },

    /// Failed to decode the field data as `JSON` in
    /// [`field.json()`](crate::Field::json) method.
    #[cfg(feature = "json")]
    DecodeJson(serde_json::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownField { field_name } => {
                let name = field_name.as_deref().unwrap_or("<unknown>");
                write!(f, "unknown field received: {name:?}")
            }
            Self::IncompleteFieldData { field_name } => {
                let name = field_name.as_deref().unwrap_or("<unknown>");
                write!(f, "field {name:?} received with incomplete data")
            }
            Self::DecodeHeaderName { name, .. } => {
                write!(f, "failed to decode field's raw header name: {name:?}")
            }
            Self::DecodeHeaderValue { .. } => {
                write!(f, "failed to decode field's raw header value")
            }
            Self::FieldSizeExceeded { limit, field_name } => {
                let name = field_name.as_deref().unwrap_or("<unknown>");
                write!(f, "field {name:?} exceeded the size limit: {limit} bytes")
            }
            Self::StreamSizeExceeded { limit } => {
                write!(f, "stream size exceeded limit: {limit} bytes")
            }
            Self::PreambleSizeExceeded { limit } => {
                write!(f, "multipart preamble exceeded limit: {limit} bytes")
            }
            Self::HeadersSizeExceeded { limit } => {
                write!(f, "field headers exceeded limit: {limit} bytes")
            }
            Self::ReadHeaderFailed(_) => write!(f, "failed to read headers"),
            Self::StreamReadFailed(_) => write!(f, "failed to read stream"),
            Self::DecodeContentType(_) => write!(f, "failed to decode Content-Type"),
            Self::IncompleteHeaders => write!(f, "failed to read field complete headers"),
            Self::IncompleteStream => write!(f, "incomplete multipart stream"),
            Self::LockFailure => write!(f, "failed to lock multipart state"),
            Self::NoMultipart => write!(f, "Content-Type is not multipart/form-data"),
            Self::NoBoundary => write!(f, "multipart boundary not found in Content-Type"),
            Self::InvalidBoundary { boundary } => {
                write!(f, "invalid multipart boundary: {boundary:?}")
            }
            #[cfg(feature = "json")]
            Self::DecodeJson(_) => write!(f, "failed to decode field data as JSON"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReadHeaderFailed(e) => Some(e),
            Self::DecodeHeaderName { cause, .. } => Some(cause.as_ref()),
            Self::DecodeHeaderValue { cause, .. } => Some(cause.as_ref()),
            Self::StreamReadFailed(e) => Some(e.as_ref()),
            Self::DecodeContentType(e) => Some(e),
            #[cfg(feature = "json")]
            Self::DecodeJson(e) => Some(e),
            Self::UnknownField { .. }
            | Self::IncompleteFieldData { .. }
            | Self::IncompleteHeaders
            | Self::IncompleteStream
            | Self::FieldSizeExceeded { .. }
            | Self::StreamSizeExceeded { .. }
            | Self::PreambleSizeExceeded { .. }
            | Self::HeadersSizeExceeded { .. }
            | Self::LockFailure
            | Self::NoMultipart
            | Self::NoBoundary
            | Self::InvalidBoundary { .. } => None,
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnknownField { field_name: left }, Self::UnknownField { field_name: right })
            | (
                Self::IncompleteFieldData { field_name: left },
                Self::IncompleteFieldData { field_name: right },
            ) => left == right,
            (Self::ReadHeaderFailed(left), Self::ReadHeaderFailed(right)) => left == right,
            (
                Self::DecodeHeaderName {
                    name: left_name,
                    cause: left_cause,
                },
                Self::DecodeHeaderName {
                    name: right_name,
                    cause: right_cause,
                },
            ) => left_name == right_name && left_cause.to_string() == right_cause.to_string(),
            (
                Self::DecodeHeaderValue {
                    value: left_value,
                    cause: left_cause,
                },
                Self::DecodeHeaderValue {
                    value: right_value,
                    cause: right_cause,
                },
            ) => left_value == right_value && left_cause.to_string() == right_cause.to_string(),
            (
                Self::FieldSizeExceeded {
                    limit: left_limit,
                    field_name: left_name,
                },
                Self::FieldSizeExceeded {
                    limit: right_limit,
                    field_name: right_name,
                },
            ) => left_limit == right_limit && left_name == right_name,
            (
                Self::StreamSizeExceeded { limit: left },
                Self::StreamSizeExceeded { limit: right },
            )
            | (
                Self::PreambleSizeExceeded { limit: left },
                Self::PreambleSizeExceeded { limit: right },
            )
            | (
                Self::HeadersSizeExceeded { limit: left },
                Self::HeadersSizeExceeded { limit: right },
            ) => left == right,
            (Self::StreamReadFailed(left), Self::StreamReadFailed(right)) => {
                left.to_string() == right.to_string()
            }
            (Self::DecodeContentType(left), Self::DecodeContentType(right)) => {
                left.to_string() == right.to_string()
            }
            (
                Self::InvalidBoundary { boundary: left },
                Self::InvalidBoundary { boundary: right },
            ) => left == right,
            #[cfg(feature = "json")]
            (Self::DecodeJson(left), Self::DecodeJson(right)) => {
                left.to_string() == right.to_string()
            }
            (Self::IncompleteHeaders, Self::IncompleteHeaders)
            | (Self::IncompleteStream, Self::IncompleteStream)
            | (Self::LockFailure, Self::LockFailure)
            | (Self::NoMultipart, Self::NoMultipart)
            | (Self::NoBoundary, Self::NoBoundary) => true,
            _ => false,
        }
    }
}

impl Eq for Error {}

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn equality_uses_variant_data_instead_of_display_text() {
        assert_ne!(
            Error::UnknownField { field_name: None },
            Error::UnknownField {
                field_name: Some("<unknown>".to_owned())
            }
        );
        assert_ne!(
            Error::ReadHeaderFailed(httparse::Error::HeaderName),
            Error::ReadHeaderFailed(httparse::Error::HeaderValue)
        );
    }

    #[test]
    fn debug_output_preserves_variant_details() {
        let error = Error::ReadHeaderFailed(httparse::Error::HeaderName);
        assert_eq!(
            format!("{error:?}"),
            "ReadHeaderFailed(HeaderName)".to_owned()
        );
    }
}
