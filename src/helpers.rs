use std::convert::TryFrom;

use http::header::{self, HeaderMap, HeaderName, HeaderValue};
use httparse::Header;

pub fn convert_raw_headers_to_header_map(raw_headers: &[Header<'_>]) -> crate::Result<HeaderMap> {
    let mut headers = HeaderMap::with_capacity(raw_headers.len());

    for raw_header in raw_headers {
        let name = HeaderName::try_from(raw_header.name).map_err(|err| {
            crate::Error::DecodeHeaderName {
                name: raw_header.name.to_owned(),
                cause: err.into(),
            }
        })?;

        let value = HeaderValue::try_from(raw_header.value).map_err(|err| {
            crate::Error::DecodeHeaderValue {
                value: raw_header.value.to_owned(),
                cause: err.into(),
            }
        })?;

        headers.append(name, value);
    }

    Ok(headers)
}

pub fn parse_content_type(headers: &HeaderMap) -> Option<crate::content_type::ContentType<'_>> {
    headers
        .get_all(header::CONTENT_TYPE)
        .iter()
        .next_back()
        .and_then(|val| crate::content_type::ContentType::parse_bytes(val.as_bytes()).ok())
}
