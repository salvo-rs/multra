use std::convert::Infallible;

use bytes::Bytes;
use futures_util::stream;
use multra::{Error, Multipart, parse_boundary};

#[test]
fn boundary_parameters_accept_http_ows_empty_segments_and_quoted_pairs() {
    for value in [
        "multipart/form-data;\tboundary=B",
        "multipart/form-data\t; boundary=B",
        "multipart/form-data;; boundary=B;\t;",
        " \tMULTIPART/FORM-DATA \t; BOUNDARY=\"B\" \t",
    ] {
        assert_eq!(parse_boundary(value), Ok("B".to_owned()), "{value:?}");
    }
    assert_eq!(
        parse_boundary(r#"multipart/form-data; note="a\"b; c"; boundary="a\?b""#),
        Ok("a?b".to_owned())
    );
    assert_eq!(
        parse_boundary("multipart/form-data; ;\t;"),
        Err(Error::NoBoundary)
    );
    assert!(matches!(
        parse_boundary("multipart/form-data; boundary=\"a\tb\""),
        Err(Error::InvalidBoundary { .. })
    ));
}

#[test]
fn malformed_content_type_parameters_are_rejected() {
    for value in [
        "multipart/form-data boundary=B",
        "multipart/form-data; boundary =B",
        "multipart/form-data; boundary= B",
        "multipart/form-data; boundary=",
        "multipart/form-data; boundary=\"B",
        "multipart/form-data; boundary=\"B\"junk",
        "multipart/form-data; boundary=\"B\\",
        "multipart/form-data; boundary=\"B\r\n\"",
        "multipart/form-data; boundary=B,other",
    ] {
        assert!(parse_boundary(value).is_err(), "{value:?}");
    }
}

fn data(content_type: &str) -> Vec<u8> {
    data_bytes(content_type.as_bytes())
}

fn data_bytes(content_type: &[u8]) -> Vec<u8> {
    let mut body = b"--B\r\nContent-Disposition: form-data; name=f\r\nContent-Type: ".to_vec();
    body.extend_from_slice(content_type);
    body.extend_from_slice(b"\r\n\r\n");
    body.extend_from_slice(b"\xe9\r\n--B--\r\n");
    body
}

#[tokio::test]
async fn part_content_type_uses_the_same_semantic_parameter_parser() {
    let body = data("text/plain\t;\t; charset=\"windows\\-1252\"; note=\"a; b\";\t;");
    for width in [1, 2, body.len()] {
        let chunks = body
            .chunks(width)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)));
        let mut parser = Multipart::new(stream::iter(chunks), "B");
        let field = parser.next_field().await.unwrap().unwrap();
        let mime = field.content_type().unwrap();
        assert_eq!(mime.type_(), "text");
        assert_eq!(mime.subtype(), "plain");
        assert_eq!(mime.get_param(mime::CHARSET).unwrap(), "windows-1252");
        assert_eq!(mime.get_param("note").unwrap(), "a; b");
        assert_eq!(field.text().await.unwrap(), "\u{e9}");
        assert!(parser.next_field().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn unrepresentable_mime_parameters_preserve_raw_headers_and_charset() {
    for content_type in [
        r#"text/plain; charset=windows-1252; note="a\"b""#,
        r#"text/plain; charset=windows-1252; note="a\\b""#,
        r#"text/plain; charset=windows-1252; note="""#,
        "text/plain; charset=windows-1252; note=\"a\tb; c\"",
    ] {
        let body = data(content_type);
        let mut parser =
            Multipart::new(stream::iter([Ok::<_, Infallible>(Bytes::from(body))]), "B");
        let field = parser.next_field().await.unwrap().unwrap();
        assert!(field.content_type().is_none(), "{content_type:?}");
        assert_eq!(
            field.headers()["content-type"].as_bytes(),
            content_type.as_bytes()
        );
        assert_eq!(field.text().await.unwrap(), "\u{e9}");
    }
}

#[tokio::test]
async fn utf8_quoted_parameters_do_not_hide_the_part_charset() {
    let content_type = "text/plain; note=\"\u{e9}\"; charset=windows-1252";
    let mut parser = Multipart::new(
        stream::iter([Ok::<_, Infallible>(Bytes::from(data(content_type)))]),
        "B",
    );
    let field = parser.next_field().await.unwrap().unwrap();
    assert_eq!(
        field.content_type().unwrap().get_param("note").unwrap(),
        "\u{e9}"
    );
    assert_eq!(
        field.headers()["content-type"].as_bytes(),
        content_type.as_bytes()
    );
    assert_eq!(field.text().await.unwrap(), "\u{e9}");
}

#[tokio::test]
async fn non_utf8_quoted_parameters_do_not_hide_the_part_charset() {
    let content_type = b"text/plain; note=\"\xe9\"; charset=windows-1252";
    let mut parser = Multipart::new(
        stream::iter([Ok::<_, Infallible>(Bytes::from(data_bytes(content_type)))]),
        "B",
    );
    let field = parser.next_field().await.unwrap().unwrap();
    assert!(field.content_type().is_none());
    assert_eq!(field.headers()["content-type"].as_bytes(), content_type);
    assert_eq!(field.text().await.unwrap(), "\u{e9}");
}
