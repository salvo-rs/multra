use http::header::{self, HeaderMap};

use crate::constants::ContentDispositionAttr;

#[derive(Debug)]
pub struct ContentDisposition {
    pub(crate) field_name: Option<String>,
    pub(crate) file_name: Option<String>,
}

impl ContentDisposition {
    pub fn parse(headers: &HeaderMap) -> Self {
        let content_disposition = headers
            .get_all(header::CONTENT_DISPOSITION)
            .iter()
            .next_back()
            .map(http::HeaderValue::as_bytes);

        let field_name = content_disposition
            .and_then(|val| ContentDispositionAttr::Name.extract_from(val))
            .map(std::borrow::Cow::into_owned);

        let file_name = content_disposition
            .and_then(|val| ContentDispositionAttr::FileName.extract_from(val))
            .map(std::borrow::Cow::into_owned);

        Self {
            field_name,
            file_name,
        }
    }
}
