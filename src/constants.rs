use std::borrow::Cow;

use encoding_rs::Encoding;

pub(crate) const DEFAULT_WHOLE_STREAM_SIZE_LIMIT: u64 = u64::MAX;
pub(crate) const DEFAULT_PER_FIELD_SIZE_LIMIT: u64 = u64::MAX;

pub(crate) const MAX_HEADERS: usize = 32;
pub(crate) const BOUNDARY_EXT: &str = "--";
pub(crate) const CR: &str = "\r";
#[allow(dead_code)]
pub(crate) const LF: &str = "\n";
pub(crate) const CRLF: &str = "\r\n";
pub(crate) const CRLF_CRLF: &str = "\r\n\r\n";

#[derive(PartialEq)]
pub(crate) enum ContentDispositionAttr {
    Name,
    FileName,
}

fn trim_ascii_ws_start(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or_else(|| &bytes[bytes.len()..], |i| &bytes[i..])
}

fn trim_ascii_ws_then(bytes: &[u8], char: u8) -> Option<&[u8]> {
    match trim_ascii_ws_start(bytes) {
        [first, rest @ ..] if *first == char => Some(rest),
        _ => None,
    }
}

fn trim_ascii_ws_end(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(&bytes[..0], |i| &bytes[..=i])
}

fn skip_to_next_parameter(header: &[u8], index: &mut usize) {
    while *index < header.len() && header[*index] != b';' {
        *index += 1;
    }
    if *index < header.len() {
        *index += 1;
    }
}

fn skip_ascii_ws(header: &[u8], index: &mut usize) {
    while *index < header.len() && header[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn parse_quoted_value(mut header: &[u8]) -> Option<(&[u8], bool)> {
    header = trim_ascii_ws_then(header, b'"')?;
    let start = 0;
    let (mut index, mut escaped) = (start, false);

    while index < header.len() {
        if header[index] == b'"' {
            let mut backslashes = 0;
            let mut cursor = index;
            while cursor > start && header[cursor - 1] == b'\\' {
                backslashes += 1;
                cursor -= 1;
            }

            if backslashes % 2 == 0 {
                return Some((&header[..index], escaped));
            }

            escaped = true;
        }

        index += 1;
    }

    None
}

fn parse_unquoted_value(header: &[u8]) -> &[u8] {
    let value = trim_ascii_ws_start(header);
    trim_ascii_ws_end(&value[..memchr::memchr(b';', value).unwrap_or(value.len())])
}

fn decode_percent_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.contains(&b'%') {
        return Some(bytes.to_vec());
    }

    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = bytes.get(index + 1)?;
            let lo = bytes.get(index + 2)?;
            let hex = [*hi, *lo];
            decoded.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    Some(decoded)
}

fn decode_value<'h>(bytes: &'h [u8], is_escaped: bool) -> Option<Cow<'h, str>> {
    if bytes.contains(&b'%') {
        return Some(String::from_utf8(decode_percent_bytes(bytes)?).ok()?.into());
    }

    let value = std::str::from_utf8(bytes).ok()?;
    if is_escaped {
        Some(value.replace(r#"\""#, "\"").into())
    } else {
        Some(value.into())
    }
}

fn decode_extended_value(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?;
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next()?;
    let _language = parts.next()?;
    let encoded = parts.next()?;

    let encoding = Encoding::for_label(charset.as_bytes())?;
    let decoded = decode_percent_bytes(encoded.as_bytes())?;
    let (text, _, had_errors) = encoding.decode(&decoded);
    if had_errors {
        return None;
    }

    Some(text.into_owned())
}

impl ContentDispositionAttr {
    /// Extract ContentDisposition Attribute from header.
    ///
    /// Some older clients may not quote the name or filename, so we allow them.
    /// If they percent-encode the value, we decode it before returning.
    pub fn extract_from<'h>(&self, header: &'h [u8]) -> Option<Cow<'h, str>> {
        if self == &ContentDispositionAttr::FileName
            && let Some(value) = self.extract_extended_from(header)
        {
            return Some(value);
        }

        let prefix = match self {
            ContentDispositionAttr::Name => &b"name"[..],
            ContentDispositionAttr::FileName => &b"filename"[..],
        };
        let mut index = 0;

        while index < header.len() {
            skip_to_next_parameter(header, &mut index);
            skip_ascii_ws(header, &mut index);
            if index >= header.len() {
                break;
            }

            let key_start = index;
            while index < header.len()
                && !header[index].is_ascii_whitespace()
                && header[index] != b'='
                && header[index] != b';'
            {
                index += 1;
            }

            let key = &header[key_start..index];
            skip_ascii_ws(header, &mut index);
            if index >= header.len() || header[index] != b'=' {
                continue;
            }

            index += 1;
            let rest = &header[index..];
            let (bytes, is_escaped) = if let Some((value, escaped)) = parse_quoted_value(rest) {
                (value, escaped)
            } else {
                (parse_unquoted_value(rest), false)
            };

            if key.eq_ignore_ascii_case(prefix) {
                return decode_value(bytes, is_escaped);
            }
        }

        None
    }

    fn extract_extended_from<'h>(&self, header: &'h [u8]) -> Option<Cow<'h, str>> {
        let prefix = match self {
            ContentDispositionAttr::Name => return None,
            ContentDispositionAttr::FileName => &b"filename*"[..],
        };
        let mut index = 0;

        while index < header.len() {
            skip_to_next_parameter(header, &mut index);
            skip_ascii_ws(header, &mut index);
            if index >= header.len() {
                break;
            }

            let key_start = index;
            while index < header.len()
                && !header[index].is_ascii_whitespace()
                && header[index] != b'='
                && header[index] != b';'
            {
                index += 1;
            }

            let key = &header[key_start..index];
            skip_ascii_ws(header, &mut index);
            if index >= header.len() || header[index] != b'=' {
                continue;
            }

            index += 1;
            if key.eq_ignore_ascii_case(prefix) {
                let value = parse_unquoted_value(&header[index..]);
                return Some(decode_extended_value(value)?.into());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_disposition_name_only() {
        let val = br#"form-data; name="my_field""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert!(filename.is_none());

        let val = br#"form-data; name=my_field  "#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert!(filename.is_none());

        let val = br#"form-data; name  =  my_field  "#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert!(filename.is_none());

        let val = br#"form-data; name  =  "#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "");
        assert!(filename.is_none());
    }

    #[test]
    fn test_content_disposition_extraction() {
        let val = br#"form-data; name="my_field"; filename="file abc.txt""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert_eq!(filename.unwrap(), "file abc.txt");

        let val = "form-data; name=\"你好\"; filename=\"file abc.txt\"".as_bytes();
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "你好");
        assert_eq!(filename.unwrap(), "file abc.txt");

        let val = "form-data; name=\"কখগ\"; filename=\"你好.txt\"".as_bytes();
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "কখগ");
        assert_eq!(filename.unwrap(), "你好.txt");
    }

    #[test]
    fn test_content_disposition_file_name_only() {
        // These are technically malformed, as RFC 7578 says the `name`
        // parameter _must_ be included. But okay.
        let val = br#"form-data; filename="file-name.txt""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(filename.unwrap(), "file-name.txt");
        assert!(name.is_none());

        let val = "form-data; filename=\"কখগ-你好.txt\"".as_bytes();
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(filename.unwrap(), "কখগ-你好.txt");
        assert!(name.is_none());
    }

    #[test]
    fn test_content_distribution_misordered_fields() {
        let val = br#"form-data; filename=file-name.txt; name=file"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(filename.unwrap(), "file-name.txt");
        assert_eq!(name.unwrap(), "file");

        let val = br#"form-data; filename="file-name.txt"; name="file""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(filename.unwrap(), "file-name.txt");
        assert_eq!(name.unwrap(), "file");

        let val = "form-data; filename=\"你好.txt\"; name=\"কখগ\"".as_bytes();
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "কখগ");
        assert_eq!(filename.unwrap(), "你好.txt");
    }

    #[test]
    fn test_content_disposition_name_unquoted() {
        let val = br#"form-data; name=my_field"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert!(filename.is_none());

        let val = br#"form-data; name=my_field; filename=file-name.txt"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert_eq!(filename.unwrap(), "file-name.txt");
    }

    #[test]
    fn test_content_disposition_name_quoted() {
        let val = br#"form-data; name="my;f;ield""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my;f;ield");
        assert!(filename.is_none());

        let val = br#"form-data; name=my_field; filename = "file;name.txt""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(filename.unwrap(), "file;name.txt");

        let val = br#"form-data; name=; filename=filename.txt"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "");
        assert_eq!(filename.unwrap(), "filename.txt");

        let val = br#"form-data; name=";"; filename=";""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), ";");
        assert_eq!(filename.unwrap(), ";");
    }

    #[test]
    fn test_content_disposition_name_escaped_quote() {
        let val = br#"form-data; name="my\"field\"name""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        assert_eq!(name.unwrap(), r#"my"field"name"#);

        let val = br#"form-data; name="myfield\"name""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        assert_eq!(name.unwrap(), r#"myfield"name"#);
    }

    #[test]
    fn test_content_disposition_case_insensitive_parameters() {
        let val = br#"form-data; NAME="my_field"; FILENAME="file-name.txt""#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my_field");
        assert_eq!(filename.unwrap(), "file-name.txt");
    }

    #[test]
    fn test_content_disposition_percent_decoded_values() {
        let val = br#"form-data; name=my%20field; filename=file%20name.txt"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "my field");
        assert_eq!(filename.unwrap(), "file name.txt");
    }

    #[test]
    fn test_content_disposition_filename_star_preferred() {
        let val = br#"form-data; name="upload"; filename="fallback.txt"; filename*=UTF-8''%E4%BD%A0%E5%A5%BD.txt"#;
        let name = ContentDispositionAttr::Name.extract_from(val);
        let filename = ContentDispositionAttr::FileName.extract_from(val);
        assert_eq!(name.unwrap(), "upload");
        assert_eq!(filename.unwrap(), "你好.txt");
    }
}
