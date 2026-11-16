use std::borrow::Cow;

use crate::{Error, Result};

/// The semantic parameters are independent of `mime`'s restricted wire parser.
#[derive(Debug)]
pub struct ContentType<'a> {
    pub(crate) essence: mime::Mime,
    parameters: Vec<(&'a str, Cow<'a, [u8]>)>,
}

impl<'a> ContentType<'a> {
    pub(crate) fn parse(value: &'a str) -> Result<Self> {
        Self::parse_bytes(value.as_bytes())
    }

    pub(crate) fn parse_bytes(value: &'a [u8]) -> Result<Self> {
        let invalid = || Error::InvalidContentType {
            content_type: String::from_utf8_lossy(value).into_owned(),
        };
        let mut parser = Parser { value, position: 0 };
        parser.skip_ows();
        let start = parser.position;
        parser.token().ok_or_else(invalid)?;
        if !parser.consume(b'/') {
            return Err(invalid());
        }
        parser.token().ok_or_else(invalid)?;
        let essence = std::str::from_utf8(&value[start..parser.position])
            .map_err(|_| invalid())?
            .parse()
            .map_err(Error::DecodeContentType)?;
        let mut parameters = Vec::new();

        loop {
            parser.skip_ows();
            if parser.position == value.len() {
                break;
            }
            if !parser.consume(b';') {
                return Err(invalid());
            }
            parser.skip_ows();
            // HTTP allows empty parameters between semicolons and at the end.
            if matches!(value.get(parser.position), None | Some(b';')) {
                continue;
            }
            let name = parser.token().ok_or_else(invalid)?;
            if !parser.consume(b'=') {
                return Err(invalid());
            }
            let parameter = if parser.consume(b'"') {
                parser.quoted_value().ok_or_else(invalid)?
            } else {
                Cow::Borrowed(parser.token().ok_or_else(invalid)?.as_bytes())
            };
            parameters.push((name, parameter));
        }

        Ok(Self {
            essence,
            parameters,
        })
    }

    pub(crate) fn get_param(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
    }

    /// Return a Mime only if it can preserve every semantic parameter.
    pub(crate) fn to_mime(&self) -> Option<mime::Mime> {
        if self.parameters.is_empty() {
            return Some(self.essence.clone());
        }
        let mut normalized = self.essence.to_string();
        for (name, value) in &self.parameters {
            let value = std::str::from_utf8(value).ok()?;
            normalized.push_str("; ");
            normalized.push_str(name);
            normalized.push('=');
            if !value.is_empty() && value.bytes().all(is_token) {
                normalized.push_str(value);
            } else {
                normalized.push('"');
                for character in value.chars() {
                    if matches!(character, '"' | '\\') {
                        normalized.push('\\');
                    }
                    normalized.push(character);
                }
                normalized.push('"');
            }
        }
        let mime = normalized.parse::<mime::Mime>().ok()?;
        if mime.params().count() != self.parameters.len()
            || !mime
                .params()
                .zip(&self.parameters)
                .all(|((name, value), (key, expected))| {
                    name.as_str().eq_ignore_ascii_case(key)
                        && if key.eq_ignore_ascii_case("charset") {
                            value.as_str().as_bytes().eq_ignore_ascii_case(expected)
                        } else {
                            value.as_str().as_bytes() == expected.as_ref()
                        }
                })
        {
            return None;
        }
        Some(mime)
    }
}

struct Parser<'a> {
    value: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn consume(&mut self, byte: u8) -> bool {
        if self.value.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn skip_ows(&mut self) {
        while matches!(self.value.get(self.position), Some(b' ' | b'\t')) {
            self.position += 1;
        }
    }

    fn token(&mut self) -> Option<&'a str> {
        let start = self.position;
        while self
            .value
            .get(self.position)
            .is_some_and(|byte| is_token(*byte))
        {
            self.position += 1;
        }
        if start == self.position {
            None
        } else {
            std::str::from_utf8(&self.value[start..self.position]).ok()
        }
    }

    fn quoted_value(&mut self) -> Option<Cow<'a, [u8]>> {
        let start = self.position;
        let mut segment = start;
        let mut decoded: Option<Vec<u8>> = None;
        let bytes = self.value;
        while let Some(&byte) = bytes.get(self.position) {
            match byte {
                b'"' => {
                    let value = if let Some(mut decoded) = decoded {
                        decoded.extend_from_slice(&bytes[segment..self.position]);
                        Cow::Owned(decoded)
                    } else {
                        Cow::Borrowed(&self.value[start..self.position])
                    };
                    self.position += 1;
                    return Some(value);
                }
                b'\\' => {
                    let decoded = decoded.get_or_insert_with(Vec::new);
                    decoded.extend_from_slice(&bytes[segment..self.position]);
                    self.position += 1;
                    let &escaped = bytes.get(self.position)?;
                    if !matches!(escaped, b'\t' | b' '..=b'~' | 0x80..=0xff) {
                        return None;
                    }
                    decoded.push(escaped);
                    self.position += 1;
                    segment = self.position;
                }
                b'\t' | b' ' | b'!' | b'#'..=b'[' | b']'..=b'~' | 0x80..=0xff => {
                    self.position += 1;
                }
                _ => return None,
            }
        }
        None
    }
}

const fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

#[cfg(test)]
mod tests {
    use super::ContentType;

    #[test]
    fn quoted_parameters_keep_semicolons_and_unescape_each_quoted_pair() {
        let parsed = ContentType::parse(r#"text/plain; note="a\"b\\c;"; x="a\?b""#).unwrap();
        assert_eq!(parsed.get_param("note"), Some("a\"b\\c;"));
        assert_eq!(parsed.get_param("x"), Some("a?b"));
        // No lossy subset of the parameters may masquerade as a complete Mime.
        assert!(parsed.to_mime().is_none());
    }

    #[test]
    fn quoted_whitespace_and_obstext_are_value_bytes() {
        let parsed = ContentType::parse("text/plain; note=\" \t\u{e9} \"").unwrap();
        assert_eq!(parsed.get_param("note"), Some(" \t\u{e9} "));
        assert!(parsed.to_mime().is_none());
        for value in ["text/plain; x=\"\0\"", "text/plain; x=\"\u{7f}\""] {
            assert!(ContentType::parse(value).is_err());
        }
    }

    #[test]
    fn duplicate_parameters_keep_first_value_selection_and_all_values() {
        let parsed = ContentType::parse("text/plain; charset=UTF-8; charset=windows-1252").unwrap();
        assert_eq!(parsed.get_param("CHARSET"), Some("UTF-8"));
        let mime = parsed.to_mime().unwrap();
        assert_eq!(mime.params().count(), 2);
        assert_eq!(mime.get_param(mime::CHARSET).unwrap(), "utf-8");
    }

    #[test]
    fn quoted_obstext_keeps_raw_octets_including_quoted_pairs() {
        for byte in 0x80..=0xff {
            for escaped in [false, true] {
                let mut header = b"text/plain; note=\"".to_vec();
                if escaped {
                    header.push(b'\\');
                }
                header.push(byte);
                header.extend_from_slice(b"\"; charset=windows-1252");
                let parsed = ContentType::parse_bytes(&header).unwrap();
                assert_eq!(parsed.parameters[0].1.as_ref(), [byte]);
                assert_eq!(parsed.get_param("charset"), Some("windows-1252"));
                assert!(parsed.to_mime().is_none());
            }
        }
    }
}
